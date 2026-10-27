[← Detailed mode](DETAILED-MODE.md) · [Back to README](../README.md) · [Signing →](SIGNING.md)

# Building

The app is written in Rust and drawn with egui; the Android side is a native activity plus a few
Java classes. Working on the UI needs nothing but Rust — building the APK needs the Android tools.

## Prerequisites

| Tool | Version used here |
|---|---|
| Rust | **1.87 or newer** (the helper uses `is_none_or` and `u64::is_multiple_of`), with the `aarch64-linux-android` target (`rustup target add aarch64-linux-android`) |
| cargo-ndk | `cargo install cargo-ndk` |
| Android SDK | build-tools **34.0.0**, platform **android-34** |
| Android NDK | **r27c** |
| JDK | **17** |
| PowerShell | both build scripts are PowerShell (Windows PowerShell 5+ or PowerShell 7) |

Set these before building:

```
JAVA_HOME          the JDK 17 folder
ANDROID_HOME       the Android SDK folder
ANDROID_NDK_HOME   the NDK folder
HEARTNET_KEYSTORE  full path to your release keystore   (see docs/SIGNING.md)
HEARTNET_KEYSTORE_PASS  its password
```

## Everyday commands

The `Makefile` wraps the common ones (`make help` lists everything):

| Command | What it does |
|---|---|
| `make test` | tests for the app and the shared crates (no Android tools needed) |
| `make test-all` | the same, plus the helper's own workspace |
| `make lint` | `cargo clippy` with warnings as errors, both workspaces |
| `make build` / `make build-release` | build the app and the shared crates |
| `make dev` | run the desktop preview |
| `make check-android` | compile the app and the helper for arm64 Android (needs only the NDK) |
| `make android-check` | say exactly what the APK build is missing on this machine |
| `make apk` / `make apk-java` | build the signed APK / only its Java part |
| `make ci` | lint + tests + build — what CI runs |

The recipes are thin wrappers: everything the Makefile does can also be run by hand, which is what the
sections below show.

## Working on the UI without a phone

There is a desktop build with made-up phone data, shaped like a phone window:

```
cd apps/taskmanager
cargo run --bin taskmgr-desktop        # or: make dev
```

It is not a mock-up of everything: memory, overall CPU, per-core load and MHz come from the machine
you run it on (`/proc/meminfo`, `/proc/stat`, `/sys/.../cpufreq`), while battery, storage, network,
heat, device info, icons and the boot list are invented by the desktop module of `sys.rs`. The
Processes tab needs the helper, so it stays empty there.

Helpful switches for previewing screens directly:

| Variable | Effect |
|---|---|
| `TASKMGR_DIR` | where settings, logs and the token live (default: a temp folder) |
| `TASKMGR_TAB` | `processes` or `startup` — opens that tab first |
| `TASKMGR_OPEN` | a pid — opens that process's panel first |

The window backend comes from winit and is chosen at compile time, so the app enables eframe's `x11`
and `wayland` features for every non-Android target (`apps/taskmanager/Cargo.toml`). Without them the
build stops with `The platform you're compiling for is not supported by winit`, which is what used to
happen on Linux. `make ci` compiles and links this on Linux today, so a normal development machine
needs nothing extra for the preview.

## Tests

```
cargo test --workspace                        # app and shared crates
cd helper && cargo test --workspace           # the helper workspace
```

Or through the Makefile: `make test`, `make test-all`, `make lint`, `make ci`.

Two things worth knowing about how these are wired:

- The helper's workspace holds **two** crates: `rust-sysmon` and `phone-shared`. Running
  `cargo test --manifest-path helper/Cargo.toml` tests only the first; from `helper/`, `cargo test
  --workspace` covers both. The Makefile uses the second form.
- `cargo clippy` with `-D warnings` is the lint gate (`make lint`). It is clean today, so a new
  warning fails the gate.

A behaviour you add or fix gets a test. The tests cover parsing and decision helpers (they are pure
functions on purpose), not the UI: 76 tests in total — 23 in the app, 17 in `adbwire`, 14 in `domain`,
5 in `linux-proc`, 4 in `heartnet-kit`, 2 in `platform-linux`, 5 in `rust-sysmon` and 6 in
`phone-shared`.

