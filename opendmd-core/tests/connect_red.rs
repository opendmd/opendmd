//! OpenDMD Connect v1, red list round 1 (P1-PLAN 8.7): the cases that need
//! state across frames. The single-frame cases are the shared vectors
//! (`connect_vectors.rs`); the transfer's and pairing window's own rules
//! are also unit tests in `src/connect/`. Every case here must be refused.
#![cfg(feature = "connect")]

use core::time::Duration;
use ed25519_dalek::SigningKey;
use opendmd_core::connect::codec::sha256;
use opendmd_core::connect::keys::{key_id, Entry, KeyList, FLAG_CAN_ADD};
use opendmd_core::connect::pair::{self, PairError, Window, PAIR_TTL};
use opendmd_core::connect::proto::{self, Command, Dev, Limits, Refusal, Session, TextError};
use opendmd_core::connect::xfer::{self, chunk_frame, parse_chunk, Quota, Transfer, XferError};

fn key(name: &str) -> SigningKey {
    SigningKey::from_bytes(&sha256(&[b"red list throwaway ", name.as_bytes()]))
}

struct Panel {
    dev: Dev,
    sess: [u8; 12],
    keys: KeyList,
    limits: Limits,
    a: SigningKey,
    b: SigningKey,
}

fn panel() -> Panel {
    let a = key("a");
    let b = key("b");
    let mut keys = KeyList::new();
    keys.add(Entry { public: a.verifying_key().to_bytes(), flags: FLAG_CAN_ADD, label: "a".into(), added: 0, added_by: None }).unwrap();
    keys.add(Entry { public: b.verifying_key().to_bytes(), flags: 0, label: "b".into(), added: 0, added_by: Some([1; 8]) }).unwrap();
    Panel { dev: Dev::mint(&[3; 10], b"red id key"), sess: [5; 12], keys, limits: Limits { max_brightness: 100 }, a, b }
}

impl Panel {
    fn frame(&self, k: &SigningKey, sess: &[u8; 12], n: u64, c: &str, args: &str) -> Vec<u8> {
        proto::sign_command(k, proto::command_body(&self.dev, sess, n, c, args).as_bytes())
    }
    fn session(&self, sess: [u8; 12]) -> Session {
        Session::new(self.dev, sess, proto::VERSION, Duration::ZERO)
    }
    fn check(&self, s: &mut Session, f: &[u8], t_ms: u64) -> Result<Command, Refusal> {
        s.check(&self.keys, &self.limits, f, Duration::from_millis(t_ms)).map(|a| a.cmd).map_err(|r| r.err)
    }
}

#[test]
fn a_flood_of_100_commands_a_second_is_rate_limited() {
    let p = panel();
    let mut s = p.session(p.sess);
    let mut ok = 0;
    let mut rate = 0;
    for n in 1..=100u64 {
        match p.check(&mut s, &p.frame(&p.a, &p.sess, n, "next", ""), n * 10) {
            Ok(_) => ok += 1,
            Err(Refusal::Rate) => rate += 1,
            Err(e) => panic!("{e:?}"),
        }
    }
    // A burst of 4, then 4 a second: 4 + 4 over the second.
    assert!((4..=9).contains(&ok), "{ok} accepted");
    assert_eq!(ok + rate, 100);
}

#[test]
fn two_keys_together_meet_the_panel_rate() {
    let p = panel();
    let mut s = p.session(p.sess);
    let mut ok = 0;
    for n in 1..=50u64 {
        for k in [&p.a, &p.b] {
            if p.check(&mut s, &p.frame(k, &p.sess, n, "next", ""), n * 20).is_ok() {
                ok += 1;
            }
        }
    }
    // Two keys at 4 a second each, under the panel's 8: about 8 + 8.
    assert!(ok <= 20, "{ok}");
}

