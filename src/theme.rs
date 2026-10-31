//! The window's colours: one set of named tokens, three appearances (Light,
//! Dark, AMOLED) and seven accent colours. The choice is kept in theme.json
//! next to the exe, shared by Brightness Lite and Audio Lite (same file in both
//! apps). Pure: no window or OS calls, so it is tested on every platform.
//!
//! A missing, empty, broken or unreadable file gives Dark + Blue. Nothing here
//! ever panics or shows an error: the theme only changes how the window looks.

use serde_json::Value;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "theme.json";
/// Larger files are taken as broken (the app writes about 50 bytes).
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// An sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    Light,
    #[default]
    Dark,
    Amoled,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::Light, Appearance::Dark, Appearance::Amoled];

    /// The name in theme.json.
    pub fn key(self) -> &'static str {
        match self {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
            Appearance::Amoled => "amoled",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
            Appearance::Amoled => "AMOLED",
        }
    }

    pub fn is_light(self) -> bool {
        self == Appearance::Light
    }

    fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.key().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Accent {
    #[default]
    Blue,
    Teal,
    Red,
    Pink,
    Green,
    Amber,
    Indigo,
}

impl Accent {
    pub const ALL: [Accent; 7] =
        [Accent::Blue, Accent::Teal, Accent::Red, Accent::Pink, Accent::Green, Accent::Amber, Accent::Indigo];

    pub fn key(self) -> &'static str {
        match self {
            Accent::Blue => "blue",
            Accent::Teal => "teal",
            Accent::Red => "red",
            Accent::Pink => "pink",
            Accent::Green => "green",
            Accent::Amber => "amber",
            Accent::Indigo => "indigo",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Accent::Blue => "Blue",
            Accent::Teal => "Teal",
            Accent::Red => "Red",
            Accent::Pink => "Pink",
            Accent::Green => "Green",
            Accent::Amber => "Amber",
            Accent::Indigo => "Indigo",
        }
    }

    /// The same shade in every appearance: each has at least 3:1 contrast
    /// against both white and black (WCAG for controls), so it works on all three.
    pub fn rgb(self) -> Rgb {
        match self {
            Accent::Blue => Rgb(0x4c, 0x8d, 0xff),
            Accent::Teal => Rgb(0x11, 0x99, 0x8f),
            Accent::Red => Rgb(0xe5, 0x48, 0x4d),
            Accent::Pink => Rgb(0xe0, 0x56, 0x9b),
            Accent::Green => Rgb(0x2a, 0x9d, 0x4a),
            Accent::Amber => Rgb(0xc2, 0x7c, 0x0e),
            Accent::Indigo => Rgb(0x6e, 0x6c, 0xf0),
        }
    }

    fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.key().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Theme {
    pub appearance: Appearance,
    pub accent: Accent,
}

/// Every colour the window uses. Nothing else in the UI names a colour
/// (except the white of switch knobs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Window background.
    pub bg: Rgb,
    /// Cards and buttons.
    pub card: Rgb,
    pub border: Rgb,
    /// Border of a hovered control.
    pub border_hover: Rgb,
    /// Slider track, switch when off, disabled icons.
    pub track: Rgb,
    /// A control under the mouse.
    pub hover: Rgb,
    /// A control being pressed.
    pub pressed: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    /// Warnings (a monitor that did not answer, a hotkey in use...).
    pub warn: Rgb,
    pub accent: Rgb,
}

