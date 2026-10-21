//! The adb connection itself, for a phone already paired: connect to the wireless-debugging port,
//! switch the socket to TLS (adb's `STLS` step, presenting our key as the client certificate), and
//! run shell commands. The messages are adb's: a 24-byte header, then the data.

use crate::key::AdbKey;
use crate::tls;
use rustls::pki_types::ServerName;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const A_CNXN: u32 = 0x4e58_4e43;
pub const A_OPEN: u32 = 0x4e45_504f;
pub const A_OKAY: u32 = 0x5941_4b4f;
pub const A_CLSE: u32 = 0x4553_4c43;
pub const A_WRTE: u32 = 0x4554_5257;
pub const A_AUTH: u32 = 0x4854_5541;
pub const A_STLS: u32 = 0x534c_5453;

pub const VERSION: u32 = 0x0100_0001;
pub const STLS_VERSION: u32 = 0x0100_0000;
pub const MAX_DATA: u32 = 256 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq)]
pub struct Msg {
    pub cmd: u32,
    pub arg0: u32,
    pub arg1: u32,
    pub data: Vec<u8>,
}

pub fn write_msg<W: Write>(w: &mut W, m: &Msg) -> io::Result<()> {
    let mut h = Vec::with_capacity(24 + m.data.len());
    h.extend_from_slice(&m.cmd.to_le_bytes());
    h.extend_from_slice(&m.arg0.to_le_bytes());
    h.extend_from_slice(&m.arg1.to_le_bytes());
    h.extend_from_slice(&(m.data.len() as u32).to_le_bytes());
    // the data checksum is no longer checked at this protocol version
    h.extend_from_slice(&0u32.to_le_bytes());
    h.extend_from_slice(&(m.cmd ^ 0xffff_ffff).to_le_bytes());
    h.extend_from_slice(&m.data);
    w.write_all(&h)?;
    w.flush()
}

pub fn read_msg<R: Read>(r: &mut R) -> io::Result<Msg> {
    let mut h = [0u8; 24];
    r.read_exact(&mut h)?;
    let w = |i: usize| u32::from_le_bytes(h[i * 4..i * 4 + 4].try_into().unwrap());
    let (cmd, arg0, arg1, len, magic) = (w(0), w(1), w(2), w(3) as usize, w(5));
    if magic != cmd ^ 0xffff_ffff {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not an adb message"));
    }
    if len > MAX_DATA as usize * 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "adb message too large"));
    }
    let mut data = vec![0u8; len];
    r.read_exact(&mut data)?;
    Ok(Msg { cmd, arg0, arg1, data })
}

pub trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

pub struct Conn {
    s: Box<dyn Stream>,
    /// The phone's description of itself (its CNXN banner), e.g. "device::ro.product.name=...".
    pub banner: String,
    next_id: u32,
}

