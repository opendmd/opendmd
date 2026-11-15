//! The panel's stable ID: `/api/status` `id` and the `_opendmd._tcp` TXT
//! record's `id=`, so an app that saved a panel finds it again after a
//! rename (a new `hostname`, a `-2` from mDNS), a new address or a reboot.
//!
//! Derived from the chip's factory MAC (the eFuse base MAC, which no
//! setting, update or erase changes), never the MAC itself: the first 60
//! bits of SHA-256 over [`PREFIX`] and the six MAC bytes, written as 12
//! characters of lowercase Crockford base32 (`0-9a-z` without `i l o u`:
//! safe in a URL, a file name and a TXT value, and unambiguous read aloud).
//! No MAC (the eFuses did not read, or read as zeros): no ID.
//!
//! **The ID is never a credential.** It is a permanent hardware identifier:
//! it survives renames, updates, erases and a change of owner, and it
//! leaves the LAN wherever apps, Connect or bug reports carry it. It does
//! not show the MAC, but it does not hide it either: one maker's MACs are
//! few enough (a few vendor prefixes, 24 bits each) to try them all
//! against an ID. Connect and apps pair with a secret of their own and use
//! the ID only as a key to look a panel up by.

use sha2::{Digest, Sha256};

/// What the MAC is hashed with: a version, so a new scheme is a new prefix.
pub const PREFIX: &[u8] = b"opendmd panel id v1\0";

/// The ID's length in characters (5 bits each: 60 bits).
pub const LEN: usize = 12;

/// Crockford's base32 alphabet, lowercase.
const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// The ID of the panel whose base MAC is `mac`; `None` for an all-zero
/// MAC (eFuses that read as nothing: every such panel would share one ID).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn from_mac(mac: &[u8; 6]) -> Option<String> {
    if *mac == [0; 6] {
        return None;
    }
    let mut h = Sha256::new();
    h.update(PREFIX);
    h.update(mac);
    let d = h.finalize();
    let bits = u64::from_be_bytes(d[..8].try_into().unwrap()) >> 4;
    Some(
        (0..LEN)
            .map(|i| ALPHABET[(bits >> (5 * (LEN - 1 - i)) & 31) as usize] as char)
            .collect(),
    )
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
        let id = |m: [u8; 6]| from_mac(&m).unwrap();
        assert_eq!(id([0x02, 0, 0, 0, 0, 0x01]), "tsjrj1qchc6a");
        assert_eq!(id([0x24, 0x0a, 0xc4, 0x12, 0x34, 0x56]), "5wkafxp3cmkf");
    }

    #[test]
    fn no_mac_no_id() {
        assert_eq!(from_mac(&[0; 6]), None);
        assert!(from_mac(&[0, 0, 0, 0, 0, 1]).is_some());
    }

    #[test]
    fn shape_and_spread() {
        let mut seen = std::collections::HashSet::new();
        for n in 0..1000u32 {
            let b = n.to_be_bytes();
            let id = from_mac(&[0x24, 0x0a, 0xc4, b[1], b[2], b[3]]).unwrap();
            assert!(valid(&id), "{id}");
            assert!(seen.insert(id));
        }
        // The MAC's bytes are not in it, in any spelling.
        let id = from_mac(&[0x24, 0x0a, 0xc4, 0x12, 0x34, 0x56]).unwrap();
        assert!(!id.contains("240ac4") && !id.contains("123456"));
        for bad in ["", "tsjrj1qchc6", "tsjrj1qchc6A", "tsjrj1qchci6"] {
            assert!(!valid(bad), "{bad:?}");
        }
    }
}
