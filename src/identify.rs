//! "Identify": shows the monitor's Windows display number in a big box in the
//! middle of that monitor for a moment. Pure display, nothing is sent to the monitor.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    SelectObject, SetBkMode, SetTextColor, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_QUALITY,
    DT_CENTER, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_BOLD, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

const SIZE: i32 = 240;
const SHOW_MS: u32 = 1500;
const TIMER_ID: usize = 1;

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut r = RECT::default();
            let _ = GetClientRect(hwnd, &mut r);
            let bg = CreateSolidBrush(COLORREF(0x001c_1816)); // #16181c
            FillRect(hdc, &r, bg);
            let _ = DeleteObject(bg);
            let font = CreateFontW(
                -150, 0, 0, 0, FW_BOLD.0 as i32, 0, 0, 0, DEFAULT_CHARSET.0 as u32,
                OUT_DEFAULT_PRECIS.0 as u32, CLIP_DEFAULT_PRECIS.0 as u32, DEFAULT_QUALITY.0 as u32,
                FF_DONTCARE.0 as u32, w!("Segoe UI"),
            );
            let old = SelectObject(hdc, font);
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, COLORREF(0x00ff_8d4c)); // #4c8dff (BGR)
            let mut buf = [0u16; 16];
            let n = GetWindowTextW(hwnd, &mut buf) as usize;
            DrawTextW(hdc, &mut buf[..n], &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            SelectObject(hdc, old);
            let _ = DeleteObject(font);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_TIMER => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Show `text` centred on the monitor at `rect` (left, top, right, bottom in
/// desktop pixels) for about a second and a half. Runs on its own thread, so
/// the caller is never blocked, and the box never takes keyboard focus.
pub fn show(text: String, rect: (i32, i32, i32, i32)) {
    std::thread::spawn(move || unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("BrightnessLiteIdentify");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: hinst.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc); // fails harmlessly if it is already registered
        let x = rect.0 + (rect.2 - rect.0 - SIZE) / 2;
        let y = rect.1 + (rect.3 - rect.1 - SIZE) / 2;
        let t: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            PCWSTR(t.as_ptr()),
            WS_POPUP | WS_VISIBLE,
            x, y, SIZE, SIZE,
            None, None, hinst, None,
        ) else {
            return;
        };
        SetTimer(hwnd, TIMER_ID, SHOW_MS, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}
