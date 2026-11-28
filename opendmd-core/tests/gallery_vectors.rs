//! The gallery signature's shared vectors (`tests/gallery-vectors.json`):
//! made here from fixed, throwaway keys, pinned in the file, and checked
//! against the firmware's verifier (`opendmd_core::gallery_sig`).
//! `tools/gallery-sign.py selftest` checks the same file in Python: the
//! messages byte for byte, its own signatures equal to these, and every red
//! case refused by its verifier too.
//!
//! Every case says what the check answers (`expect`), written by hand
//! below, not computed: the test is that the verifier agrees with them.
//!
//! After a deliberate change: `GALLERY_VECTORS=write cargo test --features
//! gallery-sig --test gallery_vectors`, then review the diff.
#![cfg(feature = "gallery-sig")]

use ed25519_dalek::{Signer, SigningKey};
use opendmd_core::gallery_sig::{self, Formula, Refusal, Trust, DOMAIN, KEY_LEN};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/gallery-vectors.json");

fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// A throwaway key's seed: never a real one (the committed-key test refuses
/// these as gallery keys).
pub fn seed(name: &str) -> [u8; 32] {
    sha256(format!("opendmd-gallery throwaway test key, never a real one: {name}").as_bytes())
}

/// The encoding of the identity point: a small-order key (order 1), for
/// which `R = identity, S = 0` verifies any message unless the check is
/// strict.
const SMALL_ORDER: [u8; KEY_LEN] = {
    let mut k = [0u8; KEY_LEN];
    k[0] = 1;
    k
};
/// The group order L, little-endian: `S + L` is the same scalar spelled
/// non-canonically.
const L: [u8; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
];

fn add_l(s: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut carry = 0u16;
    for i in 0..32 {
        let v = u16::from(s[i]) + u16::from(L[i]) + carry;
        out[i] = v as u8;
        carry = v >> 8;
    }
    assert_eq!(carry, 0);
    out
}

/// The vectors' trust: key A under ids 0 and 1 (so a flipped id is caught by
/// the message, not the lookup), key B under 2, the small-order key under 3;
/// floor 2; a player of format 3.
struct World {
    a: SigningKey,
    b: SigningKey,
    keys: Vec<(u8, [u8; KEY_LEN])>,
}

const FLOOR: u32 = 2;
const ABI: u8 = 3;

fn world() -> World {
    let a = SigningKey::from_bytes(&seed("A"));
    let b = SigningKey::from_bytes(&seed("B"));
    let keys = vec![(0, a.verifying_key().to_bytes()), (1, a.verifying_key().to_bytes()), (2, b.verifying_key().to_bytes()), (3, SMALL_ORDER)];
    World { a, b, keys }
}

#[derive(Clone)]
struct Fields {
    kid: u8,
    abi: u8,
    rev: u32,
    stem: String,
    src_len: u32,
    src_sha256: [u8; 32],
    psb_len: u32,
    psb_sha256: [u8; 32],
}

impl Fields {
    fn formula(&self) -> Formula<'_> {
        Formula {
            kid: self.kid,
            abi: self.abi,
            rev: self.rev,
            stem: &self.stem,
            src_len: self.src_len,
            src_sha256: self.src_sha256,
            psb_len: self.psb_len,
            psb_sha256: self.psb_sha256,
        }
    }
}

struct Case {
    name: &'static str,
    note: &'static str,
    f: Fields,
    player_abi: u8,
    sig: [u8; 64],
    expect: &'static str,
}

fn code(r: Result<(), Refusal>) -> &'static str {
    match r {
        Ok(()) => "ok",
        Err(Refusal::BadStem) => "bad-stem",
        Err(Refusal::UnknownKey) => "unknown-key",
        Err(Refusal::WrongAbi) => "wrong-abi",
        Err(Refusal::OldRev) => "old-rev",
        Err(Refusal::BadSignature) => "bad-signature",
    }
}

fn run(w: &World, c: &Case) -> &'static str {
    let keys: Vec<(u8, &[u8; KEY_LEN])> = w.keys.iter().map(|(id, k)| (*id, k)).collect();
    let t = Trust { keys: &keys, floor: FLOOR, abi: c.player_abi };
    code(gallery_sig::check(&t, &c.f.formula(), &c.sig))
}

fn sign(k: &SigningKey, f: &Fields) -> [u8; 64] {
    k.sign(gallery_sig::message(&f.formula()).unwrap().as_bytes()).to_bytes()
}

