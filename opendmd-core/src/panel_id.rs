//! The panel's stable ID: `/api/status` `id` and the `_opendmd._tcp` TXT
//! record's `id=`, so an app that saved a panel finds it again after a
//! rename (a new `hostname`, a `-2` from mDNS), a new address or a reboot.
//!
//! Derived from the chip's factory MAC (the eFuse base MAC, which no
//! setting, update or erase changes), never the MAC itself: the first 60
//! bits of SHA-256 over [`PREFIX`] and the six MAC bytes, written as 12
//! characters of lowercase Crockford base32 (`0-9a-z` without `i l o u`:
//! safe in a URL, a file name and a TXT value, and unambiguous read aloud).
//!
//! What it hides: the MAC is not in the ID, so the ID can be shown, logged
//! or reported without handing out the MAC. What it does not: the MAC
//! space of one maker is small (a few vendor prefixes, 24 bits each), so
//! someone with the ID and time could find the MAC by trying them all.
//! The MAC is no secret on the LAN in any case (every packet carries it):
//! the ID is for telling panels apart, not an identity to trust.

use sha2::{Digest, Sha256};

/// What the MAC is hashed with: a version, so a new scheme is a new prefix.
pub const PREFIX: &[u8] = b"opendmd panel id v1\0";

/// The ID's length in characters (5 bits each: 60 bits).
pub const LEN: usize = 12;

/// Crockford's base32 alphabet, lowercase.
const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// The ID of the panel whose base MAC is `mac`.
pub fn from_mac(mac: &[u8; 6]) -> String {
    let mut h = Sha256::new();
    h.update(PREFIX);
    h.update(mac);
    let d = h.finalize();
    let bits = u64::from_be_bytes(d[..8].try_into().unwrap()) >> 4;
    (0..LEN)
        .map(|i| ALPHABET[(bits >> (5 * (LEN - 1 - i)) & 31) as usize] as char)
        .collect()
}

/// Is `s` an ID's shape: [`LEN`] characters of the alphabet.
pub fn valid(s: &str) -> bool {
    s.len() == LEN && s.bytes().all(|b| ALPHABET.contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned: tools/mock-device.py derives the mock's ID the same way, and
    /// a change here gives every panel a new ID (the app would see new
    /// panels).
    #[test]
    fn known_ids() {
        assert_eq!(from_mac(&[0; 6]), "360bjw0vfam7");
        assert_eq!(from_mac(&[0x02, 0, 0, 0, 0, 0x01]), "tsjrj1qchc6a");
    }

    #[test]
    fn shape_and_spread() {
        let mut seen = std::collections::HashSet::new();
        for n in 0..1000u32 {
            let b = n.to_be_bytes();
            let id = from_mac(&[0x24, 0x0a, 0xc4, b[1], b[2], b[3]]);
            assert!(valid(&id), "{id}");
            assert!(seen.insert(id));
        }
        // The MAC's bytes are not in it, in any spelling.
        let id = from_mac(&[0x24, 0x0a, 0xc4, 0x12, 0x34, 0x56]);
        assert!(!id.contains("240ac4") && !id.contains("123456"));
        assert!(!valid("") && !valid("360bjw0vfam") && !valid("360bjw0vfaM7") && !valid("360bjw0vfai7"));
    }
}
