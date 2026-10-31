//! `--theme-test N [IDLE_S]` (Windows, for CI): stress the Theme panel in the
//! real window, like a user clicking very fast. The clicks go through egui's
//! input (the window's own hit testing), so they hit the same code as the mouse.
//!
//! 1. Warm-up: open the panel, every appearance x colour once, close it.
//! 2. Open/close the panel N times (every 50 clicks the window is first moved
//!    to a screen edge: it must stay inside the work area), then 60 clicks
//!    without waiting.
//! 3. Switch the appearance N times, then the colour N times.
//! 4. Show and hide the window N/4 times; every 10th time the other app
//!    "changes" theme.json first, and the window must show it.
//! 5. Leave it hidden for IDLE_S seconds (default 10) and measure its CPU time.
//!
//! Private bytes, working set, handles, GDI and USER objects are printed at
//! the start (after the warm-up), the middle and the end of each step, to
//! stdout and cycle-test.log. "theme-test: PASS" if nothing grew (beyond a
//! small allowance for caches), the window never left the work area, every
//! click did what it should and the idle CPU stayed near 0.

use crate::theme::{self, Accent, Appearance, Theme};
use crate::ui::Hits;
use eframe::egui;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{FILETIME, HWND, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetGuiResources, GetProcessHandleCount, GetProcessTimes, GR_GDIOBJECTS, GR_USEROBJECTS,
};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER};

static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Mouse events to give egui, one per frame: (position, pressed).
static CLICKS: Mutex<VecDeque<(egui::Pos2, bool)>> = Mutex::new(VecDeque::new());
/// What the window showed in its last frame.
static SNAP: Mutex<Option<Snap>> = Mutex::new(None);
static FRAMES: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
struct Snap {
    hits: Hits,
    expanded: bool,
    theme: Theme,
}

/// What the test needs from the app.
pub struct Ops {
    /// The window can be shown (first frame done, start-up finished).
    pub ready: fn() -> bool,
    pub show: fn(),
    pub hide: fn(),
    pub hwnd: fn() -> HWND,
    pub repaint: fn(),
    pub report: fn(&str),
    /// End the app with this exit code.
    pub finish: fn(i32),
}

/// `--theme-test N [IDLE_S]`.
pub fn arg() -> Option<(u32, u64)> {
    let args: Vec<String> = std::env::args().collect();
    parse(&args)
}

fn parse(args: &[String]) -> Option<(u32, u64)> {
    let i = args.iter().position(|a| a == "--theme-test")?;
    let n = args.get(i + 1).and_then(|n| n.parse().ok()).filter(|n| *n > 0)?;
    let idle = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(10);
    Some((n, idle))
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// From `raw_input_hook`: the next queued mouse event.
pub fn hook(ctx: &egui::Context, raw: &mut egui::RawInput) {
    if !active() {
        return;
    }
    let next = CLICKS.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
    if let Some((pos, pressed)) = next {
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        ctx.request_repaint();
    }
}

/// After each drawn frame (only while the test runs).
pub fn frame_done(ctx: &egui::Context, hits: Hits, expanded: bool, theme: Theme) {
    if !active() {
        return;
    }
    *SNAP.lock().unwrap_or_else(|e| e.into_inner()) = Some(Snap { hits, expanded, theme });
    FRAMES.fetch_add(1, Ordering::Relaxed);
    if !CLICKS.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
        ctx.request_repaint();
    }
}

/// Resources of this process.
#[derive(Clone, Copy, Default)]
struct Use {
    private: usize,
    working_set: usize,
    handles: u32,
    gdi: u32,
    user: u32,
}

fn usage() -> Use {
    let mut u = Use::default();
    unsafe {
        let p = GetCurrentProcess();
        let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        if GetProcessMemoryInfo(p, &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, size).is_ok() {
            u.private = c.PrivateUsage;
            u.working_set = c.WorkingSetSize;
        }
        let _ = GetProcessHandleCount(p, &mut u.handles);
        u.gdi = GetGuiResources(p, GR_GDIOBJECTS);
        u.user = GetGuiResources(p, GR_USEROBJECTS);
    }
    u
}

