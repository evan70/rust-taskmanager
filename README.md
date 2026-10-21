# Android Rust Task Manager

A task manager for Android, written in Rust: memory, CPU, temperatures, battery, storage, network, every process and what starts at boot.

Uses little memory, starts fast, asks for as few permissions as it can, and keeps everything on the
phone. See [PRIVACY.md](PRIVACY.md).

## Features

- Overview: memory, processor (every core), battery (temperature, current, power, cycles), storage, network speed, with live charts
- Processes: every process with its CPU and memory use, search and sort, and Stop for your own apps (detailed mode)
- Startup: which apps start at boot, with Stop, Freeze and Unfreeze (detailed mode)
- Works without any setup in a basic mode; detailed mode needs a one-time setup with Android's own Wireless debugging, or Shizuku if you already use it

## How detailed mode works

Android hides other apps' CPU use, memory and the chip temperatures from ordinary apps. The
basic mode shows what Android does allow. Detailed mode adds the rest by starting a small helper program
(`helper/`) on the phone with the same access the phone's `adb shell` has. The app sets that up by itself:

- **With Shizuku** (if you already use it): one tap, "Start with Shizuku".
- **Without anything else**: the app pairs with the phone's own *Wireless debugging* (Android 11 or newer,
  needs Wi-Fi but not internet) using the six-digit code Android shows once, then starts the helper. The
  pairing code, TLS connection and shell are written in `crates/adbwire`.

After the phone restarts the helper is gone; switch Wireless debugging back on and tap "Connect and
start" (no code needed), or tap "Start with Shizuku".

The helper listens only on the phone's own loopback address, and only answers requests that carry a secret
the app keeps in its own folder. Android 11 or newer is required.

## Install

Download the APK from the releases page and follow [docs/INSTALL.md](docs/INSTALL.md).

## Build

Needs Rust with the `aarch64-linux-android` target, `cargo-ndk`, the Android SDK (build-tools 34,
platform 34), NDK r27c and a JDK 17, with `JAVA_HOME`, `ANDROID_HOME` and `ANDROID_NDK_HOME` set to them.
Then, from `apps/taskmanager` (Windows PowerShell):

```
cargo test
powershell -File build-apk.ps1
```

APKs are signed with your own key, see [docs/SIGNING.md](docs/SIGNING.md).

## Layout

- `apps/taskmanager/` the app
- `crates/heartnet-kit/` code shared with the other apps (exit handling, settings, theme)
- `crates/adbwire/` a small adb client (wireless-debugging pairing, TLS connection, shell) written for this app
- `helper/` the small program the app starts on the phone to read what apps cannot see; it ships inside the APK
- `tools/build/` the shared APK build script

## More apps

- [Android Rust Gallery](https://github.com/HusniHeartnet/android-rust-gallery)
- [Android Rust Files](https://github.com/HusniHeartnet/android-rust-files)
- [Android Rust Notes](https://github.com/HusniHeartnet/android-rust-notes)
- [Android Rust Home](https://github.com/HusniHeartnet/android-rust-home)

## Licence

Copyright (C) heartnetapps. Licensed under the GNU General Public License, version 3 or (at your
option) any later version. See [LICENSE](LICENSE). The names and logos are not covered by the licence.
