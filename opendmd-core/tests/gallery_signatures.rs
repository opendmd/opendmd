//! The committed gallery signatures, checked by the firmware's own verifier
//! (`opendmd_core::gallery_sig::check`, `Trust::panel`): every project
//! Formula (`opendmd-web/formulas/*.psi`) has a current signature by every
//! key the firmware trusts, over its source as sent (the file without its
//! gallery header, what `opendmd-gallery check --out` writes) and its
//! compiled file as this tree's compiler makes it, at a revision the
//! firmware takes and for the compiled format it plays. A Formula changed,
//! added or removed since `make gallery-sign`, or a signature by any other
//! key, fails.
//!
//! **No gallery key committed yet** is a state of its own, never mistaken
//! for signed: `GALLERY_KEYS` empty, no `firmware/keys/gallery*.pub` and no
//! `opendmd-web/formulas/signatures.json`; the check says UNSIGNED and
//! passes (the firmware then refuses every add). Any mix of the three
//! fails.
//!
//! With `GALLERY_OUT` set (`make gallery-check`), the gallery tool's output
//! is held to the same bytes: what the site publishes is what was signed.
#![cfg(feature = "gallery-sig")]

use std::path::{Path, PathBuf};

use opendmd_core::gallery_sig::{self, Formula, Trust, GALLERY_KEYS, KEY_LEN};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
        return None;
    }
    Some((0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect())
}

/// A project Formula as the gallery sends it: its stem, its source without
/// the gallery header (`-- author:`, `-- licence:`, and `-- bundle` when
/// the third line says so: `opendmd-gallery`'s rule), its compiled file.
struct Published {
    stem: String,
    src: Vec<u8>,
    psb: Vec<u8>,
}

fn strip_header(text: &str) -> &str {
    let bundle = text.split('\n').nth(2).is_some_and(|l| l.trim_end() == "-- bundle");
    let skip = if bundle { 3 } else { 2 };
    let offset: usize = text.split_inclusive('\n').take(skip).map(str::len).sum();
    &text[offset.min(text.len())..]
}

fn project_formulas() -> Vec<Published> {
    let dir = repo().join("opendmd-web/formulas");
    let mut out: Vec<Published> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let stem = p.file_name()?.to_str()?.strip_suffix(".psi")?.to_string();
            let text = std::fs::read_to_string(&p).unwrap();
            let src = strip_header(&text).to_string();
            let psb = opendmd_psi::build(&src).unwrap_or_else(|e| panic!("{stem}: {}", e.msg));
            Some(Published { stem, src: src.into_bytes(), psb })
        })
        .collect();
    out.sort_by(|a, b| a.stem.cmp(&b.stem));
    out
}

/// `firmware/keys/gallery.pub` (id 0) and `gallery-<id>.pub`.
fn keys_on_disk() -> Vec<(u8, [u8; KEY_LEN])> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(repo().join("firmware/keys")).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let id = match name.strip_prefix("gallery").and_then(|r| r.strip_suffix(".pub")) {
            Some("") => 0,
            Some(n) => n.strip_prefix('-').and_then(|n| n.parse::<u8>().ok()).unwrap_or_else(|| panic!("firmware/keys/{name}: not gallery.pub or gallery-<id>.pub")),
            None => continue,
        };
        let k = std::fs::read(e.path()).unwrap();
        out.push((id, k.try_into().unwrap_or_else(|_| panic!("firmware/keys/{name} is not a 32-byte key"))));
    }
    out.sort();
    out
}

/// What the check found: a line for the log.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Unsigned(usize),
    Signed { formulas: usize, signatures: usize, keys: usize },
}

