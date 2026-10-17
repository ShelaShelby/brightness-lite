//! Loads and saves the last brightness and contrast of each monitor as JSON.
//! Each saved entry carries the monitor's key (its Windows device path, or its
//! EDID id on Linux), so a restore goes to the same physical monitor after a restart.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Values used for a monitor that has no saved value.
pub const DEFAULT_BRIGHTNESS: u32 = 50;
pub const DEFAULT_CONTRAST: u32 = 50;
const DIR_NAME: &str = "brightness-lite";
const FILE_NAME: &str = "settings.json";

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct SavedMonitor {
    pub key: String,
    pub brightness: u32,
    pub contrast: u32,
}

impl SavedMonitor {
    /// Brightness clamped to 0-100.
    pub fn brightness_pct(&self) -> u32 {
        crate::ddc::clamp(self.brightness as i64)
    }

    /// Contrast clamped to 0-100.
    pub fn contrast_pct(&self) -> u32 {
        crate::ddc::clamp(self.contrast as i64)
    }
}

#[derive(Serialize, Debug, PartialEq, Clone, Default)]
pub struct Settings {
    /// One entry per monitor ever saved, including ones not connected now.
    /// Files from before keys existed have no such list and load as empty.
    pub monitors: Vec<SavedMonitor>,
    /// The user's "Start with Windows" choice. None = never decided (first run: on).
    pub autostart: Option<bool>,
    /// The order of the cards in the window, as monitor keys. Monitors not
    /// listed go at the end. Keys of monitors not connected now are kept.
    pub order: Vec<String>,
}

impl Settings {
    pub fn find(&self, key: &str) -> Option<&SavedMonitor> {
        self.monitors.iter().find(|m| m.key == key)
    }
}

/// What the app knows about one connected monitor when saving.
pub struct Current<'a> {
    pub key: &'a str,
    pub brightness: Option<u32>,
    pub contrast: Option<u32>,
}

/// Settings to write: connected monitors replace their entry; a value that
/// could not be read keeps its old saved number (or the default); entries of
/// monitors that are not connected right now are kept untouched.
pub fn merged(old: &Settings, current: &[Current]) -> Settings {
    let mut out = old.clone();
    for c in current {
        let prev = old.find(c.key);
        let entry = SavedMonitor {
            key: c.key.to_string(),
            brightness: c.brightness.or(prev.map(|p| p.brightness)).unwrap_or(DEFAULT_BRIGHTNESS),
            contrast: c.contrast.or(prev.map(|p| p.contrast)).unwrap_or(DEFAULT_CONTRAST),
        };
        match out.monitors.iter_mut().find(|m| m.key == c.key) {
            Some(slot) => *slot = entry,
            None => out.monitors.push(entry),
        }
    }
    out
}

/// Argument the autostart entry passes, so a login start can be told apart.
pub const AUTOSTART_ARG: &str = "--autostart";

/// The "start with Windows" registry value: the full exe path in quotes, then the argument.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn startup_command(exe: &Path) -> String {
    format!("\"{}\" {}", exe.display(), AUTOSTART_ARG)
}

/// Windows: %APPDATA%\brightness-lite\settings.json
/// Linux: $XDG_CONFIG_HOME (or ~/.config)/brightness-lite/settings.json
pub fn default_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(DIR_NAME).join(FILE_NAME))
    }
    #[cfg(not(windows))]
    {
        config_base(std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME"))
            .map(|b| b.join(DIR_NAME).join(FILE_NAME))
    }
}

/// The per-user config folder on Linux. A relative XDG_CONFIG_HOME is ignored, as the spec says.
#[cfg_attr(windows, allow(dead_code))]
pub fn config_base(xdg: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let xdg = xdg.map(PathBuf::from).filter(|p| p.is_absolute());
    xdg.or_else(|| home.map(PathBuf::from).filter(|p| p.is_absolute()).map(|h| h.join(".config")))
}

/// Read settings from parsed JSON. Each part is checked on its own: a bad
/// part is dropped, the good parts are kept. Anything else gives defaults.
fn from_json(v: Value) -> Settings {
    let Value::Object(map) = v else { return Settings::default() };
    let list = |name: &str| map.get(name).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut monitors: Vec<SavedMonitor> = Vec::new();
    for m in list("monitors").into_iter().filter_map(|e| serde_json::from_value::<SavedMonitor>(e).ok()) {
        if !m.key.is_empty() && monitors.iter().all(|o| o.key != m.key) {
            monitors.push(m);
        }
    }
    Settings {
        monitors,
        autostart: map.get("autostart").and_then(Value::as_bool),
        order: crate::order::clean(list("order").into_iter().filter_map(|e| e.as_str().map(String::from))),
    }
}

/// Missing, empty or corrupt file gives defaults. Never panics.
pub fn load(path: &Path) -> Settings {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).map(from_json).unwrap_or_default()
}

/// The window thread and the monitor thread both save: one at a time, and a
/// read-change-write is never split by another save.
static SAVING: Mutex<()> = Mutex::new(());

