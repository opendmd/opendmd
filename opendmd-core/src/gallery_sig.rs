//! The gallery's signature on a Formula (the signed-gallery design): what a
//! panel will check before a remote controller may add a Formula to its
//! card (`formula-add`, protocol 5.7). Only a Formula the project signed,
//! byte for byte, may arrive that way; the key is the gallery key, never
//! the firmware's (`ota`) or a controller's (`connect`).
//!
//! **The signed message** is one fixed binary layout, no JSON
//! ([`Message`]), Ed25519 over:
//!
//! ```text
//! "opendmd-gallery-v1 formula\0"   domain, 27 bytes
//! kid   u8      the gallery key's id
//! abi   u8      psi::psb::ABI the compiled file is for
//! rev   u32 LE  the signing revision (below REV_FLOOR: refused)
//! stem  u8 length, then the stem ([a-z0-9-], 1 to 40, a letter or digit first)
//! src   u32 LE length, then the SHA-256 of the source as sent
//! psb   u32 LE length, then the SHA-256 of the compiled file
//! ```
//!
//! **The check** ([`check`]) refuses, in order and before any signature
//! work: a stem outside the rule, a key id this firmware does not know, a
//! compiled format other than the player's, a revision under the floor;
//! then the signature, strictly (`verify_strict`: no small-order key, no
//! small-order `R`, a canonical `S`). No heap: the message is built in a
//! fixed buffer.
//!
//! **The keys** ([`GALLERY_KEYS`]) are compiled in, by id, from
//! `firmware/keys/gallery*.pub`. Until the project's gallery key is
//! committed the table is empty and every check is
//! [`Refusal::UnknownKey`]: nothing can be added, whatever is offered.
//! `tests/gallery_signatures.rs` holds the table to the committed files
//! and every committed signature (`opendmd-web/formulas/signatures.json`)
//! to the firmware's own verifier; `tools/gallery-sign.py` makes them.
//!
//! Built only with the `gallery-sig` feature (the firmware's
//! `connect-formula-add`, test images): nothing of it in a release.

use ed25519_dalek::{Signature, VerifyingKey};

/// The signed message's first bytes: no Connect command
/// (`"opendmd-connect-v1 cmd\0"`), firmware bundle (`ODFW…`) or www pack
/// signature (`ODWPSIG1`) starts with them.
pub const DOMAIN: &[u8] = b"opendmd-gallery-v1 formula\0";
/// A stem, at most (bytes): the web app's save rule.
pub const MAX_STEM: usize = 40;
/// The message, at most: the domain, kid, abi, rev, the longest stem with
/// its length, and two (length, SHA-256) pairs.
pub const MAX_MESSAGE: usize = DOMAIN.len() + 1 + 1 + 4 + 1 + MAX_STEM + 2 * (4 + 32);
/// An Ed25519 public key and signature, bytes.
pub const KEY_LEN: usize = 32;
pub const SIG_LEN: usize = 64;

/// The gallery keys this firmware trusts, by id. Empty until the project's
/// gallery key is committed (`firmware/keys/gallery.pub`, made by
/// `tools/gallery-sign.py keygen`); a rotation adds the next id beside it.
/// `tests/gallery_signatures.rs` fails while this table, the committed key
/// files and its pinned hex (`PINNED`) disagree.
//
// THE SIGNING COMMIT EDITS THE NEXT LINE BY HAND, and nothing else here:
// `&[]` becomes `&[(0, include_bytes!("../../firmware/keys/gallery.pub"))]`.
pub const GALLERY_KEYS: &[(u8, &[u8; KEY_LEN])] = &[(0, include_bytes!("../../firmware/keys/gallery.pub"))];

/// The lowest signing revision this firmware takes. A published Formula
/// found harmful is revoked by signing the rest again at a higher `rev` and
/// raising this in the next firmware (a panel not updated still takes the
/// old signature: the same limit as a firmware rollback floor).
pub const REV_FLOOR: u32 = 1;

/// What a signature is over: a Formula as the gallery published it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Formula<'a> {
    pub kid: u8,
    pub abi: u8,
    pub rev: u32,
    pub stem: &'a str,
    pub src_len: u32,
    pub src_sha256: [u8; 32],
    pub psb_len: u32,
    pub psb_sha256: [u8; 32],
}

