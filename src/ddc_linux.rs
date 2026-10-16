//! Linux DDC/CI backend: talks to the monitor over /dev/i2c-N (address 0x37)
//! using the wire format in `ddcci.rs`. Monitors are found through the DRM
//! connectors in /sys/class/drm. Only VCP 0x10 and 0x12 are ever written.

use super::*;
use crate::ddcci::{self, ADDRESS, REPLY_LEN, VCP_BRIGHTNESS, VCP_CONTRAST};
use crate::edid;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::Duration;

const I2C_SLAVE: u32 = 0x0703;
const I2C_SLAVE_FORCE: u32 = 0x0706;
/// DDC/CI needs a pause between a request and its reply, and between requests.
const WAIT_MS: u64 = 50;
/// Where the monitor's EDID lives on the I2C bus.
const EDID_ADDRESS: u16 = 0x50;

fn code_of(s: Setting) -> u8 {
    match s {
        Setting::Brightness => VCP_BRIGHTNESS,
        Setting::Contrast => VCP_CONTRAST,
    }
}

fn open_bus(bus: u32, address: u16) -> io::Result<File> {
    let f = OpenOptions::new().read(true).write(true).open(format!("/dev/i2c-{bus}"))?;
    let fd = f.as_raw_fd();
    let mut r = unsafe { libc::ioctl(fd, I2C_SLAVE as _, address as libc::c_ulong) };
    if r < 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EBUSY) {
            return Err(e);
        }
        // A kernel driver already claimed the address; the monitor still answers.
        r = unsafe { libc::ioctl(fd, I2C_SLAVE_FORCE as _, address as libc::c_ulong) };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(f)
}

fn pause() {
    std::thread::sleep(Duration::from_millis(WAIT_MS));
}

/// One "get VCP" round trip: (current, maximum).
fn get_raw(bus: u32, code: u8) -> io::Result<(u32, u32)> {
    let mut f = open_bus(bus, ADDRESS)?;
    f.write_all(&ddcci::get_request(code))?;
    pause();
    let mut buf = [0u8; REPLY_LEN];
    let n = f.read(&mut buf)?;
    ddcci::parse_get_reply(&buf[..n], code)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))
}

/// One "set VCP" write.
fn put_raw(bus: u32, code: u8, value: u32) -> io::Result<()> {
    let mut f = open_bus(bus, ADDRESS)?;
    f.write_all(&ddcci::set_request(code, value.min(u16::MAX as u32) as u16))?;
    pause();
    Ok(())
}

/// Read the first EDID block straight from the I2C bus (for drivers whose
/// DRM connector has no EDID file or no `ddc` link).
fn read_edid_i2c(bus: u32) -> Option<Vec<u8>> {
    let mut f = open_bus(bus, EDID_ADDRESS).ok()?;
    f.write_all(&[0]).ok()?;
    let mut buf = vec![0u8; 128];
    f.read_exact(&mut buf).ok()?;
    edid::parse(&buf).map(|_| buf)
}

/// True if /dev/i2c-N exists but this user may not open it.
fn i2c_denied(bus: u32) -> bool {
    matches!(open_bus(bus, ADDRESS), Err(e) if e.kind() == io::ErrorKind::PermissionDenied)
}

fn fail_message(label: &str, s: Setting, what: &str, bus: u32, denied: bool) -> String {
    if denied {
        format!("{label} {}: {what} failed. You are not allowed to use /dev/i2c-{bus}. Add a udev rule or join the i2c group (see the README).", s.name())
    } else {
        format!(
            "{label} {}: {what} failed, no DDC/CI response. The monitor may be off or unplugged, or DDC/CI may be off in the monitor's menu.",
            s.name()
        )
    }
}

/// State of one setting on one monitor.
#[derive(Clone, Default)]
pub struct Channel {
    max: u32,
    pub percent: Option<u32>,
    pub error: Option<String>,
}

pub struct Monitor {
    /// 1, 2, 3 ... in connector order.
    pub display_number: Option<u32>,
    /// Stable id of this physical monitor (its EDID model and serial).
    pub key: Option<String>,
    /// Model name from the EDID.
    pub name: Option<String>,
    /// Always None on Linux (no screen position, so no Identify).
    pub rect: Option<(i32, i32, i32, i32)>,
    bus: u32,
    channels: [Channel; 2],
}

impl Monitor {
    pub fn label(&self) -> String {
        label_for(self.display_number)
    }

    pub fn percent(&self, s: Setting) -> Option<u32> {
        self.channels[s.index()].percent
    }

    pub fn errors(&self) -> Vec<String> {
        self.channels.iter().filter_map(|c| c.error.clone()).collect()
    }

    fn fail_text(&self, s: Setting, what: &str) -> String {
        fail_message(&self.label(), s, what, self.bus, i2c_denied(self.bus))
    }

