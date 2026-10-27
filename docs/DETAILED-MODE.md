[← Installing](INSTALL.md) · [Back to README](../README.md) · [Building →](BUILD.md)

# Detailed mode

Android hides other apps' CPU use, their memory and the chip temperatures from ordinary apps. Basic
mode shows only what Android allows an app to see. Detailed mode adds the rest by starting a small
helper program on the phone with the same access the phone's own `adb shell` has.

Everything happens on the phone: no computer, no other app, no internet.

## What detailed mode adds

| Reading | Basic mode | Detailed mode |
|---|---|---|
| Memory, storage, battery, network speed | yes | yes |
| Overall CPU load, every core, MHz | what Android allows | all of it |
| Chip temperatures (CPU, GPU, skin) | no | yes |
| Every process with CPU and memory | no | yes |
| Per-app memory, running/frozen state | no | yes |
| Which apps started at boot, and when | list only | with the boot log |
| Stop, Freeze, Unfreeze your own apps | no | yes |

Whether a particular phone allows a particular reading is shown in **Settings → What can this phone
show?**, so a missing number is never a mystery.

> None of this leaves the phone. See [PRIVACY.md](../PRIVACY.md).

## Starting the helper

There are two ways in, and both start the same helper.

### With Shizuku, if you already use it

One tap: **Start with Shizuku**. The app checks where you are and tells you what it needs:

| Shizuku says | The app tells you |
|---|---|
| not installed | "Shizuku is not installed." |
| installed, not running | "Shizuku is installed but not running. Open it and start it first." |
| running, app not allowed yet | asks Shizuku for permission and waits for you to allow it |
| allowed | starts the helper |

### With the phone's own Wireless debugging

No computer is involved — the app speaks to the phone's debugging service on the phone itself, using
the pairing flow Android 11 introduced.

1. Turn on **Settings → Developer options → Wireless debugging** (it needs Wi-Fi, not internet).
2. Tap **Set up detailed mode** in the app and choose the pairing route.
3. Android shows a pop-up with a six-digit code (**Pair device with pairing code**). Type those six
   digits into the app's notification.
4. The app pairs, connects and starts the helper. The pairing code is needed once.

The app makes its own adb key for this (`task-manager@phone`), keeps it in its own folder, and the
pairing, TLS connection and shell are the app's own implementation, not a shelled-out `adb`.

### What the start line does

The app copies the helper out of its own native library directory and runs it:

```
pkill -x tm-sysmon
cp '<app native lib dir>/libsysmon.so' /data/local/tmp/tm-sysmon
chmod 755 /data/local/tmp/tm-sysmon
(nohup /data/local/tmp/tm-sysmon --interval 30 --log /data/local/tmp/tm-sysmon.log \
   --api 7880 --token-file '<token file>' < /dev/null > /dev/null 2>&1 &)
```

An older copy is stopped by exact process name (`pkill -x`). If the helper does not answer, the app
tells you what it found: the file, the process, and the last lines of the helper's own log.

## After a restart

The helper does not survive a reboot. Turn Wireless debugging back on and tap **Connect and start**
(no code this time), or tap **Start with Shizuku**. Detailed mode's setting stays on, so the app also
tries a quiet reconnect when it starts.

## How the app and the helper talk

| | |
|---|---|
| Address | `127.0.0.1:7880` — the phone's own loopback, nothing else |
| Protocol | plain HTTP/1.0 with an `X-Token` header on every request |
| Secret | a random token the app keeps in its own private folder and hands to the helper as a file |
| Endpoints | `GET /v1/ping`, `GET /v1/snapshot`, `GET /v1/startup`, `POST /v1/act?op=stop\|freeze\|unfreeze` |

A request without the right token is refused with `403`.

## What the helper will not do

- It refuses anything that is not one of *your installed apps*.
- It refuses your keyboard, your home screen, your phone app, your messages app and your alarm clock.
- It refuses Android's own components (`com.android.*`, `android.*`, and the vendor namespaces and
  Google Play services that ship with the phone).
- It never raises an app that is in front of you.

Every package name is validated before it can reach a shell command, and every action returns a
result per app, so one refusal does not stop the rest:

```
3 done, 1 refused or failed (com.example.bank: refused: system component)
```

## Being polite about battery

- The app asks for a snapshot once a second **only while it is on screen**; backgrounded, it stops.
- The helper samples only while a client has asked within the last 8 seconds. Otherwise it sleeps and
  drops its baselines.

## If something does not work

| What you see | What it means |
|---|---|
| "helper not running" | the helper is not up. Connect again (no code needed). |
| "helper refused the token (switch detailed mode off and on again)" | the helper on the phone was started with an older token. Connect again so it is restarted with the current one. |
| "The phone refused: the code was wrong, or this app is no longer paired." | start the setup again and enter a fresh code. |
| "The phone is not accepting connections. Is Wireless debugging still on?" | the developer options toggle went off (it often does after a reboot). |
| "The phone did not answer in time. Keep the Wireless debugging screen open and try again." | Android closes the pairing window if the screen is not showing it. |
| "Could not find the pairing window." | no pairing port was found within 6 seconds — reopen **Pair device with pairing code** and try again. |
| "Wireless debugging does not seem to be on." | no debugging port was found within 10 seconds. |
| "The helper did not start." | the message includes the start line's own output and the result of checking the file, the process and the log. |

**Settings → What can this phone show?** lists what this particular phone permits, and the Setup
screen keeps a step-by-step log of the last 40 messages from the setup, useful when reporting a
problem.

## See Also

- [Installing](INSTALL.md) — getting the APK onto the phone
- [Building](BUILD.md) — building the app and the helper from source
- [PRIVACY.md](../PRIVACY.md) — what stays on the phone and why each permission exists
