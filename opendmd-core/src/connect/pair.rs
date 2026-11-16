//! Pairing (P1-PLAN 2.6): the panel's one-time pairing window, the MACs
//! that make a key swapped by the server useless, and the "add another
//! device" invite a controller runs.
//!
//! - The pairing secret `S`: 100 random bits as 20 base32 characters, the
//!   canonical lowercase text. Its ASCII bytes are the HMAC key.
//! - `pair_id` = SHA-256(`"opendmd pair id v1"` ‖ `S`)\[..10\], what the
//!   server sees (never `S`).
//! - `mac` = HMAC-SHA256(`S`, `"opendmd pair v1"` ‖ `dev` ‖ `pub` ‖ label),
//!   from the controller (the label's bytes as sent, last); `mac2` =
//!   HMAC-SHA256(`S`, `"opendmd paired v1"` ‖ `dev` ‖ `pub` ‖ label), the
//!   panel's answer (the same label bytes), sent only once the key is
//!   stored.
//! - Invites: a 128-bit secret (26 base32 characters), `invite_id` =
//!   SHA-256(`"opendmd invite id v1"` ‖ secret)\[..10\], `add-req`'s `mac` =
//!   HMAC-SHA256(secret, `"opendmd add v1"` ‖ `dev` ‖ `pub` ‖ label), and the
//!   4-character code both screens show, from SHA-256(`"opendmd confirm
//!   v1"` ‖ `pub`).
//!
//! The window is one use, three tries, [`PAIR_TTL`]; the core never reads
//! the clock: the caller passes `now`.

use super::codec::{base32_bits, ct_eq, hmac_sha256, sha256, unbase32_bits};
use core::time::Duration;

/// How long a pairing window or invite stays open.
pub const PAIR_TTL: Duration = Duration::from_secs(5 * 60);
/// Wrong MACs a window takes before it closes.
pub const PAIR_TRIES: u8 = 3;

const PAIR_ID_DOMAIN: &[u8] = b"opendmd pair id v1";
const PAIR_MAC_DOMAIN: &[u8] = b"opendmd pair v1";
const PAIRED_MAC_DOMAIN: &[u8] = b"opendmd paired v1";
const INVITE_ID_DOMAIN: &[u8] = b"opendmd invite id v1";
const ADD_MAC_DOMAIN: &[u8] = b"opendmd add v1";
const CONFIRM_DOMAIN: &[u8] = b"opendmd confirm v1";

/// A pairing secret's text: 20 characters, 100 bits.
pub const SECRET_CHARS: usize = 20;
/// An invite secret's text: 26 characters, 128 bits.
pub const INVITE_CHARS: usize = 26;

/// The pairing secret `S` from 13 random bytes (the low 4 bits of the
/// last are not used).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn secret_from_random(r: &[u8; 13]) -> String {
    base32_bits(r, 100)
}

/// An invite secret from 16 random bytes.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn invite_secret_from_random(r: &[u8; 16]) -> String {
    base32_bits(r, 128)
}

/// `S` as read from a link or QR: lowercased, then it must be 20 canonical
/// base32 characters.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn parse_secret(s: &str) -> Option<String> {
    let s = s.to_ascii_lowercase();
    unbase32_bits::<13>(&s, 100).map(|_| s)
}

/// An invite secret as read from a link: 26 canonical base32 characters.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn parse_invite_secret(s: &str) -> Option<String> {
    let s = s.to_ascii_lowercase();
    unbase32_bits::<16>(&s, 128).map(|_| s)
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
fn id10(domain: &[u8], secret: &str) -> [u8; 10] {
    let h = sha256(&[domain, secret.as_bytes()]);
    let mut id = [0u8; 10];
    id.copy_from_slice(&h[..10]);
    id
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn pair_id(secret: &str) -> [u8; 10] {
    id10(PAIR_ID_DOMAIN, secret)
}

#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn invite_id(secret: &str) -> [u8; 10] {
    id10(INVITE_ID_DOMAIN, secret)
}

/// `label`: its UTF-8 bytes exactly as sent (after JSON unescaping, before
/// any cleaning), the last field: every other one has a fixed length.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn pair_mac(secret: &str, dev: &[u8; 16], public: &[u8; 32], label: &[u8]) -> [u8; 32] {
    hmac_sha256(secret.as_bytes(), &[PAIR_MAC_DOMAIN, dev, public, label])
}