/// The set check (see the module docs): `sigs` the signatures file, if
/// there is one; `pubs_on_disk` whether any `gallery*.pub` is committed.
fn check_set(t: &Trust, formulas: &[Published], sigs: Option<&Value>, pubs_on_disk: bool) -> Result<Outcome, Vec<String>> {
    let mut bad = Vec::new();
    if t.keys.is_empty() {
        if sigs.is_some() || pubs_on_disk {
            return Err(vec![format!(
                "no gallery key is compiled in (GALLERY_KEYS is empty), but {}: commit the key and its table entry together with the signatures",
                if pubs_on_disk { "a firmware/keys/gallery*.pub is committed" } else { "opendmd-web/formulas/signatures.json is" }
            )]);
        }
        return Ok(Outcome::Unsigned(formulas.len()));
    }
    let Some(sigs) = sigs else {
        return Err(vec!["a gallery key is compiled in but there is no opendmd-web/formulas/signatures.json: make gallery-sign".into()]);
    };
    let Some(table) = sigs.get("formulas").and_then(Value::as_object) else {
        return Err(vec!["signatures.json has no `formulas` object".into()]);
    };
    for stem in table.keys() {
        if !formulas.iter().any(|f| &f.stem == stem) {
            bad.push(format!("{stem}: signed, but no such project Formula (stale: make gallery-sign)"));
        }
    }
    let mut n = 0;
    for f in formulas {
        let Some(list) = table.get(&f.stem).and_then(Value::as_array) else {
            bad.push(format!("{}: no signature (make gallery-sign)", f.stem));
            continue;
        };
        let mut by: Vec<u8> = Vec::new();
        for s in list {
            n += 1;
            match one(t, f, s) {
                Ok(kid) => by.push(kid),
                Err(e) => bad.push(format!("{}: {e}", f.stem)),
            }
        }
        for (kid, _) in t.keys {
            if !by.contains(kid) {
                bad.push(format!("{}: no current signature by gallery key {kid} (make gallery-sign)", f.stem));
            }
        }
    }
    if bad.is_empty() {
        Ok(Outcome::Signed { formulas: formulas.len(), signatures: n, keys: t.keys.len() })
    } else {
        Err(bad)
    }
}

/// One signature entry against the Formula as published: its fields are
/// this source's and compiled file's, and the firmware's check takes it.
fn one(t: &Trust, f: &Published, s: &Value) -> Result<u8, String> {
    let num = |k: &str| s.get(k).and_then(Value::as_u64).ok_or(format!("`{k}` missing"));
    let bytes = |k: &str, n: usize| s.get(k).and_then(Value::as_str).and_then(unhex).filter(|b| b.len() == n).ok_or(format!("`{k}` is not {n} bytes of hex"));
    let kid = u8::try_from(num("kid")?).map_err(|_| "kid past 255")?;
    let fields = Formula {
        kid,
        abi: u8::try_from(num("abi")?).map_err(|_| "abi past 255")?,
        rev: u32::try_from(num("rev")?).map_err(|_| "rev past 2^32")?,
        stem: &f.stem,
        src_len: u32::try_from(num("src_len")?).map_err(|_| "src_len past 2^32")?,
        src_sha256: bytes("src_sha256", 32)?.try_into().unwrap(),
        psb_len: u32::try_from(num("psb_len")?).map_err(|_| "psb_len past 2^32")?,
        psb_sha256: bytes("psb_sha256", 32)?.try_into().unwrap(),
    };
    let sig: [u8; 64] = bytes("sig", 64)?.try_into().unwrap();
    if fields.src_len as usize != f.src.len() || fields.src_sha256 != sha256(&f.src) {
        return Err(format!("key {kid}'s signature is for another source (stale: make gallery-sign)"));
    }
    if fields.psb_len as usize != f.psb.len() || fields.psb_sha256 != sha256(&f.psb) {
        return Err(format!("key {kid}'s signature is for another compiled file (the compiler or ABI changed: make gallery-sign)"));
    }
    let w = gallery_sig::check(t, &fields, &sig).map_err(|r| format!("key {kid}'s signature refused by the firmware's check: {r:?}"))?;
    assert_eq!(w.stem(), f.stem);
    Ok(kid)
}

fn signatures_file() -> Option<Value> {
    let p = repo().join("opendmd-web/formulas/signatures.json");
    let text = std::fs::read_to_string(&p).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
}

// THE PINNED GALLERY KEYS (id, firmware/keys/gallery.pub as hex). The
// signing commit edits the next line by hand to `&[(0, "<the 64 hex
// digits>")]`, with the table and the signatures. Once set the gallery must
// stay GALLERY SIGNED by exactly that key: deleting the key file, the table
// entry or the signatures, or a different key, fails. Empty: no key yet.
const PINNED: &[(u8, &str)] = &[];

