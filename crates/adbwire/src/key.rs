//! The key this app identifies itself with: an RSA-2048 key, its adb text form (what the phone
//! stores when you pair), and a self-signed certificate around it for the TLS handshake. The phone
//! only compares the certificate's public key with the paired key; nothing else in it matters.

use base64::Engine;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use rsa::traits::PublicKeyParts;
use rsa::{BigUint, RsaPrivateKey};

#[derive(Debug)]
pub enum KeyError {
    Rsa(String),
    Cert(String),
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyError::Rsa(e) => write!(f, "key error: {e}"),
            KeyError::Cert(e) => write!(f, "certificate error: {e}"),
        }
    }
}

impl std::error::Error for KeyError {}

#[derive(Clone)]
pub struct AdbKey {
    /// PKCS#8 DER of the private key (keep it private; the app stores it in its own folder).
    pub pkcs8: Vec<u8>,
    name: String,
    pub n: Vec<u8>,
    pub e: Vec<u8>,
    cert: Vec<u8>,
}

const MODULUS_BYTES: usize = 256;
/// Size of Android's encoded public key: 4 + 4 + 256 + 256 + 4.
pub const ANDROID_PUBKEY_SIZE: usize = 524;

impl AdbKey {
    /// A fresh key (takes a second or two). `name` is shown in the phone's list of paired devices.
    pub fn generate(name: &str) -> Result<AdbKey, KeyError> {
        let key = RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).map_err(|e| KeyError::Rsa(e.to_string()))?;
        Self::from_private(key, name)
    }

    pub fn from_pkcs8(der: &[u8], name: &str) -> Result<AdbKey, KeyError> {
        let key = RsaPrivateKey::from_pkcs8_der(der).map_err(|e| KeyError::Rsa(e.to_string()))?;
        Self::from_private(key, name)
    }

    fn from_private(key: RsaPrivateKey, name: &str) -> Result<AdbKey, KeyError> {
        if key.n().bits() != 2048 {
            return Err(KeyError::Rsa("the key must be 2048 bits".into()));
        }
        let pkcs8 = key.to_pkcs8_der().map_err(|e| KeyError::Rsa(e.to_string()))?.as_bytes().to_vec();
        let cert = make_certificate(&pkcs8)?;
        Ok(AdbKey { pkcs8, name: name.to_string(), n: key.n().to_bytes_be(), e: key.e().to_bytes_be(), cert })
    }

    /// Android's binary public key (what `android_pubkey_encode` makes).
    pub fn android_pubkey(&self) -> Vec<u8> {
        encode_android_pubkey(&self.n, &self.e)
    }

    /// The text the phone stores: base64 of the binary key, a space, a name, and a final NUL.
    pub fn adb_public_text(&self) -> Vec<u8> {
        let mut text = base64::engine::general_purpose::STANDARD.encode(self.android_pubkey()).into_bytes();
        text.push(b' ');
        text.extend_from_slice(self.name.as_bytes());
        text.push(0);
        text
    }

    /// A self-signed certificate (DER) around this key; made once, so it is the same every time.
    pub fn certificate_der(&self) -> Result<Vec<u8>, KeyError> {
        Ok(self.cert.clone())
    }
}

fn make_certificate(pkcs8: &[u8]) -> Result<Vec<u8>, KeyError> {
    let der = rustls::pki_types::PrivatePkcs8KeyDer::from(pkcs8.to_vec());
    let pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&der, &rcgen::PKCS_RSA_SHA256).map_err(|e| KeyError::Cert(e.to_string()))?;
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).map_err(|e| KeyError::Cert(e.to_string()))?;
    params.distinguished_name.push(rcgen::DnType::CommonName, "adbwire");
    let cert = params.self_signed(&pair).map_err(|e| KeyError::Cert(e.to_string()))?;
    Ok(cert.der().to_vec())
}

/// struct RSAPublicKey { u32 words (64); u32 n0inv; u8 modulus[256] LE; u8 rr[256] LE; u32 exponent }
pub fn encode_android_pubkey(n_be: &[u8], e_be: &[u8]) -> Vec<u8> {
    let n = BigUint::from_bytes_be(n_be);
    let e = BigUint::from_bytes_be(e_be);
    let n_le = to_le_padded(&n);
    // n0inv = -(n^-1) mod 2^32, found by Newton's method on the low word
    let n0 = u32::from_le_bytes(n_le[0..4].try_into().unwrap());
    let mut inv = n0;
    for _ in 0..5 {
        inv = inv.wrapping_mul(2u32.wrapping_sub(n0.wrapping_mul(inv)));
    }
    let n0inv = 0u32.wrapping_sub(inv);
    // rr = (2^2048)^2 mod n
    let rr = (BigUint::from(1u8) << (2 * 8 * MODULUS_BYTES)) % &n;
    let mut out = Vec::with_capacity(ANDROID_PUBKEY_SIZE);
    out.extend_from_slice(&((MODULUS_BYTES / 4) as u32).to_le_bytes());
    out.extend_from_slice(&n0inv.to_le_bytes());
    out.extend_from_slice(&n_le);
    out.extend_from_slice(&to_le_padded(&rr));
    let exp: u32 = e.to_bytes_be().iter().fold(0u32, |acc, b| (acc << 8) | *b as u32);
    out.extend_from_slice(&exp.to_le_bytes());
    out
}

fn to_le_padded(v: &BigUint) -> Vec<u8> {
    let mut b = v.to_bytes_le();
    b.resize(MODULUS_BYTES, 0);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_encoded_key_has_the_size_and_fields_android_expects() {
        // any odd 2048-bit number will do to check the layout and the n0inv relation
        let mut n = vec![0x80u8; 256];
        n[255] = 0xc3; // big-endian, so this is the lowest byte
        let e = vec![0x01, 0x00, 0x01];
        let enc = encode_android_pubkey(&n, &e);
        assert_eq!(enc.len(), ANDROID_PUBKEY_SIZE);
        assert_eq!(u32::from_le_bytes(enc[0..4].try_into().unwrap()), 64);
        let n0 = u32::from_le_bytes(enc[8..12].try_into().unwrap());
        let n0inv = u32::from_le_bytes(enc[4..8].try_into().unwrap());
        assert_eq!(n0.wrapping_mul(n0inv), u32::MAX, "n0 * n0inv == -1 mod 2^32");
        assert_eq!(u32::from_le_bytes(enc[520..524].try_into().unwrap()), 65537);
    }

    #[test]
    fn a_generated_key_round_trips_and_makes_a_certificate_and_text() {
        let k = AdbKey::generate("test@phone").unwrap();
        let again = AdbKey::from_pkcs8(&k.pkcs8, "test@phone").unwrap();
        assert_eq!(k.adb_public_text(), again.adb_public_text());
        let text = k.adb_public_text();
        assert_eq!(*text.last().unwrap(), 0, "ends with a NUL");
        let s = std::str::from_utf8(&text[..text.len() - 1]).unwrap();
        let (b64, name) = s.split_once(' ').unwrap();
        assert_eq!(name, "test@phone");
        let raw = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        assert_eq!(raw.len(), ANDROID_PUBKEY_SIZE);
        assert!(!k.certificate_der().unwrap().is_empty());
    }
}
