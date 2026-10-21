# Privacy

Android Rust Task Manager collects nothing and sends nothing. It has no accounts, no analytics, no advertising and
no connection to the internet or to any other device. It holds the Android network permission only to
talk to the phone's own debugging service and to its helper program, both on this phone (127.0.0.1). Your data stays on your phone, in files you can read yourself.

Permissions, and what each is for:

- Network (loopback only): talks to the phone's own debugging service and to a small helper program on this phone. Nothing is sent anywhere else.
- Notifications: to show the box where you type the pairing code.
- Shizuku (optional): lets the app use Shizuku if you have it installed.

If this ever changes it will be written here first.