#[test]
fn forged_frames_cost_no_signature_checks_past_the_panel_rate() {
    // A stream of frames under a known key id with bad signatures: after
    // the panel's burst they are refused for rate before any check.
    let p = panel();
    let mut s = p.session(p.sess);
    let mut f = p.frame(&p.a, &p.sess, 1, "next", "");
    f[20] ^= 1;
    let errs: Vec<_> = (0..20).map(|_| p.check(&mut s, &f, 0).unwrap_err()).collect();
    assert_eq!(errs.iter().filter(|e| **e == Refusal::BadSig).count(), proto::RATE_PER_PANEL as usize);
    assert!(errs[proto::RATE_PER_PANEL as usize..].iter().all(|e| *e == Refusal::Rate));
}

#[test]
fn a_reconnect_voids_everything_signed_before_it() {
    let p = panel();
    let f = p.frame(&p.a, &p.sess, 1, "next", "");
    let mut old = p.session(p.sess);
    assert!(p.check(&mut old, &f, 0).is_ok());
    let mut new = p.session([6; 12]);
    assert_eq!(p.check(&mut new, &f, 0), Err(Refusal::StaleSession));
    // Signed again for the new session, it goes.
    assert!(p.check(&mut new, &p.frame(&p.a, &[6; 12], 1, "next", ""), 0).is_ok());
}

#[test]
fn a_removed_key_is_refused_and_its_old_commands_stay_dead() {
    let mut p = panel();
    let mut s = p.session(p.sess);
    let f5 = p.frame(&p.b, &p.sess, 5, "next", "");
    assert!(p.check(&mut s, &f5, 0).is_ok());
    let kb = key_id(&p.b.verifying_key().to_bytes());
    p.keys.remove(&kb).unwrap();
    assert_eq!(p.check(&mut s, &p.frame(&p.b, &p.sess, 6, "next", ""), 0), Err(Refusal::UnknownKey));
    // Paired again in the same session: the counter was kept, so a replay
    // of its old command is still a replay.
    p.keys.add(Entry { public: p.b.verifying_key().to_bytes(), flags: 0, label: "b".into(), added: 0, added_by: None }).unwrap();
    assert_eq!(p.check(&mut s, &f5, 1000), Err(Refusal::Replay));
    assert!(p.check(&mut s, &p.frame(&p.b, &p.sess, 7, "next", ""), 2000).is_ok());
}

#[test]
fn counters_are_bounded() {
    // More keys over one session than MAX_COUNTERS: busy, never a counter
    // forgotten.
    let p = panel();
    let mut s = p.session(p.sess);
    let mut keys = KeyList::new();
    let mut refused = 0;
    for i in 0..(proto::MAX_COUNTERS + 2) {
        let k = key(&format!("k{i}"));
        if keys.len() == 8 {
            let first = keys.entries()[0].id();
            keys.remove(&first).unwrap();
        }
        keys.add(Entry { public: k.verifying_key().to_bytes(), flags: 0, label: "k".into(), added: 0, added_by: None }).unwrap();
        let f = p.frame(&k, &p.sess, 1, "next", "");
        match s.check(&keys, &p.limits, &f, Duration::from_secs(i as u64)) {
            Ok(_) => {}
            Err(r) => {
                assert_eq!(r.err, Refusal::Busy);
                refused += 1;
            }
        }
    }
    assert_eq!(refused, 2);
}

