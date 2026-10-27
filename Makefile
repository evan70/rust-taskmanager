# Android Rust Task Manager — build automation
#
# This repository has TWO Cargo workspaces: the app and shared crates at the root, and the on-phone
# helper in helper/. Targets that only make sense for one of them say so.
# The APK itself is still assembled by the PowerShell scripts (tools/build/build-apk.ps1); the
# `apk` targets call them and print what is missing when the Android toolchain is not set up.
#
# Usage: make           (same as `make help`)
#        make test      (the cheapest thing to run: no Android toolchain needed)

SHELL := bash
.ONESHELL:
.SHELLFLAGS := -eu -o pipefail -c
.DELETE_ON_ERROR:
MAKEFLAGS += --warn-undefined-variables
MAKEFLAGS += --no-builtin-rules

# --- Project ---
CARGO           ?= cargo
CARGOFLAGS      ?=
CLIPPYFLAGS     ?= -D warnings
POWERSHELL      ?= powershell
APP             ?= apps/taskmanager
HELPER_DIR      ?= helper
HELPER_MANIFEST ?= $(HELPER_DIR)/Cargo.toml
# The Android API level the APK is built against (tools/build/build-apk.ps1: --platform 30).
ANDROID_API     ?= 30

# --- Git ---
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo "dev")
COMMIT  ?= $(shell git rev-parse --short HEAD 2>/dev/null || echo "unknown")

# The Android targets read JAVA_HOME, ANDROID_HOME, ANDROID_NDK_HOME, HEARTNET_KEYSTORE and
# HEARTNET_KEYSTORE_PASS straight from the environment (see docs/BUILD.md and docs/SIGNING.md).

-include .env

.DEFAULT_GOAL := help

##@ Development

.PHONY: build
build: ## Build the app and the shared crates (debug)
	$(CARGO) build --workspace $(CARGOFLAGS)

.PHONY: build-release
build-release: ## Build the app and the shared crates (release)
	$(CARGO) build --workspace --release $(CARGOFLAGS)

.PHONY: check
check: ## Type-check the workspace without producing binaries
	$(CARGO) check --workspace --all-targets $(CARGOFLAGS)

.PHONY: dev
dev: ## Run the desktop preview (no phone, made-up phone data)
	$(CARGO) run -p taskmgr --bin taskmgr-desktop $(CARGOFLAGS)

.PHONY: info
info: ## Show the version, the commit and the workspace layout
	@echo "version: $(VERSION)"
	@echo "commit:  $(COMMIT)"
	@echo "app:     $(APP)  (package taskmgr, desktop bin taskmgr-desktop)"
	@echo "helper:  $(HELPER_DIR)  (separate workspace, package rust-sysmon)"

##@ Testing

.PHONY: test
test: ## Run the app and shared-crate tests
	$(CARGO) test --workspace $(CARGOFLAGS)

.PHONY: test-helper
test-helper: ## Run the helper workspace's tests (covers helper/ and helper/shared)
	cd $(HELPER_DIR) && $(CARGO) test --workspace $(CARGOFLAGS)

.PHONY: test-all
test-all: test test-helper ## Run every test in the repository

##@ Code Quality

.PHONY: lint
lint: ## Run clippy with warnings as errors (both workspaces)
	$(CARGO) clippy --workspace --all-targets -- $(CLIPPYFLAGS)
	cd $(HELPER_DIR) && $(CARGO) clippy --workspace --all-targets -- $(CLIPPYFLAGS)

.PHONY: fmt
fmt: ## Format all Rust sources
	$(CARGO) fmt --all
	cd $(HELPER_DIR) && $(CARGO) fmt --all

.PHONY: fmt-check
fmt-check: ## Check formatting without changing files (CI)
	$(CARGO) fmt --all -- --check
	cd $(HELPER_DIR) && $(CARGO) fmt --all -- --check

##@ Android

.PHONY: helper
helper: ## Build the on-phone helper for arm64 Android (needs cargo-ndk and the NDK)
	cd $(HELPER_DIR) && $(CARGO) ndk -t arm64-v8a --platform 30 build --release