/// `label` as for [`pair_mac`]: the bytes the panel was sent, so a label
/// rewritten on the way to the panel shows up on the way back too.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn paired_mac(secret: &str, dev: &[u8; 16], public: &[u8; 32], label: &[u8]) -> [u8; 32] {
    hmac_sha256(secret.as_bytes(), &[PAIRED_MAC_DOMAIN, dev, public, label])
}

/// `label` as for [`pair_mac`].
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn add_mac(secret: &str, dev: &[u8; 16], public: &[u8; 32], label: &[u8]) -> [u8; 32] {
    hmac_sha256(secret.as_bytes(), &[ADD_MAC_DOMAIN, dev, public, label])
}

/// The 4 characters both screens show while adding a device (uppercase
/// base32 of the first 20 bits of the hash).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn confirm_code(public: &[u8; 32]) -> String {
    base32_bits(&sha256(&[CONFIRM_DOMAIN, public]), 20).to_ascii_uppercase()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairError {
    /// No window open (never opened, used, closed, or out of tries).
    Closed,
    /// The window's time is up.
    Expired,
    /// The MAC does not match; `left`: tries remaining (0: now closed).
    BadMac { left: u8 },
}

impl PairError {
    /// The `err` in a `paired` answer.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn code(self) -> &'static str {
        match self {
            PairError::Closed => "closed",
            PairError::Expired => "expired",
            PairError::BadMac { .. } => "bad-mac",
        }
    }
}

/// One pairing (or invite) window: one secret, one use, three tries.
#[derive(Debug, Clone)]
pub struct Window {
    secret: String,
    until: Duration,
    tries: u8,
    open: bool,
    invite: bool,
}

