[Back to README](../README.md) · [Detailed mode →](DETAILED-MODE.md)

# Installing

The app is not on any store. You install the APK from this repository's releases page.

## What you need

- Android **11 or newer** (the app targets Android 14 / API 34, minSdk 30)
- an **arm64** phone (the APK ships one ABI)
- nothing else: basic mode works right after installing, with no setup and no computer

## Install

1. Download `taskmgr.apk` from the releases page.
2. Check it before installing:
   ```
   sha256sum taskmgr.apk                       # compare with the release notes
   apksigner verify --print-certs taskmgr.apk  # compare the certificate fingerprint
   ```
   The release notes list both, so you can confirm what you downloaded is what was published.
3. Open the file on the phone. Android asks once to allow installs from that source (your browser or
   file manager); allow it, then tap **Install**.
4. Open **Task Manager**. The Overview tab works immediately.

## After installing

- **Detailed mode is optional.** It shows CPU load, chip temperatures, every process and which apps
  started at boot, and it needs a one-time setup with the phone's own Wireless debugging or with
  Shizuku. See [Detailed mode](DETAILED-MODE.md).
- **Settings → What can this phone show?** lists which readings this particular phone allows.
- The app has no account, no analytics and no internet access; everything stays in the app's own
  folder. See [PRIVACY.md](../PRIVACY.md).

## Updates

Install a newer APK the same way. Your settings stay, because the same signing key is used, and
Android only accepts an update whose version code is higher than the installed one. Release notes
include the certificate fingerprint; if it ever differs from your installed app, do not install it.

## Removing the app

Uninstalling removes the app, its settings and its logs. Two things do not clean themselves up:

- If you **froze** any app in detailed mode, unfreeze it from the app before uninstalling — Android
does not undo that by itself, and the frozen app stays frozen without the app that froze it.
- The helper program lives in `/data/local/tmp` and disappears when the phone restarts. Uninstalling
does not remove it before then.

## Troubleshooting install problems

| What you see | What it means |
|---|---|
| "App not installed" / "There was a problem parsing the package" | the download is incomplete or the file is not an APK. Download it again and check the SHA-256. |
| "App not installed as package conflicts with an existing application" | a build signed with a different key is installed. Uninstall it first (its data goes too), then install this one. |
| The install is blocked with no prompt | allow installs from the app you are opening the file from, in your phone's settings. |
| Nothing happens after tapping the file | your file manager may not hand APKs to the installer; open it from the browser's download list instead. |

## See Also

- [Detailed mode](DETAILED-MODE.md) — what the extra readings need
- [Signing](SIGNING.md) — building and signing your own APK
- [Building](BUILD.md) — building from source