## Compiling the Android code without the SDK

Some of the app exists only on Android: the JNI glue and the native-activity entry point
(`#[cfg(target_os = "android")]` in `apps/taskmanager/src/sys.rs`), `crates/heartnet-kit/src/android.rs`,
and the helper's arm64 build. `make ci` compiles none of it (it builds the desktop target), and the APK
build needs the SDK, the JDK and PowerShell, so this is the cheap way to keep it honest:

```
export ANDROID_NDK_HOME=/path/to/android-ndk-r27c
make check-android
```

It compiles the app's library (`libtaskmgr.so`) and the helper for `aarch64-linux-android` with the
NDK's clang, at the API level the APK uses (30). It needs the NDK and the Rust target and nothing else,
which is why CI runs it on Linux on every push (`.github/workflows/android.yml`).

On a Linux machine that has no SDK, a bare NDK is enough; if you keep one inside the project, put it in
`.android-ndk/` (gitignored) so `export ANDROID_NDK_HOME=$PWD/.android-ndk/android-ndk-r27c` works.

## Building the APK

```
cd apps/taskmanager
powershell -File build-apk.ps1
```

That produces `apps/taskmanager/target/taskmgr.apk`. The app's script does one thing of its own —
builds the helper — and then hands over to the shared build in `tools/build/build-apk.ps1`:

1. **helper** — `cargo ndk -t arm64-v8a --platform 30 build --release` in `helper/` produces
   `rust-sysmon`, which is shipped inside the APK as `libsysmon.so`.
2. **Rust** — the app is built as a library (`cargo ndk … build --release --lib`) into
   `libtaskmgr.so`.
3. **Java** — `javac` compiles `crates/heartnet-kit/java` (the shared `KitActivity`) and the app's own
   `android/java` classes; any `.jar` in `apps/taskmanager/android/libs` is on the classpath.
4. **Dex** — `d8` turns those classes into `classes.dex`.
5. **Manifest and resources** — `aapt2` compiles `android/res` and links the manifest with it, at
   minSdk 30 and targetSdk 34.
6. **Packaging** — `classes.dex`, `lib/arm64-v8a/libtaskmgr.so` and the helper are added into the
   APK's zip.
7. **Align and sign** — `zipalign`, then `apksigner sign` with `HEARTNET_KEYSTORE`, then
   `apksigner verify`. The script prints `OK: <path> (<size> MB)` only when the APK verified.

Both Rust steps remap source paths (`--remap-path-prefix`), so panic messages do not leak the build
machine's folder layout.

### Version codes

Each build bumps `apps/taskmanager/version-code.txt` (gitignored, first build 16777473) and sets
`versionName` to `1.0.<n>`. Android only accepts an update that has the same signing key and a higher
version code, so keep the counter and the keystore.

`-SkipRust` rebuilds only the Java/manifest part and reuses the previous native libraries — handy
when changing a screen that is drawn from Java, useless for Rust changes.

## Where the pieces live

| Piece | Path |
|---|---|
| App | `apps/taskmanager/` (Rust in `src/`, Java/manifest/resources in `android/`) |
| Shared kit (used by other Heartnet apps) | `crates/heartnet-kit/` + `java/KitActivity.java` |
| adb client (pairing, TLS, shell) | `crates/adbwire/` |
| On-phone helper | `helper/` (its own Cargo workspace) + `helper/shared/` |
| The shared APK build | `tools/build/build-apk.ps1` |

## Known gaps

- Both scripts are PowerShell and call `aapt2.exe`, `d8.bat`, `zipalign.exe`, `apksigner.bat`, so
  there is no Linux or CI build path yet; porting means calling the same build-tools programs without
  the `.exe`/`.bat` suffixes.
- The APK is **arm64-v8a only**; there is no per-ABI loop.
- `cargo test` needs no Android tools at all, so it is the cheapest thing to run on any machine.

## See Also

- [Detailed mode](DETAILED-MODE.md) — what the helper does and how it is started
- [Signing](SIGNING.md) — making your own release keystore
- [Installing](INSTALL.md) — putting the result on a phone
