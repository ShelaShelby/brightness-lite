//! Windows DDC/CI backend: high-level monitor API (dxva2).

use super::*;
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, GetMonitorBrightness, GetMonitorCapabilities, GetMonitorContrast,
    GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR, SetMonitorBrightness, SetMonitorContrast,
    PHYSICAL_MONITOR,
};
use windows::Win32::Foundation::{BOOL, HANDLE, LPARAM, RECT, TRUE};
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE,
};

/// Ask EnumDisplayDevices for the monitor's device interface path.
const EDD_GET_DEVICE_INTERFACE_NAME: u32 = 1;
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, DISPLAY_DEVICEW, HDC, HMONITOR, MONITORINFO,
    MONITORINFOEXW,
};

/// Left, top, right, bottom in desktop pixels.
type Rect = (i32, i32, i32, i32);

/// State of one setting on one monitor.
#[derive(Clone, Default)]
pub struct Channel {
    min: u32,
    max: u32,
    /// Current value as 0-100, or None if the monitor never answered.
    pub percent: Option<u32>,
    /// Set when DDC/CI did not answer after all attempts.
    pub error: Option<String>,
    /// The monitor's capabilities say it does not have this setting.
    pub unsupported: bool,
}

pub struct Monitor {
    /// Windows display number (Settings > System > Display), if known.
    pub display_number: Option<u32>,
    /// Stable id of this physical monitor (its device interface path).
    pub key: Option<String>,
    /// Model name from the monitor's EDID, if Windows has it.
    pub name: Option<String>,
    /// Position on the virtual desktop (left, top, right, bottom), if known.
    pub rect: Option<Rect>,
    raw: PHYSICAL_MONITOR,
    channels: [Channel; 2],
}

fn get(h: HANDLE, s: Setting, min: &mut u32, cur: &mut u32, max: &mut u32) -> bool {
    unsafe {
        match s {
            Setting::Brightness => GetMonitorBrightness(h, min, cur, max) != 0,
            Setting::Contrast => GetMonitorContrast(h, min, cur, max) != 0,
        }
    }
}

fn put(h: HANDLE, s: Setting, raw: u32) -> bool {
    unsafe {
        match s {
            Setting::Brightness => SetMonitorBrightness(h, raw) != 0,
            Setting::Contrast => SetMonitorContrast(h, raw) != 0,
        }
    }
}

impl Monitor {
    pub fn label(&self) -> String {
        label_for(self.display_number)
    }

    pub fn percent(&self, s: Setting) -> Option<u32> {
        self.channels[s.index()].percent
    }

    /// Error messages for every setting that failed on this monitor.
    /// The error of one setting, if it failed.
    pub fn error(&self, s: Setting) -> Option<String> {
        self.channels[s.index()].error.clone()
    }

    /// False if the monitor said it does not have this setting.
    pub fn supported(&self, s: Setting) -> bool {
        !self.channels[s.index()].unsupported
    }

    fn fail_text(&self, s: Setting, what: &str) -> String {
        format!(
            "{} {}: {} failed, no DDC/CI response. The monitor may be off or unplugged, or DDC/CI may be off in the monitor's menu.",
            self.label(),
            s.name(),
            what
        )
    }

    /// Set a value 0-100. Retries up to MAX_ATTEMPTS.
    pub fn set(&mut self, s: Setting, percent: u32) -> Result<(), String> {
        let percent = clamp(percent as i64);
        let ch = &self.channels[s.index()];
        let raw = percent_to_raw(percent, ch.min, ch.max);
        for attempt in 1..=MAX_ATTEMPTS {
            if put(self.raw.hPhysicalMonitor, s, raw) {
                let ch = &mut self.channels[s.index()];
                ch.percent = Some(percent);
                ch.error = None;
                return Ok(());
            }
            if attempt < MAX_ATTEMPTS {
                std::thread::sleep(Duration::from_millis(RETRY_DELAY_MS));
            }
        }
        let msg = self.fail_text(s, "write");
        self.channels[s.index()].error = Some(msg.clone());
        Err(msg)
    }

    /// Read range and current value. Retries up to MAX_ATTEMPTS.
    fn read(&mut self, s: Setting) {
        for attempt in 1..=MAX_ATTEMPTS {
            let (mut min, mut cur, mut max) = (0u32, 0u32, 0u32);
            if get(self.raw.hPhysicalMonitor, s, &mut min, &mut cur, &mut max) && max > min {
                let ch = &mut self.channels[s.index()];
                ch.min = min;
                ch.max = max;
                ch.percent = Some(raw_to_percent(cur, min, max));
                ch.error = None;
                return;
            }
            if attempt < MAX_ATTEMPTS {
                std::thread::sleep(Duration::from_millis(RETRY_DELAY_MS));
            }
        }
        // Every read failed. If the monitor's capabilities list this setting as
        // missing, hide it; otherwise it is a real error (monitor off, DDC/CI off).
        let (mut caps, mut temps) = (0u32, 0u32);
        if unsafe { GetMonitorCapabilities(self.raw.hPhysicalMonitor, &mut caps, &mut temps) } != 0
            && caps_lack(caps, s)
        {
            self.channels[s.index()].unsupported = true;
            return;
        }
        self.channels[s.index()].error = Some(self.fail_text(s, "read"));
    }
}

