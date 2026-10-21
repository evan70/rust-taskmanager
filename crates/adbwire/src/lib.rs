//! A small adb client for the phone itself: it pairs with the phone's wireless debugging (the
//! "pair with code" flow that Android 11 introduced), connects over TLS, and runs shell commands.
//! The protocol follows Android's own adb sources.

pub mod conn;
pub mod key;
pub mod net;
pub mod pairing;
pub mod spake2;
pub mod tls;
