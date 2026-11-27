//! A Formula added by a remote controller (`formula-add`, `formula-part`;
//! protocol 5.7): the panel's side, without the card. The arguments' rules,
//! one add at a time with its two SHA-256s and its next offset, resumes,
//! the day's tally (kept on the card by the firmware), the add's own rate.
//! The firmware writes the bytes, checks the compiled program
//! (`psb::verify`) and lands the files (`firmware/src/connect/fadd.rs`).
//!
//! This is the one place a remote controller hands the panel a program
//! (every other path refuses inline Formulas and `~sent`): it lands as
//! `<stem>.psr`, never `.psb`, and whatever is read from a `.psr` is drawn
//! by the VM only (`fx_psi::card_program`), until the panel's own compiler
//! has written `<stem>.psb` from the source and removed the `.psr`.

use std::time::Duration;

use sha2::{Digest, Sha256};

use super::json::Value;
use super::keys::KeyId;
use super::proto::Refusal;

/// A stem an add may name: the web app's save rule (`[a-z0-9-]`, 1 to 40,
/// a letter or digit first), inside the remote play's.
pub const MAX_STEM: usize = 40;
/// The source's size, at most (the largest gallery Formula is 10.8 KB).
pub const MAX_SOURCE: u32 = 16 * 1024;
/// The compiled file's size, at least (its header) and at most (the
/// largest gallery Formula's is 4.4 KB).
pub const MIN_PSB: u32 = 6;
pub const MAX_PSB: u32 = 8 * 1024;
/// One part's bytes, at most: its 512 base64url characters keep the
/// largest `formula-part` body within the 700 bytes a command may be.
pub const MAX_PART: usize = 384;
/// The name the page shows, at most (characters).
pub const MAX_NAME: usize = 60;
/// Adds a day may start, and their bytes ([`Tally`]).
pub const QUOTA_ADDS: u32 = 16;
pub const QUOTA_BYTES: u64 = 256 * 1024;
/// Formulas the card may hold for an add to be taken (sources counted in the
/// add's own directory pass).
pub const MAX_FORMULAS: usize = 128;
/// One `formula-add` a panel takes in this long (its own rate: a refused or
/// over-quota key cannot make the panel read the card four times a second).
pub const ADD_EVERY: Duration = Duration::from_secs(2);
/// An add with no progress for this long is dropped (its part files too).
pub const STALE: Duration = Duration::from_secs(600);

/// `formula-add`'s arguments, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddArgs {
    pub id: u32,
    pub stem: String,
    pub name: String,
    pub src_size: u32,
    pub src_sha256: [u8; 32],
    pub psb_size: u32,
    pub psb_sha256: [u8; 32],
}

impl AddArgs {
    pub fn total(&self) -> u32 {
        self.src_size + self.psb_size
    }
}

/// The stem rule for an add.
pub fn stem_ok(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= MAX_STEM && b[0].is_ascii_alphanumeric() && b.iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// `stem`, `stem-2` … `stem-9`: where a different source under a taken
/// stem lands (none past 9: `quota`).
pub fn stem_choices(stem: &str) -> impl Iterator<Item = String> + '_ {
    std::iter::once(stem.to_string()).chain((2..=9).map(move |i| format!("{}-{i}", &stem[..stem.len().min(MAX_STEM - 2)])))
}

