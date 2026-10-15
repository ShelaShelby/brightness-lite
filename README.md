# Brightness Lite

A small Windows tray app that sets the brightness and contrast of each
external monitor over DDC/CI. It uses the Windows API directly and writes
nothing else to the monitor.

## What it does

- One card per monitor, with a Brightness and a Contrast slider (0-100).
- Monitors are numbered like Settings > System > Display. An Identify button
  shows the number on that monitor.
- Left-click the tray icon to open or hide the window. Clicking away hides it.
- Ctrl+Alt+Up and Ctrl+Alt+Down change brightness on all monitors by 5.
- Saved values are restored at login, to the same physical monitor.
- Starts with Windows (on by default, switch it off in the tray menu).
- Tray menu: Open, Start with Windows, Re-scan, Quit.

Settings are kept in %APPDATA%\brightness-lite\settings.json.

## Requirements

- Windows 10 or 11.
- A monitor with DDC/CI turned on in its own menu.

## Build

    cargo build --release

The program is then target\release\brightness-lite.exe. Run the tests with
`cargo test`.

## License

MIT. See the LICENSE file.