/// Owns the physical monitor handles and releases them on drop.
pub struct Monitors {
    pub list: Vec<Monitor>,
    /// Hints for the status lines (why nothing was found, and so on).
    pub notes: Vec<String>,
}

impl Monitors {
    pub fn empty() -> Monitors {
        Monitors { list: Vec::new(), notes: Vec::new() }
    }
}

/// The monitor's EDID as Windows keeps it in the registry (readable without admin rights).
fn read_edid(key: &str) -> Option<Vec<u8>> {
    let sub: Vec<u16> = edid_registry_subkey(key)?.encode_utf16().chain(std::iter::once(0)).collect();
    let value: Vec<u16> = "EDID".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut h = HKEY::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(sub.as_ptr()), 0, KEY_QUERY_VALUE, &mut h).is_err() {
            return None;
        }
        let mut size = 0u32;
        let mut out = None;
        if RegQueryValueExW(h, PCWSTR(value.as_ptr()), None, None, None, Some(&mut size)).is_ok()
            && size > 0
            && size <= 4096
        {
            let mut buf = vec![0u8; size as usize];
            if RegQueryValueExW(h, PCWSTR(value.as_ptr()), None, None, Some(buf.as_mut_ptr()), Some(&mut size)).is_ok()
            {
                buf.truncate(size as usize);
                out = Some(buf);
            }
        }
        let _ = RegCloseKey(h);
        out
    }
}

/// "Dell U2720Q" for a device interface path, if the registry has its EDID.
fn model_name(key: &str) -> Option<String> {
    crate::edid::parse(&read_edid(key)?).map(|e| e.display_name())
}

impl Drop for Monitors {
    fn drop(&mut self) {
        let raws: Vec<PHYSICAL_MONITOR> = self.list.iter().map(|m| m.raw).collect();
        if !raws.is_empty() {
            unsafe {
                let _ = DestroyPhysicalMonitors(&raws);
            }
        }
    }
}

unsafe extern "system" fn enum_proc(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let handles = &mut *(data.0 as *mut Vec<HMONITOR>);
    handles.push(h);
    TRUE
}

/// What Windows says about this display: number, stable key, position.
/// Matched through the HMONITOR itself, never through enumeration order.
fn display_info(h: HMONITOR) -> (Option<u32>, Option<String>, Option<Rect>) {
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if !unsafe { GetMonitorInfoW(h, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO) }.as_bool() {
        return (None, None, None);
    }
    let name = String::from_utf16_lossy(&mi.szDevice);
    let number = parse_display_number(&name);
    let r = mi.monitorInfo.rcMonitor;
    let rect = Some((r.left, r.top, r.right, r.bottom));

    let mut dd = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
    let ok = unsafe { EnumDisplayDevicesW(PCWSTR(mi.szDevice.as_ptr()), 0, &mut dd, EDD_GET_DEVICE_INTERFACE_NAME) }
        .as_bool();
    let path = String::from_utf16_lossy(&dd.DeviceID).trim_end_matches('\0').to_ascii_lowercase();
    let key = if ok && !path.is_empty() { Some(path) } else { None };
    (number, key, rect)
}

/// Find every connected monitor, ordered by Windows display number, and
/// read its current brightness and contrast.
pub fn discover() -> Monitors {
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(HDC::default(), None, Some(enum_proc), LPARAM(&mut handles as *mut _ as isize));
    }
    let mut list = Vec::new();
    for h in handles {
        let mut count = 0u32;
        if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(h, &mut count) }.is_err() || count == 0 {
            continue;
        }
        let mut arr = vec![PHYSICAL_MONITOR::default(); count as usize];
        if unsafe { GetPhysicalMonitorsFromHMONITOR(h, &mut arr) }.is_err() {
            continue;
        }
        let (number, key, rect) = display_info(h);
        // Several physical monitors behind one display (clone mode): there is
        // no way to tell which is which, so none of them is labelled or saved.
        let ambiguous = arr.len() > 1;
        for raw in arr {
            let blank = Channel { min: 0, max: 100, ..Default::default() };
            let mut m = Monitor {
                display_number: if ambiguous { None } else { number },
                key: if ambiguous { None } else { key.clone() },
                name: if ambiguous { None } else { key.as_deref().and_then(model_name) },
                rect: if ambiguous { None } else { rect },
                raw,
                channels: [blank.clone(), blank],
            };
            for s in Setting::ALL {
                m.read(s);
            }
            list.push(m);
        }
    }
    list.sort_by_key(|m| {
        let (l, t) = m.rect.map(|r| (r.0, r.1)).unwrap_or((0, 0));
        order_key(m.display_number, l, t)
    });
    Monitors { list, notes: Vec::new() }
}