/// Strict base64url (RFC 4648 section 5), no padding, canonical (the last
/// character's unused bits zero): one spelling per value.
pub fn unb64url(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32)
    };
    let b = s.as_bytes();
    if b.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(b.len() * 3 / 4);
    for ch in b.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in ch.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        let bytes = ch.len() - 1;
        // The unused low bits of a short group must be zero.
        let unused = 24 - 8 * bytes;
        if n & ((1 << unused) - 1) != 0 && ch.len() < 4 {
            return None;
        }
        for i in 0..bytes {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

/// base64url, no padding (the controller's side, and the tests').
pub fn b64url(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for ch in b.chunks(3) {
        let n = ch.iter().enumerate().fold(0u32, |n, (i, &x)| n | (u32::from(x) << (16 - 8 * i)));
        for i in 0..=ch.len() {
            s.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    s
}

/// SHA-256 of what `r` reads, in 512-byte pieces (a card file's).
pub fn sha256_of(mut r: impl std::io::Read) -> std::io::Result<[u8; 32]> {
    let mut h = Sha256::new();
    let mut b = [0u8; 512];
    loop {
        match r.read(&mut b)? {
            0 => return Ok(h.finalize().into()),
            n => h.update(&b[..n]),
        }
    }
}

fn arg<'a>(a: &'a Value, k: &str) -> Option<&'a Value> {
    a.get(k)
}

/// `formula-add`'s arguments.
pub fn add_args(a: &Value) -> Result<AddArgs, Refusal> {
    let bad = Refusal::BadArg;
    let int = |k: &str| arg(a, k).and_then(Value::as_int).ok_or(bad);
    let id = u32::try_from(int("id")?).ok().filter(|&i| i != 0).ok_or(bad)?;
    let stem = arg(a, "stem").and_then(Value::as_str).filter(|s| stem_ok(s)).ok_or(bad)?.to_string();
    let name = arg(a, "name").and_then(Value::as_str).ok_or(bad)?;
    let name = super::keys::clean_text(name, 4 * MAX_NAME, MAX_NAME).ok_or(bad)?;
    let (src_size, psb_size) = (int("src_size")?, int("psb_size")?);
    if !(1..=i64::from(MAX_SOURCE)).contains(&src_size) || !(i64::from(MIN_PSB)..=i64::from(MAX_PSB)).contains(&psb_size) {
        return Err(if src_size > i64::from(MAX_SOURCE) || psb_size > i64::from(MAX_PSB) { Refusal::TooBig } else { bad });
    }
    let hash = |k: &str| arg(a, k).and_then(Value::as_str).and_then(super::codec::unhex::<32>).ok_or(bad);
    Ok(AddArgs { id, stem, name, src_size: src_size as u32, src_sha256: hash("src_sha256")?, psb_size: psb_size as u32, psb_sha256: hash("psb_sha256")? })
}

/// `formula-part`'s arguments: `(id, off, data)`.
pub fn part_args(a: &Value) -> Result<(u32, u32, Vec<u8>), Refusal> {
    let bad = Refusal::BadArg;
    let id = arg(a, "id").and_then(Value::as_int).and_then(|i| u32::try_from(i).ok()).filter(|&i| i != 0).ok_or(bad)?;
    let off = arg(a, "off").and_then(Value::as_int).and_then(|i| u32::try_from(i).ok()).ok_or(bad)?;
    let data = arg(a, "data").and_then(Value::as_str).ok_or(bad)?;
    if data.len() > MAX_PART.div_ceil(3) * 4 {
        return Err(Refusal::TooBig);
    }
    let data = unb64url(data).filter(|d| !d.is_empty() && d.len() <= MAX_PART).ok_or(bad)?;
    Ok((id, off, data))
}

/// Which file a run of the stream's bytes goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Source,
    Psb,
}

/// The add under way.
#[derive(Debug, Clone)]
pub struct Held {
    pub k: KeyId,
    pub args: AddArgs,
    /// The stem it lands under (the args' or a next free one), as the
    /// firmware chose at the start.
    pub target: String,
    pub next: u32,
    src: Sha256,
    psb: Sha256,
    pub at: Duration,
    /// Landed (its `done` may be asked again within [`STALE`]).
    pub landed: bool,
}

/// What a `formula-add` starts.
#[derive(Debug, PartialEq, Eq)]
pub enum Start {
    /// New: the stream from 0. (`dropped`: an earlier stalled add of this
    /// key, abandoned: its part files to remove.)
    Fresh { dropped: Option<u32> },
    /// The same add signed again: carry on from `next` (`next == total`:
    /// all the bytes are in, the checks are to be tried again).
    Resume { next: u32 },
    /// Already landed: its `done` again.
    Landed { stem: String },
}

/// What a part did: its bytes go to `(Part, offset in that file)` runs;
/// `complete` once the stream is whole and both hashes match.
#[derive(Debug, PartialEq, Eq)]
pub struct Progress {
    pub runs: Vec<(Part, u32, std::ops::Range<usize>)>,
    pub next: u32,
    pub complete: bool,
}

/// The day's adds: kept on the card (the firmware's `/formulas/.add-day`)
/// so a restart does not reset it. `day`: the calendar day (Unix days) of
/// the panel's clock when it had one, else `None`. With no clock the tally
/// never resets: adds stop at the quota until a clock shows a new day
/// (Connect itself needs the network, where the clock comes from).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub day: Option<u64>,
    pub adds: u32,
    pub bytes: u64,
}

