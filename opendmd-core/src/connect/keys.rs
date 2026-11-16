//! The panel's controller list: who may command it (P1-PLAN 2.3).
//!
//! At most [`MAX_KEYS`] Ed25519 public keys, each with flags, a label
//! (≤ [`MAX_LABEL`] bytes, kept on the panel only), the date it was added
//! and the key that added it. Stored as one NVS blob ([`KeyList::encode`],
//! ≤ [`MAX_BLOB`] bytes); the public keys and flags are copied to the
//! server (`keys`) for its pre-check, but the panel trusts only this list.

use super::codec::{hex, sha256};
use super::json::push_str;
use ed25519_dalek::VerifyingKey;

/// Most controllers one panel keeps.
pub const MAX_KEYS: usize = 8;
/// A label's longest, in UTF-8 bytes.
pub const MAX_LABEL: usize = 24;
/// The blob's longest: a 3-byte header and eight full entries of 70 bytes.
pub const MAX_BLOB: usize = 3 + MAX_KEYS * ENTRY_MAX;
const ENTRY_MAX: usize = 32 + 1 + 4 + 8 + 1 + MAX_LABEL;
const BLOB_MAGIC: u8 = b'K';
const BLOB_VERSION: u8 = 1;

/// May add another controller (`add-controller`). Given to keys paired on
/// the panel (LAN link or QR), never to a key added by another controller:
/// one level only.
pub const FLAG_CAN_ADD: u8 = 1;
const KNOWN_FLAGS: u8 = FLAG_CAN_ADD;

/// A key id: the first 8 bytes of SHA-256 of the public key.
pub type KeyId = [u8; 8];

/// `k` for a public key.
pub fn key_id(public: &[u8; 32]) -> KeyId {
    let h = sha256(&[public]);
    let mut k = [0u8; 8];
    k.copy_from_slice(&h[..8]);
    k
}

/// A public key a controller may use: a valid curve point and not one of
/// small order (`verify_strict` would refuse every signature from such a
/// key anyway; refusing it here keeps it off the list).
pub fn usable_public_key(public: &[u8; 32]) -> bool {
    VerifyingKey::from_bytes(public).is_ok_and(|k| !k.is_weak())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub public: [u8; 32],
    pub flags: u8,
    pub label: String,
    /// Unix seconds when it was added, 0 if the panel had no clock.
    pub added: u32,
    /// The key that added it; `None`: paired on the panel itself.
    pub added_by: Option<KeyId>,
}

impl Entry {
    pub fn id(&self) -> KeyId {
        key_id(&self.public)
    }
    pub fn can_add(&self) -> bool {
        self.flags & FLAG_CAN_ADD != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// Already [`MAX_KEYS`].
    Full,
    /// That public key (or one with the same id) is on the list already.
    Duplicate,
    /// Not a usable Ed25519 public key.
    BadKey,
    /// A label too long or with characters a label may not hold.
    BadLabel,
    /// No such key.
    Unknown,
    /// A blob that is not one [`KeyList::encode`] writes.
    BadBlob,
}

/// Clean a label or panel name: drop control characters and the invisible
/// ones that can disguise text (zero widths, bidi controls, soft hyphen,
/// BOM), trim, and refuse it if empty or longer than `max_bytes` UTF-8
/// bytes or `max_chars` characters after that.
pub fn clean_text(s: &str, max_bytes: usize, max_chars: usize) -> Option<String> {
    let kept: String = s.chars().filter(|&c| !invisible(c)).collect();
    let t = kept.trim();
    (!t.is_empty() && t.len() <= max_bytes && t.chars().count() <= max_chars).then(|| t.to_string())
}

fn invisible(c: char) -> bool {
    c.is_control()
        || matches!(c as u32,
            0x00AD | 0x034F | 0x061C | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x180E
            | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
            | 0x3164 | 0xFE00..=0xFE0F | 0xFEFF | 0xFFA0 | 0xFFF0..=0xFFFB
            | 0xE0000..=0xE0FFF)
}

/// A label as stored: [`clean_text`] at [`MAX_LABEL`] bytes.
pub fn clean_label(s: &str) -> Option<String> {
    clean_text(s, MAX_LABEL, MAX_LABEL)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyList {
    entries: Vec<Entry>,
}

impl KeyList {
    pub fn new() -> KeyList {
        KeyList::default()
    }
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn find(&self, k: &KeyId) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id() == *k)
    }

