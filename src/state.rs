//! Shared app state: monitors, saved values, startup flag. This is the same
//! logic as before the restyle, moved out of the window code. No UI here.

use crate::cache::{Cache, View};
use crate::{ddc, store};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
/// Name of our value under the Run key.
const RUN_VALUE_NAME: &str = "BrightnessLite";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ---- start with Windows: HKCU Run key, no admin rights ----

/// The command stored under `name`, if the entry exists.
fn read_run_value(name: &str) -> Option<String> {
    let name = wide(name);
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, 0, KEY_QUERY_VALUE, &mut key).is_err() {
            return None;
        }
        let mut bytes = 0u32;
        let probe = RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, None, None, Some(&mut bytes));
        let mut out = None;
        if probe.is_ok() {
            let mut buf = vec![0u16; (bytes as usize).div_ceil(2) + 1];
            let mut size = (buf.len() * 2) as u32;
            let r = RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                None,
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut size),
            );
            if r.is_ok() {
                let n = (size as usize / 2).min(buf.len());
                out = Some(String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_string());
            }
        }
        let _ = RegCloseKey(key);
        out
    }
}

/// Set (`Some(command)`) or remove (`None`) the entry `name`. Removing a missing entry is fine.
fn write_run_value(name: &str, command: Option<&str>) -> Result<(), String> {
    let name = wide(name);
    unsafe {
        let mut key = HKEY::default();
        RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, 0, KEY_SET_VALUE, &mut key)
            .ok()
            .map_err(|e| e.to_string())?;
        let res = match command {
            Some(c) => {
                let cmd = wide(c);
                let bytes = std::slice::from_raw_parts(cmd.as_ptr() as *const u8, cmd.len() * 2);
                RegSetValueExW(key, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes)).ok()
            }
            None => match RegDeleteValueW(key, PCWSTR(name.as_ptr())) {
                e if e == ERROR_FILE_NOT_FOUND => Ok(()),
                e => e.ok(),
            },
        };
        let _ = RegCloseKey(key);
        res.map_err(|e| e.to_string())
    }
}

/// What the "Start with Windows" switch does: add or remove the entry for `exe`.
fn set_autostart_entry(name: &str, on: bool, exe: &std::path::Path) -> Result<(), String> {
    if on {
        write_run_value(name, Some(&store::startup_command(exe)))
    } else {
        write_run_value(name, None)
    }
}

/// Keep the entry pointing at the exe that is running now (it may have been moved or rebuilt).
fn refresh_autostart_entry(name: &str, exe: &std::path::Path) -> Result<(), String> {
    let want = store::startup_command(exe);
    if read_run_value(name).as_deref() == Some(want.as_str()) {
        return Ok(());
    }
    write_run_value(name, Some(&want))
}

pub fn startup_enabled() -> bool {
    read_run_value(RUN_VALUE_NAME).is_some()
}

/// First run: remember the choice "on" and register. Later runs: if the user's
/// choice is on, make sure the entry points at this exe; if off, leave it alone.
fn sync_autostart(path: Option<&std::path::Path>) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut settings = path.map(store::load).unwrap_or_default();
    if settings.autostart.is_none() {
        settings.autostart = Some(true);
        if let Some(p) = path {
            let _ = store::save(p, &settings);
        }
    }
    if settings.autostart == Some(true) {
        let _ = refresh_autostart_entry(RUN_VALUE_NAME, &exe);
    }
}

// ---- state ----

/// Work for the monitor thread. Every DDC/CI call happens there, never on the UI thread.
pub enum Cmd {
    /// First scan at startup; also puts the saved values back.
    Scan,
    /// Look for monitors again (tray "Re-scan", or after a failed write).
    Rescan,
    /// A slider was released.
    Write { generation: u64, index: usize, setting: ddc::Setting, value: u32 },
    /// Ctrl+Alt+Up / Down, brightness only, all monitors.
    Hotkey(i32),
    /// "Reset to saved values".
    Reset,
    /// Release the monitor handles and stop.
    Quit(Sender<()>),
}

