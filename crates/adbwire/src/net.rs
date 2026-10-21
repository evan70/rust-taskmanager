//! Putting the pieces on a socket: pair with a phone, or (for the tests) play the phone.

use crate::key::AdbKey;
use crate::pairing::{self, PEER_RSA_PUBKEY, PeerInfo, TLS_EXPORT_LABEL, TLS_EXPORT_LEN};
use crate::spake2::Role;
use crate::tls;
use rustls::pki_types::ServerName;
use std::io;
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_secs(15);

fn tls_err(e: rustls::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

/// Pair with the phone's pairing port using the six-digit code. On success the phone has stored
/// `key` as an allowed key; the returned info is the phone's own (its id).
pub fn pair(addr: impl ToSocketAddrs, code: &str, key: &AdbKey) -> io::Result<PeerInfo> {
    let addr = addr.to_socket_addrs()?.next().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address"))?;
    let mut tcp = TcpStream::connect_timeout(&addr, IO_TIMEOUT)?;
    tcp.set_read_timeout(Some(IO_TIMEOUT))?;
    tcp.set_write_timeout(Some(IO_TIMEOUT))?;
    tcp.set_nodelay(true)?;

    let cert = key.certificate_der().map_err(|e| io::Error::other(e.to_string()))?;
    let config = tls::client_config(&cert, &key.pkcs8).map_err(tls_err)?;
    let mut conn = rustls::ClientConnection::new(config, ServerName::try_from("adb").expect("valid name")).map_err(tls_err)?;
    while conn.is_handshaking() {
        conn.complete_io(&mut tcp)?;
    }
    let export = conn.export_keying_material([0u8; TLS_EXPORT_LEN], TLS_EXPORT_LABEL, None).map_err(tls_err)?;
    let mut stream = rustls::Stream::new(&mut conn, &mut tcp);
    let ours = PeerInfo { kind: PEER_RSA_PUBKEY, data: key.adb_public_text() };
    pairing::exchange(&mut stream, Role::Alice, code.as_bytes(), &export, &ours)
}

/// Play the phone for one pairing: accept a connection, run the conversation, say who we are.
/// Used by the tests (and by the interoperability check against the real adb client).
pub fn serve_pairing_once(listener: &TcpListener, code: &str, ours: &PeerInfo) -> io::Result<PeerInfo> {
    let (mut tcp, _) = listener.accept()?;
    tcp.set_read_timeout(Some(IO_TIMEOUT))?;
    tcp.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut conn = rustls::ServerConnection::new(tls::test_server_config()).map_err(tls_err)?;
    while conn.is_handshaking() {
        conn.complete_io(&mut tcp)?;
    }
    let export = conn.export_keying_material([0u8; TLS_EXPORT_LEN], TLS_EXPORT_LABEL, None).map_err(tls_err)?;
    let mut stream = rustls::Stream::new(&mut conn, &mut tcp);
    pairing::exchange(&mut stream, Role::Bob, code.as_bytes(), &export, ours)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pairing::PEER_DEVICE_GUID;

    fn phone_info() -> PeerInfo {
        PeerInfo { kind: PEER_DEVICE_GUID, data: b"test-phone-guid\0".to_vec() }
    }

    #[test]
    fn pairing_works_end_to_end_with_the_right_code() {
        let key = AdbKey::generate("task-manager@test").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || serve_pairing_once(&listener, "123456", &phone_info()));
        let got = pair(("127.0.0.1", port), "123456", &key).expect("pairing");
        assert_eq!(got.kind, PEER_DEVICE_GUID);
        assert_eq!(got.text(), "test-phone-guid");
        let seen = server.join().unwrap().expect("server side");
        assert_eq!(seen.kind, PEER_RSA_PUBKEY);
        assert_eq!(seen.data, key.adb_public_text().iter().copied().take_while(|b| *b != 0).collect::<Vec<u8>>(), "the phone received our public key");
    }

    #[test]
    fn the_wrong_code_is_refused_on_both_sides() {
        let key = AdbKey::generate("task-manager@test").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || serve_pairing_once(&listener, "123456", &phone_info()));
        let err = pair(("127.0.0.1", port), "654321", &key).unwrap_err();
        assert!(matches!(err.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::InvalidData), "{err:?}");
        assert!(server.join().unwrap().is_err());
    }
}
