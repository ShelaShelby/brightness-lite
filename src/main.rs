//! Brightness Lite: tray app that sets brightness and contrast of each external
//! monitor over DDC/CI. Only brightness (0x10) and contrast (0x12) are ever written.
//! Windows: Win32 + tray-icon. Linux: /dev/i2c + StatusNotifierItem tray.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod app_linux;
#[cfg(windows)]
mod app_windows;
#[cfg(any(windows, target_os = "linux"))]
mod autostart;
mod cache;
mod ddc;
#[cfg(target_os = "linux")]
mod ddcci;
mod edid;
#[cfg(target_os = "linux")]
mod hotkey_x11;
#[cfg(windows)]
mod identify;
mod order;
#[cfg(any(windows, target_os = "linux"))]
mod state;
mod store;
#[cfg(any(windows, target_os = "linux"))]
mod ui;

#[cfg(not(any(windows, target_os = "linux")))]
fn main() {
    eprintln!("Brightness Lite runs on Windows and Linux only.");
}

#[cfg(windows)]
fn main() {
    app_windows::run();
}

#[cfg(target_os = "linux")]
fn main() {
    app_linux::run();
}