fn cases(w: &World) -> Vec<Case> {
    let src = b"name \"Plasma\"\nhsv(x + t / 4, 0.9, 1)\n".to_vec();
    let psb = b"PSB\x03\x00\x06Plasma not a real program, only bytes to hash".to_vec();
    let base = Fields {
        kid: 0,
        abi: ABI,
        rev: FLOOR,
        stem: "plasma".into(),
        src_len: src.len() as u32,
        src_sha256: sha256(&src),
        psb_len: psb.len() as u32,
        psb_sha256: sha256(&psb),
    };
    let good = sign(&w.a, &base);
    let with = |f: &dyn Fn(&mut Fields)| {
        let mut x = base.clone();
        f(&mut x);
        x
    };
    let case = |name, note, f: Fields, sig, expect| Case { name, note, f, player_abi: ABI, sig, expect };
    let mut v = Vec::new();
    // Blue.
    v.push(case("ok", "key 0 signs plasma at the floor", base.clone(), good, "ok"));
    let above = with(&|f| f.rev = FLOOR + 3);
    let s = sign(&w.a, &above);
    v.push(case("ok-rev-above-floor", "a later revision is taken", above, s, "ok"));
    let b2 = with(&|f| f.kid = 2);
    let s = sign(&w.b, &b2);
    v.push(case("ok-key-2", "another trusted key under its own id", b2, s, "ok"));
    // Red: one bit flipped in each field, the signature kept.
    v.push(case("flip-kid", "kid 0 -> 1 (the same key under id 1: the message binds the id)", with(&|f| f.kid ^= 1), good, "bad-signature"));
    v.push(case("flip-abi", "abi 3 -> 2: not the player's format, refused before the signature", with(&|f| f.abi ^= 1), good, "wrong-abi"));
    let mut c = case("flip-abi-player-too", "abi 3 -> 2 on a player of format 2: the message binds it", with(&|f| f.abi ^= 1), good, "bad-signature");
    c.player_abi = ABI ^ 1;
    v.push(c);
    v.push(case("flip-rev", "rev 2 -> 3", with(&|f| f.rev ^= 1), good, "bad-signature"));
    v.push(case("flip-stem", "plasma -> qlasma", with(&|f| f.stem = "qlasma".into()), good, "bad-signature"));
    v.push(case("flip-src-len", "", with(&|f| f.src_len ^= 1), good, "bad-signature"));
    v.push(case("flip-src-sha256", "", with(&|f| f.src_sha256[31] ^= 1), good, "bad-signature"));
    v.push(case("flip-psb-len", "", with(&|f| f.psb_len ^= 1), good, "bad-signature"));
    v.push(case("flip-psb-sha256", "", with(&|f| f.psb_sha256[0] ^= 0x80), good, "bad-signature"));
    let mut s = good;
    s[0] ^= 1;
    v.push(case("flip-sig-r", "a bit of R", base.clone(), s, "bad-signature"));
    let mut s = good;
    s[40] ^= 1;
    v.push(case("flip-sig-s", "a bit of S", base.clone(), s, "bad-signature"));
    // Red: whole other things.
    v.push(case("other-stem", "plasma's signature offered for big-clock", with(&|f| f.stem = "big-clock".into()), good, "bad-signature"));
    v.push(case("other-key", "signed by key B, named as key 0", base.clone(), sign(&w.b, &base), "bad-signature"));
    let old = with(&|f| f.rev = FLOOR - 1);
    let s = sign(&w.a, &old);
    v.push(case("rev-under-floor", "validly signed at rev 1, the floor 2: revoked", old, s, "old-rev"));
    let unknown = with(&|f| f.kid = 9);
    let s = sign(&w.a, &unknown);
    v.push(case("unknown-kid", "validly signed as key 9, which the firmware does not know", unknown, s, "unknown-key"));
    v.push(case("bad-stem", "Plasma: outside the stem rule, no message", with(&|f| f.stem = "Plasma".into()), good, "bad-stem"));
    // Red: other domains' signatures by the same key.
    let msg = gallery_sig::message(&base.formula()).unwrap();
    let tail = &msg.as_bytes()[DOMAIN.len()..];
    let connect = [b"opendmd-connect-v1 cmd\0".as_slice(), tail].concat();
    v.push(case("connect-command", "key 0's signature over a Connect command (its domain, this message's fields)", base.clone(), w.a.sign(&connect).to_bytes(), "bad-signature"));
    let mut bundle = b"ODFW44mb".to_vec();
    let mut version = [0u8; 32];
    version[..13].copy_from_slice(b"1.1.0-beta.4\0");
    bundle.extend_from_slice(&version);
    bundle.extend_from_slice(&(psb.len() as u32).to_le_bytes());
    bundle.extend_from_slice(&sha256(&psb));
    v.push(case("firmware-bundle", "key 0's signature over a firmware bundle's header (ODFW44mb, version, length, SHA-256)", base.clone(), w.a.sign(&bundle).to_bytes(), "bad-signature"));
    let www = [b"ODWPSIG1\0".as_slice(), &sha256(&psb)].concat();
    v.push(case("www-pack", "key 0's signature over a www pack (ODWPSIG1, key id, SHA-256)", base.clone(), w.a.sign(&www).to_bytes(), "bad-signature"));
    v.push(case("no-domain", "key 0's signature over the fields without the domain", base.clone(), w.a.sign(tail).to_bytes(), "bad-signature"));
    // Red: the strict check's own cases.
    let mut s = [0u8; 64];
    s[0] = 1; // R = the identity, S = 0
    v.push(case("small-order-key", "key 3 is the identity point; R = identity, S = 0 verifies any message unless the check is strict", with(&|f| f.kid = 3), s, "bad-signature"));
    let mut s = good;
    s[32..].copy_from_slice(&add_l(&good[32..]));
    v.push(case("non-canonical-s", "the good signature with S + L: the same scalar, not canonical", base.clone(), s, "bad-signature"));
    v
}