/// The table the firmware compiles in is the committed key files and the
/// pinned hex, id for id, and none of them is a test vector's throwaway key.
#[test]
fn the_table_is_the_committed_keys() {
    let table: Vec<(u8, [u8; KEY_LEN])> = GALLERY_KEYS.iter().map(|&(id, k)| (id, *k)).collect();
    let mut table = table;
    table.sort();
    assert_eq!(table, keys_on_disk(), "gallery_sig::GALLERY_KEYS and firmware/keys/gallery*.pub disagree");
    let mut pinned: Vec<(u8, String)> = PINNED.iter().map(|&(id, h)| (id, h.to_string())).collect();
    pinned.sort();
    assert_eq!(table.iter().map(|(id, k)| (*id, hex(k))).collect::<Vec<_>>(), pinned, "GALLERY_KEYS and the pinned gallery key (PINNED) disagree");
    let throwaway: Vec<[u8; 32]> = ["A", "B"]
        .iter()
        .map(|n| {
            let s = sha256(format!("opendmd-gallery throwaway test key, never a real one: {n}").as_bytes());
            ed25519_dalek::SigningKey::from_bytes(&s).verifying_key().to_bytes()
        })
        .collect();
    for (id, k) in &table {
        assert!(!throwaway.contains(k), "gallery key {id} is a test vector's throwaway key");
    }
}

/// Every committed signature, by the firmware's verifier, against the
/// project's Formulas as this tree builds them.
#[test]
fn committed_signatures_hold() {
    let formulas = project_formulas();
    assert!(formulas.len() >= 30, "the project's Formulas: {}", formulas.len());
    if let Some(out) = std::env::var_os("GALLERY_OUT") {
        let out = PathBuf::from(out);
        for f in &formulas {
            let src = std::fs::read(out.join(format!("{}.psi", f.stem))).unwrap_or_else(|e| panic!("{}: {e} (the gallery tool's --out)", f.stem));
            let psb = std::fs::read(out.join(format!("{}.psb", f.stem))).unwrap_or_else(|e| panic!("{}.psb: {e}", f.stem));
            assert!(src == f.src && psb == f.psb, "{}: the gallery tool's output is not what is signed", f.stem);
        }
        let extra = std::fs::read_dir(&out).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "psi")).count();
        assert_eq!(extra, formulas.len(), "the gallery tool wrote other Formulas than the project's");
    }
    let t = Trust::panel(opendmd_psi::psb::ABI);
    match check_set(&t, &formulas, signatures_file().as_ref(), !keys_on_disk().is_empty()) {
        Ok(Outcome::Unsigned(_)) if !PINNED.is_empty() => panic!("a gallery key is pinned (PINNED): the gallery must be GALLERY SIGNED, not unsigned"),
        Ok(Outcome::Unsigned(n)) => {
            eprintln!("GALLERY UNSIGNED: no gallery key committed yet (GALLERY_KEYS empty, no firmware/keys/gallery.pub, no signatures.json): {n} project Formulas, none signed; a panel refuses every add");
        }
        Ok(Outcome::Signed { formulas, signatures, keys }) => {
            eprintln!("GALLERY SIGNED: {formulas} project Formulas, {signatures} signatures by {keys} gallery key(s), each current and taken by the firmware's check");
        }
        Err(bad) => panic!("gallery signatures:\n  {}", bad.join("\n  ")),
    }
}

// --- The set check's own red tests, with a throwaway key --------------------------

mod red {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    fn key(n: &str) -> SigningKey {
        SigningKey::from_bytes(&sha256(format!("opendmd-gallery set-check test key: {n}").as_bytes()))
    }

    fn set() -> Vec<Published> {
        ["plasma", "fire"]
            .iter()
            .map(|s| {
                let src = format!("name \"{s}\"\ngray(x)\n");
                let psb = opendmd_psi::build(&src).unwrap();
                Published { stem: s.to_string(), src: src.into_bytes(), psb }
            })
            .collect()
    }

    fn signed(k: &SigningKey, kid: u8, f: &Published, rev: u32) -> Value {
        let fields = Formula {
            kid,
            abi: opendmd_psi::psb::ABI,
            rev,
            stem: &f.stem,
            src_len: f.src.len() as u32,
            src_sha256: sha256(&f.src),
            psb_len: f.psb.len() as u32,
            psb_sha256: sha256(&f.psb),
        };
        let sig = k.sign(gallery_sig::message(&fields).unwrap().as_bytes()).to_bytes();
        json!({"kid": kid, "abi": fields.abi, "rev": rev, "src_len": fields.src_len, "src_sha256": hex(&fields.src_sha256),
               "psb_len": fields.psb_len, "psb_sha256": hex(&fields.psb_sha256), "sig": hex(&sig)})
    }

    fn file(k: &SigningKey, fs: &[Published]) -> Value {
        let mut m = serde_json::Map::new();
        for f in fs {
            m.insert(f.stem.clone(), json!([signed(k, 0, f, 1)]));
        }
        json!({"formulas": m})
    }

