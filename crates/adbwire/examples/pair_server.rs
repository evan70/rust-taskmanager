//! Plays the phone's pairing port once, so the real `adb pair` can be tried against this code:
//!
//!     cargo run --release --example pair_server -- 127.0.0.1:41999 123456
//!     adb pair 127.0.0.1:41999 123456
use adbwire::net::serve_pairing_once;
use adbwire::pairing::{PEER_DEVICE_GUID, PeerInfo};
use std::net::TcpListener;

fn main() {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:41999".to_string());
    let code = args.next().unwrap_or_else(|| "123456".to_string());
    let listener = TcpListener::bind(&addr).expect("bind");
    println!("listening on {addr}, code {code}");
    let ours = PeerInfo { kind: PEER_DEVICE_GUID, data: b"adbwire-interop-test\0".to_vec() };
    match serve_pairing_once(&listener, &code, &ours) {
        Ok(info) => println!("PAIRED: the client sent kind {} text {:?}", info.kind, info.text().chars().take(60).collect::<String>()),
        Err(e) => {
            println!("FAILED: {e}");
            std::process::exit(1);
        }
    }
}