#[test]
fn a_one_level_key_cannot_add_and_a_new_key_gets_no_flag() {
    let p = panel();
    let mut s = p.session(p.sess);
    let c = key("c").verifying_key().to_bytes();
    let args = format!(r#""pub":"{}","label":"c""#, opendmd_core::connect::codec::hex(&c));
    assert_eq!(p.check(&mut s, &p.frame(&p.b, &p.sess, 1, "add-controller", &args), 0), Err(Refusal::NotAllowed));
    match p.check(&mut s, &p.frame(&p.a, &p.sess, 1, "add-controller", &args), 0) {
        Ok(Command::AddController { public, label }) => {
            assert_eq!(public, c);
            assert_eq!(label, "c");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_full_list_refuses_another_key() {
    let mut p = panel();
    for i in 0..6 {
        p.keys.add(Entry { public: key(&format!("f{i}")).verifying_key().to_bytes(), flags: 0, label: "f".into(), added: 0, added_by: None }).unwrap();
    }
    let mut s = p.session(p.sess);
    let c = key("c").verifying_key().to_bytes();
    let args = format!(r#""pub":"{}","label":"c""#, opendmd_core::connect::codec::hex(&c));
    assert_eq!(p.check(&mut s, &p.frame(&p.a, &p.sess, 1, "add-controller", &args), 0), Err(Refusal::Quota));
}

#[test]
fn oversize_text_is_refused() {
    let big = format!(r#"{{"t":"watch","on":true,"pad":"{}"}}"#, "x".repeat(proto::MAX_TEXT_IN));
    assert_eq!(proto::parse_server_text(big.as_bytes()), Err(TextError::TooBig));
    assert_eq!(proto::parse_server_text(br#"{"t":"watch","on":true,"on":false}"#), Err(TextError::Malformed));
}

// --- Pairing -------------------------------------------------------------------

const LABEL: &[u8] = b"guest phone";

fn pairing() -> (String, [u8; 16], [u8; 32]) {
    (pair::secret_from_random(&[0x42; 13]), panel().dev.0, key("guest").verifying_key().to_bytes())
}

#[test]
fn pairing_with_a_wrong_mac_three_times_closes_the_window() {
    let (s, dev, public) = pairing();
    let t = Duration::from_secs(10);
    let mut w = Window::open(s.clone(), t);
    for left in [2, 1, 0] {
        assert_eq!(w.check(&dev, &public, LABEL, &[0; 32], t), Err(PairError::BadMac { left }));
    }
    assert_eq!(w.check(&dev, &public, LABEL, &pair::pair_mac(&s, &dev, &public, LABEL), t), Err(PairError::Closed));
}

#[test]
fn pairing_after_expiry_and_twice_is_refused() {
    let (s, dev, public) = pairing();
    let t = Duration::from_secs(10);
    let mac = pair::pair_mac(&s, &dev, &public, LABEL);
    let mut late = Window::open(s.clone(), t);
    assert_eq!(late.check(&dev, &public, LABEL, &mac, t + PAIR_TTL), Err(PairError::Expired));
    let mut twice = Window::open(s, t);
    assert!(twice.check(&dev, &public, LABEL, &mac, t).is_ok());
    assert_eq!(twice.check(&dev, &public, LABEL, &mac, t), Err(PairError::Closed));
}

#[test]
fn a_key_swapped_by_the_server_cannot_pair_or_fake_the_answer() {
    let (s, dev, public) = pairing();
    let t = Duration::from_secs(10);
    let theirs = key("server").verifying_key().to_bytes();
    let mut w = Window::open(s.clone(), t);
    // The guest's MAC with the server's key in its place.
    assert!(matches!(w.check(&dev, &theirs, LABEL, &pair::pair_mac(&s, &dev, &public, LABEL), t), Err(PairError::BadMac { .. })));
    // And the panel's answer for the server's key is not the guest's.
    assert_ne!(pair::paired_mac(&s, &dev, &theirs), pair::paired_mac(&s, &dev, &public));
}

// --- The server's checks ---------------------------------------------------------

#[test]
fn a_strangers_attach_and_a_made_up_id_are_refused() {
    let p = panel();
    let nonce = [9u8; 16];
    let stranger = key("stranger");
    let sig = proto::attach_signature(&stranger, &p.dev, &nonce);
    // Its own signature checks against its own key, but its key is on no
    // list: the server looks the key id up first.
    assert!(p.keys.find(&key_id(&stranger.verifying_key().to_bytes())).is_none());
    // Under a paired key's id, the signature fails.
    assert!(!proto::verify_attach(&p.a.verifying_key().to_bytes(), &p.dev, &nonce, &sig));
    // A replayed attach for another challenge fails.
    let good = proto::attach_signature(&p.a, &p.dev, &nonce);
    assert!(!proto::verify_attach(&p.a.verifying_key().to_bytes(), &p.dev, &[8; 16], &good));
    // A made-up id fails its tag.
    let mut made_up = p.dev;
    made_up.0[12] ^= 0x40;
    assert!(!made_up.tag_ok(b"red id key"));
    assert!(p.dev.tag_ok(b"red id key"));
}

#[test]
fn the_server_precheck_matches_the_panel_on_keys_and_signatures() {
    let p = panel();
    let keys: Vec<_> = p.keys.entries().iter().map(|e| (e.id(), e.public)).collect();
    let good = p.frame(&p.a, &p.sess, 1, "next", "");
    assert!(proto::precheck(&keys, &good).is_ok());
    let mut bad = good.clone();
    bad[40] ^= 1;
    assert_eq!(proto::precheck(&keys, &bad), Err(Refusal::BadSig));
    assert_eq!(proto::precheck(&keys, &p.frame(&key("x"), &p.sess, 1, "next", "")), Err(Refusal::UnknownKey));
}

// --- Transfers -----------------------------------------------------------------

fn send(bytes: &[u8]) -> proto::SendArgs {
    proto::SendArgs { id: 9, name: "x.gif".into(), size: bytes.len() as u32, sha256: sha256(&[bytes]), copy_size: 0, copy_sha256: None, play: false }
}

#[test]
fn chunks_outside_the_credit_and_tampered_chunks_are_refused() {
    let gif = vec![7u8; 6000];
    let mut t = Transfer::start(send(&gif), 2, Duration::ZERO);
    // Past the credit (2 chunks) before any write returned.
    let far = chunk_frame(9, 2048, &gif[2048..3072]);
    assert_eq!(t.chunk(&parse_chunk(&far).unwrap(), Duration::ZERO), Err(XferError::OutsideCredit));
    // A tampered stream completes but fails its hash.
    let mut bad = gif.clone();
    bad[5000] = 0;
    while t.offset() < t.total() {
        let at = t.offset() as usize;
        let n = (t.upto() as usize - at).min(xfer::MAX_CHUNK);
        t.chunk(&parse_chunk(&chunk_frame(9, at as u32, &bad[at..at + n])).unwrap(), Duration::ZERO).unwrap();
        t.written();
    }
    assert_eq!(t.finish(), Err(XferError::Tampered));
}

#[test]
fn a_send_over_quota_is_refused() {
    let mut q = Quota::default();
    let now = Duration::from_secs(100);
    let per = u64::from(xfer::MAX_FILE) * 2;
    let mut n = 0;
    while q.charge(per, now) {
        n += 1;
    }
    assert_eq!(n, xfer::QUOTA_BYTES / per);
}

#[test]
fn a_label_rewritten_by_the_server_is_refused() {
    let (s, dev, public) = pairing();
    let t = Duration::from_secs(10);
    // A relayed `pair` whose label the server changed: the MAC is over the
    // label as the guest sent it.
    let mut w = Window::open(s.clone(), t);
    let mac = pair::pair_mac(&s, &dev, &public, LABEL);
    assert!(matches!(w.check(&dev, &public, b"Owner (admin)", &mac, t), Err(PairError::BadMac { .. })));
    assert!(w.check(&dev, &public, LABEL, &mac, t).is_ok());
    // A relayed `add-req` likewise.
    let inv = pair::invite_secret_from_random(&[0x24; 16]);
    let mut w = Window::open_invite(inv.clone(), t);
    let mac = pair::add_mac(&inv, &dev, &public, LABEL);
    assert!(matches!(w.check(&dev, &public, b"guest phone\xe2\x80\x8b", &mac, t), Err(PairError::BadMac { .. })));
    assert!(w.check(&dev, &public, LABEL, &mac, t).is_ok());
}

// --- Transfers on the panel's slot, from the pinned vectors ----------------------

/// The pinned vectors' panel (its list, id and session) and a frame by name.
fn pinned() -> (serde_json::Value, KeyList, Dev, [u8; 12]) {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/connect-vectors.json")).unwrap()).unwrap();
    let unhex = |s: &serde_json::Value| -> Vec<u8> { (0..s.as_str().unwrap().len()).step_by(2).map(|i| u8::from_str_radix(&s.as_str().unwrap()[i..i + 2], 16).unwrap()).collect() };
    let p = &v["panel"];
    let keys = KeyList::decode(&unhex(&p["key_blob"])).unwrap();
    let dev = Dev(unhex(&p["dev_hex"]).try_into().unwrap());
    let sess: [u8; 12] = unhex(&p["sess"]).try_into().unwrap();
    (v, keys, dev, sess)
}

fn pinned_frame(v: &serde_json::Value, name: &str) -> Vec<u8> {
    let c = v["commands"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("no vector {name}"));
    let s = c["frame"].as_str().unwrap();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn the_pinned_send_charges_once_and_resumes_only_for_its_key() {
    let (v, keys, dev, sess) = pinned();
    let lim = v["limits"].clone();
    assert_eq!(lim["max_chunk"], xfer::MAX_CHUNK);
    assert_eq!(lim["max_file"], xfer::MAX_FILE);
    let mut s = Session::new(dev, sess, proto::VERSION, Duration::ZERO);
    let acc = s.check(&keys, &Limits { max_brightness: 128 }, &pinned_frame(&v, "ok-send"), Duration::ZERO).unwrap();
    let Command::Send(args) = acc.cmd else { panic!("not a send") };
    let mut slot = xfer::Slot::new(2);
    let now = Duration::from_secs(50);
    let (plan, _) = slot.check(&acc.k, &args, now).unwrap();
    assert_eq!(plan, xfer::Plan::Start { replaces: false });
    slot.start(acc.k, args.clone(), now).unwrap();
    let bytes = u64::from(args.size) + u64::from(args.copy_size);
    assert_eq!(slot.quota.used(now), bytes, "size + copy_size, when accepted");
    // The panel's first credit: its window, never more.
    let h = slot.held.as_ref().unwrap();
    assert_eq!(h.t.upto(), 2 * xfer::MAX_CHUNK as u32);
    // Signed again (a reconnect): a resume, nothing charged again.
    assert_eq!(slot.check(&acc.k, &args, now + Duration::from_secs(60)).unwrap().0, xfer::Plan::Resume);
    assert!(slot.resume(&args, now + Duration::from_secs(60)));
    assert_eq!(slot.quota.used(now), bytes);
    // Another paired key naming the same transfer: busy, nothing charged.
    let other = keys.entries().iter().map(|e| e.id()).find(|k| *k != acc.k).unwrap();
    assert_eq!(slot.check(&other, &args, now).unwrap_err(), Refusal::Busy);
    assert_eq!(slot.quota.used(now), bytes);
    // Abandoned (a cancel): no refund.
    assert!(slot.cancel(&acc.k, args.id).unwrap().is_some());
    assert_eq!(slot.quota.used(now), bytes);
}

#[test]
fn a_whole_stream_through_the_slot_with_a_tampered_copy() {
    let gif: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
    let copy: Vec<u8> = (0..1700u32).map(|i| (i % 13) as u8).collect();
    let args = proto::SendArgs { id: 5, name: "s.gif".into(), size: 3000, sha256: sha256(&[&gif]), copy_size: 1700, copy_sha256: Some(sha256(&[&copy])), play: true };
    for tamper in [None, Some(3500usize)] {
        let mut slot = xfer::Slot::new(4);
        slot.start([1; 8], args.clone(), Duration::ZERO).unwrap();
        let mut all = [gif.as_slice(), copy.as_slice()].concat();
        if let Some(at) = tamper {
            all[at] ^= 0x40;
        }
        let t = &mut slot.held.as_mut().unwrap().t;
        while !t.complete() {
            let at = t.received() as usize;
            let n = (t.upto() as usize - at).min(xfer::MAX_CHUNK);
            t.accept(&parse_chunk(&chunk_frame(5, at as u32, &all[at..at + n])).unwrap(), Duration::ZERO).unwrap();
            t.commit(&all[at..at + n]);
            t.grant_more();
            assert!(t.upto() - t.offset() <= 4 * xfer::MAX_CHUNK as u32);
        }
        assert_eq!(t.finish(), if tamper.is_some() { Err(XferError::Tampered) } else { Ok(()) });
    }
}
