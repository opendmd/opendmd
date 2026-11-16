//! OpenDMD Connect v1's shared vectors (`tests/connect-vectors.json`): made
//! here from fixed, throwaway inputs, pinned in the file, and checked
//! against the core. The Worker, the web controller and the Python mocks
//! (`tools/connect/`) check themselves against the same file.
//!
//! Every command case says what the panel answers (`expect`) and what the
//! server's pre-check does (`server`: `forward` or the refusal). The
//! expectations are written by hand below, not computed: the test is that
//! the core agrees with them.
//!
//! After a deliberate protocol change: `CONNECT_VECTORS=write cargo test
//! --features connect --test connect_vectors`, then review the diff.
#![cfg(feature = "connect")]

use core::time::Duration;
use ed25519_dalek::{Signer, SigningKey};
use opendmd_core::connect::codec::{base32, hex, hmac_sha256, sha256};
use opendmd_core::connect::keys::{key_id, Entry, KeyList, FLAG_CAN_ADD};
use opendmd_core::connect::pair;
use opendmd_core::connect::proto::{self, Dev, Limits, Negotiated, Refusal, Session};
use serde_json::{json, Value};

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/connect-vectors.json");

fn seed(name: &str) -> [u8; 32] {
    sha256(&[b"opendmd-connect throwaway test key, never a real one: ", name.as_bytes()])
}

struct World {
    a: SigningKey,
    b: SigningKey,
    stranger: SigningKey,
    dev: Dev,
    sess: [u8; 12],
    keys: KeyList,
    limits: Limits,
    id_key: [u8; 32],
    dev_random: [u8; 10],
}

fn world() -> World {
    let a = SigningKey::from_bytes(&seed("A"));
    let b = SigningKey::from_bytes(&seed("B"));
    let stranger = SigningKey::from_bytes(&seed("stranger"));
    let id_key = sha256(&[b"opendmd-connect throwaway ID_KEY"]);
    let dev_random: [u8; 10] = sha256(&[b"opendmd-connect throwaway dev"])[..10].try_into().unwrap();
    let dev = Dev::mint(&dev_random, &id_key);
    let sess: [u8; 12] = sha256(&[b"opendmd-connect throwaway sess"])[..12].try_into().unwrap();
    let mut keys = KeyList::new();
    keys.add(Entry { public: a.verifying_key().to_bytes(), flags: FLAG_CAN_ADD, label: "A phone".into(), added: 1_763_251_200, added_by: None }).unwrap();
    keys.add(Entry {
        public: b.verifying_key().to_bytes(),
        flags: 0,
        label: "B laptop".into(),
        added: 1_763_337_600,
        added_by: Some(key_id(&a.verifying_key().to_bytes())),
    })
    .unwrap();
    World { a, b, stranger, dev, sess, keys, limits: Limits { max_brightness: 128 }, id_key, dev_random }
}

struct Case {
    name: &'static str,
    note: &'static str,
    frame: Vec<u8>,
    before: Vec<Vec<u8>>,
    expect: &'static str,
    server: &'static str,
}

