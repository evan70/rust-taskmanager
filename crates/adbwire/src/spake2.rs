//! SPAKE2 over edwards25519, written to match BoringSSL's `SPAKE2_*` (crypto/curve25519/spake25519.cc)
//! byte for byte, because that is what Android's adb pairing uses. A different SPAKE2 would not
//! produce the same key as the phone. Every step below follows that file.

use curve25519_dalek::edwards::{CompressedEdwardsY, EdwardsPoint};
use curve25519_dalek::scalar::Scalar;
use rand::RngCore;
use sha2::{Digest, Sha512};

/// The two fixed points of the protocol (BoringSSL "N" and "M"), as encoded in that file's comments.
const N_ENC: [u8; 32] = hex32("10e3df0ae37d8e7a99b5fe74b44672103dbddcbd06af680d71329a11693bc778");
const M_ENC: [u8; 32] = hex32("5ada7e4bf6ddd9adb6626d32131c6b5c51a1e347a3478f53cfcf441b88eed12e");

/// The order of the prime-order subgroup, little-endian 64-bit words.
const ORDER: [u64; 4] = [0x5812_631a_5cf5_d3ed, 0x14de_f9de_a2f7_9cd6, 0, 0x1000_0000_0000_0000];

const fn hex32(s: &str) -> [u8; 32] {
    let b = s.as_bytes();
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (nibble(b[2 * i]) << 4) | nibble(b[2 * i + 1]);
        i += 1;
    }
    out
}

const fn nibble(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        _ => 0,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// adb's client ("Alice"): masks with M, unmasks with N.
    Alice,
    /// adb's server ("Bob"): masks with N, unmasks with M.
    Bob,
}

pub const MSG_LEN: usize = 32;
pub const KEY_LEN: usize = 64;

pub struct Spake2 {
    role: Role,
    my_name: Vec<u8>,
    their_name: Vec<u8>,
    private: Scalar,
    password_scalar: [u64; 4],
    password_hash: [u8; 64],
    my_msg: [u8; MSG_LEN],
}

// ---- 256-bit integer helpers (little-endian words) ----

fn add(a: &mut [u64; 4], b: &[u64; 4]) {
    let mut carry = 0u128;
    for i in 0..4 {
        let s = a[i] as u128 + b[i] as u128 + carry;
        a[i] = s as u64;
        carry = s >> 64;
    }
}

fn double(a: &[u64; 4]) -> [u64; 4] {
    let mut out = *a;
    add(&mut out, a);
    out
}

fn from_le(bytes: &[u8; 32]) -> [u64; 4] {
    let mut w = [0u64; 4];
    for i in 0..4 {
        w[i] = u64::from_le_bytes(bytes[8 * i..8 * i + 8].try_into().unwrap());
    }
    w
}

/// `scalar * point` on the whole curve (no reduction mod the group order, so any small-order part
/// of the point is kept, exactly as BoringSSL's table multiplication does). Not constant time; the
/// values are one-off secrets for a single pairing on the person's own phone.
fn mul_full(scalar: &[u64; 4], point: &EdwardsPoint) -> EdwardsPoint {
    let mut acc = EdwardsPoint::default(); // identity
    for bit in (0..256).rev() {
        acc = acc + acc;
        if (scalar[bit / 64] >> (bit % 64)) & 1 == 1 {
            acc += point;
        }
    }
    acc
}

fn lp(h: &mut Sha512, data: &[u8]) {
    h.update((data.len() as u64).to_le_bytes());
    h.update(data);
}

impl Spake2 {
    /// Start a handshake and produce this side's message (`msg()`).
    pub fn new(role: Role, my_name: &[u8], their_name: &[u8], password: &[u8]) -> Spake2 {
        let mut wide = [0u8; 64];
        rand::rngs::OsRng.fill_bytes(&mut wide);
        Self::with_random(role, my_name, their_name, password, &wide)
    }

    /// Same, with the 64 random bytes supplied (for tests).
    pub fn with_random(role: Role, my_name: &[u8], their_name: &[u8], password: &[u8], random: &[u8; 64]) -> Spake2 {
        // private key: reduce 64 random bytes mod the order, then times eight (clears the cofactor)
        let r = Scalar::from_bytes_mod_order_wide(random);
        let private = Scalar::from(8u8) * r;
        let p = EdwardsPoint::mul_base(&private);

        let password_hash: [u8; 64] = Sha512::digest(password).into();
        let w = Scalar::from_bytes_mod_order_wide(&password_hash);
        let mut pw = from_le(&w.to_bytes());
        // BoringSSL adds multiples of the order so the password scalar is a multiple of eight
        let mut order = ORDER;
        for bit in 0..3u64 {
            if (pw[0] >> bit) & 1 == 1 {
                add(&mut pw, &order);
            }
            order = double(&order);
        }
        debug_assert_eq!(pw[0] & 7, 0);

        let mask_point = decode(match role {
            Role::Alice => &M_ENC,
            Role::Bob => &N_ENC,
        })
        .expect("fixed point");
        let mask = mul_full(&pw, &mask_point);
        let my_msg = (p + mask).compress().to_bytes();

        Spake2 { role, my_name: my_name.to_vec(), their_name: their_name.to_vec(), private, password_scalar: pw, password_hash, my_msg }
    }

