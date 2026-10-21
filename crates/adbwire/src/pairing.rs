//! Wireless-debugging pairing, as Android 11 does it: a TLS 1.3 connection to the pairing port, a
//! SPAKE2 exchange whose password is the six-digit code plus a value exported from the TLS session
//! (so the code cannot be replayed on another connection), and then each side sends its identity
//! encrypted with a key from that exchange. Follows adb's pairing_auth / pairing_connection sources.

use crate::spake2::{Role, Spake2};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use std::io::{self, Read, Write};

pub const CLIENT_NAME: &[u8] = b"adb pair client\0";
pub const SERVER_NAME: &[u8] = b"adb pair server\0";
pub const PEER_INFO_SIZE: usize = 8192;
pub const TLS_EXPORT_LABEL: &[u8] = b"adb-label\0";
pub const TLS_EXPORT_LEN: usize = 64;

const VERSION: u8 = 1;
const PKT_SPAKE2: u8 = 0;
const PKT_PEER_INFO: u8 = 1;
const MAX_PAYLOAD: usize = PEER_INFO_SIZE * 2;

/// What a peer tells the other during pairing: our key (kind 0) or the phone's id (kind 1).
pub const PEER_RSA_PUBKEY: u8 = 0;
pub const PEER_DEVICE_GUID: u8 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct PeerInfo {
    pub kind: u8,
    pub data: Vec<u8>,
}

impl PeerInfo {
    /// The data up to its first NUL, as text.
    pub fn text(&self) -> String {
        let end = self.data.iter().position(|b| *b == 0).unwrap_or(self.data.len());
        String::from_utf8_lossy(&self.data[..end]).to_string()
    }

    fn to_bytes(&self) -> io::Result<Vec<u8>> {
        if self.data.len() > PEER_INFO_SIZE - 1 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "peer info too large"));
        }
        let mut b = vec![0u8; PEER_INFO_SIZE];
        b[0] = self.kind;
        b[1..1 + self.data.len()].copy_from_slice(&self.data);
        Ok(b)
    }

    fn from_bytes(b: &[u8]) -> io::Result<PeerInfo> {
        if b.len() != PEER_INFO_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("peer info is {} bytes, expected {PEER_INFO_SIZE}", b.len())));
        }
        let mut data = b[1..].to_vec();
        while data.last() == Some(&0) {
            data.pop();
        }
        Ok(PeerInfo { kind: b[0], data })
    }
}

struct Cipher {
    aead: Aes128Gcm,
    enc_seq: u64,
    dec_seq: u64,
}

fn nonce(seq: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..8].copy_from_slice(&seq.to_le_bytes());
    n
}

impl Cipher {
    fn new(material: &[u8]) -> Cipher {
        let mut key = [0u8; 16];
        Hkdf::<Sha256>::new(None, material).expand(b"adb pairing_auth aes-128-gcm key", &mut key).expect("16 bytes is a valid length");
        Cipher { aead: Aes128Gcm::new_from_slice(&key).expect("16-byte key"), enc_seq: 0, dec_seq: 0 }
    }

    fn encrypt(&mut self, data: &[u8]) -> io::Result<Vec<u8>> {
        let n = nonce(self.enc_seq);
        let out = self.aead.encrypt(Nonce::from_slice(&n), data).map_err(|_| io::Error::other("encryption failed"))?;
        self.enc_seq += 1;
        Ok(out)
    }

    fn decrypt(&mut self, data: &[u8]) -> io::Result<Vec<u8>> {
        let n = nonce(self.dec_seq);
        let out = self.aead.decrypt(Nonce::from_slice(&n), data).map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "wrong pairing code"))?;
        self.dec_seq += 1;
        Ok(out)
    }
}

fn write_packet<W: Write>(w: &mut W, kind: u8, payload: &[u8]) -> io::Result<()> {
    let mut head = [0u8; 6];
    head[0] = VERSION;
    head[1] = kind;
    head[2..6].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    w.write_all(&head)?;
    w.write_all(payload)?;
    w.flush()
}

