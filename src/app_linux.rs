//! Linux shell around the shared logic: StatusNotifierItem tray (ksni), X11
//! hotkeys (global-hotkey), hotplug/resume watcher (netlink) and the egui window.

use crate::{ddc, state::{self, Cmd, Core}, ui};
use eframe::egui;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

type Shared = Arc<Mutex<Core>>;

static CTX: OnceLock<egui::Context> = OnceLock::new();
static TRAY: OnceLock<Handle<BrightnessTray>> = OnceLock::new();
/// The tray icon is up. Without it the window behaves like a normal window.
static TRAY_OK: AtomicBool = AtomicBool::new(false);
/// The window is on screen.
static VISIBLE: AtomicBool = AtomicBool::new(false);
/// Set at the end of the first ui() call: the first frame has been painted.
static FIRST_FRAME: AtomicBool = AtomicBool::new(false);
static SHOW_REQ: AtomicBool = AtomicBool::new(false);
static TOGGLE_REQ: AtomicBool = AtomicBool::new(false);
/// Last startup state pushed to the tray menu.
static TRAY_STARTUP: AtomicBool = AtomicBool::new(false);
/// When the window last hid itself because it lost focus.
static FOCUS_HIDE: Mutex<Option<Instant>> = Mutex::new(None);
/// A tray click that follows a click-away hide must not reopen the window.
const REOPEN_GUARD: Duration = Duration::from_millis(400);

fn lock(core: &Shared) -> MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(|e| e.into_inner())
}

fn repaint() {
    if let Some(c) = CTX.get() {
        c.request_repaint();
    }
}

fn request_show() {
    SHOW_REQ.store(true, Ordering::Relaxed);
    repaint();
}

fn request_toggle() {
    TOGGLE_REQ.store(true, Ordering::Relaxed);
    repaint();
}

fn quit(core: &Shared) -> ! {
    // The monitor thread finishes what it is doing, then we leave.
    let tx = lock(core).sender();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    if tx.send(Cmd::Quit(done_tx)).is_ok() {
        let _ = done_rx.recv_timeout(Duration::from_secs(3));
    }
    std::process::exit(0);
}

// ---- icon: a blue disc with a lighter centre, drawn in code ----

fn icon_rgba() -> Vec<u8> {
    let n = 32usize;
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 + 0.5 - 16.0, y as f32 + 0.5 - 16.0);
            let d = (dx * dx + dy * dy).sqrt();
            let px = if d <= 13.0 {
                if d <= 6.0 { [0xe6, 0xee, 0xff, 0xff] } else { [0x4c, 0x8d, 0xff, 0xff] }
            } else {
                [0, 0, 0, 0]
            };
            rgba[(y * n + x) * 4..(y * n + x) * 4 + 4].copy_from_slice(&px);
        }
    }
    rgba
}

/// RGBA -> ARGB32 in network byte order, as the tray protocol wants it.
fn rgba_to_argb(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect()
}

// ---- tray ----

struct BrightnessTray {
    core: Shared,
    startup: bool,
}

impl ksni::Tray for BrightnessTray {
    fn id(&self) -> String {
        "brightness-lite".into()
    }