impl Tally {
    /// Its line on the card: `<day or -> <adds> <bytes>`.
    pub fn text(&self) -> String {
        format!("{} {} {}\n", self.day.map_or("-".to_string(), |d| d.to_string()), self.adds, self.bytes)
    }

    /// Read back; anything it cannot read is a full day (fails closed: no
    /// more adds until a clock shows a new day).
    pub fn parse(t: &str) -> Tally {
        let full = Tally { day: None, adds: QUOTA_ADDS, bytes: QUOTA_BYTES };
        let mut w = t.split_whitespace();
        let (Some(d), Some(a), Some(b), None) = (w.next(), w.next(), w.next(), w.next()) else {
            return if t.trim().is_empty() { Tally::default() } else { full };
        };
        let day = if d == "-" { None } else { match d.parse() { Ok(d) => Some(d), Err(_) => return full } };
        match (a.parse(), b.parse()) {
            (Ok(adds), Ok(bytes)) => Tally { day, adds, bytes },
            _ => full,
        }
    }

    /// As of `today` (`None`: no clock): a new day starts it again.
    fn roll(&mut self, today: Option<u64>) {
        if let Some(d) = today {
            if self.day != Some(d) {
                *self = Tally { day: Some(d), adds: 0, bytes: 0 };
            }
        }
    }

    fn room(&self, bytes: u64) -> bool {
        self.adds < QUOTA_ADDS && self.bytes + bytes <= QUOTA_BYTES
    }
}

/// What a `formula-add` may do, before the card is read.
#[derive(Debug, PartialEq, Eq)]
pub enum Pre {
    /// The same add signed again: carry on from `next` (`next == total`:
    /// all the bytes are in, the checks are to be tried again).
    Resume { next: u32 },
    /// Already landed: its `done` again.
    Landed { stem: String },
    /// A new add: the card may be read for its target.
    New,
}

/// The panel's adds: one held at a time, the day's tally, the add's rate.
#[derive(Debug, Default)]
pub struct Adds {
    pub held: Option<Held>,
    pub tally: Tally,
    last_add: Option<Duration>,
}

impl Adds {
    /// The held add, if it is not stale at `now` (else it is dropped and
    /// its id returned: its part files to remove now).
    pub fn expire(&mut self, now: Duration) -> Option<u32> {
        if self.held.as_ref().is_some_and(|h| now.saturating_sub(h.at) >= STALE) {
            return self.held.take().map(|h| h.args.id);
        }
        None
    }

    /// Before anything is read from the card: the add's own rate, the same
    /// add again, another key's add under way, the day's tally.
    pub fn pre(&mut self, k: KeyId, args: &AddArgs, now: Duration, today: Option<u64>) -> Result<Pre, Refusal> {
        if let Some(h) = &mut self.held {
            if h.k == k && &h.args == args && now.saturating_sub(h.at) < STALE {
                h.at = now;
                return Ok(if h.landed { Pre::Landed { stem: h.target.clone() } } else { Pre::Resume { next: h.next } });
            }
        }
        if self.last_add.is_some_and(|t| now.saturating_sub(t) < ADD_EVERY) {
            return Err(Refusal::Rate);
        }
        self.last_add = Some(now);
        // (A landed add holds nothing; a stalled one of another key does
        // until it is stale.)
        if self.held.as_ref().is_some_and(|h| h.k != k && !h.landed && now.saturating_sub(h.at) < STALE) {
            return Err(Refusal::Busy);
        }
        self.tally.roll(today);
        if !self.tally.room(u64::from(args.total())) {
            return Err(Refusal::Quota);
        }
        Ok(Pre::New)
    }

    /// The card said where it lands: held, charged. `Some(id)`: an earlier
    /// add given up for it, its part files to remove.
    pub fn begin(&mut self, k: KeyId, args: AddArgs, target: String, now: Duration) -> Option<u32> {
        let dropped = self.held.take().filter(|h| !h.landed).map(|h| h.args.id);
        self.tally.adds += 1;
        self.tally.bytes += u64::from(args.total());
        self.held = Some(Held { k, args, target, next: 0, src: Sha256::new(), psb: Sha256::new(), at: now, landed: false });
        dropped
    }

