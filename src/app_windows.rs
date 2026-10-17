//! Windows shell around the shared logic: tray (tray-icon), hotkeys
//! (RegisterHotKey), window show/hide (Win32 + DWM cloak) and the egui window.

use crate::{
    ddc,
    state::{self, Cmd, Core},
    ui,
};
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::ProcessStatus::{
    EmptyWorkingSet, GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, RegisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_DOWN, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
    VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowRect, IsWindowVisible,
    RegisterClassW, SetForegroundWindow, SetWindowPos, ShowWindow, SystemParametersInfoW, TranslateMessage,
    HWND_TOPMOST, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, SPI_GETWORKAREA, SWP_NOSIZE, SW_HIDE, SW_SHOW,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WINDOW_EX_STYLE, WM_DISPLAYCHANGE, WM_HOTKEY, WM_POWERBROADCAST, WNDCLASSW,
    WS_POPUP,
};

const HOTKEY_UP: i32 = 1;
const HOTKEY_DOWN: i32 = 2;

type Shared = Arc<Mutex<Core>>;

static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
static CTX: OnceLock<egui::Context> = OnceLock::new();
/// Set when the window is opened; the next frame resets focus tracking.
static JUST_OPENED: AtomicBool = AtomicBool::new(false);
/// The first monitor scan has finished and its values are in the cache.
static SCANNED: AtomicBool = AtomicBool::new(false);
/// When the window last hid itself because it lost focus.
static FOCUS_HIDE: Mutex<Option<Instant>> = Mutex::new(None);
/// Set at the end of the first update call: egui has painted its first frame.
static FIRST_FRAME: AtomicBool = AtomicBool::new(false);
/// True once that frame is done and the window is hidden again. Only then may it be shown.
static READY: AtomicBool = AtomicBool::new(false);
/// A tray click that came before READY; the window opens as soon as it is ready.
static PENDING_OPEN: AtomicBool = AtomicBool::new(false);
/// Just shown: wait for one painted frame, then uncloak.
static AWAIT_FRAME: AtomicBool = AtomicBool::new(false);
static UNCLOAK_NOW: AtomicBool = AtomicBool::new(false);
/// The tray icon could not be made: the window stays open and its close button quits.
static NO_TRAY: AtomicBool = AtomicBool::new(false);
/// A tray click that follows a click-away hide must not reopen the window.
const REOPEN_GUARD: Duration = Duration::from_millis(400);
/// Safety net: a shown window is uncloaked after this long even without a painted frame.
const UNCLOAK_AFTER: Duration = Duration::from_millis(300);
/// After the window hides, give its memory back to Windows after this long.
const TRIM_AFTER: Duration = Duration::from_secs(2);

thread_local! {
    // Tray objects are not Send; the tray handlers run on this (main) thread.
    static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
    static STARTUP_ITEM: RefCell<Option<CheckMenuItem>> = const { RefCell::new(None) };
}

fn lock(core: &Shared) -> MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(|e| e.into_inner())
}

fn hwnd() -> HWND {
    HWND(HWND_RAW.load(Ordering::Relaxed) as *mut _)
}

fn repaint() {
    if let Some(c) = CTX.get() {
        c.request_repaint();
    }
}

// ---- one reusable timer thread (no thread per show) ----

/// Things to do later. Set from any thread; done by the timer thread.
struct Due {
    uncloak: Option<Instant>,
    trim: Option<Instant>,
}

static DUE: Mutex<Due> = Mutex::new(Due { uncloak: None, trim: None });
static DUE_CHANGED: Condvar = Condvar::new();

fn schedule(f: impl FnOnce(&mut Due)) {
    let mut d = DUE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut d);
    DUE_CHANGED.notify_one();
}