impl Theme {
    pub fn palette(self) -> Palette {
        let accent = self.accent.rgb();
        match self.appearance {
            // The look of the apps before themes, unchanged.
            Appearance::Dark => Palette {
                bg: Rgb(0x16, 0x18, 0x1c),
                card: Rgb(0x1f, 0x22, 0x28),
                border: Rgb(0x2c, 0x30, 0x38),
                border_hover: Rgb(0x3a, 0x3f, 0x4a),
                track: Rgb(0x34, 0x38, 0x42),
                hover: Rgb(0x27, 0x2b, 0x33),
                pressed: Rgb(0x2c, 0x30, 0x38),
                text: Rgb(0xe6, 0xe8, 0xec),
                muted: Rgb(0x8b, 0x91, 0x9c),
                warn: Rgb(0xe8, 0xb3, 0x4a),
                accent,
            },
            // Pure black window; cards a little lifted, with a thin border so they stay visible.
            Appearance::Amoled => Palette {
                bg: Rgb(0x00, 0x00, 0x00),
                card: Rgb(0x0d, 0x0d, 0x0d),
                border: Rgb(0x26, 0x26, 0x26),
                border_hover: Rgb(0x3a, 0x3a, 0x3a),
                track: Rgb(0x2e, 0x2e, 0x2e),
                hover: Rgb(0x1a, 0x1a, 0x1a),
                pressed: Rgb(0x22, 0x22, 0x22),
                text: Rgb(0xe6, 0xe8, 0xec),
                muted: Rgb(0x8b, 0x91, 0x9c),
                warn: Rgb(0xe8, 0xb3, 0x4a),
                accent,
            },
            // Text 16:1 and muted text 5.6:1 or more on every surface (WCAG AA).
            Appearance::Light => Palette {
                bg: Rgb(0xf3, 0xf4, 0xf6),
                card: Rgb(0xff, 0xff, 0xff),
                border: Rgb(0xd5, 0xd9, 0xe0),
                border_hover: Rgb(0xb4, 0xba, 0xc4),
                track: Rgb(0xcf, 0xd4, 0xdc),
                hover: Rgb(0xf0, 0xf2, 0xf5),
                pressed: Rgb(0xe4, 0xe7, 0xec),
                text: Rgb(0x1b, 0x1f, 0x24),
                muted: Rgb(0x5b, 0x62, 0x70),
                warn: Rgb(0x8a, 0x5a, 0x00),
                accent,
            },
        }
    }
}

/// `t` of the way from `a` to `b` (0.0 = a, 1.0 = b).
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// Read the file contents. `None` (no file), broken JSON or a value that is
/// not an object gives Dark + Blue. Each field is read on its own: an unknown
/// or wrong value gets its default and the other field is still used. Unknown
/// fields are ignored.
pub fn parse(text: Option<&str>) -> Theme {
    let mut t = Theme::default();
    let Some(Value::Object(map)) = text.and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
    else {
        return t;
    };
    if let Some(a) = map.get("appearance").and_then(Value::as_str).and_then(Appearance::from_key) {
        t.appearance = a;
    }
    if let Some(a) = map.get("accent").and_then(Value::as_str).and_then(Accent::from_key) {
        t.accent = a;
    }
    t
}

pub fn to_json(t: Theme) -> String {
    format!("{{\n  \"appearance\": \"{}\",\n  \"accent\": \"{}\"\n}}\n", t.appearance.key(), t.accent.key())
}

/// theme.json in the exe's folder.
pub fn path_in(exe_dir: Option<&Path>) -> Option<PathBuf> {
    exe_dir.map(|d| d.join(FILE_NAME))
}

/// The theme in the file. A missing file gives Dark + Blue. `None`: the file is
/// there but cannot be read right now (locked, no permission); the caller then
/// keeps what it shows.
pub fn read(path: Option<&Path>) -> Option<Theme> {
    use std::io::Read;
    let Some(path) = path else { return Some(Theme::default()) };
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(Theme::default()),
        Err(_) => return None,
    };
    let mut bytes = Vec::new();
    if f.by_ref().take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).is_err() {
        return None;
    }
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Some(Theme::default());
    }
    Some(parse(Some(&String::from_utf8_lossy(&bytes))))
}

/// The temporary file for a save: one per process, so both apps saving at
/// the same moment never write into the same temporary file.
fn temp_path(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(tmp)
}

/// Write a temporary file, then rename it over theme.json. A crash, a full
/// disk or the other app saving at the same time leaves either the old file
/// or the new one, never half a file. No flush to disk: it would make a click
/// wait for the disk, and the worst a power cut can do is an empty file,
/// which reads as Dark + Blue.
pub fn save(path: &Path, t: Theme) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = temp_path(path);
    let written = std::fs::File::create(&tmp).and_then(|mut f| f.write_all(to_json(t).as_bytes()));
    let res = written.and_then(|()| std::fs::rename(&tmp, path));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// The theme the window shows and where it is kept.