    /// Set a value 0-100. Retries up to MAX_ATTEMPTS.
    pub fn set(&mut self, s: Setting, percent: u32) -> Result<(), String> {
        let percent = clamp(percent as i64);
        let raw = percent_to_raw(percent, 0, self.channels[s.index()].max);
        for attempt in 1..=MAX_ATTEMPTS {
            if put_raw(self.bus, code_of(s), raw).is_ok() {
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
            if let Ok((cur, max)) = get_raw(self.bus, code_of(s)) {
                if max > 0 {
                    let ch = &mut self.channels[s.index()];
                    ch.max = max;
                    ch.percent = Some(raw_to_percent(cur, 0, max));
                    ch.error = None;
                    return;
                }
            }
            if attempt < MAX_ATTEMPTS {
                std::thread::sleep(Duration::from_millis(RETRY_DELAY_MS));
            }
        }
        self.channels[s.index()].error = Some(self.fail_text(s, "read"));
    }
}

/// The connected monitors (nothing to release on Linux: the bus is opened per call).
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

/// A DRM connector that has a monitor plugged in.
#[derive(Debug, Clone, PartialEq)]
struct Connector {
    /// "card0-HDMI-A-1", "card0-DP-2" ...
    name: String,
    edid: Option<Vec<u8>>,
    bus: Option<u32>,
}

/// "card0-HDMI-A-1" -> "HDMI-A-1". Built-in panels have no DDC/CI and are skipped.
fn external_connector(dir_name: &str) -> Option<String> {
    let rest = dir_name.strip_prefix("card")?;
    let (digits, name) = rest.split_once('-')?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) || name.is_empty() {
        return None;
    }
    let up = name.to_ascii_uppercase();
    if ["EDP", "LVDS", "DSI"].iter().any(|p| up.starts_with(p)) {
        return None;
    }
    Some(name.to_string())
}

/// ".../i2c-5" -> 5
fn bus_number(link_target: &str) -> Option<u32> {
    let last = link_target.trim_end_matches('/').rsplit('/').next()?;
    last.strip_prefix("i2c-")?.parse().ok()
}

/// Order connectors the way people read them: DP-2 before DP-10.
fn natural_key(name: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut num = String::new();
    for c in name.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            if !num.is_empty() {
                out.push((std::mem::take(&mut text), num.parse().unwrap_or(0)));
                num.clear();
            }
            text.push(c);
        }
    }
    out.push((text, num.parse().unwrap_or(0)));
    out
}

/// One key per monitor. The EDID id is used; if two monitors share it (twins
/// with the same serial), the connector name is added so they stay apart.
fn unique_keys(items: &[(Option<String>, String)]) -> Vec<String> {
    items
        .iter()
        .map(|(id, connector)| match id {
            Some(id) if items.iter().filter(|(o, _)| o.as_ref() == Some(id)).count() == 1 => id.clone(),
            Some(id) => format!("{id}@{connector}"),
            None => format!("connector:{connector}"),
        })
        .collect()
}

fn list_connectors(drm: &Path) -> Vec<Connector> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(drm) else { return out };
    for e in rd.flatten() {
        let file_name = e.file_name().to_string_lossy().to_string();
        if external_connector(&file_name).is_none() {
            continue;
        }
        let name = file_name;
        let dir = e.path();
        let status = fs::read_to_string(dir.join("status")).unwrap_or_default();
        if status.trim() != "connected" {
            continue;
        }
        let data = fs::read(dir.join("edid")).ok().filter(|d| edid::parse(d).is_some());
        let bus = fs::read_link(dir.join("ddc")).ok().and_then(|l| bus_number(&l.to_string_lossy()));
        out.push(Connector { name, edid: data, bus });
    }
    out.sort_by_key(|c| natural_key(&c.name));
    out
}

/// Buses of NVIDIA's proprietary driver, which has no `ddc` link on its connectors.
fn nvidia_buses() -> Vec<u32> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/class/i2c-dev") {
        for e in rd.flatten() {
            let name = fs::read_to_string(e.path().join("name")).unwrap_or_default();
            if name.starts_with("NVIDIA i2c adapter") {
                if let Some(n) = bus_number(&e.file_name().to_string_lossy()) {
                    v.push(n);
                }
            }
        }
    }
    v.sort();
    v
}