pub struct Core {
    /// What the window draws. Filled by the monitor thread.
    pub cache: Cache,
    pub settings_path: Option<PathBuf>,
    /// Set by the hotkey thread when Ctrl+Alt+Up/Down could not be registered.
    pub hotkey_error: String,
    /// Set when the "Start with Windows" entry could not be changed.
    pub startup_error: String,
    pub startup: bool,
    tx: Sender<Cmd>,
}

/// Create the shared state and start the monitor thread (it scans right away).
/// `notify` is called whenever the cache changes, to redraw the window.
pub fn start(notify: fn()) -> Arc<Mutex<Core>> {
    let (tx, rx) = channel();
    let settings_path = store::default_path();
    sync_autostart(settings_path.as_deref());
    let core = Arc::new(Mutex::new(Core {
        cache: Cache::default(),
        settings_path,
        hotkey_error: String::new(),
        startup_error: String::new(),
        startup: startup_enabled(),
        tx: tx.clone(),
    }));
    let c = core.clone();
    std::thread::spawn(move || worker(c, rx, notify));
    let _ = tx.send(Cmd::Scan);
    core
}

impl Core {
    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn sender(&self) -> Sender<Cmd> {
        self.tx.clone()
    }

    /// Messages to show at the bottom of the window.
    pub fn status_lines(&self) -> Vec<String> {
        let mut lines = self.cache.error_lines();
        if self.cache.scanned && self.cache.views.is_empty() {
            lines.push("No monitors found.".into());
        }
        for extra in [&self.hotkey_error, &self.startup_error] {
            if !extra.is_empty() {
                lines.push(extra.clone());
            }
        }
        lines
    }

    /// A slider was released: show the value at once, write it in the background.
    pub fn write(&mut self, index: usize, setting: ddc::Setting, value: u32) {
        let value = ddc::clamp(value as i64);
        if let Some(v) = self.cache.views.get_mut(index) {
            v.set_value(setting, value);
        }
        self.send(Cmd::Write { generation: self.cache.generation, index, setting, value });
    }

    /// Turn "Start with Windows" on or off. On failure the flag is unchanged
    /// and the reason is shown in the status lines.
    pub fn set_startup(&mut self, on: bool) {
        let result = std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|exe| set_autostart_entry(RUN_VALUE_NAME, on, &exe));
        match result {
            Ok(()) => {
                self.startup_error.clear();
                self.startup = on;
                // Remember the choice, so it is not switched back on at the next start.
                if let Some(p) = &self.settings_path {
                    let mut settings = store::load(p);
                    settings.autostart = Some(on);
                    let _ = store::save(p, &settings);
                }
            }
            Err(e) => {
                self.startup_error = format!("Could not change the startup setting: {e}");
                self.startup = startup_enabled();
            }
        }
    }
}

// ---- monitor thread ----

fn scan(restore: bool, path: Option<&std::path::Path>) -> ddc::Monitors {
    let mut monitors = ddc::discover();
    if restore {
        // The only thing applied at startup: the saved values, to the same physical monitors.
        let saved = path.map(store::load).unwrap_or_default();
        for m in monitors.list.iter_mut() {
            let Some(entry) = m.key.as_deref().and_then(|k| saved.find(k)) else { continue };
            if m.percent(ddc::Setting::Brightness).is_some() {
                let _ = m.set(ddc::Setting::Brightness, entry.brightness_pct());
            }
            if m.percent(ddc::Setting::Contrast).is_some() {
                let _ = m.set(ddc::Setting::Contrast, entry.contrast_pct());
            }
        }
    }
    monitors
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // These touch the real HKCU Run key, but only under a throw-away name.
    fn test_name(tag: &str) -> String {
        format!("BrightnessLiteTest-{}-{}", std::process::id(), tag)
    }

    #[test]
    fn toggle_creates_and_removes_the_run_entry() {
        let name = test_name("toggle");
        let exe = Path::new("C:\\Tools\\brightness-lite.exe");
        assert_eq!(read_run_value(&name), None);

        set_autostart_entry(&name, true, exe).unwrap();
        assert_eq!(
            read_run_value(&name).as_deref(),
            Some("\"C:\\Tools\\brightness-lite.exe\" --autostart")
        );

        set_autostart_entry(&name, false, exe).unwrap();
        assert_eq!(read_run_value(&name), None);
        // Turning it off again is harmless.
        set_autostart_entry(&name, false, exe).unwrap();
    }

    #[test]
    fn moved_exe_updates_the_entry() {
        let name = test_name("moved");
        set_autostart_entry(&name, true, Path::new("C:\\Old\\brightness-lite.exe")).unwrap();
        refresh_autostart_entry(&name, Path::new("C:\\New dir\\brightness-lite.exe")).unwrap();
        assert_eq!(
            read_run_value(&name).as_deref(),
            Some("\"C:\\New dir\\brightness-lite.exe\" --autostart")
        );
        set_autostart_entry(&name, false, Path::new("x")).unwrap();
        assert_eq!(read_run_value(&name), None);
    }
}

