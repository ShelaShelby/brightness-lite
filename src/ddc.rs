//! DDC/CI access: monitor discovery, brightness (VCP 0x10) and contrast
//! (VCP 0x12) read/write via the high-level monitor API, retries and handle
//! cleanup. Nothing else is ever written to a monitor.

/// Lowest and highest brightness shown to the user.
pub const MIN_PERCENT: u32 = 0;
pub const MAX_PERCENT: u32 = 100;
/// Hotkey step, in percentage points.
pub const STEP: i32 = 5;
/// Total attempts per DDC/CI call (first try + retries).
pub const MAX_ATTEMPTS: u32 = 3;
/// Pause between attempts.
pub const RETRY_DELAY_MS: u64 = 150;

/// Clamp any value into 0-100.
pub fn clamp(v: i64) -> u32 {
    v.clamp(MIN_PERCENT as i64, MAX_PERCENT as i64) as u32
}

/// Apply a relative change and clamp.
pub fn stepped(current: u32, delta: i32) -> u32 {
    clamp(current as i64 + delta as i64)
}

/// Map 0-100 onto the monitor's own raw range.
pub fn percent_to_raw(percent: u32, min: u32, max: u32) -> u32 {
    let p = clamp(percent as i64) as u64;
    let span = max.saturating_sub(min) as u64;
    min + ((p * span + 50) / 100) as u32
}

/// Map a raw monitor value onto 0-100.
pub fn raw_to_percent(raw: u32, min: u32, max: u32) -> u32 {
    if max <= min {
        return 0;
    }
    let raw = raw.clamp(min, max);
    clamp((((raw - min) as u64 * 100 + (max - min) as u64 / 2) / (max - min) as u64) as i64)
}

/// "\\.\DISPLAY3" -> Some(3). This is the number Windows shows in
/// Settings > System > Display. Anything else -> None (never guessed).
pub fn parse_display_number(gdi_name: &str) -> Option<u32> {
    let upper = gdi_name.trim_end_matches('\0').to_ascii_uppercase();
    let rest = upper.strip_prefix("\\\\.\\DISPLAY")?;
    if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok().filter(|n| *n > 0)
}

/// Label shown above a monitor's sliders.
pub fn label_for(display_number: Option<u32>) -> String {
    match display_number {
        Some(n) => format!("Monitor {n}"),
        None => "Monitor (unknown)".to_string(),
    }
}

/// Sort key: Windows display number first, unknown ones last, then position.
pub fn order_key(display_number: Option<u32>, left: i32, top: i32) -> (u32, i32, i32) {
    (display_number.unwrap_or(u32::MAX), left, top)
}

/// The only two settings this app ever touches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Setting {
    /// VCP 0x10
    Brightness,
    /// VCP 0x12
    Contrast,
}

impl Setting {
    pub const ALL: [Setting; 2] = [Setting::Brightness, Setting::Contrast];

    pub fn index(self) -> usize {
        match self {
            Setting::Brightness => 0,
            Setting::Contrast => 1,
        }
    }

    #[cfg(test)]
    pub fn from_index(i: usize) -> Setting {
        if i == 0 {
            Setting::Brightness
        } else {
            Setting::Contrast
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Setting::Brightness => "brightness",
            Setting::Contrast => "contrast",
        }
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::*;
    use std::time::Duration;
    use windows::Win32::Devices::Display::{
        DestroyPhysicalMonitors, GetMonitorBrightness, GetMonitorContrast,
        GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR,
        SetMonitorBrightness, SetMonitorContrast, PHYSICAL_MONITOR,
    };
    use windows::Win32::Foundation::{BOOL, HANDLE, LPARAM, RECT, TRUE};
    use windows::core::PCWSTR;

    /// Ask EnumDisplayDevices for the monitor's device interface path.
    const EDD_GET_DEVICE_INTERFACE_NAME: u32 = 1;
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, DISPLAY_DEVICEW,
        HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    };

    /// State of one setting on one monitor.
    #[derive(Clone, Default)]
    pub struct Channel {
        min: u32,
        max: u32,
        /// Current value as 0-100, or None if the monitor never answered.
        pub percent: Option<u32>,
        /// Set when DDC/CI did not answer after all attempts.
        pub error: Option<String>,
    }

