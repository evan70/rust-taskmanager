# AGENTS.md

> Structural map for AI agents and new developers. Update it when the project layout changes.

## Project Overview

An Android system monitor written in Rust (memory, CPU per core, temperatures, battery, storage,
network, every process and what starts at boot), drawn with egui as a native `android-activity` app.
Detailed readings that Android hides from ordinary apps come from a small on-phone helper program,
started either through Shizuku or through the phone's own Wireless debugging. Everything stays on the
phone. See `.ai-factory/DESCRIPTION.md` for the full specification.

## Tech Stack

- **Programming language:** Rust — `edition 2024` (app and shared crates), `edition 2021` (helper)
- **UI:** egui / eframe 0.37 (glow), immediate mode; a desktop preview binary for working without a phone
- **Android integration:** `android-activity` native-activity + Java glue + JNI, Shizuku API jars
- **Networking:** own `adbwire` crate (pairing, TLS, shell); loopback HTTP with a shared token
- **Database / ORM:** none (settings in a text file, token and adb key in files on the phone)
- **Build:** PowerShell — `tools/build/build-apk.ps1` (cargo-ndk → javac → d8 → aapt2 → zipalign → apksigner)
- **Tests:** `cargo test`, 76 `#[test]` functions in 17 files

## Project Structure

```
android-rust-taskmanager/
├── apps/taskmanager/            the app: Rust UI, Java glue, manifest/resources, its own build wrapper
│   ├── src/                     lib.rs (state+screens), sys.rs (platform), sample.rs + hdata.rs (data),
│   │                            hclient.rs (loopback client), adv.rs (detailed mode), ui_adv.rs (screens)
│   └── android/                 AndroidManifest.xml, java/, libs/ (Shizuku jars), res/
├── apps/taskmanager-cli/        the CLI front end: prints one snapshot (CPU, memory, top 10)
├── crates/domain/               the snapshot types (cpu, memory, battery, thermal, disks, network) —
│                                no I/O, no OS, serde
├── crates/linux-proc/           the one place that reads /proc (meminfo, stat, self/stat, loadavg);
│                                the app's basic mode and the CLI both go through it
├── crates/platform-linux/       fills a snapshot in on Linux: /proc through linux-proc, processes,
│                                disks, network and sensors through sysinfo, battery through /sys
├── crates/heartnet-kit/         shared by every Heartnet app: lifecycle, insets, settings, theme,
│                                diagnostics, time formatting + java/KitActivity.java
├── crates/adbwire/              the adb client written for this app (key, pairing, spake2, tls, conn)
├── helper/                      the on-phone program that reads what apps cannot see; separate Cargo
│   ├── src/{main.rs,api.rs}     workspace, ships inside the APK as libsysmon.so
│   └── shared/src/lib.rs        dumpsys/logcat parsers + safety rules shared with the PC debloater
├── tools/build/build-apk.ps1    the shared APK build for all Heartnet apps
├── Makefile                     everyday commands (test, lint, build, dev, apk, ci)
├── docs/                        INSTALL.md, DETAILED-MODE.md, BUILD.md, SIGNING.md
├── .agents/skills/              agent skills: the aif-* suite, installed community skills, and this
│                                project's own heartnet-android-app
├── .ai-factory/                 AI Factory context (config.yaml, DESCRIPTION.md, ARCHITECTURE.md,
│                                rules/base.md)
├── .codex/config.toml           MCP servers (aifhub, github)
└── skills-lock.json             sources and hashes of the community skills under .agents/skills/
```

## Key Entry Points

| File | Purpose |
|------|---------|
| `apps/taskmanager-cli/src/main.rs` | the console front end: one snapshot, top 10 processes by CPU |
| `crates/domain/src/lib.rs` | `Snapshot`/`CpuInfo`/`MemoryInfo`/`Process`/`BatteryInfo`/`ThermalZone`/`DiskInfo`/`NetworkInfo`, plus `used_percent`, `watts` and `celsius_of` |
| `crates/linux-proc/src/lib.rs` | `/proc` readers and parsers: `meminfo`, `cpu_times`, `self_ticks`, `load_average` |
| `crates/platform-linux/src/lib.rs` | `snapshot()` — the only file that talks to `/proc`, `/sys` and `sysinfo` |
| `apps/taskmanager/src/lib.rs` | app state (`TaskMgr`), tabs, screens, drawing, `android_main` |
| `apps/taskmanager/src/sys.rs` | the only place that talks to the phone (`android` and `desktop` modules) |
| `apps/taskmanager/src/sample.rs` | the app's own reading (`Sample`, `History`); `/proc` parsing is in `linux-proc`, and memory, storage, network and the battery's `bat_*` keys all become `domain` types |
| `apps/taskmanager/src/hdata.rs` | the helper's replies: cpu, memory, temperatures, battery and processes are the `domain` types (the JSON is written in that shape), plus the app's own extras (history, startup list), sorting, filtering |
| `apps/taskmanager/src/hclient.rs` | loopback HTTP client and its background threads |
| `apps/taskmanager/src/adv.rs` | detailed mode: key, pairing, starting the helper, Shizuku, plain-words errors |
| `apps/taskmanager/src/ui_adv.rs` | the detailed-mode screens (processes, startup, setup, sheets) |
| `apps/taskmanager/android/AndroidManifest.xml` | permissions, `KitActivity` launcher, Shizuku provider |
| `apps/taskmanager/build-apk.ps1` | builds the helper, then delegates to the shared build |
| `tools/build/build-apk.ps1` | the APK pipeline (Rust, Java, dex, manifest, zip, align, sign, verify) |
| `helper/src/api.rs` | the helper's HTTP API (`ping`, `snapshot`, `startup`, `act`) and action safety rules |
| `helper/shared/src/lib.rs` | parsers and protection rules (`valid_pkg`, `protected_reason`) |
| `Cargo.toml` | workspace members and the release profile |
| `.ai-factory/config.yaml` | AI Factory settings: language, paths, workflow, git behaviour |