/// What [`check`] gives back for a signature it took: the Formula as it was
/// signed. **Only `check` makes one**: its fields are private and there is no
/// `Default` or other constructor, so the code that needs one (an add held,
/// `connect::fadd::Adds::begin`; `proto::Command::FormulaAdd`; the
/// firmware's `formula_add` and its landing) cannot run for a Formula no
/// trusted key signed, whatever else is changed.
///
/// ```compile_fail
/// use opendmd_core::gallery_sig::Signed;
/// let s = Signed { kid: 0, abi: 3, rev: 1, stem: String::from("x"), src_len: 1, src_sha256: [0; 32], psb_len: 6, psb_sha256: [0; 32] };
/// ```
///
/// ```compile_fail
/// let s: opendmd_core::gallery_sig::Signed = Default::default();
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    kid: u8,
    abi: u8,
    rev: u32,
    stem: String,
    src_len: u32,
    src_sha256: [u8; 32],
    psb_len: u32,
    psb_sha256: [u8; 32],
}

impl Signed {
    pub fn kid(&self) -> u8 {
        self.kid
    }
    pub fn abi(&self) -> u8 {
        self.abi
    }
    pub fn rev(&self) -> u32 {
        self.rev
    }
    pub fn stem(&self) -> &str {
        &self.stem
    }
    pub fn src(&self) -> (u32, &[u8; 32]) {
        (self.src_len, &self.src_sha256)
    }
    pub fn psb(&self) -> (u32, &[u8; 32]) {
        (self.psb_len, &self.psb_sha256)
    }
}

/// Why a signature is refused. On the wire every one is `not-allowed`
/// (protocol 5.7); the reason is for the log and the tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The stem is outside the rule (no message can be made for it).
    BadStem,
    /// A key id this firmware does not trust (or none committed yet).
    UnknownKey,
    /// Signed for another compiled format than the player's.
    WrongAbi,
    /// Signed at a revision under [`REV_FLOOR`] (revoked).
    OldRev,
    /// The signature does not verify, strictly, for that key.
    BadSignature,
}

/// The stem rule (`[a-z0-9-]`, 1 to [`MAX_STEM`], a letter or digit
/// first): the web app's save rule, the add's own.
pub fn stem_ok(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= MAX_STEM && b[0].is_ascii_alphanumeric() && b.iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// The signed message for `f`, in a fixed buffer.
#[derive(Clone, Copy)]
pub struct Message {
    buf: [u8; MAX_MESSAGE],
    len: usize,
}

impl Message {
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl core::fmt::Debug for Message {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Message({} B)", self.len)
    }
}

/// `f`'s signed message (the module docs' layout); [`Refusal::BadStem`]
/// for a stem outside the rule.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn message(f: &Formula) -> Result<Message, Refusal> {
    if !stem_ok(f.stem) {
        return Err(Refusal::BadStem);
    }
    let mut m = Message { buf: [0; MAX_MESSAGE], len: 0 };
    let mut put = |b: &[u8]| {
        m.buf[m.len..m.len + b.len()].copy_from_slice(b);
        m.len += b.len();
    };
    put(DOMAIN);
    put(&[f.kid, f.abi]);
    put(&f.rev.to_le_bytes());
    put(&[f.stem.len() as u8]);
    put(f.stem.as_bytes());
    put(&f.src_len.to_le_bytes());
    put(&f.src_sha256);
    put(&f.psb_len.to_le_bytes());
    put(&f.psb_sha256);
    Ok(m)
}

/// What a check trusts: the keys by id, the revision floor, and the
/// compiled format the player plays.
#[derive(Debug, Clone, Copy)]
pub struct Trust<'a> {
    pub keys: &'a [(u8, &'a [u8; KEY_LEN])],
    pub floor: u32,
    pub abi: u8,
}

impl Trust<'static> {
    /// This firmware's: [`GALLERY_KEYS`] and [`REV_FLOOR`], for a player of
    /// compiled format `abi` (`psi::psb::ABI`).
    pub const fn panel(abi: u8) -> Trust<'static> {
        Trust { keys: GALLERY_KEYS, floor: REV_FLOOR, abi }
    }
}

