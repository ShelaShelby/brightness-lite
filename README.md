# Brightness Lite

A very light brightness and contrast changer for your PC. Windows and Linux.

It is a small tray app that sets the brightness and contrast of each
external monitor over DDC/CI, and writes nothing else to the monitor.

## What it does

- One card per monitor with its model name, a Brightness and a Contrast slider.
- Left-click the tray icon to open or hide the window. Clicking away hides it.
- Ctrl+Alt+Up and Ctrl+Alt+Down change brightness on all monitors by 5.
- Saved values are put back at login, after the PC wakes up, and when a
  monitor is plugged in again, to the same physical monitor.
- Tray menu: Open, Start with Windows / Start at login, Re-scan, Quit.
- Windows only: monitors are numbered like Settings > System > Display, and
  an Identify button shows the number on that monitor. Starts with Windows
  (on by default).

## Requirements

- A monitor with DDC/CI turned on in its own menu.
- Windows 10 or 11, or Linux with an OpenGL driver.

## Linux notes

Download `brightness-lite-linux-x86_64.tar.gz`, unpack it and run `./brightness-lite`.

- Monitors are reached through /dev/i2c-*. Once, as root:

      echo i2c-dev > /etc/modules-load.d/i2c-dev.conf && modprobe i2c-dev
      echo 'KERNEL=="i2c-[0-9]*", TAG+="uaccess"' > /etc/udev/rules.d/60-brightness-lite.rules
      udevadm control --reload && udevadm trigger

- The tray icon needs a StatusNotifier host. KDE, XFCE and most others have
  one; GNOME needs the AppIndicator extension. Without one the window just
  stays open.
- The hotkeys work on X11 only, not on Wayland.
- "Start at login" is off until you turn it on.
- Laptop screens are skipped (they have no DDC/CI). Monitors on NVIDIA's
  proprietary driver may not be found.

Settings are kept in %APPDATA%\brightness-lite\settings.json on Windows and
~/.config/brightness-lite/settings.json on Linux.

## Build

    cargo build --release

Run the tests with `cargo test`. A new release is built by GitHub when a tag
like v0.2.0 is pushed.

## License

MIT. See the LICENSE file.