    /// A part from `k`: strictly at `next`, inside the stream.
    pub fn part(&mut self, k: KeyId, id: u32, off: u32, data: &[u8], now: Duration) -> Result<Progress, Refusal> {
        let h = self.held.as_mut().filter(|h| h.args.id == id && !h.landed && now.saturating_sub(h.at) < STALE).ok_or(Refusal::BadArg)?;
        if h.k != k {
            return Err(Refusal::NotAllowed);
        }
        let total = h.args.total();
        if off != h.next || u64::from(off) + data.len() as u64 > u64::from(total) {
            return Err(Refusal::BadArg);
        }
        let src = h.args.src_size;
        let mut runs = Vec::new();
        let (a, b) = (off, off + data.len() as u32);
        if a < src {
            let end = b.min(src);
            h.src.update(&data[..(end - a) as usize]);
            runs.push((Part::Source, a, 0..(end - a) as usize));
        }
        if b > src {
            let start = a.max(src);
            let from = (start - a) as usize;
            h.psb.update(&data[from..]);
            runs.push((Part::Psb, start - src, from..data.len()));
        }
        h.next = b;
        h.at = now;
        let mut complete = false;
        if b == total {
            let ok = h.src.clone().finalize()[..] == h.args.src_sha256 && h.psb.clone().finalize()[..] == h.args.psb_sha256;
            if !ok {
                self.held = None;
                return Err(Refusal::BadArg);
            }
            complete = true;
        }
        Ok(Progress { runs, next: b, complete })
    }

    /// The held add, if it is `k`'s `id` with all its bytes in.
    pub fn whole(&self, k: &KeyId, id: u32) -> Option<&Held> {
        self.held.as_ref().filter(|h| &h.k == k && h.args.id == id && h.next == h.args.total() && !h.landed)
    }

    /// It landed as `stem` (kept for a lost `done`; it holds nothing).
    pub fn landed(&mut self, stem: &str, now: Duration) {
        if let Some(h) = &mut self.held {
            h.landed = true;
            h.target = stem.to_string();
            h.at = now;
        }
    }

    /// It failed for good: dropped (its part files to remove).
    pub fn drop_held(&mut self) -> Option<u32> {
        self.held.take().map(|h| h.args.id)
    }

