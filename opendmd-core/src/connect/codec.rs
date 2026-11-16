//! The encodings Connect's messages use, and HMAC-SHA256.
//!
//! - **Hex** (lowercase only) for every binary field inside JSON: key ids,
//!   public keys, signatures, MACs, nonces, hashes. One spelling per value,
//!   so two parsers can never disagree on what a field says.
//! - **Base32** (RFC 4648's alphabet, lowercase, no padding) for the two
//!   values people may see or a QR carries: the panel's Connect id `dev`
//!   (16 bytes, 26 characters) and the pairing secrets. Canonical only: the
//!   unused low bits of the last character must be zero.
//! - [`hmac_sha256`] (RFC 2104) over the `sha2` crate, for the pairing
//!   MACs and the id tag; [`ct_eq`] to compare MACs.

use sha2::{Digest, Sha256};

const HEX: &[u8; 16] = b"0123456789abcdef";

/// `bytes` as lowercase hex.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[usize::from(b >> 4)] as char);
        s.push(HEX[usize::from(b & 15)] as char);
    }
    s
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Exactly `N` bytes of lowercase hex (`2 N` characters), else `None`.
/// Uppercase is refused: one spelling per value.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let s = s.as_bytes();
    if s.len() != 2 * N {
        return None;
    }
    let mut out = [0u8; N];
    for (k, o) in out.iter_mut().enumerate() {
        *o = (hex_digit(s[2 * k])? << 4) | hex_digit(s[2 * k + 1])?;
    }
    Some(out)
}

const B32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// The first `bits` bits of `bytes` in base32, `ceil(bits / 5)` characters.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn base32_bits(bytes: &[u8], bits: usize) -> String {
    let chars = bits.div_ceil(5);
    let mut s = String::with_capacity(chars);
    for c in 0..chars {
        let mut v = 0u8;
        for b in 0..5 {
            let bit = c * 5 + b;
            let on = bit < bits && bit / 8 < bytes.len() && bytes[bit / 8] & (0x80 >> (bit % 8)) != 0;
            v = (v << 1) | u8::from(on);
        }
        s.push(B32[usize::from(v)] as char);
    }
    s
}

/// `bytes` in base32 (all their bits).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn base32(bytes: &[u8]) -> String {
    base32_bits(bytes, bytes.len() * 8)
}

/// Decode canonical lowercase base32 of exactly `bits` bits into `N` bytes
/// (`N * 8 >= bits`; any bits past `bits` come out zero). `None` for a wrong
/// length, a character outside the alphabet (uppercase included) or a
/// non-zero padding bit.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn unbase32_bits<const N: usize>(s: &str, bits: usize) -> Option<[u8; N]> {
    let s = s.as_bytes();
    if s.len() != bits.div_ceil(5) || N * 8 < bits {
        return None;
    }
    let mut out = [0u8; N];
    for (c, &ch) in s.iter().enumerate() {
        let v = B32.iter().position(|&a| a == ch)? as u8;
        for b in 0..5 {
            let bit = c * 5 + b;
            let on = v & (0x10 >> b) != 0;
            if bit >= bits {
                if on {
                    return None;
                }
            } else if on {
                out[bit / 8] |= 0x80 >> (bit % 8);
            }
        }
    }
    Some(out)
}

/// Decode canonical base32 of exactly `N` bytes.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn unbase32<const N: usize>(s: &str) -> Option<[u8; N]> {
    unbase32_bits::<N>(s, N * 8)
}

/// SHA-256 of the concatenation of `parts`.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// HMAC-SHA256 (RFC 2104) of the concatenation of `parts` under `key`.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&sha256(&[key]));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    for p in parts {
        inner.update(p);
    }
    let inner: [u8; 32] = inner.finalize().into();
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner);
    outer.finalize().into()
}

/// Constant-time equality of two equal-length byte strings (a MAC
/// compare); different lengths are simply unequal.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut d = 0u8;
    for (x, y) in a.iter().zip(b) {
        d |= x ^ y;
    }
    // Keep the optimiser from short-circuiting the loop.
    core::hint::black_box(d) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip_and_strictness() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
        assert_eq!(unhex::<3>("00abff"), Some([0x00, 0xab, 0xff]));
        assert_eq!(unhex::<3>("00ABff"), None, "uppercase refused");
        assert_eq!(unhex::<3>("00abf"), None);
        assert_eq!(unhex::<3>("00abfff0"), None);
        assert_eq!(unhex::<1>("g0"), None);
    }

    #[test]
    fn base32_rfc4648_vectors() {
        // RFC 4648 section 10, lowercase and unpadded.
        for (i, o) in [("", ""), ("f", "my"), ("fo", "mzxq"), ("foo", "mzxw6"), ("foob", "mzxw6yq"), ("fooba", "mzxw6ytb"), ("foobar", "mzxw6ytboi")] {
            assert_eq!(base32(i.as_bytes()), o);
        }
        assert_eq!(&unbase32::<6>("mzxw6ytboi").unwrap(), b"foobar");
        assert_eq!(unbase32::<6>("MZXW6YTBOI"), None, "uppercase refused");
        // "foobar" is 48 bits: the 10th character carries 2 padding bits.
        assert_eq!(unbase32::<6>("mzxw6ytboj"), None, "non-zero padding bit");
        assert_eq!(unbase32::<6>("mzxw6ytbo"), None);
        assert_eq!(unbase32::<6>("mzxw6ytbo1"), None);
    }

    #[test]
    fn base32_of_100_bits() {
        let b = [0xffu8; 13];
        let s = base32_bits(&b, 100);
        assert_eq!(s.len(), 20);
        assert_eq!(s, "77777777777777777777");
        let back = unbase32_bits::<13>(&s, 100).unwrap();
        assert_eq!(&back[..12], &[0xff; 12]);
        assert_eq!(back[12], 0xf0);
    }

    #[test]
    fn hmac_rfc4231() {
        // Test case 1.
        let key = [0x0b; 20];
        assert_eq!(
            hex(&hmac_sha256(&key, &[b"Hi There"])),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // Test case 2, the message in two parts.
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", &[b"what do ya want ", b"for nothing?"])),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 6: a key longer than the block.
        let key = [0xaa; 131];
        assert_eq!(
            hex(&hmac_sha256(&key, &[b"Test Using Larger Than Block-Size Key - Hash Key First"])),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn ct_eq_basics() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