fn build() -> Value {
    let w = world();
    let cases: Vec<Value> = cases(&w)
        .iter()
        .map(|c| {
            assert_eq!(run(&w, c), c.expect, "{}", c.name);
            json!({
                "name": c.name,
                "note": c.note,
                "kid": c.f.kid,
                "abi": c.f.abi,
                "rev": c.f.rev,
                "stem": c.f.stem,
                "src_len": c.f.src_len,
                "src_sha256": hex(&c.f.src_sha256),
                "psb_len": c.f.psb_len,
                "psb_sha256": hex(&c.f.psb_sha256),
                "player_abi": c.player_abi,
                "message": gallery_sig::message(&c.f.formula()).ok().map(|m| hex(m.as_bytes())),
                "sig": hex(&c.sig),
                "expect": c.expect,
            })
        })
        .collect();
    json!({
        "about": "OpenDMD gallery signatures (opendmd_core::gallery_sig): throwaway keys only. Made by opendmd-core/tests/gallery_vectors.rs, checked by tools/gallery-sign.py selftest.",
        "domain": hex(DOMAIN),
        "max_message": gallery_sig::MAX_MESSAGE,
        "floor": FLOOR,
        "keys": w.keys.iter().map(|(id, k)| json!({"kid": id, "pub": hex(k)})).collect::<Vec<_>>(),
        "seeds": {"A": hex(&seed("A")), "B": hex(&seed("B"))},
        "signers": {"0": "A", "1": "A", "2": "B"},
        "cases": cases,
    })
}

#[test]
fn vectors_are_pinned() {
    let got = serde_json::to_string_pretty(&build()).unwrap() + "\n";
    if std::env::var("GALLERY_VECTORS").as_deref() == Ok("write") {
        std::fs::write(PATH, &got).unwrap();
        return;
    }
    let pinned = std::fs::read_to_string(PATH).expect("tests/gallery-vectors.json (GALLERY_VECTORS=write makes it)");
    assert!(pinned == got, "the core no longer makes tests/gallery-vectors.json: a format change? (GALLERY_VECTORS=write, then review the diff)");
}

/// The pinned file read back as another implementation would: every case
/// through the verifier, from its own fields.
#[test]
fn the_pinned_cases_hold() {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(PATH).unwrap()).unwrap();
    let keys: Vec<(u8, [u8; KEY_LEN])> =
        v["keys"].as_array().unwrap().iter().map(|k| (k["kid"].as_u64().unwrap() as u8, unhex(k["pub"].as_str().unwrap()).try_into().unwrap())).collect();
    let refs: Vec<(u8, &[u8; KEY_LEN])> = keys.iter().map(|(id, k)| (*id, k)).collect();
    let cases = v["cases"].as_array().unwrap();
    assert!(cases.len() >= 24);
    let mut reds = 0;
    for c in cases {
        let stem = c["stem"].as_str().unwrap();
        let f = Formula {
            kid: c["kid"].as_u64().unwrap() as u8,
            abi: c["abi"].as_u64().unwrap() as u8,
            rev: c["rev"].as_u64().unwrap() as u32,
            stem,
            src_len: c["src_len"].as_u64().unwrap() as u32,
            src_sha256: unhex(c["src_sha256"].as_str().unwrap()).try_into().unwrap(),
            psb_len: c["psb_len"].as_u64().unwrap() as u32,
            psb_sha256: unhex(c["psb_sha256"].as_str().unwrap()).try_into().unwrap(),
        };
        let t = Trust { keys: &refs, floor: v["floor"].as_u64().unwrap() as u32, abi: c["player_abi"].as_u64().unwrap() as u8 };
        let sig: [u8; 64] = unhex(c["sig"].as_str().unwrap()).try_into().unwrap();
        assert_eq!(code(gallery_sig::check(&t, &f, &sig)), c["expect"].as_str().unwrap(), "{}", c["name"]);
        if let Some(m) = c["message"].as_str() {
            assert_eq!(hex(gallery_sig::message(&f).unwrap().as_bytes()), m, "{}", c["name"]);
        }
        reds += usize::from(c["expect"] != "ok");
    }
    assert!(reds >= 20, "the red cases are there");
}

/// The shipped trust refuses the vectors' good signature: their keys are
/// throwaway ones, never the gallery's.
#[test]
fn the_firmware_trusts_no_test_key() {
    let w = world();
    let c = &cases(&w)[0];
    assert_eq!(c.expect, "ok");
    assert_ne!(code(gallery_sig::check(&Trust::panel(ABI), &c.f.formula(), &c.sig)), "ok");
    for (_, k) in gallery_sig::GALLERY_KEYS {
        assert!(w.keys.iter().all(|(_, t)| t != *k), "a throwaway key is in GALLERY_KEYS");
    }
}
