# Android Rust Task Manager

[![Lint](https://github.com/evan70/rust-taskmanager/actions/workflows/lint.yml/badge.svg)](https://github.com/evan70/rust-taskmanager/actions/workflows/lint.yml)
[![Tests](https://github.com/evan70/rust-taskmanager/actions/workflows/tests.yml/badge.svg)](https://github.com/evan70/rust-taskmanager/actions/workflows/tests.yml)
[![Build](https://github.com/evan70/rust-taskmanager/actions/workflows/build.yml/badge.svg)](https://github.com/evan70/rust-taskmanager/actions/workflows/build.yml)

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

More about it, including the security model and what to do when it will not start: [detailed mode](docs/DETAILED-MODE.md).

## Install

Download the APK from the releases page and follow [docs/INSTALL.md](docs/INSTALL.md).

## Build

Needs Rust with the `aarch64-linux-android` target, `cargo-ndk`, the Android SDK (build-tools 34,
platform 34), NDK r27c and a JDK 17, with `JAVA_HOME`, `ANDROID_HOME` and `ANDROID_NDK_HOME` set to them.
```
make test        # tests for the app and the shared crates
make test-all    # also the helper's own workspace
make dev         # run the desktop preview, no phone needed
```

For an APK, from `apps/taskmanager` (Windows PowerShell):

```
cargo test
powershell -File build-apk.ps1
```

`make help` lists every target (`make ci` runs lint, tests and build; `make android-check` says what
the APK build is missing; `make apk` builds it).

APKs are signed with your own key, see [docs/SIGNING.md](docs/SIGNING.md). The whole pipeline, the
desktop preview and what each tool is for: [docs/BUILD.md](docs/BUILD.md).

## CI

Every push to `main` and every pull request runs four workflows, in `.github/workflows/`:
[lint](https://github.com/evan70/rust-taskmanager/actions/workflows/lint.yml) (clippy, warnings as
errors), [tests](https://github.com/evan70/rust-taskmanager/actions/workflows/tests.yml) (on stable
and beta, plus a coverage report kept as a build artifact),
[build](https://github.com/evan70/rust-taskmanager/actions/workflows/build.yml) and
[security](https://github.com/evan70/rust-taskmanager/actions/workflows/security.yml) (RustSec
advisory check on both lockfiles, weekly too, and a dependency review on pull requests).

They run the same `make` targets you run yourself — `make ci` is lint, tests and build. The APK is
not built in CI: that needs Windows, the Android SDK/NDK and your keystore (see [docs/BUILD.md](docs/BUILD.md)).

## Layout

- `apps/taskmanager/` the app
- `crates/heartnet-kit/` code shared with the other apps (exit handling, settings, theme)
- `crates/adbwire/` a small adb client (wireless-debugging pairing, TLS connection, shell) written for this app
- `helper/` the small program the app starts on the phone to read what apps cannot see; it ships inside the APK
- `tools/build/` the shared APK build script

## Documentation

| Guide | Description |
|-------|-------------|
| [Installing](docs/INSTALL.md) | Downloading, checking and installing the APK |
| [Detailed mode](docs/DETAILED-MODE.md) | What Android hides, the two ways to set it up, the security model |
| [Building](docs/BUILD.md) | Toolchain, environment variables, the APK pipeline, desktop preview |
| [Signing](docs/SIGNING.md) | Your own release keystore and fingerprints |
| [Privacy](PRIVACY.md) | What stays on your phone, and what each permission is for |
| [Contributing](CONTRIBUTING.md) | Tests, style, and how to send a change |
| [Security](SECURITY.md) | Reporting a problem privately |
| [Third-party notices](THIRD_PARTY.md) | Licences of everything that ships in the APK |

## More apps

- [Android Rust Gallery](https://github.com/HusniHeartnet/android-rust-gallery)
- [Android Rust Files](https://github.com/HusniHeartnet/android-rust-files)
- [Android Rust Notes](https://github.com/HusniHeartnet/android-rust-notes)
- [Android Rust Home](https://github.com/HusniHeartnet/android-rust-home)

## Licence

Copyright (C) heartnetapps. Licensed under the GNU General Public License, version 3 or (at your
option) any later version. See [LICENSE](LICENSE). The names and logos are not covered by the licence.