    /// `k` was removed (remotely or on the LAN): its add goes.
    pub fn forget_key(&mut self, k: &KeyId) -> Option<u32> {
        if self.held.as_ref().is_some_and(|h| &h.k == k) {
            return self.drop_held();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect::codec::{hex, sha256};
    use crate::connect::json;

    fn args(src: &[u8], psb: &[u8], stem: &str) -> AddArgs {
        AddArgs { id: 7, stem: stem.into(), name: "Rift patrol".into(), src_size: src.len() as u32, src_sha256: sha256(&[src]), psb_size: psb.len() as u32, psb_sha256: sha256(&[psb]) }
    }
    const K: KeyId = [1; 8];
    const K2: KeyId = [2; 8];
    fn t(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn base64url_strictly() {
        for n in 0..10 {
            let b: Vec<u8> = (0..n).map(|i| (i * 37 + 5) as u8).collect();
            assert_eq!(unb64url(&b64url(&b)), Some(b));
        }
        assert_eq!(unb64url("AB"), None, "unused bits set");
        assert_eq!(unb64url("AA="), None, "padding");
        assert_eq!(unb64url("A"), None);
        assert_eq!(unb64url("A+/A"), None, "the standard alphabet's + and /");
    }

    #[test]
    fn the_arguments() {
        let src = b"name \"Rift\"\ngray(x)";
        let psb = b"PSB\x03\x00\x04Rift";
        let a = |extra: &str| format!(r#"{{"id":7,"stem":"rift-patrol","name":"Rift","src_size":{},"src_sha256":"{}","psb_size":{},"psb_sha256":"{}"{extra}}}"#, src.len(), hex(&sha256(&[src])), psb.len(), hex(&sha256(&[psb])));
        let v = json::parse(a("").as_bytes()).unwrap();
        assert_eq!(add_args(&v).unwrap().stem, "rift-patrol");
        for (bad, want) in [
            (a("").replace("rift-patrol", "Rift"), Refusal::BadArg),
            (a("").replace("rift-patrol", "../x"), Refusal::BadArg),
            (a("").replace("rift-patrol", "rift_patrol"), Refusal::BadArg),
            (a("").replace("rift-patrol", &"r".repeat(41)), Refusal::BadArg),
            (a("").replace("\"id\":7", "\"id\":0"), Refusal::BadArg),
            (a("").replace(&format!("\"src_size\":{}", src.len()), "\"src_size\":16385"), Refusal::TooBig),
            (a("").replace(&format!("\"psb_size\":{}", psb.len()), "\"psb_size\":8193"), Refusal::TooBig),
            (a("").replace(&format!("\"psb_size\":{}", psb.len()), "\"psb_size\":5"), Refusal::BadArg),
            (a("").replace(&hex(&sha256(&[src])), &hex(&sha256(&[src])).to_uppercase()), Refusal::BadArg),
        ] {
            assert_eq!(add_args(&json::parse(bad.as_bytes()).unwrap()), Err(want), "{bad}");
        }
        let p = |s: &str| part_args(&json::parse(s.as_bytes()).unwrap());
        assert_eq!(p(r#"{"id":7,"off":0,"data":"AQID"}"#), Ok((7, 0, vec![1, 2, 3])));
        assert_eq!(p(r#"{"id":7,"off":0,"data":""}"#), Err(Refusal::BadArg));
        assert_eq!(p(&format!(r#"{{"id":7,"off":0,"data":"{}"}}"#, b64url(&[0; MAX_PART + 3]))), Err(Refusal::TooBig));
        assert!(p(&format!(r#"{{"id":7,"off":0,"data":"{}"}}"#, b64url(&[0; MAX_PART]))).is_ok());
        // The largest part's body (the largest n, id and offset) is a command.
        let dev = crate::connect::proto::Dev([0; 16]);
        let body = crate::connect::proto::command_body(&dev, &[0; 12], crate::connect::proto::MAX_N, "formula-part",
            &format!(r#""id":{},"off":{},"data":"{}""#, u32::MAX, u32::MAX, b64url(&[0xff; MAX_PART])));
        assert!(body.len() <= crate::connect::proto::MAX_BODY, "{}", body.len());
        assert_eq!(stem_choices("rift").collect::<Vec<_>>()[..3], ["rift", "rift-2", "rift-3"]);
        assert!(stem_choices(&"r".repeat(40)).all(|s| s.len() <= MAX_STEM));
    }

    /// `pre` then `begin`, as the firmware does once the card named a target.
    fn start(adds: &mut Adds, k: KeyId, a: AddArgs, now: Duration, today: Option<u64>) -> Result<Pre, Refusal> {
        let p = adds.pre(k, &a, now, today)?;
        if p == Pre::New {
            let stem = a.stem.clone();
            adds.begin(k, a, stem, now);
        }
        Ok(p)
    }

    #[test]
    fn a_stream_split_across_its_two_files() {
        let src = vec![b's'; 700];
        let psb = vec![b'p'; 300];
        let mut adds = Adds::default();
        assert_eq!(start(&mut adds, K, args(&src, &psb, "x"), t(0), None), Ok(Pre::New));
        let all: Vec<u8> = [src.clone(), psb.clone()].concat();
        let p = adds.part(K, 7, 0, &all[..384], t(1)).unwrap();
        assert_eq!(p.runs, vec![(Part::Source, 0, 0..384)]);
        let p = adds.part(K, 7, 384, &all[384..768], t(1)).unwrap();
        assert_eq!(p.runs, vec![(Part::Source, 384, 0..316), (Part::Psb, 0, 316..384)]);
        let p = adds.part(K, 7, 768, &all[768..], t(1)).unwrap();
        assert!(p.complete && p.next == 1000);
        assert!(adds.whole(&K, 7).is_some());
    }

    #[test]
    fn red_parts() {
        let src = vec![b's'; 500];
        let psb = vec![b'p'; 100];
        let all: Vec<u8> = [src.clone(), psb.clone()].concat();
        let mut adds = Adds::default();
        start(&mut adds, K, args(&src, &psb, "x"), t(0), None).unwrap();
        assert_eq!(adds.part(K, 7, 10, &all[10..20], t(1)), Err(Refusal::BadArg));
        assert_eq!(adds.part(K2, 7, 0, &all[..10], t(1)), Err(Refusal::NotAllowed));
        assert_eq!(adds.part(K, 8, 0, &all[..10], t(1)), Err(Refusal::BadArg));
        adds.part(K, 7, 0, &all[..384], t(1)).unwrap();
        // The same part again (a replay from an older session, past the
        // counter): not at `next`, refused, nothing written twice.
        assert_eq!(adds.part(K, 7, 0, &all[..384], t(2)), Err(Refusal::BadArg));
        let mut wrong = all[384..].to_vec();
        wrong[0] ^= 1;
        assert_eq!(adds.part(K, 7, 384, &wrong, t(3)), Err(Refusal::BadArg));
        assert!(adds.held.is_none());
    }

    #[test]
    fn resume_landed_busy_replace_rate() {
        let (src, psb) = (vec![1u8; 10], vec![2u8; 10]);
        let a = args(&src, &psb, "x");
        let mut adds = Adds::default();
        start(&mut adds, K, a.clone(), t(0), None).unwrap();
        adds.part(K, 7, 0, &src[..5], t(1)).unwrap();
        // Signed again (a new session): from where it was, never rated.
        assert_eq!(start(&mut adds, K, a.clone(), t(1), None), Ok(Pre::Resume { next: 5 }));
        // The add's own rate: another add within 2 s.
        assert_eq!(start(&mut adds, K2, args(&src, &psb, "y"), t(1), None), Err(Refusal::Rate));
        // Another key while it is held: busy.
        assert_eq!(start(&mut adds, K2, args(&src, &psb, "y"), t(3), None), Err(Refusal::Busy));
        // The same key's other add replaces it.
        let mut b = a.clone();
        b.id = 9;
        assert_eq!(adds.pre(K, &b, t(6), None), Ok(Pre::New));
        assert_eq!(adds.begin(K, b.clone(), "x".into(), t(6)), Some(7));
        adds.landed("x-2", t(7));
        assert_eq!(start(&mut adds, K, b, t(8), None), Ok(Pre::Landed { stem: "x-2".into() }));
        // A landed add holds nothing: another key may start at once.
        assert_eq!(start(&mut adds, K2, args(&src, &psb, "y"), t(11), None), Ok(Pre::New));
        // A stalled one goes after 10 minutes.
        assert_eq!(adds.expire(t(11 + 600)), Some(7));
        assert!(adds.forget_key(&K2).is_none());
    }

    #[test]
    fn red_forget_and_expiry() {
        let (src, psb) = (vec![1u8; 10], vec![2u8; 10]);
        let mut adds = Adds::default();
        start(&mut adds, K, args(&src, &psb, "x"), t(0), None).unwrap();
        assert_eq!(adds.forget_key(&K), Some(7), "the key removed: its add goes");
        start(&mut adds, K, args(&src, &psb, "x"), t(10), None).unwrap();
        assert_eq!(adds.part(K, 7, 0, &src[..5], t(10 + 600)), Err(Refusal::BadArg), "a stale add takes no part");
    }

    #[test]
    fn the_tally_and_its_day() {
        let (src, psb) = (vec![1u8; 100], vec![2u8; 100]);
        let mut adds = Adds::default();
        let mut now = 0;
        let mut go = |adds: &mut Adds, id: u32, today| {
            now += 3;
            let mut a = args(&src, &psb, "x");
            a.id = id;
            start(adds, K, a, t(now), today)
        };
        for i in 0..QUOTA_ADDS {
            assert_eq!(go(&mut adds, i + 1, Some(20000)), Ok(Pre::New), "{i}");
        }
        assert_eq!(go(&mut adds, 99, Some(20000)), Err(Refusal::Quota), "the 17th of the day");
        // A restart: the tally comes back from the card, still full.
        let kept = Tally::parse(&adds.tally.text());
        assert_eq!(kept, adds.tally);
        let mut again = Adds { tally: kept, ..Adds::default() };
        assert_eq!(go(&mut again, 100, Some(20000)), Err(Refusal::Quota), "a restart does not reset it");
        // No clock: it stands; a new day by the clock starts it again.
        assert_eq!(go(&mut again, 101, None), Err(Refusal::Quota), "no clock: no new day");
        assert_eq!(go(&mut again, 102, Some(20001)), Ok(Pre::New), "the next day");
        // A tally the card holds torn or garbled is a full day.
        for bad in ["x", "1 2", "1 2 3 4", "20001 a 3", "z 1 1"] {
            assert!(!Tally::parse(bad).room(1), "{bad:?}");
        }
        assert_eq!(Tally::parse(""), Tally::default());
        // Bytes: 256 KB a day.
        let mut adds = Adds::default();
        let big = AddArgs { src_size: MAX_SOURCE, psb_size: MAX_PSB, ..args(&src, &psb, "x") };
        let mut n = 0u32;
        while start(&mut adds, K, AddArgs { id: n + 1, ..big.clone() }, t(u64::from(n) * 3), Some(1)).is_ok() {
            n += 1;
        }
        assert_eq!(u64::from(n) * u64::from(big.total()), QUOTA_BYTES - QUOTA_BYTES % u64::from(big.total()));
    }
}