# The app has code that only exists on Android (`#[cfg(target_os = "android")]`: the JNI glue in
# sys.rs, the native-activity entry point, heartnet-kit's android module). Nothing else in this file
# compiles it — `make ci` builds the desktop target and the APK is assembled on Windows — so this is
# the check that keeps it from rotting. It needs the NDK and nothing else.
.PHONY: check-android
check-android: ## Compile the app and the helper for arm64 Android (needs ANDROID_NDK_HOME)
	@test -n "$${ANDROID_NDK_HOME:-}" || { echo "ANDROID_NDK_HOME is not set (needs NDK r27c — see docs/BUILD.md)"; exit 1; }
	prebuilt=$$(echo "$$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*)
	clang="$$prebuilt/bin/aarch64-linux-android$(ANDROID_API)-clang"
	[ -x "$$clang" ] || { echo "no $$clang — is ANDROID_NDK_HOME a full NDK? (see docs/BUILD.md)"; exit 1; }
	export CC_aarch64_linux_android="$$clang" CXX_aarch64_linux_android="$$clang++"
	export AR_aarch64_linux_android="$$prebuilt/bin/llvm-ar"
	export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$$clang"
	$(CARGO) build --lib --target aarch64-linux-android $(CARGOFLAGS)
	cd $(HELPER_DIR) && $(CARGO) build --target aarch64-linux-android $(CARGOFLAGS)

.PHONY: android-check
android-check: ## Check whether this machine can build the APK, and say what is missing
	@ok=1
	if [ -z "$${JAVA_HOME:-}" ]; then echo "JAVA_HOME is not set (needs JDK 17)"; ok=0; fi
	if [ -z "$${ANDROID_HOME:-}" ]; then echo "ANDROID_HOME is not set (needs build-tools 34.0.0 and platform android-34)"; ok=0; fi
	if [ -z "$${ANDROID_NDK_HOME:-}" ]; then echo "ANDROID_NDK_HOME is not set (NDK r27c)"; ok=0; fi
	if [ -z "$${HEARTNET_KEYSTORE:-}" ]; then echo "HEARTNET_KEYSTORE is not set — needed to sign the APK (docs/SIGNING.md)"; ok=0; fi
	if [ -z "$${HEARTNET_KEYSTORE_PASS:-}" ]; then echo "HEARTNET_KEYSTORE_PASS is not set"; ok=0; fi
	command -v $(POWERSHELL) >/dev/null 2>&1 || { echo "$(POWERSHELL) is not on PATH (the APK build is PowerShell-only today)"; ok=0; }
	command -v cargo-ndk >/dev/null 2>&1 || { echo "cargo-ndk is not installed: cargo install cargo-ndk"; ok=0; }
	rustup target list --installed 2>/dev/null | grep -qx aarch64-linux-android || { echo "the Android target is missing: rustup target add aarch64-linux-android"; ok=0; }
	if [ "$$ok" -eq 0 ]; then echo; echo "See docs/BUILD.md for the full list."; exit 1; fi
	echo "The Android build environment looks complete."

.PHONY: apk
apk: android-check ## Build the signed APK (Windows PowerShell; see docs/BUILD.md)
	$(POWERSHELL) -File $(APP)/build-apk.ps1

.PHONY: apk-java
apk-java: android-check ## Rebuild only the Java/manifest part of the APK (reuses the last Rust build)
	$(POWERSHELL) -File $(APP)/build-apk.ps1 -SkipRust

##@ CI

# `fmt-check` is deliberately NOT part of `ci`: the existing sources are written in a compact style
# that rustfmt would rewrite from top to bottom. Run `make fmt` when you decide to adopt rustfmt.
.PHONY: ci
ci: lint test-all build ## Run everything CI runs on the host (no Android toolchain needed)
	@echo "ci: all checks passed."

##@ Cleanup

.PHONY: clean
clean: ## Remove build artifacts from both workspaces
	$(CARGO) clean
	$(CARGO) clean --manifest-path $(HELPER_MANIFEST)

.PHONY: clean-apk
clean-apk: ## Remove the APK build output (keeps the Rust target directories)
	rm -rf $(APP)/target/apk-build $(APP)/target/taskmgr.apk
	@echo "removed $(APP)/target/apk-build and $(APP)/target/taskmgr.apk (version-code.txt is kept)"

##@ Help

.PHONY: help
help: ## Show this help
	@awk 'BEGIN {FS = ":.*##"; printf "Usage:\n  make \033[36m<target>\033[0m\n"} \
		/^[a-zA-Z_-]+:.*?## / {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2} \
		/^##@/ {printf "\n\033[1m%s\033[0m\n", substr($$0, 5)}' $(MAKEFILE_LIST)
	@echo
	@echo "`check-android` needs the NDK; the APK targets need PowerShell + the SDK too; everything else is plain cargo."