impl Window {
    /// A panel's pairing window for `S`, open from `now`.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn open(secret: String, now: Duration) -> Window {
        Window::open_for(secret, now, PAIR_TTL)
    }
    /// The same, open for `ttl` (an emulator image's shorter one).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn open_for(secret: String, now: Duration, ttl: Duration) -> Window {
        Window { secret, until: now + ttl, tries: PAIR_TRIES, open: true, invite: false }
    }
    /// An inviting controller's window for an invite secret.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn open_invite(secret: String, now: Duration) -> Window {
        Window { invite: true, ..Window::open(secret, now) }
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn secret(&self) -> &str {
        &self.secret
    }
    /// What the server knows this window by: `pair_id` or `invite_id`.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn id(&self) -> [u8; 10] {
        if self.invite {
            invite_id(&self.secret)
        } else {
            pair_id(&self.secret)
        }
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn is_open(&self, now: Duration) -> bool {
        self.open && now < self.until
    }
    /// The time left, while open.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn left(&self, now: Duration) -> Option<Duration> {
        self.is_open(now).then(|| self.until - now)
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Check a guest's `mac` over `pub` and `label` (its bytes as sent) for
    /// this panel `dev`. On success the window is used up and the answer's
    /// `mac2` returned; the caller cleans the label only now, and sends
    /// `mac2` only after the key is stored. A wrong MAC spends a try.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn check(&mut self, dev: &[u8; 16], public: &[u8; 32], label: &[u8], mac: &[u8; 32], now: Duration) -> Result<[u8; 32], PairError> {
        if !self.open {
            return Err(PairError::Closed);
        }
        if now >= self.until {
            self.open = false;
            return Err(PairError::Expired);
        }
        let want = if self.invite { add_mac(&self.secret, dev, public, label) } else { pair_mac(&self.secret, dev, public, label) };
        if !ct_eq(&want, mac) {
            self.tries = self.tries.saturating_sub(1);
            if self.tries == 0 {
                self.open = false;
            }
            return Err(PairError::BadMac { left: self.tries });
        }
        self.open = false;
        Ok(paired_mac(&self.secret, dev, public, label))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV: [u8; 16] = [7; 16];
    const PUB: [u8; 32] = [9; 32];
    const L: &[u8] = b"Den phone";
    const T0: Duration = Duration::from_secs(1000);

    fn secret() -> String {
        secret_from_random(&[0x5a; 13])
    }

    #[test]
    fn secrets_are_canonical() {
        let s = secret();
        assert_eq!(s.len(), SECRET_CHARS);
        assert_eq!(parse_secret(&s.to_ascii_uppercase()), Some(s.clone()));
        assert_eq!(parse_secret(&s[..19]), None);
        assert_eq!(parse_secret("0000000000000000000a"), None, "0 is not base32");
        let i = invite_secret_from_random(&[0xa5; 16]);
        assert_eq!(i.len(), INVITE_CHARS);
        assert_eq!(parse_invite_secret(&i), Some(i.clone()));
        assert_eq!(confirm_code(&PUB).len(), 4);
    }

    #[test]
    fn right_mac_pairs_once() {
        let s = secret();
        let mut w = Window::open(s.clone(), T0);
        let mac2 = w.check(&DEV, &PUB, L, &pair_mac(&s, &DEV, &PUB, L), T0).unwrap();
        assert_eq!(mac2, paired_mac(&s, &DEV, &PUB, L));
        assert_ne!(mac2, paired_mac(&s, &DEV, &PUB, b"Den phone "));
        // One use.
        assert_eq!(w.check(&DEV, &PUB, L, &pair_mac(&s, &DEV, &PUB, L), T0), Err(PairError::Closed));
    }

    #[test]
    fn three_wrong_macs_close_it() {
        let s = secret();
        let mut w = Window::open(s.clone(), T0);
        let wrong = [0u8; 32];
        assert_eq!(w.check(&DEV, &PUB, L, &wrong, T0), Err(PairError::BadMac { left: 2 }));
        assert_eq!(w.check(&DEV, &PUB, L, &wrong, T0), Err(PairError::BadMac { left: 1 }));
        assert_eq!(w.check(&DEV, &PUB, L, &wrong, T0), Err(PairError::BadMac { left: 0 }));
        // Now even the right MAC is refused.
        assert_eq!(w.check(&DEV, &PUB, L, &pair_mac(&s, &DEV, &PUB, L), T0), Err(PairError::Closed));
    }

    #[test]
    fn expires() {
        let s = secret();
        let mut w = Window::open(s.clone(), T0);
        assert!(w.is_open(T0 + PAIR_TTL - Duration::from_millis(1)));
        assert_eq!(w.left(T0 + Duration::from_secs(60)), Some(PAIR_TTL - Duration::from_secs(60)));
        assert_eq!(w.left(T0 + PAIR_TTL), None);
        assert_eq!(w.check(&DEV, &PUB, L, &pair_mac(&s, &DEV, &PUB, L), T0 + PAIR_TTL), Err(PairError::Expired));
        assert_eq!(w.check(&DEV, &PUB, L, &pair_mac(&s, &DEV, &PUB, L), T0), Err(PairError::Closed));
    }

    #[test]
    fn a_swapped_key_or_another_panel_fails() {
        let s = secret();
        let mac = pair_mac(&s, &DEV, &PUB, L);
        let mut w = Window::open(s.clone(), T0);
        // The server swaps in its own key, keeping the guest's MAC.
        assert!(matches!(w.check(&DEV, &[8; 32], L, &mac, T0), Err(PairError::BadMac { .. })));
        // The MAC was made for another panel.
        assert!(matches!(w.check(&[6; 16], &PUB, L, &mac, T0), Err(PairError::BadMac { .. })));
        // An invite's MAC is not a pairing MAC.
        assert!(matches!(w.check(&DEV, &PUB, L, &add_mac(&s, &DEV, &PUB, L), T0), Err(PairError::BadMac { left: 0 })));
    }

    #[test]
    fn the_label_is_inside_the_mac() {
        let s = secret();
        let mac = pair_mac(&s, &DEV, &PUB, L);
        let mut w = Window::open(s.clone(), T0);
        // A server rewriting the label in a relayed `pair`.
        assert!(matches!(w.check(&DEV, &PUB, b"Den phone ", &mac, T0), Err(PairError::BadMac { .. })));
        assert!(matches!(w.check(&DEV, &PUB, b"Admin", &mac, T0), Err(PairError::BadMac { .. })));
        assert!(w.check(&DEV, &PUB, L, &mac, T0).is_ok());
        // And in a relayed `add-req`.
        let i = invite_secret_from_random(&[2; 16]);
        let mut w = Window::open_invite(i.clone(), T0);
        assert!(w.check(&DEV, &PUB, b"x", &add_mac(&i, &DEV, &PUB, L), T0).is_err());
        assert!(w.check(&DEV, &PUB, L, &add_mac(&i, &DEV, &PUB, L), T0).is_ok());
    }

    #[test]
    fn invites_use_their_own_domain() {
        let s = invite_secret_from_random(&[1; 16]);
        let mut w = Window::open_invite(s.clone(), T0);
        assert_eq!(w.id(), invite_id(&s));
        assert!(w.check(&DEV, &PUB, L, &pair_mac(&s, &DEV, &PUB, L), T0).is_err());
        assert!(w.check(&DEV, &PUB, L, &add_mac(&s, &DEV, &PUB, L), T0).is_ok());
    }
}