fn start_timer() {
    std::thread::spawn(|| loop {
        // Never hold the lock during a Win32 call: the window thread may be
        // waiting for it in schedule() while that call waits for the window thread.
        let (uncloak, trim) = {
            let mut d = DUE.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                let now = Instant::now();
                let uncloak = d.uncloak.is_some_and(|t| t <= now);
                let trim = d.trim.is_some_and(|t| t <= now);
                if uncloak || trim {
                    if uncloak {
                        d.uncloak = None;
                    }
                    if trim {
                        d.trim = None;
                    }
                    break (uncloak, trim);
                }
                d = match [d.uncloak, d.trim].into_iter().flatten().min() {
                    Some(t) => DUE_CHANGED.wait_timeout(d, t - now).unwrap_or_else(|e| e.into_inner()).0,
                    None => DUE_CHANGED.wait(d).unwrap_or_else(|e| e.into_inner()),
                };
            }
        };
        if uncloak {
            cloak(false);
        }
        if trim && !window_visible() {
            trim_working_set();
        }
    });
}

/// Let Windows take back the pages of a hidden window. This only lowers the
/// number Task Manager shows (the working set); pages are read back in when
/// the window is opened again. It frees no memory and fixes no leak.
fn trim_working_set() {
    unsafe {
        let _ = EmptyWorkingSet(GetCurrentProcess());
    }
}

/// (PrivateUsage, WorkingSetSize) in bytes.
fn memory_info() -> Option<(usize, usize)> {
    let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, size) }
        .ok()?;
    Some((c.PrivateUsage, c.WorkingSetSize))
}

// ---- show / hide the window (plain Win32, works while egui is idle) ----

/// A cloaked window is not composited: nothing of it can flash on screen.
fn cloak(on: bool) {
    let value: i32 = on.into();
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd(), DWMWA_CLOAK, &value as *const i32 as *const _, 4);
    }
}

fn show_window() {
    if !READY.load(Ordering::Relaxed) || !SCANNED.load(Ordering::Relaxed) {
        // The first frame is not painted yet, or the first scan is still running:
        // open as soon as both are done.
        PENDING_OPEN.store(true, Ordering::Relaxed);
        repaint();
        return;
    }
    // Cloaked while it appears, uncloaked after one painted frame, so the
    // window is never on screen before its dark content is.
    cloak(true);
    AWAIT_FRAME.store(true, Ordering::Relaxed);
    // Safety net: never leave the window cloaked. Shown again: no trim.
    schedule(|d| {
        d.uncloak = Some(Instant::now() + UNCLOAK_AFTER);
        d.trim = None;
    });
    unsafe {
        let h = hwnd();
        let mut wa = RECT::default();
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut wa as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        let mut r = RECT::default();
        let _ = GetWindowRect(h, &mut r);
        let (w, hh) = (r.right - r.left, r.bottom - r.top);
        let _ = SetWindowPos(h, HWND_TOPMOST, wa.right - w - 8, wa.bottom - hh - 8, 0, 0, SWP_NOSIZE);
        let _ = ShowWindow(h, SW_SHOW);
        let _ = BringWindowToTop(h);
        let _ = SetForegroundWindow(h);
    }
    JUST_OPENED.store(true, Ordering::Relaxed);
    if let Some(c) = CTX.get() {
        c.send_viewport_cmd(egui::ViewportCommand::Focus);
        c.request_repaint();
    }
}

fn hide_window() {
    unsafe {
        let _ = ShowWindow(hwnd(), SW_HIDE);
    }
    schedule(|d| d.trim = Some(Instant::now() + TRIM_AFTER));
}

fn window_visible() -> bool {
    unsafe { IsWindowVisible(hwnd()) }.as_bool()
}

fn mouse_button_down() -> bool {
    unsafe { [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].iter().any(|k| (GetAsyncKeyState(k.0 as i32) as u16) & 0x8000 != 0) }
}

fn toggle_window() {
    if window_visible() {
        hide_window();
    } else {
        let just_hid = FOCUS_HIDE.lock().ok().and_then(|g| *g).is_some_and(|t| t.elapsed() < REOPEN_GUARD);
        if !just_hid {
            show_window();
        }
    }
}