fn cases(w: &World) -> Vec<Case> {
    let body = |n: u64, c: &str, args: &str| proto::command_body(&w.dev, &w.sess, n, c, args);
    let sign = |k: &SigningKey, body: &str| proto::sign_command(k, body.as_bytes());
    let a = |n: u64, c: &str, args: &str| sign(&w.a, &body(n, c, args));
    let case = |name, note, frame, expect, server| Case { name, note, frame, before: vec![], expect, server };
    let mut v = Vec::new();

    // Blue: what must work.
    v.push(case("ok-next", "a paired key, the current session", a(1, "next", ""), "ok", "forward"));
    v.push(case("ok-play-gif", "", a(1, "play", r#""path":"/gifs/Arcade/Pac Man.gif""#), "ok", "forward"));
    v.push(case("ok-play-formula", "", a(1, "play", r#""path":"@formula:fire""#), "ok", "forward"));
    v.push(case("ok-play-formula-file", "", a(1, "play", r#""path":"/formulas/big-clock.psi""#), "ok", "forward"));
    v.push(case("ok-brightness-ceiling", "max_brightness is 128", a(1, "brightness", r#""v":128"#), "ok", "forward"));
    v.push(case("ok-playlist", "", a(1, "playlist", r#""set":"consoles""#), "ok", "forward"));
    v.push(case("ok-no-args", "`a` may be left out", sign(&w.a, &format!(r#"{{"v":1,"dev":"{}","sess":"{}","n":1,"c":"pause"}}"#, w.dev.text(), hex(&w.sess))), "ok", "forward"));
    v.push(case(
        "ok-bytes-as-sent",
        "whitespace and another key order: the signature is over the bytes, no canonical form",
        sign(&w.a, &format!("{{ \"c\" : \"prev\",\n \"n\":1, \"sess\":\"{}\", \"dev\":\"{}\", \"v\":1, \"a\":{{}}, \"later\":[1,2] }}", hex(&w.sess), w.dev.text())),
        "ok",
        "forward",
    ));
    v.push(case("ok-b-removes-a", "any paired key may remove any key", sign(&w.b, &body(1, "remove-controller", &format!(r#""k":"{}""#, hex(&key_id(&w.a.verifying_key().to_bytes()))))), "ok", "forward"));
    let new_pub = SigningKey::from_bytes(&seed("C")).verifying_key().to_bytes();
    v.push(case("ok-a-adds", "A has can_add", a(1, "add-controller", &format!(r#""pub":"{}","label":"C tablet""#, hex(&new_pub))), "ok", "forward"));
    let sha = hex(&[0x11; 32]);
    v.push(case("ok-send", "", a(1, "send", &format!(r#""id":7,"name":"cat.gif","size":524288,"sha256":"{sha}","copy_size":1000,"copy_sha256":"{sha}","play":true"#)), "ok", "forward"));
    v.push(case("ok-name", "", a(1, "name", r#""name":"Den panel""#), "ok", "forward"));

    // Red: signatures and keys.
    let mut unsigned = a(1, "next", "");
    unsigned[9..73].fill(0);
    v.push(case("unsigned", "an all-zero signature", unsigned, "bad-sig", "bad-sig"));
    let mut by_b = sign(&w.b, &body(1, "next", ""));
    by_b[1..9].copy_from_slice(&key_id(&w.a.verifying_key().to_bytes()));
    v.push(case("wrong-signer", "B's signature under A's key id", by_b, "bad-sig", "bad-sig"));
    let server_key = SigningKey::from_bytes(&seed("server"));
    let mut forged = sign(&server_key, &body(1, "next", ""));
    forged[1..9].copy_from_slice(&key_id(&w.a.verifying_key().to_bytes()));
    v.push(case("made-by-the-server", "the server signs with a key of its own under A's id", forged, "bad-sig", "bad-sig"));
    let mut tampered = a(1, "brightness", r#""v":10"#);
    let at = tampered.len() - 3;
    tampered[at] = b'9';
    v.push(case("tampered-body", "\"v\":10 changed to \"v\":19 after signing", tampered, "bad-sig", "bad-sig"));
    let compact = body(1, "next", "");
    let mut resent = sign(&w.a, &compact);
    resent.truncate(73);
    resent.extend_from_slice(compact.replace(',', ", ").as_bytes());
    v.push(case("reserialised", "signed compact, sent with spaces: not the bytes signed", resent, "bad-sig", "bad-sig"));
    v.push(case("unknown-key", "a stranger's key", sign(&w.stranger, &body(1, "next", "")), "unknown-key", "unknown-key"));

    // Red: replay and session.
    let first = a(5, "next", "");
    v.push(Case { name: "replayed", note: "the same frame twice", frame: first.clone(), before: vec![first.clone()], expect: "replay", server: "forward" });
    v.push(Case { name: "old-counter", note: "n=4 after n=5", frame: a(4, "next", ""), before: vec![first.clone()], expect: "replay", server: "forward" });
    v.push(Case { name: "same-counter-new-body", note: "n=5 again with another command", frame: a(5, "pause", ""), before: vec![first.clone()], expect: "replay", server: "forward" });
    v.push(Case { name: "counter-is-per-key", note: "B's n=1 after A's n=5", frame: sign(&w.b, &body(1, "next", "")), before: vec![first], expect: "ok", server: "forward" });
    let other_sess = proto::command_body(&w.dev, &[0xee; 12], 1, "next", "");
    v.push(case("wrong-session", "signed for an earlier connection", sign(&w.a, &other_sess), "stale-session", "forward"));
    let other_dev = proto::command_body(&Dev::mint(&[9; 10], &w.id_key), &w.sess, 1, "next", "");
    v.push(case("wrong-panel", "signed for another panel", sign(&w.a, &other_dev), "stale-session", "forward"));
    v.push(case("n-zero", "", a(0, "next", ""), "bad-arg", "forward"));
    let raw = |s: String| sign(&w.a, &s);
    let pre = format!(r#""v":1,"dev":"{}","sess":"{}""#, w.dev.text(), hex(&w.sess));
    v.push(case("n-missing", "", raw(format!(r#"{{{pre},"c":"next"}}"#)), "bad-arg", "forward"));
    v.push(case("n-fraction", "", raw(format!(r#"{{{pre},"n":2.0,"c":"next"}}"#)), "bad-arg", "forward"));
    v.push(case("n-beyond-2^53", "", raw(format!(r#"{{{pre},"n":9007199254740993,"c":"next"}}"#)), "bad-arg", "forward"));
    v.push(case("n-string", "", raw(format!(r#"{{{pre},"n":"2","c":"next"}}"#)), "bad-arg", "forward"));
    v.push(case("v-2-in-body", "the body's version must be the session's", raw(format!(r#"{{"v":2,"dev":"{}","sess":"{}","n":1,"c":"next"}}"#, w.dev.text(), hex(&w.sess))), "bad-arg", "forward"));
    v.push(case("dev-uppercase", "one spelling per value", raw(format!(r#"{{"v":1,"dev":"{}","sess":"{}","n":1,"c":"next"}}"#, w.dev.text().to_uppercase(), hex(&w.sess))), "stale-session", "forward"));

    // Red: the allow-list.
    for (name, c) in [
        ("never-update", "update"),
        ("never-firmware", "firmware"),
        ("never-delete", "delete"),
        ("never-wifi", "wifi"),
        ("never-reboot", "reboot"),
        ("never-restart", "restart"),
        ("never-config", "config"),
        ("never-api-key", "api-key"),
        ("never-forget", "forget"),
        ("never-upper-case", "PLAY"),
        ("never-empty", ""),
    ] {
        v.push(case(name, "not in the allow-list", a(1, c, ""), "not-allowed", "forward"));
    }
    v.push(case("duplicate-c", "\"c\" twice (next, then reboot): JSON parsers disagree, so refused", raw(format!(r#"{{{pre},"n":1,"c":"next","c":"reboot"}}"#)), "bad-arg", "forward"));
    v.push(case("duplicate-escaped-c", "the second \"c\" spelled \\u0063", raw(format!(r#"{{{pre},"n":1,"c":"next","\u0063":"reboot"}}"#)), "bad-arg", "forward"));
    v.push(case("b-cannot-add", "a one-level key trying add-controller", sign(&w.b, &body(1, "add-controller", &format!(r#""pub":"{}","label":"x""#, hex(&new_pub)))), "not-allowed", "forward"));

    // Red: arguments.
    for (name, path) in [
        ("path-dotdot", "/gifs/../wifi.json"),
        ("path-dotdot-deep", "/gifs/a/../../sets/x.gif"),
        ("path-cache", "/cache/Arcade/ab/x.gif"),
        ("path-sent-formula", "@formula:~sent"),
        ("path-inline-formula", "@formula?psb=AAAA"),
        ("path-virtual", "@card:boot"),
        ("path-not-gif", "/gifs/a.txt"),
        ("path-relative", "gifs/a.gif"),
        ("path-backslash", "/gifs/a\\\\..\\\\b.gif"),
        ("path-hidden", "/gifs/.Trash/a.gif"),
    ] {
        v.push(case(name, "", a(1, "play", &format!(r#""path":"{path}""#)), "bad-arg", "forward"));
    }
    v.push(case("path-missing", "", a(1, "play", ""), "bad-arg", "forward"));
    v.push(case("brightness-over-ceiling", "129 over max_brightness 128", a(1, "brightness", r#""v":129"#), "bad-arg", "forward"));
    v.push(case("brightness-negative", "", a(1, "brightness", r#""v":-1"#), "bad-arg", "forward"));
    v.push(case("brightness-string", "", a(1, "brightness", r#""v":"100""#), "bad-arg", "forward"));
    v.push(case("playlist-path", "", a(1, "playlist", r#""set":"../filelist""#), "bad-arg", "forward"));
    v.push(case("a-not-object", "", raw(format!(r#"{{{pre},"n":1,"c":"next","a":[]}}"#)), "bad-arg", "forward"));
    v.push(case("send-over-512k", "", a(1, "send", &format!(r#""id":7,"name":"cat.gif","size":524289,"sha256":"{sha}""#)), "too-big", "forward"));
    v.push(case("send-copy-over-512k", "", a(1, "send", &format!(r#""id":7,"name":"cat.gif","size":10,"sha256":"{sha}","copy_size":524289,"copy_sha256":"{sha}""#)), "too-big", "forward"));
    v.push(case("send-name-slash", "", a(1, "send", &format!(r#""id":7,"name":"../cat.gif","size":10,"sha256":"{sha}""#)), "bad-arg", "forward"));
    v.push(case("send-id-zero", "", a(1, "send", &format!(r#""id":0,"name":"cat.gif","size":10,"sha256":"{sha}""#)), "bad-arg", "forward"));
    v.push(case("send-copy-without-hash", "", a(1, "send", &format!(r#""id":7,"name":"cat.gif","size":10,"sha256":"{sha}","copy_size":5"#)), "bad-arg", "forward"));
    let weak = {
        let mut p = [0u8; 32];
        p[0] = 1;
        p
    };
    v.push(case("add-weak-key", "a small-order public key", a(1, "add-controller", &format!(r#""pub":"{}","label":"x""#, hex(&weak))), "bad-arg", "forward"));
    v.push(case("add-existing-key", "", a(1, "add-controller", &format!(r#""pub":"{}","label":"x""#, hex(&w.b.verifying_key().to_bytes()))), "bad-arg", "forward"));
    v.push(case("add-invisible-label", "a label of zero-width characters only", a(1, "add-controller", &format!(r#""pub":"{}","label":"\u200b\u202e""#, hex(&new_pub))), "bad-arg", "forward"));
    v.push(case("remove-unknown", "", a(1, "remove-controller", r#""k":"0011223344556677""#), "bad-arg", "forward"));
    v.push(case("name-too-long", "33 characters", a(1, "name", &format!(r#""name":"{}""#, "n".repeat(33))), "bad-arg", "forward"));

    // Red: frames.
    let long_path = format!(r#""path":"/gifs/{}.gif""#, "x".repeat(700));
    v.push(case("body-over-700", "", a(1, "play", &long_path), "too-big", "too-big"));
    let mut short = a(1, "next", "");
    short.truncate(72);
    v.push(case("truncated-frame", "shorter than type, key id and signature: dropped, no ack", short, "bad-frame", "bad-frame"));
    let mut wrong_type = a(1, "next", "");
    wrong_type[0] = 0x03;
    v.push(case("unknown-frame-type", "dropped, no ack", wrong_type, "bad-frame", "bad-frame"));
    v.push(case("empty-body", "signed, but nothing to parse", sign(&w.a, ""), "bad-arg", "forward"));
    v.push(case("body-not-object", "", sign(&w.a, "[1]"), "bad-arg", "forward"));
    let mut bad_utf8 = body(1, "name", r#""name":"xx""#).into_bytes();
    let at = bad_utf8.len() - 4;
    bad_utf8[at] = 0xff;
    v.push(case("body-bad-utf8", "", proto::sign_command(&w.a, &bad_utf8), "bad-arg", "forward"));
    v.push(case("body-too-deep", "", a(1, "next", r#""x":{"y":{"z":{}}}"#), "bad-arg", "forward"));
    v.push(case("body-lone-surrogate", "", a(1, "name", r#""name":"\ud800x""#), "bad-arg", "forward"));
    v
}

fn run_case(w: &World, c: &Case) -> String {
    let mut s = Session::new(w.dev, w.sess, 1, Duration::ZERO);
    for f in &c.before {
        s.check(&w.keys, &w.limits, f, Duration::ZERO).expect("a `before` frame is accepted");
    }
    match s.check(&w.keys, &w.limits, &c.frame, Duration::ZERO) {
        Ok(_) => "ok".into(),
        Err(r) => {
            // A refusal is acked to its key, unless there is none.
            assert_eq!(r.ack().is_none(), r.err == Refusal::BadFrame, "{}", c.name);
            r.err.code().into()
        }
    }
}

fn server_case(w: &World, c: &Case) -> String {
    let keys: Vec<_> = w.keys.entries().iter().map(|e| (e.id(), e.public)).collect();
    match proto::precheck(&keys, &c.frame) {
        Ok(_) => "forward".into(),
        Err(r) => r.code().into(),
    }
}

fn controller(name: &str, k: &SigningKey, list: &KeyList) -> Value {
    let p = k.verifying_key().to_bytes();
    let e = list.find(&key_id(&p));
    json!({
        "name": name,
        "seed": hex(&k.to_bytes()),
        "pub": hex(&p),
        "k": hex(&key_id(&p)),
        "on_list": e.is_some(),
        "flags": e.map(|e| e.flags),
        "label": e.map(|e| e.label.clone()),
    })
}

fn build() -> Value {
    let w = world();
    let s_random: [u8; 13] = sha256(&[b"opendmd-connect throwaway pairing secret"])[..13].try_into().unwrap();
    let s = pair::secret_from_random(&s_random);
    let a_pub = w.a.verifying_key().to_bytes();
    let c_key = SigningKey::from_bytes(&seed("C"));
    let c_pub = c_key.verifying_key().to_bytes();
    let i_random: [u8; 16] = sha256(&[b"opendmd-connect throwaway invite secret"])[..16].try_into().unwrap();
    let inv = pair::invite_secret_from_random(&i_random);
    let nonce: [u8; 16] = sha256(&[b"opendmd-connect throwaway challenge"])[..16].try_into().unwrap();
    let secret = sha256(&[b"opendmd-connect throwaway panel secret"]);

    let negotiation: Vec<Value> = [
        (vec!["odc1"], "accept:1"),
        (vec!["d.x", "odc1"], "accept:1"),
        (vec!["odc1", "odc2"], "accept:1"),
        (vec!["odc2"], "too-new:1"),
        (vec!["odc01"], "none"),
        (vec!["ODC1"], "none"),
        (vec![], "none"),
    ]
    .into_iter()
    .map(|(offered, want)| {
        let got = match proto::negotiate(offered.iter().copied()) {
            Negotiated::Accept(v) => format!("accept:{v}"),
            Negotiated::TooOld { min } => format!("too-old:{min}"),
            Negotiated::TooNew { max } => format!("too-new:{max}"),
            Negotiated::NoVersion => "none".into(),
        };
        assert_eq!(got, want, "{offered:?}");
        json!({"offered": offered, "result": want})
    })
    .collect();

    let commands: Vec<Value> = cases(&w)
        .iter()
        .map(|c| {
            assert_eq!(run_case(&w, c), c.expect, "panel: {}", c.name);
            assert_eq!(server_case(&w, c), c.server, "server: {}", c.name);
            let body = c.frame.get(73..).unwrap_or(&[]);
            json!({
                "name": c.name,
                "note": c.note,
                "frame": hex(&c.frame),
                "body": core::str::from_utf8(body).ok(),
                "before": c.before.iter().map(|f| hex(f)).collect::<Vec<_>>(),
                "expect": c.expect,
                "server": c.server,
            })
        })
        .collect();

    let forged_dev = {
        let mut d = w.dev;
        d.0[15] ^= 1;
        d
    };
    let attach_sig = proto::attach_signature(&w.a, &w.dev, &nonce);
    assert!(proto::verify_attach(&a_pub, &w.dev, &nonce, &attach_sig));
    let stranger_sig = proto::attach_signature(&w.stranger, &w.dev, &nonce);

    json!({
        "about": "OpenDMD Connect protocol v1 shared test vectors (docs/connect-protocol.md). Every key, secret and id here is a throwaway made from a fixed string: never use one for anything real. Made by opendmd-core/tests/connect_vectors.rs; check, do not edit by hand.",
        "version": proto::VERSION,
        "min_version": proto::MIN_VERSION,
        "limits": {
            "max_text": proto::MAX_TEXT,
            "max_body": proto::MAX_BODY,
            "command_head": proto::COMMAND_HEAD,
            "max_chunk": opendmd_core::connect::xfer::MAX_CHUNK,
            "max_file": opendmd_core::connect::xfer::MAX_FILE,
            "max_keys": opendmd_core::connect::keys::MAX_KEYS,
            "max_label_bytes": opendmd_core::connect::keys::MAX_LABEL,
            "max_n": proto::MAX_N,
            "rate_per_key": proto::RATE_PER_KEY,
            "rate_per_panel": proto::RATE_PER_PANEL,
        },
        "domains": {
            "cmd": String::from_utf8_lossy(proto::CMD_DOMAIN),
            "attach": String::from_utf8_lossy(proto::ATTACH_DOMAIN),
            "dev_tag": "opendmd-connect-v1 dev\u{0}",
            "pair_id": "opendmd pair id v1",
            "pair_mac": "opendmd pair v1",
            "paired_mac": "ok",
            "invite_id": "opendmd invite id v1",
            "add_mac": "opendmd add v1",
            "confirm": "opendmd confirm v1",
        },
        "allowed": proto::ALLOWED,
        "never": proto::NEVER,
        "codec": {
            "base32": [
                {"hex": hex(b"foobar"), "text": base32(b"foobar")},
                {"hex": hex(&w.dev.0), "text": w.dev.text()},
            ],
            "base32_refused": ["MZXW6YTBOI", "mzxw6ytboj", "mzxw6ytbo1"],
            "hmac_sha256": [{"key": hex(b"Jefe"), "msg": hex(b"what do ya want for nothing?"), "mac": hex(&hmac_sha256(b"Jefe", &[b"what do ya want for nothing?"]))}],
        },
        "panel": {
            "id_key": hex(&w.id_key),
            "dev_random": hex(&w.dev_random),
            "dev": w.dev.text(),
            "dev_hex": hex(&w.dev.0),
            "dev_subprotocol": w.dev.subprotocol(),
            "forged_dev": forged_dev.text(),
            "sess": hex(&w.sess),
            "secret": hex(&secret),
            "register_body": proto::register_body(&secret),
            "max_brightness": w.limits.max_brightness,
            "key_blob": hex(&w.keys.encode()),
            "keys_message": w.keys.keys_message(),
        },
        "controllers": [controller("A", &w.a, &w.keys), controller("B", &w.b, &w.keys), controller("stranger", &w.stranger, &w.keys), controller("C", &c_key, &w.keys)],
        "negotiation": negotiation,
        "attach": {
            "nonce": hex(&nonce),
            "dev": w.dev.text(),
            "by_a": hex(&attach_sig),
            "by_stranger": hex(&stranger_sig),
            "challenge": format!(r#"{{"t":"challenge","nonce":"{}"}}"#, hex(&nonce)),
            "message": format!(r#"{{"t":"attach","k":"{}","sig":"{}"}}"#, hex(&key_id(&a_pub)), hex(&attach_sig)),
        },
        "pairing": {
            "random": hex(&s_random),
            "secret": s,
            "pair_id": hex(&pair::pair_id(&s)),
            "pub": hex(&c_pub),
            "mac": hex(&pair::pair_mac(&s, &w.dev.0, &c_pub)),
            "mac2": hex(&pair::paired_mac(&s, &c_pub)),
            "mac_for_a_swapped_key": hex(&pair::pair_mac(&s, &w.dev.0, &a_pub)),
            "link": format!("https://opendmd.dev/connect/#pair={s}"),
            "qr": format!("https://opendmd.dev/c#{s}"),
        },
        "invite": {
            "random": hex(&i_random),
            "secret": inv,
            "invite_id": hex(&pair::invite_id(&inv)),
            "pub": hex(&c_pub),
            "mac": hex(&pair::add_mac(&inv, &w.dev.0, &c_pub)),
            "confirm": pair::confirm_code(&c_pub),
            "link": format!("https://opendmd.dev/c#add={}.{}", w.dev.text(), inv),
        },
        "commands": commands,
        "acks": {
            "ok": proto::ack_ok(&key_id(&a_pub), 1, None),
            "ok_with_result": proto::ack_ok(&key_id(&a_pub), 2, Some("{\"paused\":true}")),
            "err": proto::ack_err(&key_id(&a_pub), Some(3), Refusal::Replay),
            "err_without_n": proto::ack_err(&key_id(&a_pub), None, Refusal::BadSig),
        },
        "signature_check": {
            "note": "Ed25519 (RFC 8032) over the cmd domain then the body; a sanity line for a new implementation",
            "seed": hex(&w.a.to_bytes()),
            "msg": hex(&[proto::CMD_DOMAIN, b"{}"].concat()),
            "sig": hex(&w.a.sign(&[proto::CMD_DOMAIN, b"{}"].concat()).to_bytes()),
        },
    })
}

#[test]
fn vectors_are_pinned() {
    let got = serde_json::to_string_pretty(&build()).unwrap() + "\n";
    if std::env::var("CONNECT_VECTORS").as_deref() == Ok("write") {
        std::fs::write(PATH, &got).unwrap();
        return;
    }
    let pinned = std::fs::read_to_string(PATH).expect("tests/connect-vectors.json (CONNECT_VECTORS=write makes it)");
    assert!(pinned == got, "the core no longer makes tests/connect-vectors.json: a protocol change? (CONNECT_VECTORS=write, then review the diff)");
}

/// The pinned file read back as another implementation would: every case
/// through a fresh session.
#[test]
fn the_pinned_cases_hold() {
    let w = world();
    let pinned: Value = serde_json::from_str(&std::fs::read_to_string(PATH).unwrap()).unwrap();
    let unhex = |s: &str| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect::<Vec<u8>>();
    let cmds = pinned["commands"].as_array().unwrap();
    assert!(cmds.len() >= 60, "{}", cmds.len());
    for c in cmds {
        let mut s = Session::new(w.dev, w.sess, 1, Duration::ZERO);
        for b in c["before"].as_array().unwrap() {
            assert!(s.check(&w.keys, &w.limits, &unhex(b.as_str().unwrap()), Duration::ZERO).is_ok());
        }
        let got = match s.check(&w.keys, &w.limits, &unhex(c["frame"].as_str().unwrap()), Duration::ZERO) {
            Ok(_) => "ok",
            Err(r) => r.err.code(),
        };
        assert_eq!(got, c["expect"].as_str().unwrap(), "{}", c["name"]);
    }
    assert_eq!(KeyList::decode(&unhex(pinned["panel"]["key_blob"].as_str().unwrap())).unwrap(), w.keys);
}
