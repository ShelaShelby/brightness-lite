# Changelog

What changed in each version, newest first.

## 0.3.3

Fixes:

- Linux: the window opens above the tray icon, like on Windows.

## 0.3.2

Fixes:

- Linux: on Wayland desktops the app now runs through XWayland, so opening from the tray, click-away hiding and always-on-top work like on Windows.

## 0.3.1

Fixes:

- Linux: DisplayPort monitors on AMD (and Intel) now use the DP AUX I2C bus, so brightness and contrast work.

## 0.3.0

The last planned version: fixes and polish only, no new features.
Settings files from older versions still load, including the card order.

Fixes:

- Swapping cards: swapping with a monitor that was new (not in the saved
  order yet) did nothing. It works now.
- Swapping cards: a monitor without an id no longer jumps around when
  other cards are swapped. It always stays at the end.
- Two identical monitors that report the same id now get their own id,
  so their values and places never mix.
- Empty, broken or repeated entries in the saved card order are ignored.
- If a monitor was unplugged while you dragged a slider, letting go could
  send the value to another monitor. The drag is now dropped instead.
- A setting the monitor says it does not have (often contrast) is hidden,
  instead of showing a disabled slider and a warning.
- "Reset to saved values" no longer writes to settings the monitor does
  not have, and looks for monitors again if a write fails.
- Settings file: one bad entry no longer throws away the whole file. Only
  that entry is dropped.
- Settings file: two parts of the app saving at the same moment could undo
  each other's change (for example the card order). Saves now happen one
  at a time, read and write together.
- Settings file: the file is flushed to disk before it replaces the old
  one, so a power cut cannot leave an empty file.
- Windows: the app no longer crashes if the tray icon cannot be made. The
  window then stays open instead.

Less CPU and a smaller app:

- Linux: the hotkeys no longer wake the app 20 times a second. They now
  wait for the key itself (own code instead of the global-hotkey library).
- Linux: the app no longer checks the clock every 30 seconds to notice a
  wake-up. It now listens for the system's wake-up message (logind). The
  old check stays as a backup when that message is not available.
- Linux: the tray library now uses a runtime whose helper threads stop when
  they have nothing to do. Before, one of them woke up twice a second.
- Measured on Linux (X11, window idle): once the helper threads have
  stopped, 0 system calls and 0 CPU time over 20 seconds. 0.2.3 woke up
  more than 20 times a second.
- Release build is optimised for size with `opt-level = "s"`, which gave a
  smaller file than `"z"`. Link-time optimisation, one codegen unit,
  stripped, abort on panic, as before.
- The tray icon is drawn by one shared function on both systems.

Numbers:

| | 0.2.3 | 0.3.0 |
| --- | --- | --- |
| Linux app, unpacked | 7,493,136 bytes | 7,146,856 bytes (-4.6%) |
| Linux download (.tar.gz) | 3,125,558 bytes | 3,206,010 bytes (+2.6%) |
| Windows app (.exe) | 4,913,664 bytes | in the release notes |

The Linux numbers were built here with Rust 1.99.1. The Windows file can
only be built on GitHub; the release notes list its exact size.

Windows cycle test, 0.2.3 (open and hide the window 300 times, on GitHub's
Windows runner with Mesa software OpenGL):

| Point | PrivateUsage | WorkingSetSize |
| --- | --- | --- |
| Start | 57.81 MB | 0.56 MB |
| After 75 | 57.81 MB | 7.43 MB |
| After 150 | 57.81 MB | 7.47 MB |
| After 300 | 57.81 MB | 7.49 MB |
| 3 s later | 57.81 MB | 0.19 MB |

No growth. For 0.3.0 the same test runs on every push and now fails the
build if it does not finish. Its numbers are in the CI job summary.

Checks:

- `cargo fmt --check` and `cargo clippy -- -D warnings` now run in CI on
  Windows and Linux.
- 76 tests, among them the card order (1, 2, 3 and more monitors, swap
  first, middle and last, swap twice, unplugged and new monitors, missing
  ids, junk in the saved order), loading old and broken settings files,
  and saving from many threads at once.

Documents: the README is rewritten in plain words. This file replaces
REPORT.md.

## 0.2.3

- A swap button between two monitor cards. The order is saved and kept
  after a restart, a replug and a wake-up.
- Windows: one timer thread for the whole app instead of a new thread each
  time the window opens.
- Windows: memory is given back to Windows 2 seconds after the window
  hides.
- Windows: a `--cycle-test N` option for CI that opens and hides the window
  N times and prints the memory use.

## 0.2.2

- Smaller build.
- A burst of hotkey presses is sent to the monitor as one change.
- Clearer message when Linux does not allow access to /dev/i2c.
- New download file names, and GitHub makes the release by itself.

## 0.2.1

- Windows: "Start with Windows" works on new user accounts, where the Run
  registry key does not exist yet.

## 0.2.0

- Linux version.
- Saved values are put back after the computer wakes up and when a monitor
  is plugged in again.
- Monitor model names on the cards.

## 0.1.0

- First version for Windows.