    pub struct Monitor {
        /// Windows display number (Settings > System > Display), if known.
        pub display_number: Option<u32>,
        /// Stable id of this physical monitor (its device interface path).
        pub key: Option<String>,
        /// Position on the virtual desktop (left, top, right, bottom), if known.
        pub rect: Option<(i32, i32, i32, i32)>,
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
        pub fn errors(&self) -> Vec<String> {
            self.channels.iter().filter_map(|c| c.error.clone()).collect()
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
            self.channels[s.index()].error = Some(self.fail_text(s, "read"));
        }
    }

    /// Owns the physical monitor handles and releases them on drop.
    pub struct Monitors {
        pub list: Vec<Monitor>,
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
    fn display_info(h: HMONITOR) -> (Option<u32>, Option<String>, Option<(i32, i32, i32, i32)>) {
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
        let ok = unsafe {
            EnumDisplayDevicesW(PCWSTR(mi.szDevice.as_ptr()), 0, &mut dd, EDD_GET_DEVICE_INTERFACE_NAME)
        }
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
            let _ = EnumDisplayMonitors(
                HDC::default(),
                None,
                Some(enum_proc),
                LPARAM(&mut handles as *mut _ as isize),
            );
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
        Monitors { list }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_limits_to_0_100() {
        assert_eq!(clamp(-20), 0);
        assert_eq!(clamp(0), 0);
        assert_eq!(clamp(57), 57);
        assert_eq!(clamp(100), 100);
        assert_eq!(clamp(101), 100);
        assert_eq!(clamp(i64::MAX), 100);
        assert_eq!(clamp(i64::MIN), 0);
    }

    #[test]
    fn stepping_is_clamped() {
        assert_eq!(stepped(98, STEP), 100);
        assert_eq!(stepped(3, -STEP), 0);
        assert_eq!(stepped(50, STEP), 55);
        assert_eq!(stepped(50, -STEP), 45);
    }

    #[test]
    fn display_number_from_gdi_name() {
        assert_eq!(parse_display_number("\\\\.\\DISPLAY1"), Some(1));
        assert_eq!(parse_display_number("\\\\.\\display12"), Some(12));
        assert_eq!(parse_display_number("\\\\.\\DISPLAY2\0\0\0"), Some(2));
        assert_eq!(parse_display_number("\\\\.\\DISPLAY"), None);
        assert_eq!(parse_display_number("\\\\.\\DISPLAY0"), None);
        assert_eq!(parse_display_number("\\\\.\\DISPLAYV1"), None);
        assert_eq!(parse_display_number("DISPLAY1"), None);
        assert_eq!(parse_display_number(""), None);
    }

    #[test]
    fn labels_use_display_number_or_unknown() {
        assert_eq!(label_for(Some(2)), "Monitor 2");
        assert_eq!(label_for(None), "Monitor (unknown)");
    }

    #[test]
    fn order_follows_display_number_not_enumeration() {
        // Enumeration returned display 2 first; the list must still start with 1.
        let mut v = vec![(Some(2), 1920, 0), (None, 0, 0), (Some(1), 0, 0), (Some(3), -1920, 0)];
        v.sort_by_key(|(n, l, t)| order_key(*n, *l, *t));
        let numbers: Vec<_> = v.iter().map(|x| x.0).collect();
        assert_eq!(numbers, vec![Some(1), Some(2), Some(3), None]);
    }

    #[test]
    fn setting_index_roundtrip() {
        for s in Setting::ALL {
            assert_eq!(Setting::from_index(s.index()), s);
        }
    }

    #[test]
    fn raw_mapping_roundtrip() {
        assert_eq!(percent_to_raw(0, 0, 100), 0);
        assert_eq!(percent_to_raw(100, 0, 100), 100);
        assert_eq!(percent_to_raw(200, 0, 100), 100);
        assert_eq!(percent_to_raw(50, 10, 90), 50);
        assert_eq!(raw_to_percent(50, 10, 90), 50);
        assert_eq!(raw_to_percent(500, 10, 90), 100);
        assert_eq!(raw_to_percent(5, 0, 0), 0);
    }
}