/// Write to a temp file, flush it to disk, then rename it over the real one,
/// so a crash or power cut can never leave a half-written settings file.
fn write_file(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    let _guard = SAVING.lock().unwrap_or_else(|e| e.into_inner());
    write_file(path, settings)
}

/// Load, change and save in one step, so two threads can never undo each other's change.
pub fn update(path: &Path, change: impl FnOnce(&mut Settings)) -> std::io::Result<()> {
    let _guard = SAVING.lock().unwrap_or_else(|e| e.into_inner());
    let mut settings = load(path);
    change(&mut settings);
    write_file(path, &settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("brightness-lite-test-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        d.join("settings.json")
    }

    fn entry(key: &str, b: u32, c: u32) -> SavedMonitor {
        SavedMonitor { key: key.into(), brightness: b, contrast: c }
    }

    #[test]
    fn save_then_load_roundtrip_both_values() {
        let p = temp("roundtrip");
        let s = Settings {
            monitors: vec![entry("a", 10, 0), entry("b", 80, 45), entry("c", 100, 100)],
            ..Default::default()
        };
        save(&p, &s).unwrap();
        assert_eq!(load(&p), s);
    }

    #[test]
    fn save_leaves_no_temp_file_and_overwrites() {
        let p = temp("atomic");
        save(&p, &Settings { monitors: vec![entry("a", 1, 2)], ..Default::default() }).unwrap();
        save(&p, &Settings { monitors: vec![entry("a", 3, 4)], ..Default::default() }).unwrap();
        assert!(!p.with_extension("json.tmp").exists());
        assert_eq!(load(&p).find("a"), Some(&entry("a", 3, 4)));
    }

    #[test]
    fn missing_file_gives_defaults() {
        let p = temp("missing");
        assert_eq!(load(&p), Settings::default());
    }

    #[test]
    fn corrupt_file_gives_defaults() {
        let p = temp("corrupt");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        for bad in [
            "{ not json !!",
            "",
            "{\"monitors\": \"oops\"}",
            "{\"monitors\": [{\"key\":\"a\",\"brightness\":-5,\"contrast\":1}]}",
        ] {
            std::fs::write(&p, bad).unwrap();
            assert_eq!(load(&p), Settings::default(), "input: {bad}");
        }
    }

    #[test]
    fn old_format_file_loads_as_empty() {
        let p = temp("old");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{\"brightness\": [30, 60], \"contrast\": [5, 6]}").unwrap();
        assert_eq!(load(&p), Settings::default());
    }

    #[test]
    fn out_of_range_values_are_clamped_for_both() {
        let m = entry("a", 500, 900);
        assert_eq!(m.brightness_pct(), 100);
        assert_eq!(m.contrast_pct(), 100);
        let m = entry("a", 7, 12);
        assert_eq!((m.brightness_pct(), m.contrast_pct()), (7, 12));
        let m = entry("a", u32::MAX, u32::MAX);
        assert_eq!((m.brightness_pct(), m.contrast_pct()), (100, 100));
    }

    #[test]
    fn autostart_choice_roundtrip_and_default() {
        let p = temp("autostart");
        assert_eq!(load(&p).autostart, None); // never decided
        save(&p, &Settings { autostart: Some(false), ..Default::default() }).unwrap();
        assert_eq!(load(&p).autostart, Some(false));
        // Saving monitors keeps the choice.
        let kept = merged(&load(&p), &[Current { key: "a", brightness: Some(1), contrast: Some(2) }]);
        assert_eq!(kept.autostart, Some(false));
    }

    #[test]
    fn file_without_autostart_field_loads_as_undecided() {
        let p = temp("noauto");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{\"monitors\": []}").unwrap();
        assert_eq!(load(&p).autostart, None);
    }

    #[test]
    fn find_matches_by_key_only() {
        let s = Settings { monitors: vec![entry("x", 1, 2), entry("y", 3, 4)], ..Default::default() };
        assert_eq!(s.find("y").unwrap().brightness, 3);
        assert!(s.find("z").is_none());
    }

    #[test]
    fn merged_updates_connected_and_keeps_disconnected() {
        let old = Settings { monitors: vec![entry("a", 10, 20), entry("gone", 77, 66)], ..Default::default() };
        let cur = [
            Current { key: "a", brightness: Some(40), contrast: Some(50) },
            Current { key: "new", brightness: Some(5), contrast: Some(6) },
        ];
        let out = merged(&old, &cur);
        assert_eq!(out.find("a"), Some(&entry("a", 40, 50)));
        assert_eq!(out.find("new"), Some(&entry("new", 5, 6)));
        assert_eq!(out.find("gone"), Some(&entry("gone", 77, 66)));
        assert_eq!(out.monitors.len(), 3);
    }

    #[test]
    fn merged_keeps_old_value_when_unreadable() {
        let old = Settings { monitors: vec![entry("a", 10, 20)], ..Default::default() };
        let cur = [Current { key: "a", brightness: None, contrast: Some(55) }];
        assert_eq!(merged(&old, &cur).find("a"), Some(&entry("a", 10, 55)));
        let cur = [Current { key: "b", brightness: None, contrast: None }];
        assert_eq!(merged(&old, &cur).find("b"), Some(&entry("b", DEFAULT_BRIGHTNESS, DEFAULT_CONTRAST)));
    }

    #[test]
    fn order_roundtrip_and_kept_by_value_saves() {
        let p = temp("order");
        assert!(load(&p).order.is_empty());
        let s = Settings { order: vec!["b".into(), "a".into()], ..Default::default() };
        save(&p, &s).unwrap();
        assert_eq!(load(&p).order, vec!["b", "a"]);
        // Saving brightness/contrast keeps the order.
        let kept = merged(&load(&p), &[Current { key: "a", brightness: Some(1), contrast: Some(2) }]);
        assert_eq!(kept.order, vec!["b", "a"]);
        // A file from before the order existed loads with an empty order.
        std::fs::write(&p, "{\"monitors\": [], \"autostart\": true}").unwrap();
        assert_eq!(load(&p).order, Vec::<String>::new());
    }

    #[test]
    fn bad_parts_are_dropped_good_parts_kept() {
        let p = temp("lenient");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let text = r#"{
            "monitors": [
                {"key": "a", "brightness": 10, "contrast": 20},
                {"key": "bad", "brightness": -5, "contrast": 1},
                {"key": "", "brightness": 1, "contrast": 1},
                {"key": "a", "brightness": 99, "contrast": 99},
                "junk"
            ],
            "autostart": "yes",
            "order": ["b", "", 7, null, "b", "a"],
            "unknown": 1
        }"#;
        std::fs::write(&p, text).unwrap();
        let s = load(&p);
        assert_eq!(s.monitors, vec![entry("a", 10, 20)]);
        assert_eq!(s.autostart, None);
        assert_eq!(s.order, vec!["b", "a"]);
        // An order that is not a list is ignored, the rest still loads.
        std::fs::write(&p, r#"{"monitors": [{"key": "a", "brightness": 1, "contrast": 2}], "order": "a"}"#).unwrap();
        assert_eq!(load(&p).monitors.len(), 1);
        assert!(load(&p).order.is_empty());
    }

    #[test]
    fn files_written_by_0_2_load_unchanged() {
        let p = temp("v02");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        // A file in the 0.2.3 format from Windows (JSON escapes each backslash of the key).
        let text = r#"{
  "monitors": [
    {
      "key": "\\\\?\\display#del40e5#5&2a5c7f4d&0&uid4352#{e6f07b5f}",
      "brightness": 40,
      "contrast": 75
    }
  ],
  "autostart": true,
  "order": [
    "x",
    "\\\\?\\display#del40e5#5&2a5c7f4d&0&uid4352#{e6f07b5f}"
  ]
}"#;
        std::fs::write(&p, text).unwrap();
        let s = load(&p);
        let key = r"\\?\display#del40e5#5&2a5c7f4d&0&uid4352#{e6f07b5f}";
        assert_eq!(s.monitors, vec![entry(key, 40, 75)]);
        assert_eq!(s.autostart, Some(true));
        assert_eq!(s.order, vec!["x", key]);
        // Saving writes the same format back.
        save(&p, &s).unwrap();
        assert_eq!(load(&p), s);
    }

    #[test]
    fn update_keeps_changes_from_other_threads() {
        let p = temp("update");
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let p = p.clone();
                std::thread::spawn(move || {
                    for i in 0..10 {
                        update(&p, |s| s.order.push(format!("t{n}-{i}"))).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(load(&p).order.len(), 80);
        assert!(!p.with_extension("json.tmp").exists());
    }

    #[test]
    fn save_into_a_folder_that_cannot_be_made_fails_without_panic() {
        let p = temp("blocked");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let file = p.parent().unwrap().join("file");
        std::fs::write(&file, "x").unwrap();
        assert!(save(&file.join("settings.json"), &Settings::default()).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn config_base_follows_the_xdg_rules() {
        use std::ffi::OsString;
        let o = |s: &str| Some(OsString::from(s));
        assert_eq!(config_base(o("/x/cfg"), o("/home/a")), Some(PathBuf::from("/x/cfg")));
        assert_eq!(config_base(None, o("/home/a")), Some(PathBuf::from("/home/a/.config")));
        assert_eq!(config_base(o("relative"), o("/home/a")), Some(PathBuf::from("/home/a/.config")));
        assert_eq!(config_base(o(""), o("/home/a")), Some(PathBuf::from("/home/a/.config")));
        assert_eq!(config_base(None, None), None);
        assert_eq!(config_base(None, o("rel")), None);
    }

    #[test]
    fn startup_command_is_quoted_path() {
        let c = startup_command(Path::new("C:\\Program Files\\bl\\brightness-lite.exe"));
        assert!(c.starts_with('"') && c.ends_with(" --autostart"));
        assert!(c.contains("brightness-lite.exe\" --autostart"));
        assert!(c.contains("Program Files"));
    }
}
