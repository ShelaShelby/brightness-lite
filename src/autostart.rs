//! "Start with Windows" / "Start at login".
//! Windows: a value under HKCU\...\Run (no admin rights).
//! Linux: a .desktop file in ~/.config/autostart.

/// What a first run does when the user has never chosen: on for Windows (the
/// app should always be there after Windows starts), off for Linux.
pub const DEFAULT_ON: bool = cfg!(windows);

#[cfg(windows)]
pub use windows_impl::*;

#[cfg(windows)]
mod windows_impl {
    use crate::store;
    use std::path::Path;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
    };

    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    /// Name of our value under the Run key.
    const RUN_VALUE_NAME: &str = "BrightnessLite";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The command stored under `name`, if the entry exists.
    fn read_run_value(name: &str) -> Option<String> {
        let name = wide(name);
        unsafe {
            let mut key = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, 0, KEY_QUERY_VALUE, &mut key).is_err() {
                return None;
            }
            let mut bytes = 0u32;
            let probe = RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, None, None, Some(&mut bytes));
            let mut out = None;
            if probe.is_ok() {
                let mut buf = vec![0u16; (bytes as usize).div_ceil(2) + 1];
                let mut size = (buf.len() * 2) as u32;
                let r = RegQueryValueExW(
                    key,
                    PCWSTR(name.as_ptr()),
                    None,
                    None,
                    Some(buf.as_mut_ptr() as *mut u8),
                    Some(&mut size),
                );
                if r.is_ok() {
                    let n = (size as usize / 2).min(buf.len());
                    out = Some(String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_string());
                }
            }
            let _ = RegCloseKey(key);
            out
        }
    }

    /// Set (`Some(command)`) or remove (`None`) the entry `name`. Removing a missing entry is fine.
    fn write_run_value(name: &str, command: Option<&str>) -> Result<(), String> {
        let name = wide(name);
        unsafe {
            let mut key = HKEY::default();
            RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, 0, KEY_SET_VALUE, &mut key)
                .ok()
                .map_err(|e| e.to_string())?;
            let res = match command {
                Some(c) => {
                    let cmd = wide(c);
                    let bytes = std::slice::from_raw_parts(cmd.as_ptr() as *const u8, cmd.len() * 2);
                    RegSetValueExW(key, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes)).ok()
                }
                None => match RegDeleteValueW(key, PCWSTR(name.as_ptr())) {
                    e if e == ERROR_FILE_NOT_FOUND => Ok(()),
                    e => e.ok(),
                },
            };
            let _ = RegCloseKey(key);
            res.map_err(|e| e.to_string())
        }
    }

    /// Add or remove the entry `name` for `exe`.
    fn set_entry(name: &str, on: bool, exe: &Path) -> Result<(), String> {
        if on {
            write_run_value(name, Some(&store::startup_command(exe)))
        } else {
            write_run_value(name, None)
        }
    }

    /// Keep the entry pointing at the exe that is running now (it may have been moved or rebuilt).
    fn refresh_entry(name: &str, exe: &Path) -> Result<(), String> {
        let want = store::startup_command(exe);
        if read_run_value(name).as_deref() == Some(want.as_str()) {
            return Ok(());
        }
        write_run_value(name, Some(&want))
    }

    pub fn enabled() -> bool {
        read_run_value(RUN_VALUE_NAME).is_some()
    }

    pub fn set(on: bool, exe: &Path) -> Result<(), String> {
        set_entry(RUN_VALUE_NAME, on, exe)
    }

    pub fn refresh(exe: &Path) -> Result<(), String> {
        refresh_entry(RUN_VALUE_NAME, exe)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // These touch the real HKCU Run key, but only under a throw-away name.
        fn test_name(tag: &str) -> String {
            format!("BrightnessLiteTest-{}-{}", std::process::id(), tag)
        }

        #[test]
        fn toggle_creates_and_removes_the_run_entry() {
            let name = test_name("toggle");
            let exe = Path::new("C:\\Tools\\brightness-lite.exe");
            assert_eq!(read_run_value(&name), None);

            set_entry(&name, true, exe).unwrap();
            assert_eq!(
                read_run_value(&name).as_deref(),
                Some("\"C:\\Tools\\brightness-lite.exe\" --autostart")
            );

            set_entry(&name, false, exe).unwrap();
            assert_eq!(read_run_value(&name), None);
            // Turning it off again is harmless.
            set_entry(&name, false, exe).unwrap();
        }

        #[test]
        fn moved_exe_updates_the_entry() {
            let name = test_name("moved");
            set_entry(&name, true, Path::new("C:\\Old\\brightness-lite.exe")).unwrap();
            refresh_entry(&name, Path::new("C:\\New dir\\brightness-lite.exe")).unwrap();
            assert_eq!(
                read_run_value(&name).as_deref(),
                Some("\"C:\\New dir\\brightness-lite.exe\" --autostart")
            );
            set_entry(&name, false, Path::new("x")).unwrap();
            assert_eq!(read_run_value(&name), None);
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux_impl::*;

#[cfg(target_os = "linux")]
mod linux_impl {
    use crate::store;
    use std::path::{Path, PathBuf};

    fn entry_path() -> Option<PathBuf> {
        let base = store::config_base(std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME"))?;
        Some(base.join("autostart").join("brightness-lite.desktop"))
    }

    /// An argument for the Exec= line: quoted, with the characters the Desktop
    /// Entry spec names escaped, then the string-level backslash escape on top.
    pub(super) fn exec_quote(arg: &str) -> String {
        let mut q = String::from("\"");
        for c in arg.chars() {
            match c {
                '"' | '`' | '$' | '\\' => {
                    q.push('\\');
                    q.push(c);
                }
                '%' => q.push_str("%%"),
                _ => q.push(c),
            }
        }
        q.push('"');
        q.replace('\\', "\\\\")
    }

    pub(super) fn desktop_file(exe: &Path) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=Brightness Lite\nComment=Monitor brightness and contrast\nExec={} {}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            exec_quote(&exe.to_string_lossy()),
            store::AUTOSTART_ARG
        )
    }

    pub(super) fn write_entry(path: &Path, exe: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, desktop_file(exe)).map_err(|e| e.to_string())
    }

    pub(super) fn remove_entry(path: &Path) -> Result<(), String> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub(super) fn refresh_entry(path: &Path, exe: &Path) -> Result<(), String> {
        if std::fs::read_to_string(path).ok().as_deref() == Some(desktop_file(exe).as_str()) {
            return Ok(());
        }
        write_entry(path, exe)
    }

    pub fn enabled() -> bool {
        entry_path().is_some_and(|p| p.exists())
    }

    pub fn set(on: bool, exe: &Path) -> Result<(), String> {
        let path = entry_path().ok_or("no home folder")?;
        if on {
            write_entry(&path, exe)
        } else {
            remove_entry(&path)
        }
    }

    pub fn refresh(exe: &Path) -> Result<(), String> {
        let path = entry_path().ok_or("no home folder")?;
        refresh_entry(&path, exe)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn temp(tag: &str) -> PathBuf {
            let d = std::env::temp_dir().join(format!("bl-autostart-{}-{}", std::process::id(), tag));
            let _ = std::fs::remove_dir_all(&d);
            d.join("autostart").join("brightness-lite.desktop")
        }

        #[test]
        fn exec_quoting() {
            assert_eq!(exec_quote("/opt/bl/brightness-lite"), "\"/opt/bl/brightness-lite\"");
            assert_eq!(exec_quote("/opt/my apps/bl"), "\"/opt/my apps/bl\"");
            assert_eq!(exec_quote("/a\"b"), "\"/a\\\\\"b\"");
            assert_eq!(exec_quote("/a$b"), "\"/a\\\\$b\"");
            assert_eq!(exec_quote("/a%b"), "\"/a%%b\"");
            assert_eq!(exec_quote("/a\\b"), "\"/a\\\\\\\\b\"");
        }

        #[test]
        fn toggle_creates_and_removes_the_entry() {
            let p = temp("toggle");
            let exe = Path::new("/home/a/Apps/brightness-lite");
            write_entry(&p, exe).unwrap();
            let text = std::fs::read_to_string(&p).unwrap();
            assert!(text.contains("Exec=\"/home/a/Apps/brightness-lite\" --autostart\n"));
            assert!(text.starts_with("[Desktop Entry]\n"));
            remove_entry(&p).unwrap();
            assert!(!p.exists());
            remove_entry(&p).unwrap(); // already gone: fine
        }

        #[test]
        fn moved_exe_updates_the_entry() {
            let p = temp("moved");
            write_entry(&p, Path::new("/old/bl")).unwrap();
            refresh_entry(&p, Path::new("/new dir/bl")).unwrap();
            assert!(std::fs::read_to_string(&p).unwrap().contains("Exec=\"/new dir/bl\" --autostart"));
            // Same exe again changes nothing.
            refresh_entry(&p, Path::new("/new dir/bl")).unwrap();
        }
    }
}