fn quit(core: &Shared) -> ! {
    // The monitor thread releases the physical monitor handles, then we leave.
    let tx = lock(core).sender();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    if tx.send(Cmd::Quit(done_tx)).is_ok() {
        let _ = done_rx.recv_timeout(Duration::from_secs(3));
    }
    TRAY.with(|t| t.borrow_mut().take()); // removes the tray icon
    std::process::exit(0);
}

// ---- egui app ----

struct BrightnessApp {
    core: Shared,
    ui: ui::UiState,
    /// True once the window has had focus since it was opened.
    armed: bool,
}

impl eframe::App for BrightnessApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The close button only hides the window; Quit is in the tray menu.
        if ctx.input(|i| i.viewport().close_requested()) {
            if NO_TRAY.load(Ordering::Relaxed) {
                quit(&self.core);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            hide_window();
        }

        // First frame is done: hide the window again (eframe shows it after the
        // first paint) and from now on allow opening it.
        if !READY.load(Ordering::Relaxed) && FIRST_FRAME.load(Ordering::Relaxed) {
            hide_window(); // also trims once, a moment later
            cloak(false);
            READY.store(true, Ordering::Relaxed);
            if NO_TRAY.load(Ordering::Relaxed) {
                show_window(); // nothing else could ever open it
            }
        }
        if UNCLOAK_NOW.swap(false, Ordering::Relaxed) {
            cloak(false);
        }
        if !SCANNED.load(Ordering::Relaxed) && lock(&self.core).cache.scanned {
            SCANNED.store(true, Ordering::Relaxed);
            if READY.load(Ordering::Relaxed) && PENDING_OPEN.swap(false, Ordering::Relaxed) {
                show_window();
            }
        }

        if JUST_OPENED.swap(false, Ordering::Relaxed) {
            self.ui.reset_drag();
            self.armed = false; // focus must be seen again before click-away can hide
        }

        // Click-away: hide when focus is lost. A mouse button still held down
        // (a drag, or the click that took the focus) delays it until release.
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
        if focused {
            self.armed = true;
        } else if self.armed && window_visible() && !NO_TRAY.load(Ordering::Relaxed) {
            if self.ui.dragging() || mouse_button_down() {
                ctx.request_repaint_after(Duration::from_millis(50));
            } else {
                self.armed = false;
                if let Ok(mut g) = FOCUS_HIDE.lock() {
                    *g = Some(Instant::now());
                }
                hide_window();
            }
        }

        let on = lock(&self.core).startup;
        STARTUP_ITEM.with(|s| {
            if let Some(item) = s.borrow().as_ref() {
                if item.is_checked() != on {
                    item.set_checked(on);
                }
            }
        });
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        {
            let mut core = lock(&self.core);
            ui::draw(ui, &mut core, &mut self.ui);
        }
        // Flags for the next logic() call, which runs after this frame is painted.
        let first = !FIRST_FRAME.swap(true, Ordering::Relaxed);
        let shown = AWAIT_FRAME.swap(false, Ordering::Relaxed);
        if shown {
            UNCLOAK_NOW.store(true, Ordering::Relaxed);
        }
        if first || shown {
            ui.ctx().request_repaint();
        }
    }

    /// Dark clear colour, so no default white can ever show.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        ui::BG.to_normalized_gamma_f32()
    }
}

// ---- tray ----