#[derive(Debug, Default)]
pub struct Store {
    pub path: Option<PathBuf>,
    pub theme: Theme,
}

impl Store {
    /// Read at start-up: the file's theme, or Dark + Blue.
    pub fn open(path: Option<PathBuf>) -> Self {
        let theme = read(path.as_deref()).unwrap_or_default();
        Store { path, theme }
    }

    /// Read the file again (the window is shown; the other app may have
    /// changed it). True if the theme changed. Unreadable now: keep the current one.
    pub fn reload(&mut self) -> bool {
        if self.path.is_none() {
            return false;
        }
        match read(self.path.as_deref()) {
            Some(t) if t != self.theme => {
                self.theme = t;
                true
            }
            _ => false,
        }
    }

    /// The user picked `t`: use it at once and save it. A failed save (read-only
    /// folder or file, the file locked) is ignored: the window still shows `t`.
    /// True if the theme changed.
    pub fn set(&mut self, t: Theme) -> bool {
        if t == self.theme {
            return false;
        }
        self.theme = t;
        if let Some(p) = &self.path {
            let _ = save(p, t);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("theme-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn all_themes() -> Vec<Theme> {
        Appearance::ALL
            .into_iter()
            .flat_map(|appearance| Accent::ALL.into_iter().map(move |accent| Theme { appearance, accent }))
            .collect()
    }

    fn luminance(c: Rgb) -> f64 {
        let lin = |v: u8| {
            let v = v as f64 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2)
    }

    fn contrast(a: Rgb, b: Rgb) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// All 21 combinations: the surfaces come from the appearance, the accent
    /// from the colour, and nothing else changes.
    #[test]
    fn every_combination_gives_its_tokens() {
        let dark = Theme::default().palette();
        assert_eq!(dark.bg, Rgb(0x16, 0x18, 0x1c));
        assert_eq!(dark.card, Rgb(0x1f, 0x22, 0x28));
        assert_eq!(dark.accent, Rgb(0x4c, 0x8d, 0xff));
        for t in all_themes() {
            let p = t.palette();
            assert_eq!(p.accent, t.accent.rgb(), "{t:?}");
            let base = Theme { accent: Accent::Blue, ..t }.palette();
            assert_eq!(Palette { accent: base.accent, ..p }, base, "{t:?}: only the accent may differ");
            match t.appearance {
                Appearance::Amoled => {
                    assert_eq!(p.bg, Rgb(0, 0, 0));
                    assert_eq!(p.card, Rgb(0x0d, 0x0d, 0x0d));
                    assert!(p.border != p.card && p.border != p.bg);
                }
                Appearance::Light => assert!(luminance(p.bg) > 0.8 && luminance(p.card) > 0.8),
                Appearance::Dark => assert_eq!(Palette { accent: dark.accent, ..p }, dark),
            }
        }
        // The 7 accents are all different, and so are the 3 appearances.
        let accents: std::collections::HashSet<_> = Accent::ALL.iter().map(|a| a.rgb()).collect();
        assert_eq!(accents.len(), 7);
        let bgs: std::collections::HashSet<_> =
            Appearance::ALL.iter().map(|&a| Theme { appearance: a, ..Theme::default() }.palette().bg).collect();
        assert_eq!(bgs.len(), 3);
    }

    #[test]
    fn text_is_readable_everywhere() {
        for t in all_themes() {
            let p = t.palette();
            for surface in [p.bg, p.card, p.hover, p.pressed] {
                assert!(contrast(p.text, surface) >= 7.0, "{t:?} text");
            }
            // Muted text and warnings sit on the window and the cards (hovered
            // controls switch their text to `text`).
            for surface in [p.bg, p.card] {
                assert!(contrast(p.muted, surface) >= 4.5, "{t:?} muted");
                assert!(contrast(p.warn, surface) >= 4.5, "{t:?} warn");
            }
            // The accent (slider fill, switch, rings) stands out from the card and the track.
            assert!(contrast(p.accent, p.card) >= 2.9, "{t:?} accent");
        }
        for a in Accent::ALL {
            assert!(contrast(a.rgb(), Rgb(255, 255, 255)) >= 3.0, "{a:?} on white");
            assert!(contrast(a.rgb(), Rgb(0, 0, 0)) >= 3.0, "{a:?} on black");
        }
    }

    #[test]
    fn round_trip_for_all_21() {
        for t in all_themes() {
            assert_eq!(parse(Some(&to_json(t))), t);
        }
        assert_eq!(to_json(Theme::default()), "{\n  \"appearance\": \"dark\",\n  \"accent\": \"blue\"\n}\n");
    }

    #[test]
    fn bad_files_give_dark_blue() {
        let d = Theme::default();
        assert_eq!(d, Theme { appearance: Appearance::Dark, accent: Accent::Blue });
        for text in [
            "",
            "   ",
            "\u{feff}",
            "{",
            "{ appearance: light",
            "null",
            "[\"light\", \"red\"]",
            "\"light\"",
            "42",
            "{\"appearance\": \"light\", \"accent\": \"re",
            "{}",
            "{\"appearance\": 1, \"accent\": true}",
            "{\"appearance\": [\"light\"], \"accent\": {\"x\": 1}}",
            "{\"appearance\": null, \"accent\": null}",
            "{\"appearance\": \"sepia\", \"accent\": \"purple\"}",
            "{\"appearance\": \"\", \"accent\": \"\"}",
        ] {
            assert_eq!(parse(Some(text)), d, "{text:?}");
        }
        assert_eq!(parse(None), d);
    }

    #[test]
    fn fields_are_read_on_their_own() {
        let t = parse(Some("{\"appearance\": \"light\", \"accent\": \"purple\"}"));
        assert_eq!(t, Theme { appearance: Appearance::Light, accent: Accent::Blue });
        let t = parse(Some("{\"appearance\": 3, \"accent\": \"teal\"}"));
        assert_eq!(t, Theme { appearance: Appearance::Dark, accent: Accent::Teal });
        // Extra fields, any case, spaces, a BOM: all fine.
        let t = parse(Some("\u{feff}{\"x\": 1, \"appearance\": \" AMOLED \", \"accent\": \"Indigo\", \"y\": [1]}"));
        assert_eq!(t, Theme { appearance: Appearance::Amoled, accent: Accent::Indigo });
    }

    #[test]
    fn garbage_never_panics() {
        let good = to_json(Theme { appearance: Appearance::Amoled, accent: Accent::Amber });
        for end in 0..=good.len() {
            let _ = parse(Some(&good[..end]));
        }
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let alphabet = b"{}[]\":,. \n\\u0123456789abcdeflightdarkamoledbluetealred";
        for _ in 0..20_000 {
            let mut text = String::new();
            for _ in 0..(seed % 48) {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                text.push(alphabet[(seed % alphabet.len() as u64) as usize] as char);
            }
            let _ = parse(Some(&text));
            seed = seed.wrapping_add(1);
        }
    }

    #[test]
    fn missing_empty_huge_and_folder() {
        let d = dir("files");
        let p = d.join(FILE_NAME);
        assert_eq!(read(Some(&p)), Some(Theme::default()));
        assert_eq!(read(None), Some(Theme::default()));
        std::fs::write(&p, b"").unwrap();
        assert_eq!(read(Some(&p)), Some(Theme::default()));
        std::fs::write(&p, [0xff, 0xfe, 0x00, 0x7b]).unwrap();
        assert_eq!(read(Some(&p)), Some(Theme::default()));
        // Larger than any real file: broken, never loaded whole.
        let mut big = to_json(Theme { appearance: Appearance::Light, accent: Accent::Red }).into_bytes();
        big.resize(MAX_FILE_BYTES as usize + 10, b' ');
        std::fs::write(&p, &big).unwrap();
        assert_eq!(read(Some(&p)), Some(Theme::default()));
        // A folder where the file should be: cannot be read (or reads as nothing).
        std::fs::remove_file(&p).unwrap();
        std::fs::create_dir(&p).unwrap();
        assert!(matches!(read(Some(&p)), None | Some(Theme { appearance: Appearance::Dark, accent: Accent::Blue })));
        // ...and saving over it fails quietly, leaving nothing behind.
        let mut s = Store { path: Some(p.clone()), theme: Theme::default() };
        assert!(s.set(Theme { appearance: Appearance::Light, accent: Accent::Teal }));
        assert_eq!(s.theme.appearance, Appearance::Light);
        assert!(!s.reload(), "unreadable: keep what is shown");
        assert_eq!(s.theme.appearance, Appearance::Light);
        let names: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from(FILE_NAME)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn save_replaces_and_leaves_no_temp_file() {
        let d = dir("save");
        let p = d.join(FILE_NAME);
        for t in all_themes() {
            save(&p, t).unwrap();
            assert_eq!(read(Some(&p)), Some(t));
        }
        let names: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from(FILE_NAME)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A save cut off before the rename (crash, power cut): the temporary file
    /// is half written, theme.json is the old one and still reads fine. The
    /// next save replaces both.
    #[test]
    fn interrupted_save_keeps_the_old_file() {
        let d = dir("crash");
        let p = d.join(FILE_NAME);
        let old = Theme { appearance: Appearance::Light, accent: Accent::Pink };
        save(&p, old).unwrap();
        let full = to_json(Theme { appearance: Appearance::Amoled, accent: Accent::Green });
        for cut in 0..full.len() {
            std::fs::write(temp_path(&p), &full[..cut]).unwrap();
            assert_eq!(read(Some(&p)), Some(old), "cut at {cut}");
        }
        let new = Theme { appearance: Appearance::Amoled, accent: Accent::Green };
        save(&p, new).unwrap();
        assert_eq!(read(Some(&p)), Some(new));
        assert!(!temp_path(&p).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_folder_and_file() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("ro");
        let p = d.join(FILE_NAME);
        let t = Theme { appearance: Appearance::Light, accent: Accent::Amber };
        save(&p, t).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o555)).unwrap();
        let mut s = Store::open(Some(p.clone()));
        assert_eq!(s.theme, t);
        let root = is_root();
        let other = Theme { appearance: Appearance::Dark, accent: Accent::Red };
        assert!(s.set(other)); // shown at once even though it cannot be saved
        assert_eq!(s.theme, other);
        if !root {
            assert_eq!(read(Some(&p)), Some(t), "file unchanged");
        }
        // Unreadable file: keep what is shown, start-up gives Dark + Blue.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
        if !root {
            assert_eq!(read(Some(&p)), None);
            assert!(!s.reload());
            assert_eq!(s.theme, other);
            assert_eq!(Store::open(Some(p.clone())).theme, Theme::default());
        }
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Root ignores file permissions, so those checks are skipped for it.
    #[cfg(unix)]
    fn is_root() -> bool {
        std::env::var("USER").is_ok_and(|u| u == "root")
            || std::fs::read_to_string("/proc/self/status")
                .is_ok_and(|s| s.lines().any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(1) == Some("0")))
    }

    /// Windows: a read-only theme.json cannot be replaced; the theme is still
    /// shown, nothing breaks, and the file keeps its old contents.
    #[cfg(windows)]
    #[test]
    fn read_only_file_windows() {
        let d = dir("ro");
        let p = d.join(FILE_NAME);
        let t = Theme { appearance: Appearance::Light, accent: Accent::Amber };
        save(&p, t).unwrap();
        let mut perm = std::fs::metadata(&p).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&p, perm.clone()).unwrap();
        let mut s = Store::open(Some(p.clone()));
        let other = Theme { appearance: Appearance::Amoled, accent: Accent::Red };
        assert!(s.set(other));
        assert_eq!(s.theme, other);
        assert_eq!(read(Some(&p)), Some(t));
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(&p, perm).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Windows: the other app has theme.json open with no sharing (locked).
    /// Reading gives "unreadable" (keep the current theme), saving fails
    /// quietly, and once the lock is gone both work again.
    #[cfg(windows)]
    #[test]
    fn locked_file_windows() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = dir("lock");
        let p = d.join(FILE_NAME);
        let t = Theme { appearance: Appearance::Light, accent: Accent::Teal };
        save(&p, t).unwrap();
        let lock = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&p).unwrap();
        assert_eq!(read(Some(&p)), None);
        let mut s = Store { path: Some(p.clone()), theme: t };
        assert!(!s.reload());
        assert_eq!(s.theme, t);
        assert_eq!(Store::open(Some(p.clone())).theme, Theme::default());
        let other = Theme { appearance: Appearance::Amoled, accent: Accent::Indigo };
        assert!(s.set(other));
        assert_eq!(s.theme, other);
        drop(lock);
        assert_eq!(read(Some(&p)), Some(t), "the locked file was not touched");
        assert!(!temp_path(&p).exists());
        save(&p, other).unwrap();
        assert_eq!(read(Some(&p)), Some(other));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Two apps on one file: what one picks, the other shows the next time
    /// its window is shown (reload); nothing changes before that.
    #[test]
    fn the_other_app_picks_it_up_when_shown() {
        let d = dir("sync");
        let p = d.join(FILE_NAME);
        let mut a = Store::open(Some(p.clone()));
        let mut b = Store::open(Some(p.clone()));
        assert_eq!(a.theme, Theme::default());
        assert!(!b.reload());
        for t in all_themes().into_iter().rev() {
            let before = b.theme;
            a.set(t);
            assert_eq!(b.theme, before, "nothing changes until the window is shown");
            assert_eq!(b.reload(), t != before);
            assert_eq!(b.theme, t);
            assert!(!b.reload(), "nothing new");
        }
        // And back the other way.
        let t = Theme { appearance: Appearance::Light, accent: Accent::Red };
        assert!(b.set(t));
        assert!(a.reload());
        assert_eq!(a.theme, t);
        // Setting the same theme again writes nothing.
        let m = std::fs::metadata(&p).unwrap().modified().unwrap();
        assert!(!a.set(t));
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), m);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Both apps saving at once while one reads: every read gives one of the
    /// saved themes, never a broken file (which would read as Dark + Blue).
    #[test]
    fn saving_at_the_same_time_never_breaks_the_file() {
        let d = dir("race");
        let p = d.join(FILE_NAME);
        let first = Theme { appearance: Appearance::Light, accent: Accent::Teal };
        save(&p, first).unwrap();
        // Neither writer ever writes Dark + Blue, so seeing it means a broken read.
        let themes: Vec<Theme> = all_themes().into_iter().filter(|t| *t != Theme::default()).collect();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writers: Vec<_> = (0..2)
            .map(|w| {
                let (p, themes) = (p.clone(), themes.clone());
                std::thread::spawn(move || {
                    for i in 0..300 {
                        // Different temp names, as two processes would have.
                        let t = themes[(i * 3 + w) % themes.len()];
                        let tmp = {
                            let mut s = p.as_os_str().to_owned();
                            s.push(format!(".w{w}.tmp"));
                            PathBuf::from(s)
                        };
                        if std::fs::write(&tmp, to_json(t)).is_ok() {
                            // On Windows a rename can fail while the other one renames: that save is lost, nothing breaks.
                            let _ = std::fs::rename(&tmp, &p);
                        }
                    }
                })
            })
            .collect();
        let reader = {
            let (p, stop) = (p.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut n = 0;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Some(t) = read(Some(&p)) {
                        assert_ne!(t, Theme::default(), "a broken file was read");
                        n += 1;
                    }
                }
                n
            })
        };
        for w in writers {
            w.join().unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(reader.join().unwrap() > 0);
        assert_ne!(read(Some(&p)), Some(Theme::default()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn path_is_next_to_the_exe() {
        let exe_dir = Path::new("C:\\Tools");
        assert_eq!(path_in(Some(exe_dir)), Some(exe_dir.join("theme.json")));
        assert_eq!(path_in(None), None);
        // No path (exe folder unknown): Dark + Blue, choices are kept in memory only.
        let mut s = Store::open(None);
        assert_eq!(s.theme, Theme::default());
        assert!(s.set(Theme { appearance: Appearance::Light, accent: Accent::Red }));
        assert!(!s.reload());
    }

    #[test]
    fn mix_ends_and_middle() {
        let (a, b) = (Rgb(0, 100, 200), Rgb(200, 100, 0));
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert_eq!(mix(a, b, 0.5), Rgb(100, 100, 100));
        assert_eq!(mix(a, b, 7.0), b);
    }
}