    fn title(&self) -> String {
        "Brightness Lite".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![ksni::Icon { width: 32, height: 32, data: rgba_to_argb(&icon_rgba()) }]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        request_toggle();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open".into(),
                activate: Box::new(|_| request_show()),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: ui::STARTUP_LABEL.into(),
                checked: self.startup,
                activate: Box::new(|t: &mut BrightnessTray| {
                    let want = !t.startup;
                    let mut g = lock(&t.core);
                    g.set_startup(want);
                    t.startup = g.startup;
                    TRAY_STARTUP.store(t.startup, Ordering::Relaxed);
                    drop(g);
                    repaint();
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Re-scan".into(),
                activate: Box::new(|t: &mut BrightnessTray| lock(&t.core).send(Cmd::Rescan)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut BrightnessTray| quit(&t.core)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Start the tray. False if the desktop has no StatusNotifier host (GNOME without
/// the AppIndicator extension, for example).
fn start_tray(core: &Shared) -> bool {
    let startup = lock(core).startup;
    TRAY_STARTUP.store(startup, Ordering::Relaxed);
    match (BrightnessTray { core: core.clone(), startup }).spawn() {
        Ok(handle) => {
            let _ = TRAY.set(handle);
            true
        }
        Err(_) => false,
    }
}

// ---- hotkeys: Ctrl+Alt+Up / Down, brightness only (X11 only) ----

/// Why the hotkeys cannot work here, if they cannot.
fn hotkey_blocker(session_type: Option<&str>, wayland_display: bool) -> Option<&'static str> {
    if wayland_display || session_type == Some("wayland") {
        Some("Hotkeys Ctrl+Alt+Up/Down need an X11 session. They do not work on Wayland.")
    } else {
        None
    }
}

fn start_hotkeys(core: Shared) {
    let session = std::env::var("XDG_SESSION_TYPE").ok();
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    if let Some(msg) = hotkey_blocker(session.as_deref(), wayland) {
        lock(&core).hotkey_error = msg.into();
        return;
    }
    let mods = Some(Modifiers::CONTROL | Modifiers::ALT);
    let up = HotKey::new(mods, Code::ArrowUp);
    let down = HotKey::new(mods, Code::ArrowDown);
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(_) => {
            lock(&core).hotkey_error = "Hotkeys Ctrl+Alt+Up/Down could not be set up.".into();
            return;
        }
    };
    if manager.register(up).is_err() || manager.register(down).is_err() {
        lock(&core).hotkey_error = "Hotkeys Ctrl+Alt+Up/Down are already used by another program.".into();
    }
    let (up_id, down_id) = (up.id(), down.id());
    let c = core.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        if e.state() != HotKeyState::Pressed {
            return;
        }
        if e.id() == up_id {
            lock(&c).send(Cmd::Hotkey(ddc::STEP));
        } else if e.id() == down_id {
            lock(&c).send(Cmd::Hotkey(-ddc::STEP));
        }
        repaint();
    }));
    // The manager must stay alive for as long as the app runs.
    std::mem::forget(manager);
}

// ---- monitor plug/unplug and wake-up: put the saved values back ----

/// A kernel uevent is NUL-separated "KEY=value" fields. This is true for a
/// display hotplug (a monitor was plugged in or out).
fn is_display_hotplug(msg: &[u8]) -> bool {
    let mut drm = false;
    let mut hotplug = false;
    for field in msg.split(|b| *b == 0) {
        match field {
            b"SUBSYSTEM=drm" => drm = true,
            b"HOTPLUG=1" => hotplug = true,
            _ => {}
        }
    }
    drm && hotplug
}

/// True if the time the computer spent asleep grew by more than `min` between two checks.
fn slept(prev_offset: Duration, offset: Duration, min: Duration) -> bool {
    offset > prev_offset + min
}

/// CLOCK_BOOTTIME keeps counting during sleep, CLOCK_MONOTONIC does not:
/// their difference is the total time spent asleep.
fn sleep_offset() -> Option<Duration> {
    fn now(clock: libc::clockid_t) -> Option<Duration> {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        if unsafe { libc::clock_gettime(clock, &mut ts) } != 0 {
            return None;
        }
        Some(Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32))
    }
    let boot = now(libc::CLOCK_BOOTTIME)?;
    let mono = now(libc::CLOCK_MONOTONIC)?;
    Some(boot.saturating_sub(mono))
}

const TICK_MS: i32 = 15_000;