/// Make the tray icon and its menu. False if Windows refused (then the window stays open).
fn create_tray(core: &Shared) -> bool {
    let menu = Menu::new();
    let open = MenuItem::new("Open", true, None);
    let startup = CheckMenuItem::new(ui::STARTUP_LABEL, true, lock(core).startup, None);
    let rescan = MenuItem::new("Re-scan", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    let _ = menu.append_items(&[&open, &startup, &rescan, &PredefinedMenuItem::separator(), &quit_item]);

    let (open_id, startup_id, rescan_id, quit_id) =
        (open.id().clone(), startup.id().clone(), rescan.id().clone(), quit_item.id().clone());

    let mut builder =
        TrayIconBuilder::new().with_menu(Box::new(menu)).with_menu_on_left_click(false).with_tooltip("Brightness Lite");
    if let Ok(icon) = Icon::from_rgba(ui::icon_rgba(), 32, 32) {
        builder = builder.with_icon(icon);
    }
    // If the taskbar is not there yet (early at login), tray-icon adds the icon
    // when it appears. An error here means the tray window itself could not be made.
    let Ok(tray) = builder.build() else { return false };
    TRAY.with(|t| *t.borrow_mut() = Some(tray));
    STARTUP_ITEM.with(|s| *s.borrow_mut() = Some(startup));

    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            toggle_window();
        }
    }));

    let c = core.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == open_id {
            show_window();
        } else if e.id == startup_id {
            // The item has already flipped itself; follow it, or flip it back on failure.
            let want = STARTUP_ITEM.with(|s| s.borrow().as_ref().map(|i| i.is_checked()));
            if let Some(want) = want {
                let mut g = lock(&c);
                g.set_startup(want);
                let actual = g.startup;
                drop(g);
                STARTUP_ITEM.with(|s| {
                    if let Some(i) = s.borrow().as_ref() {
                        i.set_checked(actual);
                    }
                });
                repaint();
            }
        } else if e.id == rescan_id {
            lock(&c).send(Cmd::Rescan);
        } else if e.id == quit_id {
            quit(&c);
        }
    }));
    true
}

// ---- hotkeys: Ctrl+Alt+Up / Down, brightness only ----

fn start_hotkeys(core: Shared) {
    std::thread::spawn(move || unsafe {
        let mods = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
        if RegisterHotKey(HWND::default(), HOTKEY_UP, mods, VK_UP.0 as u32).is_err()
            || RegisterHotKey(HWND::default(), HOTKEY_DOWN, mods, VK_DOWN.0 as u32).is_err()
        {
            lock(&core).hotkey_error = "Hotkeys Ctrl+Alt+Up/Down are already used by another program.".into();
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            if msg.message == WM_HOTKEY {
                match msg.wParam.0 as i32 {
                    HOTKEY_UP => lock(&core).send(Cmd::Hotkey(ddc::STEP)),
                    HOTKEY_DOWN => lock(&core).send(Cmd::Hotkey(-ddc::STEP)),
                    _ => {}
                }
                repaint();
            }
        }
    });
}

// ---- wake-up and monitor plug/unplug: put the saved values back ----

static EVENT_TX: OnceLock<Mutex<std::sync::mpsc::Sender<Cmd>>> = OnceLock::new();