/// Find every connected external monitor and read its brightness and contrast.
pub fn discover() -> Monitors {
    let mut notes = Vec::new();
    let mut connectors = list_connectors(Path::new("/sys/class/drm"));

    // Connectors without a ddc link: match them to an NVIDIA bus by comparing EDIDs.
    if connectors.iter().any(|c| c.bus.is_none()) {
        for bus in nvidia_buses() {
            if connectors.iter().any(|c| c.bus == Some(bus)) {
                continue;
            }
            if let Some(e) = read_edid_i2c(bus) {
                if let Some(c) = connectors.iter_mut().find(|c| c.bus.is_none() && c.edid.as_deref().and_then(|d| d.get(..128)) == Some(&e[..])) {
                    c.bus = Some(bus);
                }
            }
        }
    }

    if connectors.is_empty() {
        return Monitors { list: Vec::new(), notes };
    }
    if !Path::new("/dev").read_dir().map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().starts_with("i2c-"))).unwrap_or(false) {
        notes.push("No /dev/i2c-* devices found. Load the i2c-dev module (see the README).".into());
    }

    let parsed: Vec<Option<edid::Edid>> = connectors.iter().map(|c| c.edid.as_deref().and_then(edid::parse)).collect();
    let ids: Vec<(Option<String>, String)> =
        parsed.iter().zip(&connectors).map(|(p, c)| (p.as_ref().map(|e| e.stable_id()), c.name.clone())).collect();
    let keys = unique_keys(&ids);

    let mut list = Vec::new();
    for ((c, p), key) in connectors.iter().zip(&parsed).zip(keys) {
        let Some(bus) = c.bus else {
            notes.push(format!("Monitor on {}: no I2C bus found for it, so it cannot be controlled.", c.name));
            continue;
        };
        let blank = Channel { max: 100, ..Default::default() };
        let mut m = Monitor {
            display_number: Some(list.len() as u32 + 1),
            key: Some(key),
            name: Some(p.as_ref().map(|e| e.display_name()).unwrap_or_else(|| external_connector(&c.name).unwrap_or_else(|| c.name.clone()))),
            rect: None,
            bus,
            channels: [blank.clone(), blank],
        };
        for s in Setting::ALL {
            m.read(s);
        }
        list.push(m);
    }
    Monitors { list, notes }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_names() {
        assert_eq!(external_connector("card0-HDMI-A-1").as_deref(), Some("HDMI-A-1"));
        assert_eq!(external_connector("card1-DP-2").as_deref(), Some("DP-2"));
        assert_eq!(external_connector("card0-eDP-1"), None);
        assert_eq!(external_connector("card0-LVDS-1"), None);
        assert_eq!(external_connector("card0-DSI-1"), None);
        assert_eq!(external_connector("card0"), None);
        assert_eq!(external_connector("renderD128"), None);
        assert_eq!(external_connector("version"), None);
        assert_eq!(external_connector("cardX-DP-1"), None);
    }

    #[test]
    fn permission_failure_is_named() {
        let m = fail_message("Monitor 1", Setting::Brightness, "write", 4, true);
        assert!(m.contains("not allowed to use /dev/i2c-4"));
        let m = fail_message("Monitor 1", Setting::Brightness, "write", 4, false);
        assert!(m.contains("no DDC/CI response") && !m.contains("not allowed"));
    }

    #[test]
    fn bus_numbers() {
        assert_eq!(bus_number("../../../../i2c-5"), Some(5));
        assert_eq!(bus_number("/sys/devices/pci0000:00/i2c-12/"), Some(12));
        assert_eq!(bus_number("i2c-x"), None);
        assert_eq!(bus_number("card0"), None);
    }

    #[test]
    fn connectors_sort_naturally() {
        let mut v = vec!["DP-10", "DP-2", "HDMI-A-1", "DP-1"];
        v.sort_by_key(|n| natural_key(n));
        assert_eq!(v, vec!["DP-1", "DP-2", "DP-10", "HDMI-A-1"]);
    }

    #[test]
    fn twin_monitors_get_different_keys() {
        let k = unique_keys(&[
            (Some("del40e5-00000001".into()), "card0-DP-1".into()),
            (Some("del40e5-00000001".into()), "card0-DP-2".into()),
            (Some("gsm5b09-0000abcd".into()), "card0-HDMI-A-1".into()),
            (None, "card1-DP-1".into()),
        ]);
        assert_eq!(
            k,
            vec!["del40e5-00000001@card0-DP-1", "del40e5-00000001@card0-DP-2", "gsm5b09-0000abcd", "connector:card1-DP-1"]
        );
    }

    #[test]
    fn connectors_come_from_a_fake_sysfs() {
        let root = std::env::temp_dir().join(format!("bl-drm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let edid_bytes = edid::sample("DEL", 0x40e5, 9, Some("DELL U2720Q"));
        for (dir, status, with_edid) in [
            ("card0-DP-2", "connected", true),
            ("card0-DP-1", "disconnected", false),
            ("card0-eDP-1", "connected", true),
            ("card0-HDMI-A-1", "connected", false),
        ] {
            let d = root.join(dir);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("status"), format!("{status}\n")).unwrap();
            if with_edid {
                fs::write(d.join("edid"), &edid_bytes).unwrap();
            }
        }
        fs::create_dir_all(root.join("i2c-7")).unwrap();
        std::os::unix::fs::symlink("../i2c-7", root.join("card0-DP-2").join("ddc")).unwrap();

        let found = list_connectors(&root);
        let names: Vec<_> = found.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["card0-DP-2", "card0-HDMI-A-1"]);
        assert_eq!(found[0].bus, Some(7));
        assert_eq!(edid::parse(found[0].edid.as_deref().unwrap()).unwrap().display_name(), "DELL U2720Q");
        assert_eq!(found[1].bus, None);
        assert!(found[1].edid.is_none());
        let _ = fs::remove_dir_all(&root);
    }
}