fn start_hotplug(core: Shared) {
    let tx = lock(&core).sender();
    std::thread::spawn(move || {
        // Kernel uevents (group 1). Needs no special rights.
        let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, libc::NETLINK_KOBJECT_UEVENT) };
        let mut listening = fd >= 0;
        if listening {
            let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
            addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
            addr.nl_groups = 1;
            let r = unsafe {
                libc::bind(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_nl>() as u32)
            };
            listening = r == 0;
        }
        let mut last = sleep_offset();
        let mut buf = vec![0u8; 8192];
        loop {
            let mut ready = false;
            if listening {
                let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
                let r = unsafe { libc::poll(&mut p, 1, TICK_MS) };
                ready = r > 0 && p.revents & libc::POLLIN != 0;
            } else {
                std::thread::sleep(Duration::from_millis(TICK_MS as u64));
            }
            if ready {
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), 0) };
                if n > 0 && is_display_hotplug(&buf[..n as usize]) && tx.send(Cmd::DevicesChanged).is_err() {
                    return;
                }
            }
            // The computer woke up: monitors may have lost their settings.
            let now = sleep_offset();
            if let (Some(a), Some(b)) = (last, now) {
                if slept(a, b, Duration::from_secs(2)) && tx.send(Cmd::DevicesChanged).is_err() {
                    return;
                }
            }
            if now.is_some() {
                last = now;
            }
        }
    });
}

// ---- only one copy at a time ----

/// Take an exclusive lock on `path`. None means another copy already holds it.
/// The returned file must stay open for as long as the app runs.
fn single_instance(path: &Path) -> Option<File> {
    let f = OpenOptions::new().create(true).truncate(false).write(true).open(path).ok()?;
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        Some(f)
    } else {
        None
    }
}

fn lock_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(dir) => dir.join("brightness-lite.lock"),
        None => std::env::temp_dir().join(format!("brightness-lite-{uid}.lock")),
    }
}

// ---- egui app ----

struct BrightnessApp {
    core: Shared,
    ui: ui::UiState,
    /// True once the window has had focus since it was opened.
    armed: bool,
    first_hide_done: bool,
}

impl BrightnessApp {
    fn show(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        VISIBLE.store(true, Ordering::Relaxed);
        self.ui.reset_drag();
        self.armed = false; // focus must be seen again before click-away can hide
        ctx.request_repaint();
    }

    fn hide(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        VISIBLE.store(false, Ordering::Relaxed);
    }
}

