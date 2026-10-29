# Brightness Lite

A small, light app that changes the brightness and contrast of your
external monitors. It works on Windows and Linux.

It sits in the tray (the small icons next to the clock). Click the icon and
a window opens with one card per monitor. Each card has a Brightness slider
and a Contrast slider. The app only ever changes these two settings on your
monitor. It uses DDC/CI, the standard way a computer talks to a monitor
over the video cable.

## What you need

- A monitor with DDC/CI turned on. Most monitors have it. If the sliders
  do not work, look for "DDC/CI" in the monitor's own menu and turn it on.
- Windows 10 or 11 (64-bit), or a 64-bit Linux desktop.
- Laptop screens are not supported. They do not have DDC/CI.

## Download

Go to the [Releases page](https://github.com/ShelaShelby/brightness-lite/releases/latest)
and download the file for your computer:

| Your computer | File to download |
| --- | --- |
| Windows | `brightness-lite-windows-x64.exe` |
| Linux | `brightness-lite-linux-x64.tar.gz` |

`SHA256SUMS.txt` lets you check that the download is not damaged.

## Install on Windows

1. Put `brightness-lite-windows-x64.exe` in a folder where it can stay,
   for example `C:\Tools`.
2. Double-click it. A blue round icon appears in the tray. If you do not
   see it, click the small arrow next to the tray icons.
3. Windows may show a "Windows protected your PC" warning, because the
   file is not signed. Click "More info", then "Run anyway".

It starts with Windows from now on. You can turn this off in the window
("Start with Windows") or in the tray menu.

## Install on Linux

1. Unpack the file and start the app:

       tar -xzf brightness-lite-linux-x64.tar.gz
       ./brightness-lite

2. Linux only lets the app talk to monitors once you allow it. Run these
   commands once, as root (for example with `sudo -i`), then log out and
   back in:

       echo i2c-dev > /etc/modules-load.d/i2c-dev.conf
       modprobe i2c-dev
       echo 'KERNEL=="i2c-[0-9]*", TAG+="uaccess"' > /etc/udev/rules.d/60-brightness-lite.rules
       udevadm control --reload
       udevadm trigger

Good to know on Linux:

- The tray icon needs a desktop with a system tray. KDE, XFCE, Cinnamon
  and most others have one. On GNOME, install the "AppIndicator" extension.
  Without a tray, the window just stays open.
- If the app does not start, install the keyboard library. On Ubuntu or
  Debian: `sudo apt install libxkbcommon-x11-0`.
- Monitors on NVIDIA's own driver may not be found.
- Debug log: quit the app from the tray menu, then start it from a terminal
  with `BRIGHTNESS_LITE_DEBUG=1 ./brightness-lite 2> brightness-lite.log`.
  The file then lists every tray click, open, hide and focus change.

## Update

Close the app (tray icon, then "Quit"). Download the new file and put it in
place of the old one, with the same name. Start it again. Your settings
are kept.

On Windows you can also give the new file another name or folder. The app
updates its "start with Windows" entry by itself when it starts.

## How to use it

- Click the tray icon to open or hide the window. Click anywhere else and
  it hides again.
- Move a slider to change brightness or contrast. The value is sent when
  you let go.
- If a monitor has no contrast setting, only its brightness slider is
  shown.
- With two or more monitors, a small arrow button sits between the cards.
  Click it to swap the two cards. This only changes the order on screen.
  The sliders always stay with their own monitor.
- "Reset to saved values" puts every monitor back to the last values the
  app saved.
- On Windows, "Identify" shows the monitor's number in big digits on that
  monitor. The numbers are the same as in Windows Settings > Display.
- Tray menu: Open, Start with Windows (or Start at login), Re-scan (look
  for monitors again), Quit.

The app remembers your values. It puts them back when you log in, when
the computer wakes up from sleep, and when you plug a monitor in again.

## Hotkeys

| Keys | What they do |
| --- | --- |
| Ctrl + Alt + Up | All monitors a bit brighter (+5 out of 100) |
| Ctrl + Alt + Down | All monitors a bit darker (-5 out of 100) |

On Linux the hotkeys work on X11 only, not on Wayland.

## Start with the computer

- Windows: on by default. Turn it off with the "Start with Windows" switch
  in the window or in the tray menu.
- Linux: off by default. Turn it on with "Start at login". This puts a file
  in `~/.config/autostart`.

## Settings file

Your values, the card order and the startup choice are kept in one file:

- Windows: `%APPDATA%\brightness-lite\settings.json`
- Linux: `~/.config/brightness-lite/settings.json`

You do not need to edit it. If the file is broken or deleted, the app
starts with default values and writes a new file the next time it saves.

## Build from source

You need [Rust](https://rustup.rs). Then:

    git clone https://github.com/ShelaShelby/brightness-lite
    cd brightness-lite
    cargo build --release

The app is then in `target/release`. Run the tests with `cargo test`.

A new release is made by GitHub on its own when the version number in
`Cargo.toml` changes on the main branch.

## License

MIT. See the [LICENSE](LICENSE) file.
