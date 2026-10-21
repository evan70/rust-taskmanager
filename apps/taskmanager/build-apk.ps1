# Builds target\taskmgr.apk: the helper program (rust-sysmon, from the workspace root) is built first and
# shipped inside the APK as libsysmon.so, then the shared kit build does the rest.
param([switch]$SkipRust)
if (-not $env:JAVA_HOME -or -not $env:ANDROID_HOME -or -not $env:ANDROID_NDK_HOME) { Write-Host 'Set JAVA_HOME, ANDROID_HOME and ANDROID_NDK_HOME first (see README.md)'; exit 1 }
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
$helperRoot = (Resolve-Path "$PSScriptRoot\..\..\helper").Path
$bin = "$helperRoot\target\aarch64-linux-android\release\rust-sysmon"
if (-not $SkipRust) {
    Write-Host '== helper'
    Push-Location $helperRoot
    # keep the build machine's folder names out of the binary (panic messages carry file paths)
    $ws = (Resolve-Path "$PSScriptRoot\..\..").Path
    $env:CARGO_ENCODED_RUSTFLAGS = "--remap-path-prefix=$ws=/heartnet" + [char]0x1f + "--remap-path-prefix=$env:USERPROFILE=/home/build"
    cargo ndk -t arm64-v8a --platform 30 build --release 2>&1 | ForEach-Object { "$_" } | Where-Object { $_ -match '^error|Finished' }
    Pop-Location
}
if (-not (Test-Path $bin)) { Write-Host 'FAILED: helper build'; exit 1 }
& "$PSScriptRoot\..\..\tools\build\build-apk.ps1" -AppDir $PSScriptRoot -Lib taskmgr -SkipRust:$SkipRust -ExtraNative "libsysmon.so=$bin"