impl eframe::App for BrightnessApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let tray = TRAY_OK.load(Ordering::Relaxed);

        // The close button only hides the window (Quit is in the tray menu).
        // Without a tray there is nothing to bring it back, so it quits.
        if ctx.input(|i| i.viewport().close_requested()) {
            if tray {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.hide(ctx);
            } else {
                quit(&self.core);
            }
        }

        // eframe shows the window after its first paint; with a tray, hide it again.
        if !self.first_hide_done && FIRST_FRAME.load(Ordering::Relaxed) {
            self.first_hide_done = true;
            if tray {
                self.hide(ctx);
            } else {
                VISIBLE.store(true, Ordering::Relaxed);
            }
        }

        if self.first_hide_done {
            if TOGGLE_REQ.swap(false, Ordering::Relaxed) {
                if VISIBLE.load(Ordering::Relaxed) {
                    self.hide(ctx);
                } else {
                    let just_hid = FOCUS_HIDE
                        .lock()
                        .ok()
                        .and_then(|g| *g)
                        .is_some_and(|t| t.elapsed() < REOPEN_GUARD);
                    if !just_hid {
                        self.show(ctx);
                    }
                }
            }
            if SHOW_REQ.swap(false, Ordering::Relaxed) {
                self.show(ctx);
            }
        }

        // Click-away: hide when focus is lost. A slider drag in progress delays it.
        if tray && VISIBLE.load(Ordering::Relaxed) {
            let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
            if focused {
                self.armed = true;
            } else if self.armed {
                if self.ui.dragging() {
                    ctx.request_repaint_after(Duration::from_millis(50));
                } else {
                    self.armed = false;
                    if let Ok(mut g) = FOCUS_HIDE.lock() {
                        *g = Some(Instant::now());
                    }
                    self.hide(ctx);
                }
            }
        }

        // Keep the tray menu's checkbox in step with the window's switch.
        let on = lock(&self.core).startup;
        if TRAY_STARTUP.swap(on, Ordering::Relaxed) != on {
            if let Some(h) = TRAY.get() {
                h.update(|t| t.startup = on);
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        {
            let mut core = lock(&self.core);
            ui::draw(ui, &mut core, &mut self.ui);
        }
        if !FIRST_FRAME.swap(true, Ordering::Relaxed) {
            ui.ctx().request_repaint();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        ui::BG.to_normalized_gamma_f32()
    }
}

pub fn run() {
    // A second copy exits quietly.
    let Some(_instance) = single_instance(&lock_path()) else {
        return;
    };

    let core: Shared = state::start(repaint);
    let tray_ok = start_tray(&core);
    TRAY_OK.store(tray_ok, Ordering::Relaxed);
    if !tray_ok {
        lock(&core).startup_error =
            "No system tray found, so the window stays open. Install a StatusNotifier/AppIndicator host to get the tray icon.".into();
    }
    start_hotkeys(core.clone());
    start_hotplug(core.clone());

    let height = 150.0 + 165.0;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Brightness Lite")
            .with_app_id("brightness-lite")
            .with_inner_size([ui::WINDOW_W, height])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_minimize_button(false)
            .with_always_on_top()
            .with_taskbar(!tray_ok)
            .with_icon(egui::IconData { rgba: icon_rgba(), width: 32, height: 32 })
            .with_visible(false),
        ..Default::default()
    };

    let result = eframe::run_native(
        "Brightness Lite",
        options,
        Box::new(move |cc| {
            ui::apply_theme(&cc.egui_ctx);
            let _ = CTX.set(cc.egui_ctx.clone());
            Ok(Box::new(BrightnessApp { core: core.clone(), ui: ui::UiState::default(), armed: false, first_hide_done: false }))
        }),
    );
    if let Err(e) = result {
        eprintln!("window error: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_instance_is_refused() {
        let p = std::env::temp_dir().join(format!("bl-lock-test-{}", std::process::id()));
        let first = single_instance(&p);
        assert!(first.is_some());
        assert!(single_instance(&p).is_none());
        drop(first);
        // Released when the first copy goes away.
        assert!(single_instance(&p).is_some());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn display_hotplug_events_are_recognised() {
        let yes = b"change@/devices/pci0000:00/0000:00:02.0/drm/card0\0ACTION=change\0DEVPATH=/devices/pci0000:00/0000:00:02.0/drm/card0\0SUBSYSTEM=drm\0HOTPLUG=1\0CONNECTOR=102\0SEQNUM=5\0";
        assert!(is_display_hotplug(yes));
        let no_hotplug = b"change@/x\0ACTION=change\0SUBSYSTEM=drm\0SEQNUM=5\0";
        assert!(!is_display_hotplug(no_hotplug));
        let other = b"change@/x\0ACTION=change\0SUBSYSTEM=usb\0HOTPLUG=1\0";
        assert!(!is_display_hotplug(other));
        assert!(!is_display_hotplug(b""));
        assert!(!is_display_hotplug(b"SUBSYSTEM=drmx\0HOTPLUG=1\0"));
    }

    #[test]
    fn sleep_detection() {
        let s = Duration::from_secs;
        assert!(!slept(s(0), s(0), s(2)));
        assert!(!slept(s(100), s(101), s(2)));
        assert!(slept(s(100), s(160), s(2)));
        // The clocks can only drift apart, never back.
        assert!(!slept(s(100), s(50), s(2)));
    }

    #[test]
    fn hotkeys_are_blocked_on_wayland_only() {
        assert!(hotkey_blocker(Some("wayland"), false).is_some());
        assert!(hotkey_blocker(None, true).is_some());
        assert!(hotkey_blocker(Some("x11"), false).is_none());
        assert!(hotkey_blocker(None, false).is_none());
    }

    #[test]
    fn tray_icon_is_argb() {
        let rgba = icon_rgba();
        let argb = rgba_to_argb(&rgba);
        assert_eq!(argb.len(), rgba.len());
        // Centre pixel: opaque light blue -> A R G B.
        let i = (16 * 32 + 16) * 4;
        assert_eq!(&argb[i..i + 4], &[0xff, 0xe6, 0xee, 0xff]);
        // Corner pixel: transparent.
        assert_eq!(argb[0], 0);
    }
}