fn views_of(monitors: &ddc::Monitors) -> Vec<View> {
    monitors
        .list
        .iter()
        .map(|m| View {
            display_number: m.display_number,
            key: m.key.clone(),
            rect: m.rect,
            values: [m.percent(ddc::Setting::Brightness), m.percent(ddc::Setting::Contrast)],
            errors: m.errors(),
        })
        .collect()
}

fn lock(core: &Mutex<Core>) -> std::sync::MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(|e| e.into_inner())
}

fn worker(core: Arc<Mutex<Core>>, rx: Receiver<Cmd>, notify: fn()) {
    let path = lock(&core).settings_path.clone();
    let mut monitors = ddc::Monitors { list: Vec::new() };
    let mut generation = 0u64;

    while let Ok(first) = rx.recv() {
        // Take everything that is waiting. Of several writes to the same slider only the last counts.
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let mut keep = vec![true; batch.len()];
        let mut seen: Vec<(u64, usize, ddc::Setting)> = Vec::new();
        for (n, c) in batch.iter().enumerate().rev() {
            if let Cmd::Write { generation, index, setting, .. } = c {
                let k = (*generation, *index, *setting);
                if seen.contains(&k) {
                    keep[n] = false;
                } else {
                    seen.push(k);
                }
            }
        }

        let mut failed = false;
        let mut save = false;
        let mut quit = None;
        for (cmd, keep) in batch.into_iter().zip(keep) {
            if !keep {
                continue;
            }
            match cmd {
                Cmd::Scan => {
                    monitors = scan(true, path.as_deref());
                    generation += 1;
                }
                Cmd::Rescan => {
                    monitors = scan(false, path.as_deref());
                    generation += 1;
                }
                Cmd::Write { generation: g, index, setting, value } => {
                    if g == generation {
                        if let Some(m) = monitors.list.get_mut(index) {
                            failed |= m.set(setting, value).is_err();
                            save = true;
                        }
                    }
                }
                Cmd::Hotkey(delta) => {
                    for m in monitors.list.iter_mut() {
                        if let Some(cur) = m.percent(ddc::Setting::Brightness) {
                            failed |= m.set(ddc::Setting::Brightness, ddc::stepped(cur, delta)).is_err();
                        }
                    }
                    save = true;
                }
                Cmd::Reset => {
                    let saved = path.as_deref().map(store::load).unwrap_or_default();
                    for m in monitors.list.iter_mut() {
                        let Some(entry) = m.key.as_deref().and_then(|k| saved.find(k)) else { continue };
                        let _ = m.set(ddc::Setting::Brightness, entry.brightness_pct());
                        let _ = m.set(ddc::Setting::Contrast, entry.contrast_pct());
                    }
                }
                Cmd::Quit(done) => quit = Some(done),
            }
        }

        // A failed write usually means the monitor was unplugged or switched off: look again.
        if failed && quit.is_none() {
            monitors = scan(false, path.as_deref());
            generation += 1;
        }

        let views = views_of(&monitors);
        lock(&core).cache.replace(views.clone(), generation);
        notify();
        if save {
            if let Some(p) = &path {
                let _ = store::save(p, &crate::cache::to_settings(&views, &store::load(p)));
            }
        }
        if let Some(done) = quit {
            drop(monitors); // releases the physical monitor handles
            let _ = done.send(());
            return;
        }
    }
}