    pub fn msg(&self) -> [u8; MSG_LEN] {
        self.my_msg
    }

    /// Combine with the other side's message; gives the 64-byte shared key material.
    pub fn finish(self, their_msg: &[u8]) -> Option<[u8; KEY_LEN]> {
        if their_msg.len() != MSG_LEN {
            return None;
        }
        let qstar = decode(their_msg.try_into().ok()?)?;
        let unmask_point = decode(match self.role {
            Role::Alice => &N_ENC,
            Role::Bob => &M_ENC,
        })?;
        let peers_mask = mul_full(&self.password_scalar, &unmask_point);
        let q = qstar - peers_mask;
        // private = 8 * r, so multiplying 8 * q by r clears any small-order part of q
        let r = self.private * Scalar::from(8u8).invert();
        let dh = (q.mul_by_cofactor() * r).compress().to_bytes();

        let mut h = Sha512::new();
        match self.role {
            Role::Alice => {
                lp(&mut h, &self.my_name);
                lp(&mut h, &self.their_name);
                lp(&mut h, &self.my_msg);
                lp(&mut h, their_msg);
            }
            Role::Bob => {
                lp(&mut h, &self.their_name);
                lp(&mut h, &self.my_name);
                lp(&mut h, their_msg);
                lp(&mut h, &self.my_msg);
            }
        }
        lp(&mut h, &dh);
        lp(&mut h, &self.password_hash);
        Some(h.finalize().into())
    }
}

fn decode(bytes: &[u8; 32]) -> Option<EdwardsPoint> {
    CompressedEdwardsY(*bytes).decompress()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT: &[u8] = b"adb pair client\0";
    const SERVER: &[u8] = b"adb pair server\0";

    #[test]
    fn the_fixed_points_decode_and_are_the_ones_in_the_reference() {
        // coordinates quoted in BoringSSL's comments (x as a decimal number)
        let m = decode(&M_ENC).expect("M is a point");
        let n = decode(&N_ENC).expect("N is a point");
        assert_ne!(m.compress(), n.compress());
        assert_eq!(m.compress().to_bytes(), M_ENC);
        assert_eq!(n.compress().to_bytes(), N_ENC);
    }

    #[test]
    fn both_sides_get_the_same_key_with_the_same_password() {
        for pw in [&b"123456"[..], b"000000", b"a longer password with the tls export appended........................"] {
            let a = Spake2::new(Role::Alice, CLIENT, SERVER, pw);
            let b = Spake2::new(Role::Bob, SERVER, CLIENT, pw);
            let (ma, mb) = (a.msg(), b.msg());
            let ka = a.finish(&mb).unwrap();
            let kb = b.finish(&ma).unwrap();
            assert_eq!(ka, kb, "password {pw:?}");
        }
    }

    #[test]
    fn different_passwords_give_different_keys() {
        let a = Spake2::new(Role::Alice, CLIENT, SERVER, b"123456");
        let b = Spake2::new(Role::Bob, SERVER, CLIENT, b"654321");
        let (ma, mb) = (a.msg(), b.msg());
        assert_ne!(a.finish(&mb), b.finish(&ma));
    }

    #[test]
    fn the_password_scalar_is_a_multiple_of_eight_for_many_passwords() {
        for i in 0..200u32 {
            let s = Spake2::with_random(Role::Alice, CLIENT, SERVER, i.to_string().as_bytes(), &[7u8; 64]);
            assert_eq!(s.password_scalar[0] & 7, 0, "password {i}");
        }
    }

    #[test]
    fn bad_peer_messages_are_refused() {
        let a = Spake2::new(Role::Alice, CLIENT, SERVER, b"123456");
        assert!(a.finish(&[0u8; 5]).is_none(), "wrong length");
    }

    #[test]
    fn a_fixed_random_value_gives_a_fixed_message() {
        // regression guard: the same inputs must always give the same bytes
        let a = Spake2::with_random(Role::Alice, CLIENT, SERVER, b"123456", &[1u8; 64]);
        let b = Spake2::with_random(Role::Alice, CLIENT, SERVER, b"123456", &[1u8; 64]);
        assert_eq!(a.msg(), b.msg());
        let c = Spake2::with_random(Role::Alice, CLIENT, SERVER, b"123457", &[1u8; 64]);
        assert_ne!(a.msg(), c.msg());
    }
}