fn read_packet<R: Read>(r: &mut R, expect: u8) -> io::Result<Vec<u8>> {
    let mut head = [0u8; 6];
    r.read_exact(&mut head)?;
    if head[0] != VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("pairing version {} is not supported", head[0])));
    }
    if head[1] != expect {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected pairing packet type {}", head[1])));
    }
    let len = u32::from_be_bytes(head[2..6].try_into().unwrap()) as usize;
    if len == 0 || len > MAX_PAYLOAD {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "pairing payload size is out of range"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// Run the pairing conversation on a connection that is already TLS-protected. `code` is the six
/// digits; `tls_export` is `adb-label` exported from the TLS session (64 bytes). Returns what the
/// other side said about itself.
pub fn exchange<S: Read + Write>(stream: &mut S, role: Role, code: &[u8], tls_export: &[u8], ours: &PeerInfo) -> io::Result<PeerInfo> {
    let mut password = code.to_vec();
    password.extend_from_slice(tls_export);
    let (me, them) = match role {
        Role::Alice => (CLIENT_NAME, SERVER_NAME),
        Role::Bob => (SERVER_NAME, CLIENT_NAME),
    };
    let spake = Spake2::new(role, me, them, &password);
    write_packet(stream, PKT_SPAKE2, &spake.msg())?;
    let their_msg = read_packet(stream, PKT_SPAKE2)?;
    let material = spake.finish(&their_msg).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "the other side's SPAKE2 message was not valid"))?;
    let mut cipher = Cipher::new(&material);

    let sealed = cipher.encrypt(&ours.to_bytes()?)?;
    write_packet(stream, PKT_PEER_INFO, &sealed)?;
    let theirs = read_packet(stream, PKT_PEER_INFO)?;
    PeerInfo::from_bytes(&cipher.decrypt(&theirs)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn peer_info_survives_the_wire_format() {
        let p = PeerInfo { kind: PEER_RSA_PUBKEY, data: b"QUFB key@host\0".to_vec() };
        let b = p.to_bytes().unwrap();
        assert_eq!(b.len(), PEER_INFO_SIZE);
        let back = PeerInfo::from_bytes(&b).unwrap();
        assert_eq!(back.kind, 0);
        assert_eq!(back.text(), "QUFB key@host");
        assert!(PeerInfo { kind: 0, data: vec![1; PEER_INFO_SIZE] }.to_bytes().is_err(), "too large");
        assert!(PeerInfo::from_bytes(&[0u8; 10]).is_err(), "wrong size");
    }

    #[test]
    fn packets_have_adbs_header() {
        let mut out = Vec::new();
        write_packet(&mut out, PKT_SPAKE2, &[9, 8, 7]).unwrap();
        assert_eq!(out, [1, 0, 0, 0, 0, 3, 9, 8, 7]);
        assert_eq!(read_packet(&mut Cursor::new(out), PKT_SPAKE2).unwrap(), [9, 8, 7]);
    }

    #[test]
    fn bad_packets_are_refused() {
        // wrong version, wrong type, zero and huge lengths
        for bytes in [vec![2u8, 0, 0, 0, 0, 1, 0], vec![1, 1, 0, 0, 0, 1, 0], vec![1, 0, 0, 0, 0, 0], vec![1, 0, 0, 1, 0, 0, 0]] {
            assert!(read_packet(&mut Cursor::new(bytes), PKT_SPAKE2).is_err());
        }
        assert!(read_packet(&mut Cursor::new(vec![1u8, 0, 0, 0, 0, 5, 1]), PKT_SPAKE2).is_err(), "cut short");
    }

    #[test]
    fn the_cipher_counts_messages_and_rejects_tampering() {
        let mut a = Cipher::new(&[5u8; 64]);
        let mut b = Cipher::new(&[5u8; 64]);
        let c1 = a.encrypt(b"first").unwrap();
        let c2 = a.encrypt(b"first").unwrap();
        assert_ne!(c1, c2, "each message uses its own nonce");
        assert_eq!(b.decrypt(&c1).unwrap(), b"first");
        assert_eq!(b.decrypt(&c2).unwrap(), b"first");
        let mut bad = a.encrypt(b"x").unwrap();
        bad[0] ^= 1;
        assert!(b.decrypt(&bad).is_err());
        let mut other = Cipher::new(&[6u8; 64]);
        let good = a.encrypt(b"y").unwrap();
        assert!(other.decrypt(&good).is_err());
    }
}