    /// Add a key; the label is cleaned ([`clean_label`]) first.
    pub fn add(&mut self, mut e: Entry) -> Result<KeyId, KeyError> {
        if !usable_public_key(&e.public) {
            return Err(KeyError::BadKey);
        }
        if e.flags & !KNOWN_FLAGS != 0 {
            return Err(KeyError::BadKey);
        }
        e.label = clean_label(&e.label).ok_or(KeyError::BadLabel)?;
        let k = e.id();
        if self.find(&k).is_some() {
            return Err(KeyError::Duplicate);
        }
        if self.entries.len() >= MAX_KEYS {
            return Err(KeyError::Full);
        }
        self.entries.push(e);
        Ok(k)
    }

    pub fn remove(&mut self, k: &KeyId) -> Result<Entry, KeyError> {
        let i = self.entries.iter().position(|e| e.id() == *k).ok_or(KeyError::Unknown)?;
        Ok(self.entries.remove(i))
    }

    /// The NVS blob: `K`, version 1, count, then per entry the public key,
    /// flags, `added` (u32 big-endian), `added_by` (8 bytes, zero for
    /// none), the label's length and the label.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![BLOB_MAGIC, BLOB_VERSION, self.entries.len() as u8];
        for e in &self.entries {
            b.extend_from_slice(&e.public);
            b.push(e.flags);
            b.extend_from_slice(&e.added.to_be_bytes());
            b.extend_from_slice(&e.added_by.unwrap_or([0; 8]));
            b.push(e.label.len() as u8);
            b.extend_from_slice(e.label.as_bytes());
        }
        b
    }

    /// Read a blob back, refusing anything [`encode`](Self::encode) would
    /// not have written: trailing bytes, too many entries, a bad key or
    /// label, unknown flags, duplicates.
    pub fn decode(b: &[u8]) -> Result<KeyList, KeyError> {
        if b.len() < 3 || b.len() > MAX_BLOB || b[0] != BLOB_MAGIC || b[1] != BLOB_VERSION {
            return Err(KeyError::BadBlob);
        }
        let count = usize::from(b[2]);
        if count > MAX_KEYS {
            return Err(KeyError::BadBlob);
        }
        let mut list = KeyList::new();
        let mut at = 3;
        for _ in 0..count {
            let fixed = b.get(at..at + 46).ok_or(KeyError::BadBlob)?;
            let public: [u8; 32] = fixed[..32].try_into().map_err(|_| KeyError::BadBlob)?;
            let flags = fixed[32];
            let added = u32::from_be_bytes(fixed[33..37].try_into().map_err(|_| KeyError::BadBlob)?);
            let by: [u8; 8] = fixed[37..45].try_into().map_err(|_| KeyError::BadBlob)?;
            let len = usize::from(fixed[45]);
            at += 46;
            let label = b.get(at..at + len).ok_or(KeyError::BadBlob)?;
            at += len;
            let label = core::str::from_utf8(label).map_err(|_| KeyError::BadBlob)?;
            if clean_label(label).as_deref() != Some(label) {
                return Err(KeyError::BadBlob);
            }
            let e = Entry { public, flags, label: label.to_string(), added, added_by: (by != [0; 8]).then_some(by) };
            list.add(e).map_err(|_| KeyError::BadBlob)?;
        }
        if at != b.len() {
            return Err(KeyError::BadBlob);
        }
        Ok(list)
    }

    /// The `keys` message to the server: the public keys and flags only
    /// (labels stay on the panel).
    pub fn keys_message(&self) -> String {
        let mut s = String::from(r#"{"t":"keys","keys":["#);
        for (i, e) in self.entries.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(r#"{{"k":"{}","pub":"{}","f":{}}}"#, hex(&e.id()), hex(&e.public), e.flags));
        }
        s.push_str("]}");
        s
    }

    /// The `controllers` command's result `r`: per key its id, label,
    /// flags, date and who added it (`null`: paired on the panel), and the
    /// last use this boot (`last_s`, the caller's seconds, `null` if not
    /// used), which only the caller knows.
    pub fn controllers_result(&self, last_use: impl Fn(&KeyId) -> Option<u64>) -> String {
        let mut s = String::from("[");
        for (i, e) in self.entries.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let k = e.id();
            s.push_str(&format!(r#"{{"k":"{}","label":"#, hex(&k)));
            push_str(&mut s, &e.label);
            s.push_str(&format!(r#","f":{},"added":{},"added_by":"#, e.flags, e.added));
            match e.added_by {
                Some(by) => s.push_str(&format!("\"{}\"", hex(&by))),
                None => s.push_str("null"),
            }
            match last_use(&k) {
                Some(t) => s.push_str(&format!(r#","last_s":{t}}}"#)),
                None => s.push_str(r#","last_s":null}"#),
            }
        }
        s.push(']');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn public(seed: u8) -> [u8; 32] {
        SigningKey::from_bytes(&[seed; 32]).verifying_key().to_bytes()
    }

    fn entry(seed: u8, label: &str) -> Entry {
        Entry { public: public(seed), flags: FLAG_CAN_ADD, label: label.into(), added: 1_763_000_000, added_by: None }
    }

    #[test]
    fn eight_and_no_more() {
        let mut l = KeyList::new();
        for s in 1..=8 {
            l.add(entry(s, "phone")).unwrap();
        }
        assert_eq!(l.add(entry(9, "phone")), Err(KeyError::Full));
        assert_eq!(l.len(), 8);
    }

    #[test]
    fn refuses_duplicates_weak_keys_and_bad_labels() {
        let mut l = KeyList::new();
        l.add(entry(1, "a")).unwrap();
        assert_eq!(l.add(entry(1, "b")), Err(KeyError::Duplicate));
        // The identity point: small order.
        let mut weak = entry(2, "w");
        weak.public = [0; 32];
        weak.public[0] = 1;
        assert_eq!(l.add(weak), Err(KeyError::BadKey));
        assert_eq!(l.add(entry(3, "")), Err(KeyError::BadLabel));
        assert_eq!(l.add(entry(3, "\u{200b}\u{202e} \t")), Err(KeyError::BadLabel));
        assert_eq!(l.add(entry(3, &"x".repeat(25))), Err(KeyError::BadLabel));
        let mut f = entry(3, "f");
        f.flags = 0x80;
        assert_eq!(l.add(f), Err(KeyError::BadKey));
        // A label is cleaned: invisible characters go.
        let k = l.add(entry(4, " Ross\u{200b}'s\u{202e} phone ")).unwrap();
        assert_eq!(l.find(&k).unwrap().label, "Ross's phone");
    }

    #[test]
    fn blob_round_trip_fits_nvs() {
        let mut l = KeyList::new();
        for s in 1..=8 {
            let mut e = entry(s, &"é".repeat(12));
            if s > 1 {
                e.flags = 0;
                e.added_by = Some(key_id(&public(1)));
            }
            l.add(e).unwrap();
        }
        let b = l.encode();
        assert!(b.len() <= MAX_BLOB && MAX_BLOB <= 600, "{} / {}", b.len(), MAX_BLOB);
        assert_eq!(KeyList::decode(&b).unwrap(), l);
        assert_eq!(KeyList::decode(&KeyList::new().encode()).unwrap(), KeyList::new());
    }

    #[test]
    fn blob_refuses_anything_else() {
        let mut l = KeyList::new();
        l.add(entry(1, "a")).unwrap();
        l.add(entry(2, "b")).unwrap();
        let good = l.encode();
        let mut bad = good.clone();
        bad.push(0);
        assert_eq!(KeyList::decode(&bad), Err(KeyError::BadBlob), "trailing byte");
        assert_eq!(KeyList::decode(&good[..good.len() - 1]), Err(KeyError::BadBlob), "short");
        let mut bad = good.clone();
        bad[2] = 9;
        assert_eq!(KeyList::decode(&bad), Err(KeyError::BadBlob), "count");
        let mut bad = good.clone();
        bad[0] = b'X';
        assert_eq!(KeyList::decode(&bad), Err(KeyError::BadBlob), "magic");
        let mut bad = good.clone();
        bad[1] = 2;
        assert_eq!(KeyList::decode(&bad), Err(KeyError::BadBlob), "version");
        // The second entry made a copy of the first: a duplicate.
        let mut dup = KeyList::new();
        dup.add(entry(1, "a")).unwrap();
        let mut d = dup.encode();
        let one = d[3..].to_vec();
        d.extend_from_slice(&one);
        d[2] = 2;
        assert_eq!(KeyList::decode(&d), Err(KeyError::BadBlob), "duplicate");
        // A label with a control character written past the cleaner.
        let mut c = dup.encode();
        let n = c.len();
        c[n - 1] = b'\n';
        assert_eq!(KeyList::decode(&c), Err(KeyError::BadBlob), "label");
        assert_eq!(KeyList::decode(&[]), Err(KeyError::BadBlob));
    }

    #[test]
    fn messages() {
        let mut l = KeyList::new();
        let k = l.add(entry(1, "a\"b")).unwrap();
        let m = l.keys_message();
        assert!(m.starts_with(r#"{"t":"keys","keys":[{"k":""#), "{m}");
        assert!(!m.contains("a\\\"b"), "labels stay on the panel");
        let r = l.controllers_result(|x| (*x == k).then_some(12));
        assert!(r.contains(r#""label":"a\"b""#) && r.contains(r#""last_s":12"#) && r.contains(r#""added_by":null"#), "{r}");
        assert_eq!(l.remove(&k).map(|e| e.label), Ok("a\"b".to_string()));
        assert_eq!(l.remove(&k), Err(KeyError::Unknown));
    }
}