unsafe extern "system" fn events_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let interesting = match msg {
        // A monitor was plugged in or out, or the display setup changed.
        WM_DISPLAYCHANGE => true,
        // The computer woke up.
        WM_POWERBROADCAST => matches!(wp.0 as u32, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND),
        _ => false,
    };
    if interesting {
        if let Some(tx) = EVENT_TX.get() {
            if let Ok(tx) = tx.lock() {
                let _ = tx.send(Cmd::DevicesChanged);
            }
        }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// A hidden window that only listens for those two events. The monitor thread
/// waits for things to settle before it looks.
fn start_events(core: Shared) {
    let tx = lock(&core).sender();
    let _ = EVENT_TX.set(Mutex::new(tx));
    std::thread::spawn(|| unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let class = w!("BrightnessLiteEvents");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(events_proc),
            hInstance: module.into(),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return;
        }
        // Never shown; top-level, so it receives the broadcast messages.
        if CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Brightness Lite events"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            module,
            None,
        )
        .is_err()
        {
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

// ---- --cycle-test N: open and hide the window N times, print the memory use ----

fn cycle_test_arg() -> Option<u32> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--cycle-test")?;
    args.get(i + 1).and_then(|n| n.parse().ok()).filter(|n| *n > 0)
}

/// Printed to stdout and appended to cycle-test.log in the current folder
/// (this is a GUI program, so stdout may go nowhere).
fn report(line: &str) {
    use std::io::Write;
    println!("{line}");
    let _ = std::io::stdout().flush();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("cycle-test.log") {
        let _ = writeln!(f, "{line}");
    }
}

fn report_memory(label: &str) {
    match memory_info() {
        Some((private, ws)) => report(&format!(
            "{label:>10}  PrivateUsage {:8.2} MB  WorkingSetSize {:8.2} MB",
            private as f64 / 1048576.0,
            ws as f64 / 1048576.0
        )),
        None => report(&format!("{label:>10}  GetProcessMemoryInfo failed")),
    }
}

/// Runs beside the window: waits until it can be opened, then opens and hides
/// it `n` times about 50 ms apart, like a user clicking the tray icon.
fn start_cycle_test(n: u32, core: Shared) {
    std::thread::spawn(move || {
        let t0 = Instant::now();
        while !(READY.load(Ordering::Relaxed) && SCANNED.load(Ordering::Relaxed)) {
            if t0.elapsed() > Duration::from_secs(60) {
                report("cycle-test: the window was not ready after 60 s");
                std::process::exit(2);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // Let the startup trim (2 s after the first hide) happen first.
        std::thread::sleep(TRIM_AFTER + Duration::from_secs(1));
        report(&format!("cycle-test: {n} open/hide cycles"));
        report_memory("cycle 0");
        let marks = [n / 4, n / 2, n];
        for c in 1..=n {
            show_window();
            std::thread::sleep(Duration::from_millis(50));
            hide_window();
            std::thread::sleep(Duration::from_millis(50));
            if marks.contains(&c) {
                report_memory(&format!("cycle {c}"));
            }
        }
        std::thread::sleep(TRIM_AFTER + Duration::from_secs(1));
        report_memory("+3 s idle");
        let tx = lock(&core).sender();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        if tx.send(Cmd::Quit(done_tx)).is_ok() {
            let _ = done_rx.recv_timeout(Duration::from_secs(3));
        }
        std::process::exit(0);
    });
}

/// Take the named mutex. None means another copy already holds it.
/// The returned handle must stay alive for as long as the app runs.
fn single_instance(name: PCWSTR) -> Option<HANDLE> {
    unsafe {
        let handle = CreateMutexW(None, false, name).ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            None
        } else {
            Some(handle)
        }
    }
}

pub fn run() {
    // A second copy exits quietly; it must not touch the monitors or the registry.
    let Some(_instance) = single_instance(w!("Local\\BrightnessLite-single-instance")) else {
        return;
    };

    // Starts the monitor thread; the first scan runs in the background.
    let core: Shared = state::start(repaint);
    // First guess for the height; the real height is measured on the first frames.
    let height = 150.0 + 165.0;

    let cycle_test = cycle_test_arg();
    // No depth, stencil or multisampling buffers: egui needs none of them.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Brightness Lite")
            .with_inner_size([ui::WINDOW_W, height])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_minimize_button(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_visible(false),
        depth_buffer: 0,
        stencil_buffer: 0,
        multisampling: 0,
        ..Default::default()
    };

    let result = eframe::run_native(
        "Brightness Lite",
        options,
        Box::new(move |cc| {
            ui::apply_theme(&cc.egui_ctx);
            let _ = CTX.set(cc.egui_ctx.clone());
            if let Ok(h) = cc.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    HWND_RAW.store(w.hwnd.get(), Ordering::Relaxed);
                }
            }
            // eframe shows the window after its first paint; keep that invisible.
            cloak(true);
            if !create_tray(&core) {
                NO_TRAY.store(true, Ordering::Relaxed);
                lock(&core).startup_error = "The tray icon could not be made, so the window stays open.".into();
            }
            start_hotkeys(core.clone());
            start_events(core.clone());
            start_timer();
            if let Some(n) = cycle_test {
                start_cycle_test(n, core.clone());
            }
            Ok(Box::new(BrightnessApp { core: core.clone(), ui: ui::UiState::default(), armed: false }))
        }),
    );
    if let Err(e) = result {
        eprintln!("window error: {e}");
        if cycle_test.is_some() {
            report(&format!("cycle-test: the window could not be created: {e}"));
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_instance_is_refused() {
        let name = w!("Local\\BrightnessLite-test-mutex");
        let first = single_instance(name);
        assert!(first.is_some());
        assert!(single_instance(name).is_none());
    }
}
