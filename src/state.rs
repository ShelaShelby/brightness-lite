//! Shared app state: monitors, saved values, startup flag. This is the same
//! logic as before the restyle, moved out of the window code. No UI here.

use crate::cache::{Cache, View};
use crate::{ddc, store};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::autostart;

/// After a plug/unplug or wake-up event, wait this long for more events before looking.
const SETTLE: Duration = Duration::from_secs(3);
/// If a monitor still does not answer after waking up, look again after this long.
const RETRY_AFTER: Duration = Duration::from_secs(5);
/// How many more times to look again.
const RETRY_TIMES: u32 = 3;

/// First run: remember the default choice and register. Later runs: if the
/// user's choice is on, make sure the entry points at this exe; if off, leave it alone.
fn sync_autostart(path: Option<&std::path::Path>) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut settings = path.map(store::load).unwrap_or_default();
    if settings.autostart.is_none() {
        settings.autostart = Some(autostart::DEFAULT_ON);
        if let Some(p) = path {
            let _ = store::save(p, &settings);
        }
    }
    if settings.autostart == Some(true) {
        let _ = autostart::refresh(&exe);
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
    /// A monitor was plugged in or out, or the computer woke up: after things
    /// settle, look again and put the saved values back.
    DevicesChanged,
    /// Release the monitor handles and stop.
    Quit(Sender<()>),
}

pub struct Core {
    /// What the window draws. Filled by the monitor thread.
    pub cache: Cache,
    pub settings_path: Option<PathBuf>,
    /// Set by the hotkey thread when Ctrl+Alt+Up/Down could not be registered.
    pub hotkey_error: String,
    /// Set when the startup entry could not be changed.
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
        startup: autostart::enabled(),
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
        lines.extend(self.cache.notes.iter().cloned());
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

    /// Turn the startup entry on or off. On failure the flag is unchanged
    /// and the reason is shown in the status lines.
    pub fn set_startup(&mut self, on: bool) {
        let result = std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|exe| autostart::set(on, &exe));
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
                self.startup = autostart::enabled();
            }
        }
    }
}

// ---- monitor thread ----

/// Put the saved values back, to the same physical monitors. A value that is
/// already right is not written again.
fn restore_saved(monitors: &mut ddc::Monitors, saved: &store::Settings) {
    for m in monitors.list.iter_mut() {
        let Some(entry) = m.key.as_deref().and_then(|k| saved.find(k)) else { continue };
        for (setting, want) in [
            (ddc::Setting::Brightness, entry.brightness_pct()),
            (ddc::Setting::Contrast, entry.contrast_pct()),
        ] {
            if m.percent(setting).is_some_and(|now| now != want) {
                let _ = m.set(setting, want);
            }
        }
    }
}

fn scan(restore: bool, path: Option<&std::path::Path>) -> ddc::Monitors {
    let mut monitors = ddc::discover();
    if restore {
        let saved = path.map(store::load).unwrap_or_default();
        restore_saved(&mut monitors, &saved);
    }
    monitors
}

fn views_of(monitors: &ddc::Monitors) -> Vec<View> {
    monitors
        .list
        .iter()
        .map(|m| View {
            display_number: m.display_number,
            name: m.name.clone(),
            key: m.key.clone(),
            rect: m.rect,
            values: [m.percent(ddc::Setting::Brightness), m.percent(ddc::Setting::Contrast)],
            errors: m.errors(),
        })
        .collect()
}

/// True if a write meant for the old list still points at the same monitors.
fn same_layout(a: &[View], b: &[View]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.key == y.key && x.display_number == y.display_number)
}

fn lock(core: &Mutex<Core>) -> std::sync::MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(|e| e.into_inner())
}

/// Decides when to look again after plug/unplug/wake-up events: once things
/// have been quiet for SETTLE, then a few more times while a monitor still
/// does not answer (they can take several seconds to wake up).
#[derive(Default)]
struct Settle {
    due: Option<Instant>,
    tries: u32,
}

impl Settle {
    fn event(&mut self, now: Instant) {
        self.due = Some(now + SETTLE);
        self.tries = 0;
    }

    /// How long the worker may sleep before it has to look. None = no limit.
    fn wait(&self, now: Instant) -> Option<Duration> {
        self.due.map(|d| d.saturating_duration_since(now))
    }

    fn is_due(&self, now: Instant) -> bool {
        self.due.is_some_and(|d| d <= now)
    }

    /// Called after a look. `all_answered` = no monitor reported an error.
    fn looked(&mut self, now: Instant, all_answered: bool) {
        if !all_answered && self.tries < RETRY_TIMES {
            self.tries += 1;
            self.due = Some(now + RETRY_AFTER);
        } else {
            self.due = None;
            self.tries = 0;
        }
    }
}

/// Of several writes to the same slider only the last counts, and a run of
/// hotkey presses (key repeat) becomes one step of the summed size.
fn coalesce(batch: Vec<Cmd>) -> Vec<Cmd> {
    let total: i32 = batch.iter().map(|c| if let Cmd::Hotkey(d) = c { *d } else { 0 }).sum();
    let last_hotkey = batch.iter().rposition(|c| matches!(c, Cmd::Hotkey(_)));
    let mut seen: Vec<(u64, usize, ddc::Setting)> = Vec::new();
    let mut out = Vec::with_capacity(batch.len());
    for (n, c) in batch.into_iter().enumerate().rev() {
        match c {
            Cmd::Write { generation, index, setting, .. } => {
                let k = (generation, index, setting);
                if seen.contains(&k) {
                    continue;
                }
                seen.push(k);
                out.push(c);
            }
            Cmd::Hotkey(_) => {
                if Some(n) == last_hotkey {
                    out.push(Cmd::Hotkey(total));
                }
            }
            c => out.push(c),
        }
    }
    out.reverse();
    out
}

