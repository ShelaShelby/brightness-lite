# Brightness Lite 0.2.3 report

## New
- Swap button (drawn arrows, no font glyphs) between neighbouring cards, shown with 2+ monitors.
  The order is saved as monitor keys (`order` in settings.json) and applied on every scan, so it
  holds after restart, replug and wake. Unknown monitors go at the end; an unplugged monitor keeps
  its place. Cards stay bound to their real monitor (sliders, Identify, hotkeys unchanged).
  Tests: order save/load, unknown monitors, unplug/replug. Checked by hand on Linux (Xvfb).
- README and Cargo.toml say "Windows and Linux".

## Memory (Windows): what was found and changed
- Every window open started a new thread (300 ms uncloak safety net). Now one reusable timer
  thread does this for all opens.
- Checked, no per-open growth found: egui ids are stable (cards use `push_id(monitor index)`,
  the swap button's id is its position), the drag map is cleared on open and holds at most
  2 entries per monitor, repaints stop once the window is hidden, nothing is pushed to a Vec
  or map per open.
- 2 s after a hide (cancelled if the window is shown again) and once after the first hide at
  startup, the working set is trimmed (`EmptyWorkingSet`). This only lowers the number Task
  Manager shows; the pages come back when the window opens. It is not a leak fix.
- NativeOptions now set depth, stencil and multisampling to 0 (they already were by default).
  The unused monospace font (Hack) is no longer loaded; emoji fonts were already off.

## Cycle test numbers
- `brightness-lite.exe --cycle-test N` (Windows only, not in the README) opens and hides the
  window N times, 50 ms apart, and prints PrivateUsage and WorkingSetSize at 0, N/4, N/2, N
  cycles and 3 s later (after the trim). Output also goes to `cycle-test.log`.
- CI runs it on windows-latest with N=300 (step "Cycle test", continue-on-error) and puts the
  numbers in the job summary. If the runner's own OpenGL cannot make the window, it retries
  with Mesa's software OpenGL (llvmpipe).
- Before/after numbers are not in this file: 0.2.2 has no cycle test, and this file was pushed
  in the same single push that first ran the test. See the CI job summary of this commit.
- A runner has no GPU: its numbers show growth in our code and egui, not in a real GPU driver,
  where the reported per-open growth (~99 MB idle) may come from; that was not reproduced.

## Not verified
- No Windows machine: the timer, trim and cycle test were checked with
  `cargo check`/`clippy --target x86_64-pc-windows-msvc` and CI only.
- No real monitor: DDC reads/writes and the swap with real monitor keys are untested here.
