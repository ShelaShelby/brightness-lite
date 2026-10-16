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

/// Registry path of the EDID for a monitor's device interface path.
/// The interface path looks like `\\?\display#del40e5#5&2a5c7f4d&0&uid4352#{guid}`; the EDID
/// is under `HKLM\SYSTEM\CurrentControlSet\Enum\DISPLAY\DEL40E5\5&2a5c7f4d&0&uid4352\Device Parameters`.
pub fn edid_registry_subkey(interface_path: &str) -> Option<String> {
    let rest = interface_path.strip_prefix("\\\\?\\")?;
    let mut parts = rest.split('#');
    let class = parts.next()?;
    let model = parts.next()?;
    let instance = parts.next()?;
    let ok = |p: &str| !p.is_empty() && !p.contains(['\\', '/']);
    if !class.eq_ignore_ascii_case("display") || !ok(model) || !ok(instance) {
        return None;
    }
    Some(format!(
        "SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\{}\\{}\\Device Parameters",
        model.to_ascii_uppercase(),
        instance
    ))
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
#[path = "ddc_windows.rs"]
mod backend;
#[cfg(target_os = "linux")]
#[path = "ddc_linux.rs"]
mod backend;

#[cfg(any(windows, target_os = "linux"))]
pub use backend::*;

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
    fn edid_subkey_from_interface_path() {
        let p = "\\\\?\\display#del40e5#5&2a5c7f4d&0&uid4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
        assert_eq!(
            edid_registry_subkey(p).as_deref(),
            Some("SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\DEL40E5\\5&2a5c7f4d&0&uid4352\\Device Parameters")
        );
        assert_eq!(edid_registry_subkey("display#a#b"), None);
        assert_eq!(edid_registry_subkey("\\\\?\\usb#a#b#c"), None);
        assert_eq!(edid_registry_subkey("\\\\?\\display#..\\x#b#c"), None);
        assert_eq!(edid_registry_subkey("\\\\?\\display##b#c"), None);
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