fn worker(core: Arc<Mutex<Core>>, rx: Receiver<Cmd>, notify: fn()) {
    let path = lock(&core).settings_path.clone();
    let mut monitors = ddc::Monitors::empty();
    let mut generation = 0u64;
    let mut settle = Settle::default();

    loop {
        let first = match settle.wait(Instant::now()) {
            None => match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            },
            Some(d) => match rx.recv_timeout(d) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
        };

        // Take everything that is waiting, then drop what a later command makes pointless.
        let mut batch: Vec<Cmd> = first.into_iter().collect();
        batch.extend(rx.try_iter());
        let batch = coalesce(batch);

        let mut failed = false;
        let mut save = false;
        let mut quit = None;
        let mut rescanned = false;
        for cmd in batch {
            match cmd {
                Cmd::Scan => {
                    monitors = scan(true, path.as_deref());
                    rescanned = true;
                }
                Cmd::Rescan => {
                    monitors = scan(false, path.as_deref());
                    rescanned = true;
                }
                Cmd::DevicesChanged => settle.event(Instant::now()),
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

        // Things settled after a plug/unplug/wake-up: look again, put the saved values back.
        let looking = quit.is_none() && settle.is_due(Instant::now());
        if looking {
            monitors = scan(true, path.as_deref());
            rescanned = true;
            // Nothing is saved here: the saved values were just applied, not changed.
        } else if failed && quit.is_none() {
            // A failed write usually means the monitor was unplugged or switched off: look again.
            monitors = scan(false, path.as_deref());
            rescanned = true;
        }

        let views = views_of(&monitors);
        if looking {
            settle.looked(Instant::now(), views.iter().all(|v| v.errors.is_empty()));
        }
        if quit.is_some() {
            settle = Settle::default();
        }
        let changed = {
            let mut g = lock(&core);
            let layout_same = same_layout(&g.cache.views, &views);
            // The generation only moves when the list of monitors changed, so a write that
            // is still on its way keeps pointing at the right monitor.
            if rescanned && (!layout_same || !g.cache.scanned) {
                generation += 1;
            }
            let notes = monitors.notes.clone();
            let changed = !g.cache.scanned || g.cache.views != views || g.cache.notes != notes || g.cache.generation != generation;
            if changed {
                g.cache.replace(views.clone(), generation, notes);
            }
            changed
        };
        if changed {
            notify();
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, secs: u64) -> Instant {
        t0 + Duration::from_secs(secs)
    }

    #[test]
    fn settle_waits_for_quiet_then_looks() {
        let t0 = Instant::now();
        let mut s = Settle::default();
        assert_eq!(s.wait(t0), None);
        assert!(!s.is_due(t0));
        s.event(t0);
        assert_eq!(s.wait(t0), Some(SETTLE));
        assert!(!s.is_due(at(t0, 2)));
        // Another event restarts the quiet period.
        s.event(at(t0, 2));
        assert!(!s.is_due(at(t0, 3)));
        assert!(s.is_due(at(t0, 2) + SETTLE));
        assert_eq!(s.wait(at(t0, 60)), Some(Duration::ZERO));
    }

    #[test]
    fn settle_retries_a_few_times_then_gives_up() {
        let t0 = Instant::now();
        let mut s = Settle::default();
        s.event(t0);
        let mut now = at(t0, 10);
        for _ in 0..RETRY_TIMES {
            s.looked(now, false);
            assert!(s.due.is_some());
            now += RETRY_AFTER;
            assert!(s.is_due(now));
        }
        s.looked(now, false);
        assert!(s.due.is_none());
    }

    #[test]
    fn settle_stops_when_everything_answers() {
        let t0 = Instant::now();
        let mut s = Settle::default();
        s.event(t0);
        s.looked(at(t0, 5), true);
        assert!(s.due.is_none());
        assert_eq!(s.tries, 0);
    }

    #[test]
    fn new_event_resets_the_retry_count() {
        let t0 = Instant::now();
        let mut s = Settle::default();
        s.event(t0);
        s.looked(at(t0, 5), false);
        s.looked(at(t0, 10), false);
        assert_eq!(s.tries, 2);
        s.event(at(t0, 11));
        assert_eq!(s.tries, 0);
    }

    #[test]
    fn coalesce_keeps_last_write_and_sums_hotkeys() {
        let w = |g, index, value| Cmd::Write { generation: g, index, setting: ddc::Setting::Brightness, value };
        let out = coalesce(vec![w(1, 0, 10), Cmd::Hotkey(5), w(1, 1, 20), w(1, 0, 30), Cmd::Hotkey(5), Cmd::Rescan, Cmd::Hotkey(-5)]);
        assert_eq!(out.len(), 4);
        assert!(matches!(out[0], Cmd::Write { index: 1, value: 20, .. }));
        assert!(matches!(out[1], Cmd::Write { index: 0, value: 30, .. }));
        assert!(matches!(out[2], Cmd::Rescan));
        assert!(matches!(out[3], Cmd::Hotkey(5)));
        assert!(coalesce(Vec::new()).is_empty());
    }

    fn v(key: &str, n: u32) -> View {
        View { key: Some(key.into()), display_number: Some(n), ..Default::default() }
    }

    #[test]
    fn layout_ignores_values_but_not_monitors() {
        let mut a = vec![v("a", 1), v("b", 2)];
        let mut b = a.clone();
        b[0].values = [Some(5), Some(6)];
        assert!(same_layout(&a, &b));
        b.reverse();
        assert!(!same_layout(&a, &b));
        a.pop();
        assert!(!same_layout(&a, &[v("a", 1), v("b", 2)]));
    }
}
