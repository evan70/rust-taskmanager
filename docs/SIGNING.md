[← Building](BUILD.md) · [Back to README](../README.md)

# Signing

Android only installs signed APKs. The key is yours: it is never in this repository, and the build
reads it from two environment variables.

## Make a keystore (once)

```
keytool -genkeypair -v -keystore heartnet-release.keystore -alias heartnet -keyalg RSA -keysize 4096 -validity 10000
```

Keep the keystore file **and its password** somewhere safe and backed up:

- If the key is lost, you can no longer update the apps installed from it — every future build would
  have a different identity, and Android refuses to replace an app with a differently signed one.
- The password is not recoverable. Store it in your password manager, not in the repository.

## Point the build at it

In the shell that builds (see [Building](BUILD.md)):

```
$env:HEARTNET_KEYSTORE = '<full path to>\heartnet-release.keystore'
$env:HEARTNET_KEYSTORE_PASS = '<the password>'
```

`tools/build/build-apk.ps1` uses both for `apksigner sign` and then runs `apksigner verify` on the
result. If either variable is missing, the build stops with
`Set HEARTNET_KEYSTORE and HEARTNET_KEYSTORE_PASS first (see docs/SIGNING.md)`.

Signing is only needed for an APK. `cargo test` and the desktop build need no keystore at all.

## What to publish with a release

```
apksigner verify --print-certs target/taskmgr.apk
sha256sum target/taskmgr.apk
```

Put the certificate fingerprint and the SHA-256 in the release notes, so people can check what they
downloaded before installing it (see [Installing](INSTALL.md)). Keeping the same key across releases
is what lets their updates keep their data.

## Debug builds

The desktop build involves no signing at all. `-SkipRust` only skips the Rust compile step (useful
while changing Java or the manifest) — the build still signs with your keystore and still bumps the
version code.

For your own testing, a locally kept key is enough: just remember that anything signed with it cannot
be updated by the released app, and vice versa.

## See Also

- [Building](BUILD.md) — the full build, including the keystore environment variables
- [Installing](INSTALL.md) — how people verify a release before installing
- [SECURITY.md](../SECURITY.md) — reporting a security problem privately
