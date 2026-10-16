# Brightness Lite 0.2.2 report

## Bugs fixed
- Key repeat on Ctrl+Alt+Up/Down queued one slow DDC write per press; a burst is now merged into one step (test added).
- Windows: `GetMessageW` returns -1 on error, which counted as "true" and could spin the hotkey, events and Identify loops; now only >0 continues.
- Settings saved from the window thread and the monitor thread shared one temp file; saves are now serialized.
- Linux: no permission for /dev/i2c-N now shows a clear "not allowed to use /dev/i2c-N" line instead of a long list of guesses (test added).
- Linux: hotplug socket is closed if it cannot be bound.
- Checked, no change needed: handles/fds are released (Windows `Drop`, Linux opens per call), corrupt/missing settings load as defaults, autostart paths, sliders already write only on release.

## Lighter
- Release profile: opt-level "z", full LTO, codegen-units 1, panic abort, strip.
- Linux wake-up check timer 15 s -> 30 s (the only periodic wake-up; nothing wakes while the window is hidden).
- No unused dependencies found (`cargo tree`: serde, serde_json, eframe, ksni, global-hotkey, libc; eframe already has default features off).

| Linux x86_64, Xvfb, software GL | 0.2.1 | 0.2.2 |
| --- | --- | --- |
| Binary size | 9.64 MB | 7.52 MB |
| Idle CPU, 20 s (window hidden) | 0.10% | 0.10% |
| Resident memory | 117.5 MB | 116.4 MB |

RAM is dominated by the software GL driver in the test setup; real GPU numbers differ.

## Releases
- Files: `brightness-lite-windows-x64.exe`, `brightness-lite-linux-x64.tar.gz`, `SHA256SUMS.txt`.
- `release.yml` runs on push to main (not for README/REPORT/LICENSE-only changes), by hand, and on `v*` tags; it creates the tag itself.

## Not verified
- Windows idle CPU/RAM were not measured; Windows code was only checked with `cargo check --target x86_64-pc-windows-msvc` and CI.
- No real monitor was available: DDC reads/writes are untested here.