/// Does `sig` sign `f` for `t`? The cheap refusals first (stem, key id,
/// format, floor), then one strict Ed25519 verification; the [`Signed`]
/// witness when it does.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn check(t: &Trust, f: &Formula, sig: &[u8; SIG_LEN]) -> Result<Signed, Refusal> {
    let msg = message(f)?;
    let key = t.keys.iter().find(|(id, _)| *id == f.kid).map(|&(_, k)| k).ok_or(Refusal::UnknownKey)?;
    if f.abi != t.abi {
        return Err(Refusal::WrongAbi);
    }
    if f.rev < t.floor {
        return Err(Refusal::OldRev);
    }
    let key = VerifyingKey::from_bytes(key).map_err(|_| Refusal::BadSignature)?;
    key.verify_strict(msg.as_bytes(), &Signature::from_bytes(sig)).map_err(|_| Refusal::BadSignature)?;
    Ok(Signed {
        kid: f.kid,
        abi: f.abi,
        rev: f.rev,
        stem: f.stem.to_string(),
        src_len: f.src_len,
        src_sha256: f.src_sha256,
        psb_len: f.psb_len,
        psb_sha256: f.psb_sha256,
    })
}

/// A witness for the crate's own unit tests (the add's state machine):
/// made the only way there is, `check`, over a signature by the vectors'
/// throwaway key A (`tests/gallery_vectors.rs`), which no firmware trusts.
#[cfg(test)]
pub(crate) fn signed_for_tests(f: &Formula) -> Signed {
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};
    let seed: [u8; 32] = Sha256::digest(b"opendmd-gallery throwaway test key, never a real one: A").into();
    let k = SigningKey::from_bytes(&seed);
    let public = k.verifying_key().to_bytes();
    let keys = [(f.kid, &public)];
    let sig = k.sign(message(f).expect("a stem in the rule").as_bytes()).to_bytes();
    check(&Trust { keys: &keys, floor: 0, abi: f.abi }, f, &sig).expect("the test key's own signature")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(stem: &str) -> Formula<'_> {
        Formula { kid: 0, abi: 3, rev: 1, stem, src_len: 10, src_sha256: [1; 32], psb_len: 6, psb_sha256: [2; 32] }
    }

    /// The layout, byte for byte, and its longest case within the buffer.
    #[test]
    fn the_message_layout() {
        let m = message(&f("plasma")).unwrap();
        let b = m.as_bytes();
        assert_eq!(&b[..27], DOMAIN);
        assert_eq!(&b[27..33], &[0, 3, 1, 0, 0, 0]);
        assert_eq!(b[33], 6);
        assert_eq!(&b[34..40], b"plasma");
        assert_eq!(&b[40..44], &10u32.to_le_bytes());
        assert_eq!(&b[44..76], &[1; 32]);
        assert_eq!(&b[76..80], &6u32.to_le_bytes());
        assert_eq!(&b[80..], &[2; 32]);
        let long = "a".repeat(MAX_STEM);
        assert_eq!(message(&f(&long)).unwrap().as_bytes().len(), MAX_MESSAGE);
        assert_eq!(MAX_MESSAGE, 146);
    }

    /// The witness is the Formula as signed, field for field.
    #[test]
    fn the_witness_is_what_was_signed() {
        let w = signed_for_tests(&f("plasma"));
        assert_eq!((w.kid(), w.abi(), w.rev(), w.stem()), (0, 3, 1, "plasma"));
        assert_eq!(w.src(), (10, &[1; 32]));
        assert_eq!(w.psb(), (6, &[2; 32]));
    }

    #[test]
    fn stems_outside_the_rule_make_no_message() {
        let long = "a".repeat(MAX_STEM + 1);
        for s in ["", "Plasma", "-x", "a.b", "a/b", "../x", "a b", "é", long.as_str()] {
            assert_eq!(message(&f(s)).unwrap_err(), Refusal::BadStem, "{s:?}");
        }
        for s in ["a", "0", "plasma-2", "rift-patrol"] {
            assert!(message(&f(s)).is_ok(), "{s:?}");
        }
    }

    /// No gallery key is committed yet: this firmware refuses every
    /// signature, whatever it is.
    #[test]
    fn with_no_key_committed_every_signature_is_refused() {
        if GALLERY_KEYS.is_empty() {
            assert_eq!(check(&Trust::panel(3), &f("plasma"), &[0; SIG_LEN]).map(drop), Err(Refusal::UnknownKey));
        }
        const { assert!(REV_FLOOR >= 1) };
    }
}
