//! Ctrl+Alt+Up / Down on X11. Both keys are grabbed on the root window, then
//! a thread waits inside the X connection until one is pressed. It never
//! wakes up on its own, so it costs nothing while idle.

use x11rb::connection::{Connection, RequestConnection};
use x11rb::errors::ReplyError;
use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
use x11rb::protocol::{xkb, ErrorKind, Event};
use x11rb::rust_connection::RustConnection;

/// X11 key symbols of the arrow keys.
const XK_UP: u32 = 0xff52;
const XK_DOWN: u32 = 0xff54;

/// Ctrl and Alt, as X11 modifier bits.
const MODS: u16 = 0x4 | 0x8;
/// The modifiers that count when a key comes in: Shift, Ctrl, Alt, Super.
/// Caps Lock, Num Lock and the others are ignored.
const COUNTED: u16 = 0x1 | 0x4 | 0x8 | 0x40;
/// A grab only matches the exact modifiers, and X11 counts Num Lock (0x10) and
/// Caps Lock (0x2) as modifiers while they are on. So each key is grabbed with
/// all four combinations of them.
const LOCKS: [u16; 4] = [0, 0x2, 0x10, 0x2 | 0x10];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Up,
    Down,
}

/// The first keycode that produces `sym`. `keysyms` is the keyboard map: `per`
/// symbols for each keycode, starting at `min`.
pub fn keycode_for(min: u8, per: usize, keysyms: &[u32], sym: u32) -> Option<u8> {
    if per == 0 {
        return None;
    }
    let i = keysyms.chunks(per).position(|k| k.contains(&sym))?;
    u8::try_from(min as usize + i).ok()
}

/// True if a key press with modifier state `state` is our Ctrl+Alt combination.
pub fn is_ctrl_alt(state: u16) -> bool {
    state & COUNTED == MODS
}

/// Why the hotkeys could not be set up, as the line shown in the window.
#[derive(Debug, PartialEq)]
pub enum Failure {
    /// No X server (or it refused us).
    NoX11,
    /// Another program already uses one of the keys.
    Taken,
}

fn grab(conn: &RustConnection, root: u32, code: u8) -> Result<(), Failure> {
    for lock in LOCKS {
        let mods = ModMask::from(MODS | lock);
        let cookie =
            conn.grab_key(false, root, mods, code, GrabMode::ASYNC, GrabMode::ASYNC).map_err(|_| Failure::NoX11)?;
        match cookie.check() {
            Ok(()) => {}
            Err(ReplyError::X11Error(e)) if e.error_kind == ErrorKind::Access => return Err(Failure::Taken),
            Err(_) => return Err(Failure::NoX11),
        }
    }
    Ok(())
}

/// Grab both keys and start the thread that calls `on_press` once per key
/// press (holding a key down does not repeat it).
pub fn start(on_press: impl Fn(Key) + Send + 'static) -> Result<(), Failure> {
    let (conn, screen) = RustConnection::connect(None).map_err(|_| Failure::NoX11)?;
    let setup = conn.setup();
    let root = setup.roots.get(screen).ok_or(Failure::NoX11)?.root;
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let map = conn
        .get_keyboard_mapping(min, max.saturating_sub(min).saturating_add(1))
        .map_err(|_| Failure::NoX11)?
        .reply()
        .map_err(|_| Failure::NoX11)?;
    let per = map.keysyms_per_keycode as usize;
    let up = keycode_for(min, per, &map.keysyms, XK_UP).ok_or(Failure::NoX11)?;
    let down = keycode_for(min, per, &map.keysyms, XK_DOWN).ok_or(Failure::NoX11)?;

    // Ask for no fake key releases during auto-repeat, so a held key is seen as
    // one press. Without the XKB extension a held key just repeats.
    if conn.extension_information(xkb::X11_EXTENSION_NAME).ok().flatten().is_some() {
        if let Ok(c) = xkb::ConnectionExt::xkb_use_extension(&conn, 1, 0) {
            let _ = c.reply();
        }
        let flag = xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT;
        let id = xkb::ID::USE_CORE_KBD.into();
        if let Ok(c) =
            xkb::ConnectionExt::xkb_per_client_flags(&conn, id, flag, flag, 0u32.into(), 0u32.into(), 0u32.into())
        {
            let _ = c.reply();
        }
    }

    grab(&conn, root, up)?;
    grab(&conn, root, down)?;
    let _ = conn.flush();

    std::thread::spawn(move || {
        let mut held: Option<u8> = None;
        // wait_for_event blocks until the X server sends something.
        while let Ok(event) = conn.wait_for_event() {
            match event {
                Event::KeyPress(e) if is_ctrl_alt(u16::from(e.state)) && held != Some(e.detail) => {
                    held = Some(e.detail);
                    if e.detail == up {
                        on_press(Key::Up);
                    } else if e.detail == down {
                        on_press(Key::Down);
                    }
                }
                Event::KeyRelease(e) if held == Some(e.detail) => held = None,
                _ => {}
            }
        }
        // The X server went away: nothing more to listen to.
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycode_lookup() {
        // Keycodes 8, 9, 10 with two symbols each.
        let map = [0x61, 0x41, XK_UP, 0, 0x62, XK_DOWN];
        assert_eq!(keycode_for(8, 2, &map, XK_UP), Some(9));
        assert_eq!(keycode_for(8, 2, &map, XK_DOWN), Some(10));
        assert_eq!(keycode_for(8, 2, &map, 0x1234), None);
        assert_eq!(keycode_for(8, 0, &map, XK_UP), None);
        assert_eq!(keycode_for(255, 1, &[0, XK_UP], XK_UP), None); // would pass keycode 255
    }

    #[test]
    fn only_ctrl_alt_counts() {
        assert!(is_ctrl_alt(0x4 | 0x8));
        assert!(is_ctrl_alt(0x4 | 0x8 | 0x2 | 0x10)); // Caps Lock and Num Lock on
        assert!(!is_ctrl_alt(0x4 | 0x8 | 0x1)); // with Shift
        assert!(!is_ctrl_alt(0x4 | 0x8 | 0x40)); // with Super
        assert!(!is_ctrl_alt(0x4));
        assert!(!is_ctrl_alt(0));
    }
}