/// User + kernel CPU time of this process.
fn cpu_time() -> Duration {
    let (mut a, mut b, mut k, mut u) =
        (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut k, &mut u);
    }
    let t = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32 | u64::from(f.dwLowDateTime)) * 100;
    Duration::from_nanos(t(k) + t(u))
}

fn snap() -> Option<Snap> {
    *SNAP.lock().unwrap_or_else(|e| e.into_inner())
}

struct Run {
    ops: Ops,
    failures: Vec<String>,
}

impl Run {
    fn report(&self, line: &str) {
        (self.ops.report)(line);
    }

    fn mark(&self, label: &str) -> Use {
        let u = usage();
        self.report(&format!(
            "{label:>16}  Private {:7.2} MB  WorkingSet {:7.2} MB  handles {:4}  GDI {:3}  USER {:3}",
            u.private as f64 / 1048576.0,
            u.working_set as f64 / 1048576.0,
            u.handles,
            u.gdi,
            u.user
        ));
        u
    }

    fn fail(&mut self, why: String) {
        if self.failures.len() < 20 {
            self.report(&format!("theme-test: problem: {why}"));
        }
        self.failures.push(why);
    }

    /// Wait until the queued events are used and `frames` more frames are drawn.
    fn settle(&mut self, frames: u64) -> bool {
        let start = FRAMES.load(Ordering::Relaxed);
        let t0 = Instant::now();
        loop {
            let empty = CLICKS.lock().unwrap_or_else(|e| e.into_inner()).is_empty();
            if empty && FRAMES.load(Ordering::Relaxed) >= start + frames {
                return true;
            }
            if t0.elapsed() > Duration::from_secs(10) {
                self.fail("the window stopped drawing frames".into());
                return false;
            }
            (self.ops.repaint)();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn queue_click(&self, pos: egui::Pos2) {
        let mut q = CLICKS.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back((pos, true));
        q.push_back((pos, false));
    }

    /// Click and wait until the window drew the result.
    fn click(&mut self, pick: impl Fn(&Hits) -> egui::Rect) -> Option<Snap> {
        let s = snap()?;
        self.queue_click(pick(&s.hits).center());
        (self.ops.repaint)();
        // Press, release, and the frame after.
        if !self.settle(3) {
            return None;
        }
        snap()
    }

    fn set_expanded(&mut self, on: bool) {
        if snap().is_some_and(|s| s.expanded != on) {
            self.click(|h| h.palette);
        }
    }

    fn work_area(&self) -> RECT {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        unsafe {
            let _ = GetMonitorInfoW(MonitorFromWindow((self.ops.hwnd)(), MONITOR_DEFAULTTONEAREST), &mut mi);
        }
        mi.rcWork
    }

    fn window_rect(&self) -> RECT {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect((self.ops.hwnd)(), &mut r);
        }
        r
    }

    /// Move the window to one of four places: top left, at the bottom (where
    /// it opens), just above the bottom, at the right edge.
    fn move_to(&self, spot: u32) {
        let (wa, r) = (self.work_area(), self.window_rect());
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let (x, y) = match spot % 4 {
            0 => (wa.left + 8, wa.top + 8),
            1 => (wa.right - w - 8, wa.bottom - h - 8),
            2 => (wa.left + 40, wa.bottom - h - 40),
            _ => (wa.right - w, (wa.top + wa.bottom - h) / 2),
        };
        unsafe {
            let _ = SetWindowPos((self.ops.hwnd)(), None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    fn check_on_screen(&mut self, what: &str) {
        let (wa, r) = (self.work_area(), self.window_rect());
        let fits_height = r.bottom - r.top <= wa.bottom - wa.top;
        if r.top < wa.top || (fits_height && r.bottom > wa.bottom) {
            self.fail(format!(
                "{what}: window {}..{} outside the work area {}..{}",
                r.top, r.bottom, wa.top, wa.bottom
            ));
        }
    }
}

pub fn start(n: u32, idle_s: u64, theme_path: Option<std::path::PathBuf>, ops: Ops) {
    ACTIVE.store(true, Ordering::Relaxed);
    let _ = std::thread::Builder::new().spawn(move || {
        let mut run = Run { ops, failures: Vec::new() };
        let t0 = Instant::now();
        while !(run.ops.ready)() {
            if t0.elapsed() > Duration::from_secs(60) {
                run.report("theme-test: the window was not ready after 60 s");
                (run.ops.finish)(2);
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // Start-up memory trims happen in the first seconds.
        std::thread::sleep(Duration::from_secs(3));
        // Start from Dark + Blue, as on a first run.
        if let Some(p) = &theme_path {
            let _ = std::fs::remove_file(p);
        }
        (run.ops.show)();
        run.settle(5);
        let Some(first) = snap() else {
            run.report("theme-test: FAIL: no frame was drawn");
            (run.ops.finish)(1);
            return;
        };
        run.report(&format!("theme-test: {n} clicks per step, {} show/hide, {idle_s} s idle", (n / 4).max(1)));
        if first.expanded || first.theme != Theme::default() {
            run.fail("the panel must start closed, with Dark + Blue".into());
        }

        // 1. Warm-up.
        run.set_expanded(true);
        for a in 0..3 {
            for c in 0..7 {
                run.click(|h| h.appearance[a].0);
                run.click(|h| h.accent[c].0);
            }
        }
        run.set_expanded(false);
        let start = run.mark("start");

        // 2. Open/close.
        let mut expect = false;
        for i in 0..n {
            let moved = i % 50 == 0;
            if moved {
                run.move_to(i / 50);
                run.settle(1);
            }
            expect = !expect;
            match run.click(|h| h.palette) {
                Some(s) if s.expanded == expect => {}
                Some(_) => run.fail(format!("open/close click {i} did not toggle the panel")),
                None => break,
            }
            if moved || i % 50 == 1 {
                run.check_on_screen(&format!("open/close click {i}"));
            }
            if i + 1 == n / 2 {
                run.mark("open/close 1/2");
            }
        }
        // 60 clicks without waiting for the window in between.
        for _ in 0..60 {
            if let Some(s) = snap() {
                run.queue_click(s.hits.palette.center());
            }
        }
        (run.ops.repaint)();
        run.settle(3);
        if snap().is_some_and(|s| s.expanded != expect) {
            run.fail("after 60 fast clicks the panel is in the wrong state".into());
        }
        run.check_on_screen("after 60 fast clicks");
        run.mark("open/close end");

        // 3. Appearance, then colour, with the panel open.
        run.move_to(1);
        run.set_expanded(true);
        for i in 0..n {
            let Some(s) = snap() else { break };
            let next = (Appearance::ALL.iter().position(|&a| a == s.theme.appearance).unwrap_or(0) + 1) % 3;
            match run.click(|h| h.appearance[next].0) {
                Some(s) if s.theme.appearance == Appearance::ALL[next] && s.expanded => {}
                Some(_) => run.fail(format!("appearance click {i} did not switch")),
                None => break,
            }
            if i + 1 == n / 2 {
                run.mark("appearance 1/2");
            }
        }
        run.mark("appearance end");
        for i in 0..n {
            let Some(s) = snap() else { break };
            let next = (Accent::ALL.iter().position(|&a| a == s.theme.accent).unwrap_or(0) + 1) % 7;
            match run.click(|h| h.accent[next].0) {
                Some(s) if s.theme.accent == Accent::ALL[next] && s.expanded => {}
                Some(_) => run.fail(format!("colour click {i} did not switch")),
                None => break,
            }
            if i + 1 == n / 2 {
                run.mark("colour 1/2");
            }
        }
        run.check_on_screen("after the colour clicks");
        run.mark("colour end");
        if let (Some(p), Some(s)) = (&theme_path, snap()) {
            if theme::read(Some(p)) != Some(s.theme) {
                run.fail("theme.json does not hold the theme on screen".into());
            }
        }

        // 4. Show and hide; sometimes the other app changes the theme first.
        let m = (n / 4).max(1);
        for i in 0..m {
            (run.ops.hide)();
            std::thread::sleep(Duration::from_millis(20));
            let other = Theme { appearance: Appearance::ALL[(i as usize) % 3], accent: Accent::ALL[(i as usize) % 7] };
            let sync = i % 10 == 0 && theme_path.is_some();
            if sync {
                if let Some(p) = &theme_path {
                    let _ = theme::save(p, other);
                }
            }
            (run.ops.show)();
            run.settle(2);
            match snap() {
                Some(s) if s.expanded => run.fail(format!("show {i}: the panel did not start closed")),
                Some(s) if sync && s.theme != other => {
                    run.fail(format!("show {i}: the other app's theme is not shown"))
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(20));
            if i + 1 == m / 2 {
                run.mark("show/hide 1/2");
            }
        }
        run.check_on_screen("after show/hide");
        let end = run.mark("show/hide end");

        // 5. Idle, hidden.
        (run.ops.hide)();
        std::thread::sleep(Duration::from_secs(3));
        let before = run.mark("idle start");
        let cpu0 = cpu_time();
        let t_idle = Instant::now();
        std::thread::sleep(Duration::from_secs(idle_s));
        let cpu = cpu_time().saturating_sub(cpu0);
        let wall = t_idle.elapsed();
        let idle_end = run.mark("idle end");
        let pct = cpu.as_secs_f64() / wall.as_secs_f64().max(0.001) * 100.0;
        run.report(&format!(
            "theme-test: idle CPU {:.3} s in {:.0} s = {pct:.3} %",
            cpu.as_secs_f64(),
            wall.as_secs_f64()
        ));

        // Allowances: egui's caches (glyphs, layout) settle in the warm-up;
        // a leak of one object per click would be thousands.
        let grew = |a: u32, b: u32| b.saturating_sub(a);
        if grew(start.handles, end.handles) > 20 {
            run.fail(format!("handles grew {} -> {}", start.handles, end.handles));
        }
        if grew(start.gdi, end.gdi) > 5 {
            run.fail(format!("GDI objects grew {} -> {}", start.gdi, end.gdi));
        }
        if grew(start.user, end.user) > 5 {
            run.fail(format!("USER objects grew {} -> {}", start.user, end.user));
        }
        if end.private.saturating_sub(start.private) > 4 * 1048576 {
            run.fail(format!("private bytes grew by more than 4 MB: {} -> {}", start.private, end.private));
        }
        if idle_end.private.saturating_sub(before.private) > 256 * 1024 {
            run.fail(format!("private bytes grew while idle: {} -> {}", before.private, idle_end.private));
        }
        if pct > 0.5 {
            run.fail(format!("idle CPU {pct:.3} % (more than 0.5 %)"));
        }
        if let Some(p) = &theme_path {
            let _ = std::fs::remove_file(p);
        }
        if run.failures.is_empty() {
            run.report("theme-test: PASS");
            (run.ops.finish)(0);
        } else {
            run.report(&format!("theme-test: FAIL ({} problems)", run.failures.len()));
            (run.ops.finish)(1);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments() {
        let v = |s: &str| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(parse(&v("x --theme-test 2000")), Some((2000, 10)));
        assert_eq!(parse(&v("x --theme-test 2000 600")), Some((2000, 600)));
        assert_eq!(parse(&v("x --theme-test 0")), None);
        assert_eq!(parse(&v("x --theme-test")), None);
        assert_eq!(parse(&v("x")), None);
    }
}