fn tls_err(e: rustls::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

impl Conn {
    /// Connect to the phone's wireless-debugging port. Fails if this key has not been paired.
    pub fn connect(addr: impl ToSocketAddrs, key: &AdbKey) -> io::Result<Conn> {
        let addr = addr.to_socket_addrs()?.next().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address"))?;
        let mut tcp = TcpStream::connect_timeout(&addr, IO_TIMEOUT)?;
        tcp.set_read_timeout(Some(IO_TIMEOUT))?;
        tcp.set_write_timeout(Some(IO_TIMEOUT))?;
        tcp.set_nodelay(true)?;
        write_msg(&mut tcp, &Msg { cmd: A_CNXN, arg0: VERSION, arg1: MAX_DATA, data: b"host::\0".to_vec() })?;
        let first = read_msg(&mut tcp)?;
        match first.cmd {
            A_STLS => {}
            A_AUTH => return Err(io::Error::new(io::ErrorKind::Unsupported, "the phone asked for old-style adb authentication, not TLS")),
            c => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected reply {c:#x} to the connection request"))),
        }
        write_msg(&mut tcp, &Msg { cmd: A_STLS, arg0: STLS_VERSION, arg1: 0, data: Vec::new() })?;

        let cert = key.certificate_der().map_err(|e| io::Error::other(e.to_string()))?;
        let config = tls::client_config(&cert, &key.pkcs8).map_err(tls_err)?;
        let conn = rustls::ClientConnection::new(config, ServerName::try_from("adb").expect("valid name")).map_err(tls_err)?;
        let mut s = rustls::StreamOwned::new(conn, tcp);
        let reply = read_msg(&mut s).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof || e.kind() == io::ErrorKind::ConnectionReset {
                io::Error::new(io::ErrorKind::PermissionDenied, "the phone closed the connection: this app is probably not paired with it (anymore)")
            } else {
                e
            }
        })?;
        if reply.cmd != A_CNXN {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("expected the phone's CNXN, got {:#x}", reply.cmd)));
        }
        Ok(Conn { s: Box::new(s), banner: String::from_utf8_lossy(&reply.data).trim_end_matches('\0').to_string(), next_id: 1 })
    }

    /// Run a shell command and return what it printed (standard output and error together). The
    /// command runs without a terminal, so something it starts in the background keeps running.
    pub fn shell(&mut self, command: &str) -> io::Result<String> {
        let local = self.next_id;
        self.next_id += 1;
        // "raw" asks adbd for a plain pipe. An ordinary shell gets a terminal, and when the command ends
        // the terminal closes and hangs up (kills) everything the command started in the background.
        let mut data = format!("shell,raw:{command}").into_bytes();
        data.push(0);
        write_msg(&mut self.s, &Msg { cmd: A_OPEN, arg0: local, arg1: 0, data })?;
        let mut remote = 0;
        let mut out = Vec::new();
        loop {
            let m = read_msg(&mut self.s)?;
            match m.cmd {
                A_OKAY => remote = m.arg0,
                A_WRTE => {
                    out.extend_from_slice(&m.data);
                    write_msg(&mut self.s, &Msg { cmd: A_OKAY, arg0: local, arg1: m.arg0, data: Vec::new() })?;
                }
                A_CLSE => {
                    if remote != 0 || m.arg0 != 0 {
                        // acknowledge the close if it was a close of our stream
                        let _ = write_msg(&mut self.s, &Msg { cmd: A_CLSE, arg0: local, arg1: m.arg0, data: Vec::new() });
                    }
                    break;
                }
                _ => {}
            }
        }
        Ok(String::from_utf8_lossy(&out).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::CertificateDer;
    use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
    use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
    use std::net::TcpListener;
    use std::sync::Arc;

    /// A stand-in phone: speaks CNXN/STLS, then TLS that insists on one specific client certificate,
    /// then answers `shell:` commands (it "runs" echo and unknown commands print nothing).
    #[derive(Debug)]
    struct OnlyThisClient {
        allowed: Vec<u8>,
    }

    impl ClientCertVerifier for OnlyThisClient {
        fn root_hint_subjects(&self) -> &[DistinguishedName] {
            &[]
        }
        fn verify_client_cert(&self, end: &CertificateDer<'_>, _i: &[CertificateDer<'_>], _now: rustls::pki_types::UnixTime) -> Result<ClientCertVerified, rustls::Error> {
            if end.as_ref() == self.allowed.as_slice() { Ok(ClientCertVerified::assertion()) } else { Err(rustls::Error::General("unknown key".into())) }
        }
        fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls12_signature(m, c, d, &tls::provider().signature_verification_algorithms)
        }
        fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls13_signature(m, c, d, &tls::provider().signature_verification_algorithms)
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            tls::provider().signature_verification_algorithms.supported_schemes()
        }
    }

    fn fake_phone(listener: TcpListener, allowed_cert: Vec<u8>) -> std::thread::JoinHandle<io::Result<()>> {
        std::thread::spawn(move || {
            let (mut tcp, _) = listener.accept()?;
            tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
            let hello = read_msg(&mut tcp)?;
            assert_eq!(hello.cmd, A_CNXN);
            assert_eq!(hello.data, b"host::\0");
            write_msg(&mut tcp, &Msg { cmd: A_STLS, arg0: STLS_VERSION, arg1: 0, data: vec![] })?;
            let reply = read_msg(&mut tcp)?;
            assert_eq!(reply.cmd, A_STLS);

            let cert = rcgen::generate_simple_self_signed(vec!["fake-phone".to_string()]).unwrap();
            let key = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
            let cfg = rustls::ServerConfig::builder_with_provider(tls::provider())
                .with_protocol_versions(&[&rustls::version::TLS13])
                .unwrap()
                .with_client_cert_verifier(Arc::new(OnlyThisClient { allowed: allowed_cert }))
                .with_single_cert(vec![cert.cert.der().clone()], rustls::pki_types::PrivateKeyDer::Pkcs8(key))
                .unwrap();
            let conn = rustls::ServerConnection::new(Arc::new(cfg)).map_err(tls_err)?;
            let mut s = rustls::StreamOwned::new(conn, tcp);
            write_msg(&mut s, &Msg { cmd: A_CNXN, arg0: VERSION, arg1: MAX_DATA, data: b"device::ro.product.name=fake;\0".to_vec() })?;
            loop {
                let m = match read_msg(&mut s) {
                    Ok(m) => m,
                    Err(_) => return Ok(()),
                };
                if m.cmd == A_OPEN {
                    let cmd = String::from_utf8_lossy(&m.data).trim_end_matches('\0').to_string();
                    let remote = 77;
                    write_msg(&mut s, &Msg { cmd: A_OKAY, arg0: remote, arg1: m.arg0, data: vec![] })?;
                    assert!(cmd.starts_with("shell,raw:"), "the client must ask for a raw shell (no terminal), got {cmd:?}");
                    let text = cmd.strip_prefix("shell,raw:echo ").map(|t| format!("{t}\n")).unwrap_or_default();
                    // send the answer in two pieces, as adbd does with longer output
                    let (a, b) = text.as_bytes().split_at(text.len() / 2);
                    for part in [a, b] {
                        if part.is_empty() {
                            continue;
                        }
                        write_msg(&mut s, &Msg { cmd: A_WRTE, arg0: remote, arg1: m.arg0, data: part.to_vec() })?;
                        let ack = read_msg(&mut s)?;
                        assert_eq!(ack.cmd, A_OKAY);
                    }
                    write_msg(&mut s, &Msg { cmd: A_CLSE, arg0: remote, arg1: m.arg0, data: vec![] })?;
                    let _ = read_msg(&mut s); // our CLSE back
                }
            }
        })
    }

    #[test]
    fn messages_have_adbs_24_byte_header() {
        let mut out = Vec::new();
        write_msg(&mut out, &Msg { cmd: A_OKAY, arg0: 1, arg1: 2, data: vec![9] }).unwrap();
        assert_eq!(out.len(), 25);
        assert_eq!(&out[0..4], b"OKAY");
        assert_eq!(&out[20..24], &(A_OKAY ^ 0xffff_ffff).to_le_bytes());
        let m = read_msg(&mut io::Cursor::new(out)).unwrap();
        assert_eq!(m, Msg { cmd: A_OKAY, arg0: 1, arg1: 2, data: vec![9] });
        // a corrupt magic is refused
        let mut bad = vec![0u8; 24];
        bad[0] = 1;
        assert!(read_msg(&mut io::Cursor::new(bad)).is_err());
    }

    #[test]
    fn a_paired_key_connects_and_runs_a_command() {
        let key = AdbKey::generate("task-manager@test").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let phone = fake_phone(listener, key.certificate_der().unwrap());
        let mut c = Conn::connect(("127.0.0.1", port), &key).expect("connect");
        assert!(c.banner.starts_with("device::"), "{}", c.banner);
        assert_eq!(c.shell("echo hello there").unwrap(), "hello there\n");
        assert_eq!(c.shell("echo again").unwrap(), "again\n", "a second command on the same connection");
        assert_eq!(c.shell("nothing").unwrap(), "");
        drop(c);
        let _ = phone.join();
    }

    #[test]
    fn an_unpaired_key_is_turned_away_with_a_clear_message() {
        let paired = AdbKey::generate("paired@test").unwrap();
        let stranger = AdbKey::generate("stranger@test").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let phone = fake_phone(listener, paired.certificate_der().unwrap());
        let err = match Conn::connect(("127.0.0.1", port), &stranger) {
            Ok(_) => panic!("must not connect"),
            Err(e) => e,
        };
        assert!(err.kind() == io::ErrorKind::PermissionDenied || err.kind() == io::ErrorKind::InvalidData, "{err:?}");
        let _ = phone.join();
    }
}