## Documentation

| Document | Path | Description |
|----------|------|-------------|
| README | `README.md` | landing page: what the app is, features, build, layout |
| Installing | `docs/INSTALL.md` | downloading, checking and installing the APK |
| Detailed mode | `docs/DETAILED-MODE.md` | what Android hides, the two setup paths, the security model, troubleshooting |
| Building | `docs/BUILD.md` | toolchain, environment variables, the APK pipeline, desktop preview |
| Signing | `docs/SIGNING.md` | your own release keystore and fingerprints |
| Privacy | `PRIVACY.md` | what stays on the phone, and what each permission is for |
| Security | `SECURITY.md` | how to report a vulnerability privately |
| Third party | `THIRD_PARTY.md` | licences of everything shipped |
| Project spec | `.ai-factory/DESCRIPTION.md` | stack, architecture notes, non-functional requirements |
| Architecture | `.ai-factory/ARCHITECTURE.md` | architecture pattern, folder rules, dependency rules |

## AI Context Files

| File | Purpose |
|------|---------|
| `AGENTS.md` | this structural map |
| `.ai-factory/config.yaml` | AI Factory settings (language `sk`/`en`, paths, git behaviour) |
| `.ai-factory/DESCRIPTION.md` | what the project is, from a codebase analysis |
| `.ai-factory/ARCHITECTURE.md` | how code is organised and what depends on what |
| `.ai-factory/rules/base.md` | conventions detected in this codebase (naming, errors, logging, tests) |
| `.agents/skills/` | the skills an agent loads here, including `heartnet-android-app` for new apps |
| `.codex/config.toml` | MCP servers available to the agent (`aifhub`, `github`) |
| `skills-lock.json` | where the community skills came from and what they hashed to |

## Build & Development Commands

Everyday commands live in the `Makefile`; the APK itself is still assembled by the PowerShell scripts
in `apps/taskmanager/` and `tools/build/`.

| Command | What it does |
|---------|--------------|
| `make` | list every target |
| `make test` | tests for the app and the shared crates |
| `make test-helper` | tests for the helper workspace (`helper/` **and** `helper/shared`) |
| `make test-all` | both of the above |
| `make lint` | clippy over both workspaces, warnings are errors |
| `make build` | build the app and the shared crates (debug) |
| `make dev` | run the desktop preview (no phone, made-up phone data) |
| `make check-android` | compile the app and the helper for arm64 Android (needs only the NDK) |
| `make android-check` | say what is missing before attempting an APK build |
| `make apk` | build the signed APK (needs PowerShell + SDK/NDK) |
| `make ci` | lint + tests + build — what CI runs |
| `make clean` | remove build artifacts from both workspaces |

`make fmt` and `make fmt-check` exist but are deliberately **not** part of `make ci`: the sources are
written in a compact style that rustfmt would rewrite file by file.

## Agent Rules

- Run shell commands one at a time — do not chain them with `&&` or `;`.
  - Wrong: `git checkout main && git pull`
  - Right: `git checkout main`, then `git pull origin main`
- Git behaviour comes from `.ai-factory/config.yaml`: `base_branch: main`, `create_branches: true`
  with `branch_prefix: feature/`, so `$aif-plan full` starts a `feature/…` branch.
- Verify changes the way this project does: `make test` (or `make test-all` when the helper is
  involved), `make lint`, and for Android work `make android-check` then `make apk` plus
  `apksigner verify`. `make ci` runs the whole gate.
- `make ci` never compiles `#[cfg(target_os = "android")]` code. After touching anything the app
  compiles for the phone (sys.rs, heartnet-kit's android module, the helper, the Java glue), run
  `make check-android` — CI does, in `.github/workflows/android.yml`.
- Never add network access, analytics or accounts. `INTERNET` exists for loopback only.
- Do not touch the build machine's paths in generated binaries: keep the `--remap-path-prefix` lines
  in both build scripts.