    #[test]
    fn a_whole_current_set_passes() {
        let k = key("ok");
        let pk = k.verifying_key().to_bytes();
        let keys = [(0u8, &pk)];
        let t = Trust { keys: &keys, floor: 1, abi: opendmd_psi::psb::ABI };
        let fs = set();
        assert_eq!(check_set(&t, &fs, Some(&file(&k, &fs)), true), Ok(Outcome::Signed { formulas: 2, signatures: 2, keys: 1 }));
    }

    #[test]
    fn stale_missing_extra_and_foreign_signatures_fail() {
        let k = key("ok");
        let pk = k.verifying_key().to_bytes();
        let keys = [(0u8, &pk)];
        let t = Trust { keys: &keys, floor: 1, abi: opendmd_psi::psb::ABI };
        let fs = set();
        let good = file(&k, &fs);
        let fails = |v: &Value, fs: &[Published], what: &str| {
            let r = check_set(&t, fs, Some(v), true);
            assert!(r.is_err(), "{what}: {r:?}");
        };
        // A source changed since it was signed.
        let mut changed = set();
        changed[0].src = b"name \"plasma\"\ngray(y)\n".to_vec();
        changed[0].psb = opendmd_psi::build("name \"plasma\"\ngray(y)\n").unwrap();
        fails(&good, &changed, "a stale signature (the source changed)");
        // A compiled file changed (a compiler or ABI change) with the same source.
        let mut recompiled = set();
        recompiled[1].psb.push(0);
        fails(&good, &recompiled, "a stale signature (the compiled file changed)");
        // A Formula with no signature.
        let mut more = set();
        more.push(Published { stem: "new-one".into(), src: b"gray(t)\n".to_vec(), psb: opendmd_psi::build("gray(t)\n").unwrap() });
        fails(&good, &more, "a missing signature");
        // A signature for a Formula that is gone.
        fails(&good, &fs[..1], "a signature with no Formula");
        // Signed by a throwaway key: never passes a check against the committed one.
        fails(&file(&key("throwaway"), &fs), &fs, "a throwaway key's signatures");
        // Under the floor, another ABI, an unknown id.
        let mut v = good.clone();
        v["formulas"]["plasma"] = json!([signed(&k, 0, &fs[0], 0)]);
        fails(&v, &fs, "a revision under the floor");
        let mut v = good.clone();
        v["formulas"]["plasma"][0]["kid"] = json!(5);
        fails(&v, &fs, "an unknown key id");
        let mut v = good.clone();
        v["formulas"]["fire"][0]["sig"] = json!("00".repeat(64));
        fails(&v, &fs, "a bad signature");
        let mut v = good.clone();
        v["formulas"]["fire"] = json!([]);
        fails(&v, &fs, "an empty list");
        // A second key trusted: each Formula needs its signature too.
        let k2 = key("two");
        let pk2 = k2.verifying_key().to_bytes();
        let keys2 = [(0u8, &pk), (1u8, &pk2)];
        let t2 = Trust { keys: &keys2, floor: 1, abi: opendmd_psi::psb::ABI };
        assert!(check_set(&t2, &fs, Some(&good), true).is_err(), "a changeover: key 1's signatures missing");
        let mut both = good.clone();
        for f in &fs {
            both["formulas"][&f.stem].as_array_mut().unwrap().push(signed(&k2, 1, f, 1));
        }
        assert!(check_set(&t2, &fs, Some(&both), true).is_ok());
        // No signatures file with a key compiled in.
        assert!(check_set(&t, &fs, None, true).is_err(), "a key but no signatures");
    }

    /// The no-key state passes only whole: no key compiled in, none
    /// committed, no signatures; it is reported as unsigned, never signed.
    #[test]
    fn no_key_committed_is_its_own_state() {
        let t = Trust { keys: &[], floor: 1, abi: opendmd_psi::psb::ABI };
        let fs = set();
        assert_eq!(check_set(&t, &fs, None, false), Ok(Outcome::Unsigned(2)));
        assert!(check_set(&t, &fs, None, true).is_err(), "a key file committed, the table empty");
        assert!(check_set(&t, &fs, Some(&file(&key("ok"), &fs)), false).is_err(), "signatures with no key compiled in");
    }

    #[test]
    fn the_header_comes_off_as_the_gallery_takes_it() {
        assert_eq!(strip_header("-- author: OpenDMD\n-- licence: CC0-1.0\nname \"X\"\ngray(x)\n"), "name \"X\"\ngray(x)\n");
        assert_eq!(strip_header("-- author: OpenDMD\n-- licence: CC0-1.0\n-- bundle\nname \"X\"\n"), "name \"X\"\n");
    }
}
