//! A Formula added by a remote controller (`formula-add`, `formula-part`;
//! protocol 5.7): the panel's side, without the card. The arguments' rules,
//! the gallery's signature, one add at a time with its two SHA-256s and its
//! next offset, resumes, the adds started a day (in RAM), the add's own
//! rate. The firmware writes the bytes, checks the compiled program
//! (`psb::verify`) and lands the files (`firmware/src/connect/fadd.rs`).
//!
//! This is the one place a remote controller hands the panel a program
//! (every other path refuses inline Formulas and `~sent`), and only a
//! Formula the project signed may arrive that way: [`admit`] checks the
//! gallery's signature (`crate::gallery_sig::check`) over the stem, the
//! sizes and the hashes the add names, right after the argument rules and
//! before anything else (the parser), and an add is held only with the
//! [`Signed`] witness it returns, over exactly those bytes ([`Adds::begin`]).
//! Nothing else can make one: the command, the firmware's add and its
//! landing all take it.

use std::time::Duration;

use sha2::{Digest, Sha256};

use super::json::Value;
use super::keys::KeyId;
use super::proto::Refusal;
pub use crate::gallery_sig::Signed;

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
/// Adds one key may start in a day of uptime, and all keys together
/// ([`Adds`], in RAM: a restart starts it again, and nothing remote can
/// restart a panel). It bounds abandoned adds' part files; what may land is
/// bounded by the signature.
pub const DAY_ADDS: usize = 16;
pub const PANEL_DAY_ADDS: usize = 32;
pub const DAY: Duration = Duration::from_secs(24 * 3600);
/// A held add's whole life, from its start, however often it is sent
/// again (the firmware's test image may set a shorter one).
pub const LIFETIME: Duration = Duration::from_secs(15 * 60);
/// Gallery signature checks (an Ed25519 verify each, on a task of its own):
/// per key one every 2 s with a burst of 2; per panel one a second with a
/// burst of 3. Taken before the verify; a held add sent again by its own key
/// is not verified again (its witness is compared instead).
pub const VERIFY_KEY: (u64, u64) = (500_000, 2_000_000);
pub const VERIFY_PANEL: (u64, u64) = (1_000_000, 3_000_000);
/// One card read for an add (its directory pass: a new add, or the same
/// Formula found already there) per key in 2 s, per panel in 1 s.
pub const PANEL_ADD_EVERY: Duration = Duration::from_secs(1);
/// Formulas the card may hold for an add to be taken (sources counted in the
/// add's own directory pass).
pub const MAX_FORMULAS: usize = 128;
/// One card read for an add per key in this long ([`PANEL_ADD_EVERY`] for
/// the panel): a key cannot make the panel read the card four times a second.
pub const ADD_EVERY: Duration = Duration::from_secs(2);
/// An add with no progress for this long is dropped (its part files too).
pub const STALE: Duration = Duration::from_secs(600);


/// `formula-add`'s arguments, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddArgs {
    pub id: u32,
    pub stem: String,
    pub src_size: u32,
    pub src_sha256: [u8; 32],
    pub psb_size: u32,
    pub psb_sha256: [u8; 32],
}

impl AddArgs {
    pub fn total(&self) -> u32 {
        self.src_size + self.psb_size
    }

    /// Whether `w` is a signature over exactly these bytes, under this stem.
    pub fn signed_by(&self, w: &Signed) -> bool {
        w.stem() == self.stem && w.src() == (self.src_size, &self.src_sha256) && w.psb() == (self.psb_size, &self.psb_sha256)
    }
}

/// A `formula-add` the gallery's signature admitted: its arguments and the
/// witness over them (boxed in the command: every command's frames stay
/// the size they were).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    pub args: AddArgs,
    pub signed: Signed,
}

/// `formula-add`'s signature fields: the gallery key's id, the compiled
/// format and the signing revision it was signed for, and the signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub kid: u8,
    pub abi: u8,
    pub rev: u32,
    pub sig: [u8; 64],
}

/// The gallery's signature checked over exactly what the add names (its
/// argument stem, never a landing stem; the sizes and hashes the parts are
/// held to) for `t`: the witness, or `NotAllowed` for any refusal.
#[inline(never)]
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn admit(args: &AddArgs, c: &Claim, t: &crate::gallery_sig::Trust) -> Result<Signed, Refusal> {
    crate::gallery_sig::check(t, &formula_of(args, c), &c.sig).map_err(|_| Refusal::NotAllowed)
}

/// A gallery signature check to run, owning all it needs (nothing borrowed
/// from the caller's stack: a runner may give up waiting for it).
#[derive(Debug)]
pub struct VerifyJob {
    args: AddArgs,
    claim: Claim,
    trust: crate::gallery_sig::Trust<'static>,
}

impl VerifyJob {
    /// The check itself ([`admit`]).
    pub fn run(self) -> Result<Signed, Refusal> {
        admit(&self.args, &self.claim, &self.trust)
    }
}

/// Where the gallery's signature check runs: given the job, its result, or
/// `None` when it could not run it, or gave up waiting (`busy`). The
/// firmware runs it on a task of its own ([`set_verify_runner`]), off the
/// network worker's stack; with none set (the host), in place.
pub type Runner = fn(Box<VerifyJob>) -> Option<Result<Signed, Refusal>>;
static RUNNER: std::sync::OnceLock<Runner> = std::sync::OnceLock::new();

/// The firmware's [`Runner`] (once, at start).
pub fn set_verify_runner(r: Runner) {
    let _ = RUNNER.set(r);
}

/// [`admit`] through the [`Runner`], after its cheap refusals
/// (`gallery_sig::precheck`: no allocation, no task).
pub fn admit_where_it_runs(args: &AddArgs, c: &Claim, t: &crate::gallery_sig::Trust<'static>) -> Result<Signed, Refusal> {
    crate::gallery_sig::precheck(t, &formula_of(args, c)).map_err(|_| Refusal::NotAllowed)?;
    // (Boxed: only a pointer moves through the frames below, the worker's.)
    let job = Box::new(VerifyJob { args: args.clone(), claim: c.clone(), trust: *t });
    match RUNNER.get() {
        Some(run) => run(job).unwrap_or(Err(Refusal::Busy)),
        None => job.run(),
    }
}

fn formula_of<'a>(args: &'a AddArgs, c: &Claim) -> crate::gallery_sig::Formula<'a> {
    crate::gallery_sig::Formula {
        kid: c.kid,
        abi: c.abi,
        rev: c.rev,
        stem: &args.stem,
        src_len: args.src_size,
        src_sha256: args.src_sha256,
        psb_len: args.psb_size,
        psb_sha256: args.psb_sha256,
    }
}

/// Tokens in millionths, refilled at a rate and capped (the verify rates).
#[derive(Debug, Clone, Copy)]
struct Tokens {
    micro: u64,
    last: Duration,
}

impl Tokens {
    fn full(cap: u64, now: Duration) -> Tokens {
        Tokens { micro: cap, last: now }
    }

    fn take(&mut self, now: Duration, (per_s, cap): (u64, u64)) -> bool {
        let dt = now.saturating_sub(self.last).as_micros().min(u128::from(u64::MAX)) as u64;
        self.micro = self.micro.saturating_add(dt.saturating_mul(per_s) / 1_000_000).min(cap);
        self.last = self.last.max(now);
        if self.micro >= 1_000_000 {
            self.micro -= 1_000_000;
            true
        } else {
            false
        }
    }

    fn has(&self, now: Duration, (per_s, cap): (u64, u64)) -> bool {
        let dt = now.saturating_sub(self.last).as_micros().min(u128::from(u64::MAX)) as u64;
        self.micro.saturating_add(dt.saturating_mul(per_s) / 1_000_000).min(cap) >= 1_000_000
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

/// SHA-256 fed in pieces (the firmware's read-back of a part file).
#[derive(Default)]
pub struct Hash(Sha256);

impl Hash {
    pub fn update(&mut self, b: &[u8]) {
        self.0.update(b);
    }
    pub fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
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

/// `formula-add`'s arguments, and its signature's fields (`kid` 0-255,
/// `abi` 0-255, `rev` 0 to 2^32-1, `sig` strict base64url of exactly 64
/// bytes: else `bad-arg`).
pub fn add_args(a: &Value) -> Result<(AddArgs, Claim), Refusal> {
    let bad = Refusal::BadArg;
    let int = |k: &str| arg(a, k).and_then(Value::as_int).ok_or(bad);
    let id = u32::try_from(int("id")?).ok().filter(|&i| i != 0).ok_or(bad)?;
    let stem = arg(a, "stem").and_then(Value::as_str).filter(|s| stem_ok(s)).ok_or(bad)?.to_string();
    let kid = u8::try_from(int("kid")?).map_err(|_| bad)?;
    let abi = u8::try_from(int("abi")?).map_err(|_| bad)?;
    let rev = u32::try_from(int("rev")?).map_err(|_| bad)?;
    let sig = arg(a, "sig").and_then(Value::as_str).filter(|s| s.len() == 86).and_then(unb64url).and_then(|b| <[u8; 64]>::try_from(b).ok()).ok_or(bad)?;
    let (src_size, psb_size) = (int("src_size")?, int("psb_size")?);
    if !(1..=i64::from(MAX_SOURCE)).contains(&src_size) || !(i64::from(MIN_PSB)..=i64::from(MAX_PSB)).contains(&psb_size) {
        return Err(if src_size > i64::from(MAX_SOURCE) || psb_size > i64::from(MAX_PSB) { Refusal::TooBig } else { bad });
    }
    let hash = |k: &str| arg(a, k).and_then(Value::as_str).and_then(super::codec::unhex::<32>).ok_or(bad);
    let args = AddArgs { id, stem, src_size: src_size as u32, src_sha256: hash("src_sha256")?, psb_size: psb_size as u32, psb_sha256: hash("psb_sha256")? };
    Ok((args, Claim { kid, abi, rev, sig }))
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
    /// The gallery's signature over them, as `gallery_sig::check` took it.
    signed: Signed,
    /// The stem it lands under (the args' or a next free one), as the
    /// firmware chose at the start.
    pub target: String,
    pub next: u32,
    src: Sha256,
    psb: Sha256,
    /// Its last progress (a part), and its start (its absolute life).
    pub at: Duration,
    pub born: Duration,
    /// Its `.psi` is on the card already, the same source: only the `.psb`
    /// is to be put back (a refill, the design's step 6).
    pub refill: bool,
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

/// The panel's adds: one held at a time; the adds started in the last day of
/// uptime, per key and in all; the card reads for adds, per key and in all;
/// the gallery verifies' rates, per key and in all.
#[derive(Debug)]
pub struct Adds {
    pub held: Option<Held>,
    started: Vec<(KeyId, Duration)>,
    reads: Vec<(KeyId, Duration)>,
    panel_read: Option<Duration>,
    verify_keys: Vec<(KeyId, Tokens)>,
    verify_panel: Option<Tokens>,
    lifetime: Duration,
}

impl Default for Adds {
    fn default() -> Adds {
        Adds::with_lifetime(LIFETIME)
    }
}

impl Adds {
    /// With a held add's absolute life `lifetime` ([`LIFETIME`] normally).
    pub fn with_lifetime(lifetime: Duration) -> Adds {
        Adds { held: None, started: Vec::new(), reads: Vec::new(), panel_read: None, verify_keys: Vec::new(), verify_panel: None, lifetime }
    }

    fn live(&self, h: &Held, now: Duration) -> bool {
        now.saturating_sub(h.at) < STALE && (h.landed || now.saturating_sub(h.born) < self.lifetime)
    }

    /// The held add, if it is still live at `now` (else it is dropped and
    /// its id returned: its part files to remove now): no progress for
    /// [`STALE`], or not landed within its absolute life.
    pub fn expire(&mut self, now: Duration) -> Option<u32> {
        if self.held.as_ref().is_some_and(|h| !self.live(h, now)) {
            return self.held.take().map(|h| h.args.id);
        }
        None
    }

    /// The gallery's signature for `k`'s add (the command's parser, before
    /// anything else): the cheap refusals; the held add sent again by its own
    /// key, the same signed statement, takes its witness again (no verify);
    /// else the verify rates (`rate`), then the verify where the firmware
    /// runs it.
    pub fn admit(&mut self, k: KeyId, args: &AddArgs, c: &Claim, now: Duration, t: &crate::gallery_sig::Trust<'static>) -> Result<Signed, Refusal> {
        crate::gallery_sig::precheck(t, &formula_of(args, c)).map_err(|_| Refusal::NotAllowed)?;
        if let Some(h) = self.held.as_ref().filter(|h| h.k == k && &h.args == args && self.live(h, now)) {
            let w = &h.signed;
            if (w.kid(), w.abi(), w.rev(), w.sig()) == (c.kid, c.abi, c.rev, &c.sig) {
                return Ok(w.clone());
            }
        }
        let key = match self.verify_keys.iter().position(|(x, _)| *x == k) {
            Some(i) => i,
            None => {
                // (Bounded: the oldest bucket goes past 32 keys; the key
                // list holds 16.)
                if self.verify_keys.len() >= 32 {
                    self.verify_keys.remove(0);
                }
                self.verify_keys.push((k, Tokens::full(VERIFY_KEY.1, now)));
                self.verify_keys.len() - 1
            }
        };
        let panel = self.verify_panel.get_or_insert(Tokens::full(VERIFY_PANEL.1, now));
        if !self.verify_keys[key].1.has(now, VERIFY_KEY) || !panel.has(now, VERIFY_PANEL) {
            return Err(Refusal::Rate);
        }
        self.verify_keys[key].1.take(now, VERIFY_KEY);
        panel.take(now, VERIFY_PANEL);
        admit_where_it_runs(args, c, t)
    }

    fn last_read(&self, k: &KeyId) -> Option<Duration> {
        self.reads.iter().find(|(x, _)| x == k).map(|&(_, t)| t)
    }

    /// Before anything is read from the card (and after the signature):
    /// the card-read rate (per key and in all), the same add again, another
    /// key's add under way, the adds started in the last day (per key and
    /// in all). Nothing is counted here: [`note_read`](Self::note_read)
    /// and [`begin`](Self::begin) do, once the card is read.
    pub fn pre(&mut self, k: KeyId, args: &AddArgs, now: Duration) -> Result<Pre, Refusal> {
        let rated = self.last_read(&k).is_some_and(|t| now.saturating_sub(t) < ADD_EVERY)
            || self.panel_read.is_some_and(|t| now.saturating_sub(t) < PANEL_ADD_EVERY);
        let live = self.held.as_ref().is_some_and(|h| self.live(h, now));
        if let Some(h) = self.held.as_ref().filter(|h| live && h.k == k && &h.args == args) {
            // (Its checks tried again, with all the bytes in: a card read,
            // rated too. Sent again, it is never made younger.)
            let whole = !h.landed && h.next == h.args.total();
            if whole && rated {
                return Err(Refusal::Rate);
            }
            let p = if h.landed { Pre::Landed { stem: h.target.clone() } } else { Pre::Resume { next: h.next } };
            if whole {
                self.note_read(k, now);
            }
            return Ok(p);
        }
        if rated {
            return Err(Refusal::Rate);
        }
        // (A landed add holds nothing; a live one of another key does.)
        if self.held.as_ref().is_some_and(|h| live && h.k != k && !h.landed) {
            return Err(Refusal::Busy);
        }
        self.day_room(k, now)?;
        Ok(Pre::New)
    }

    /// The card is read for `k`'s add now (its directory pass): counted
    /// against the card-read rate, per key and in all (an add found already
    /// there too).
    pub fn note_read(&mut self, k: KeyId, now: Duration) {
        match self.reads.iter_mut().find(|(x, _)| *x == k) {
            Some(r) => r.1 = now,
            None => {
                self.reads.retain(|&(_, t)| now.saturating_sub(t) < DAY);
                self.reads.push((k, now));
            }
        }
        self.panel_read = Some(now);
    }

    fn day_room(&mut self, k: KeyId, now: Duration) -> Result<(), Refusal> {
        self.started.retain(|&(_, t)| now.saturating_sub(t) < DAY);
        if self.started.len() >= PANEL_DAY_ADDS || self.started.iter().filter(|(x, _)| *x == k).count() >= DAY_ADDS {
            return Err(Refusal::Quota);
        }
        Ok(())
    }

    /// The card said where it lands: held, counted (`Quota` past the day's
    /// [`DAY_ADDS`]), with the gallery's signature over exactly these bytes
    /// (`BadArg` for another's).
    /// `Ok(Some(id))`: an earlier add given up for it, its part files to
    /// remove.
    ///
    /// The witness comes from `gallery_sig::check` ([`admit`]; this
    /// compiles, and is refused at run time: no key signed it):
    ///
    /// ```no_run
    /// use opendmd_core::connect::{fadd::{add_args, admit, Adds}, json};
    /// use opendmd_core::gallery_sig::Trust;
    /// let (a, c) = add_args(&json::parse(br#"{"id":1,"stem":"x","kid":0,"abi":3,"rev":1,"sig":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","src_size":1,"src_sha256":"00000000000000000000000000000000000000000000000000000000000000ff","psb_size":6,"psb_sha256":"00000000000000000000000000000000000000000000000000000000000000ff"}"#).unwrap()).unwrap();
    /// let w = admit(&a, &c, &Trust::panel(3)).unwrap();
    /// Adds::default().begin([0; 8], a, w, "x".into(), Default::default()).unwrap();
    /// ```
    ///
    /// and from nowhere else: the same with a witness made by hand does not
    /// compile, so neither does any landing without a signature.
    ///
    /// ```compile_fail
    /// use opendmd_core::connect::{fadd::{add_args, Adds, Signed}, json};
    /// let (a, _) = add_args(&json::parse(br#"{"id":1,"stem":"x","kid":0,"abi":3,"rev":1,"sig":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","src_size":1,"src_sha256":"00000000000000000000000000000000000000000000000000000000000000ff","psb_size":6,"psb_sha256":"00000000000000000000000000000000000000000000000000000000000000ff"}"#).unwrap()).unwrap();
    /// let w = Signed { kid: 0, abi: 3, rev: 1, stem: a.stem.clone(), src_len: a.src_size, src_sha256: a.src_sha256, psb_len: a.psb_size, psb_sha256: a.psb_sha256 };
    /// Adds::default().begin([0; 8], a, w, "x".into(), Default::default()).unwrap();
    /// ```
    pub fn begin(&mut self, k: KeyId, args: AddArgs, signed: Signed, target: String, now: Duration) -> Result<Option<u32>, Refusal> {
        self.begin_as(k, args, signed, target, false, now)
    }

    /// [`begin`](Self::begin), `refill`: the source is on the card already
    /// (its hash the signed one) and only the `.psb` is to be put back.
    pub fn begin_as(&mut self, k: KeyId, args: AddArgs, signed: Signed, target: String, refill: bool, now: Duration) -> Result<Option<u32>, Refusal> {
        if !args.signed_by(&signed) {
            return Err(Refusal::BadArg);
        }
        // (The day's room first: nothing held is given up for a refusal.)
        self.day_room(k, now)?;
        let dropped = self.held.take().filter(|h| !h.landed).map(|h| h.args.id);
        self.started.push((k, now));
        self.held = Some(Held { k, args, signed, target, next: 0, src: Sha256::new(), psb: Sha256::new(), at: now, born: now, refill, landed: false });
        Ok(dropped)
    }

    /// A part from `k`: strictly at `next`, inside the stream. With no add
    /// held at all: `NotAllowed` (no add was admitted).
    pub fn part(&mut self, k: KeyId, id: u32, off: u32, data: &[u8], now: Duration) -> Result<Progress, Refusal> {
        if self.held.is_none() {
            return Err(Refusal::NotAllowed);
        }
        let live = self.held.as_ref().is_some_and(|h| self.live(h, now));
        let h = self.held.as_mut().filter(|h| live && h.args.id == id && !h.landed).ok_or(Refusal::BadArg)?;
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

    /// The held add, if it is `k`'s `id` with all its bytes in, and the
    /// gallery's signature it was admitted with (what a landing takes).
    pub fn whole(&self, k: &KeyId, id: u32) -> Option<(&Held, &Signed)> {
        self.held.as_ref().filter(|h| &h.k == k && h.args.id == id && h.next == h.args.total() && !h.landed).map(|h| (h, &h.signed))
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
        AddArgs { id: 7, stem: stem.into(), src_size: src.len() as u32, src_sha256: sha256(&[src]), psb_size: psb.len() as u32, psb_sha256: sha256(&[psb]) }
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
        let sig = b64url(&[7; 64]);
        let a = |extra: &str| format!(r#"{{"id":7,"stem":"rift-patrol","kid":0,"abi":3,"rev":1,"sig":"{sig}","src_size":{},"src_sha256":"{}","psb_size":{},"psb_sha256":"{}"{extra}}}"#, src.len(), hex(&sha256(&[src])), psb.len(), hex(&sha256(&[psb])));
        let v = json::parse(a("").as_bytes()).unwrap();
        let (args, c) = add_args(&v).unwrap();
        assert_eq!((args.stem.as_str(), c.kid, c.abi, c.rev, c.sig), ("rift-patrol", 0, 3, 1, [7; 64]));
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
            // The signature's fields: ranges, and exactly 64 bytes of strict base64url.
            (a("").replace("\"kid\":0", "\"kid\":256"), Refusal::BadArg),
            (a("").replace("\"kid\":0", "\"kid\":-1"), Refusal::BadArg),
            (a("").replace("\"abi\":3", "\"abi\":256"), Refusal::BadArg),
            (a("").replace("\"rev\":1", "\"rev\":4294967296"), Refusal::BadArg),
            (a("").replace("\"rev\":1,", ""), Refusal::BadArg),
            (a("").replace(&sig, &b64url(&[7; 63])), Refusal::BadArg),
            (a("").replace(&sig, &b64url(&[7; 65])), Refusal::BadArg),
            (a("").replace(&sig, &format!("{sig}==")), Refusal::BadArg),
            (a("").replace(&sig, &sig.replace('-', "+").replace('_', "/").replacen('H', "+", 1)), Refusal::BadArg),
            (a("").replace(&sig, &(sig[..85].to_string() + "x")), Refusal::BadArg),
        ] {
            assert_eq!(add_args(&json::parse(bad.as_bytes()).unwrap()).map(drop), Err(want), "{bad}");
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

    /// A gallery signature over `a` (the test key's: the only way a
    /// witness is made, `gallery_sig::check`).
    fn witness(a: &AddArgs) -> Signed {
        crate::gallery_sig::signed_for_tests(&crate::gallery_sig::Formula {
            kid: 0,
            abi: 3,
            rev: 1,
            stem: &a.stem,
            src_len: a.src_size,
            src_sha256: a.src_sha256,
            psb_len: a.psb_size,
            psb_sha256: a.psb_sha256,
        })
    }

    /// `pre` then `begin`, as the firmware does once the card named a target.
    fn start(adds: &mut Adds, k: KeyId, a: AddArgs, now: Duration) -> Result<Pre, Refusal> {
        let p = adds.pre(k, &a, now)?;
        if p == Pre::New {
            adds.note_read(k, now);
            let stem = a.stem.clone();
            let w = witness(&a);
            adds.begin(k, a, w, stem, now)?;
        }
        Ok(p)
    }

    /// The vectors' test key A as a `'static` trust, and its claim over `a`.
    fn test_trust() -> crate::gallery_sig::Trust<'static> {
        use ed25519_dalek::SigningKey;
        let seed: [u8; 32] = Sha256::digest(b"opendmd-gallery throwaway test key, never a real one: A").into();
        let public: &'static [u8; 32] = Box::leak(Box::new(SigningKey::from_bytes(&seed).verifying_key().to_bytes()));
        let keys: &'static [(u8, &'static [u8; 32])] = Box::leak(Box::new([(0u8, public)]));
        crate::gallery_sig::Trust::for_tests(keys, 1, 3)
    }

    fn claim(a: &AddArgs) -> Claim {
        use ed25519_dalek::{Signer, SigningKey};
        let seed: [u8; 32] = Sha256::digest(b"opendmd-gallery throwaway test key, never a real one: A").into();
        let c = Claim { kid: 0, abi: 3, rev: 1, sig: [0; 64] };
        let sig = SigningKey::from_bytes(&seed).sign(crate::gallery_sig::message(&formula_of(a, &c)).unwrap().as_bytes()).to_bytes();
        Claim { sig, ..c }
    }

    /// The verify's own guards: the cheap refusals before anything; the
    /// verify rates (per key one in 2 s, burst 2; per panel one a second,
    /// burst 3) before the verify; the held add sent again by its own key
    /// takes its witness again with no verify (here: with no verify left
    /// in the rate); any change to the statement is verified anew.
    #[test]
    fn admit_guards_the_verify() {
        let tr = test_trust();
        let a = args(b"gray(x)\n", b"PSB\x03\x00\x00", "x");
        let c = claim(&a);
        let mut adds = Adds::default();
        for (what, bad) in [("kid", Claim { kid: 9, ..c.clone() }), ("abi", Claim { abi: 4, ..c.clone() }), ("rev", Claim { rev: 0, ..c.clone() })] {
            assert_eq!(adds.admit(K, &a, &bad, t(0), &tr), Err(Refusal::NotAllowed), "{what}");
        }
        assert!(adds.verify_keys.is_empty() && adds.verify_panel.is_none(), "the cheap refusals spend no rate");
        // Two for the key at once, then one in 2 s.
        let garbage = Claim { sig: [7; 64], ..c.clone() };
        assert_eq!(adds.admit(K, &a, &garbage, t(0), &tr), Err(Refusal::NotAllowed));
        assert_eq!(adds.admit(K, &a, &garbage, t(0), &tr), Err(Refusal::NotAllowed));
        assert_eq!(adds.admit(K, &a, &garbage, t(0), &tr), Err(Refusal::Rate), "the key's burst used");
        assert_eq!(adds.admit(K, &a, &garbage, Duration::from_millis(1900), &tr), Err(Refusal::Rate));
        assert_eq!(adds.admit(K, &a, &garbage, t(2), &tr), Err(Refusal::NotAllowed), "one in 2 s");
        // The panel's: three keys at once take its burst of 3 (one left over
        // from above refilled), the next key is rated.
        let mut p = Adds::default();
        for (i, k) in [[3u8; 8], [4; 8], [5; 8]].into_iter().enumerate() {
            assert_eq!(p.admit(k, &a, &garbage, t(10), &tr), Err(Refusal::NotAllowed), "key {i}");
        }
        assert_eq!(p.admit([6; 8], &a, &garbage, t(10), &tr), Err(Refusal::Rate), "the panel's burst used");
        // The held add, sent again by its own key: its witness, no verify.
        let mut h = Adds::default();
        let w = h.admit(K, &a, &c, t(0), &tr).unwrap();
        h.admit(K, &a, &c, t(0), &tr).unwrap();
        h.begin(K, a.clone(), w.clone(), "x".into(), t(0)).unwrap();
        for _ in 0..10 {
            assert_eq!(h.admit(K, &a, &c, t(0), &tr), Ok(w.clone()), "no verify: none left in the rate");
        }
        assert_eq!(h.admit(K2, &a, &c, t(0), &tr).map(drop), Ok(()), "another key: verified (its own rate)");
        assert_eq!(h.admit(K, &a, &garbage, t(0), &tr), Err(Refusal::Rate), "another statement: verified, rated");
    }

    /// A held add's life is absolute: sent again, it is not made younger;
    /// past it, it goes, and its parts are refused.
    #[test]
    fn a_held_add_has_an_absolute_life() {
        let (src, psb) = (vec![1u8; 10], vec![2u8; 10]);
        let a = args(&src, &psb, "x");
        let mut adds = Adds::with_lifetime(t(90));
        start(&mut adds, K, a.clone(), t(0)).unwrap();
        for s in [20, 40, 60, 80] {
            assert_eq!(start(&mut adds, K, a.clone(), t(s)), Ok(Pre::Resume { next: 0 }));
            adds.part(K, 7, 0, &src[..1], t(s)).ok();
            adds.held.as_mut().unwrap().next = 0;
        }
        assert_eq!(adds.expire(t(89)), None);
        assert_eq!(adds.expire(t(90)), Some(7), "90 s from its start, whatever was sent since");
        assert_eq!(adds.part(K, 7, 0, &src[..1], t(91)), Err(Refusal::NotAllowed));
    }

    /// The day's count and the card-read rate are per key, with a ceiling
    /// for the panel; a refusal gives up nothing held; only a card read
    /// counts against the read rate.
    #[test]
    fn per_key_counts_and_the_panel_ceiling() {
        let (src, psb) = (vec![1u8; 10], vec![2u8; 10]);
        let mut adds = Adds::default();
        let mut now = 10u64;
        let mut go = |adds: &mut Adds, k: KeyId, id: u32| {
            now += 3;
            let r = start(adds, k, AddArgs { id, ..args(&src, &psb, "x") }, t(now));
            // (Each lands: a landed add holds nothing.)
            adds.landed("x", t(now));
            r
        };
        for i in 0..DAY_ADDS as u32 {
            assert_eq!(go(&mut adds, K, 1 + i), Ok(Pre::New));
        }
        assert_eq!(go(&mut adds, K, 99), Err(Refusal::Quota), "K's 17th");
        for i in 0..DAY_ADDS as u32 {
            assert_eq!(go(&mut adds, K2, 100 + i), Ok(Pre::New), "K2 has its own 16");
        }
        assert_eq!(go(&mut adds, [9; 8], 200), Err(Refusal::Quota), "the panel's 32");
        // A refused begin keeps the held add.
        let held = adds.held.as_ref().map(|h| h.args.id);
        let b = AddArgs { id: 300, ..args(&src, &psb, "x") };
        assert_eq!(adds.begin(K, b.clone(), witness(&b), "x".into(), t(now + 10)), Err(Refusal::Quota));
        assert_eq!(adds.held.as_ref().map(|h| h.args.id), held);
        // `pre` alone reads nothing: no rate spent.
        let mut r = Adds::default();
        for i in 0..5u32 {
            assert_eq!(r.pre(K, &AddArgs { id: 500 + i, ..args(&src, &psb, "x") }, t(1)), Ok(Pre::New));
        }
        r.note_read(K, t(1));
        assert_eq!(r.pre(K, &args(&src, &psb, "x"), t(2)), Err(Refusal::Rate));
    }

    /// An add is held only with a signature over exactly its bytes, and a
    /// part with no add held is not-allowed.
    #[test]
    fn an_add_is_held_only_with_its_own_signature() {
        let (src, psb) = (b"gray(x)\n".to_vec(), b"PSB\x03\x00\x00".to_vec());
        let mut adds = Adds::default();
        assert_eq!(adds.part(K, 7, 0, &src, t(0)).err(), Some(Refusal::NotAllowed), "no add held");
        let a = args(&src, &psb, "x");
        let other = witness(&args(b"gray(y)\n", &psb, "x"));
        assert_eq!(adds.begin(K, a.clone(), other, "x".into(), t(0)), Err(Refusal::BadArg), "another Formula's signature");
        let stem = witness(&args(&src, &psb, "y"));
        assert_eq!(adds.begin(K, a.clone(), stem, "x".into(), t(0)), Err(Refusal::BadArg), "another stem's signature");
        assert!(adds.held.is_none());
        let w = witness(&a);
        assert_eq!(adds.begin(K, a.clone(), w.clone(), "x".into(), t(0)), Ok(None));
        adds.part(K, 7, 0, &[src.clone(), psb.clone()].concat(), t(1)).unwrap();
        assert_eq!(adds.whole(&K, 7).map(|(_, s)| s.clone()), Some(w));
    }

    #[test]
    fn a_stream_split_across_its_two_files() {
        let src = vec![b's'; 700];
        let psb = vec![b'p'; 300];
        let mut adds = Adds::default();
        assert_eq!(start(&mut adds, K, args(&src, &psb, "x"), t(0)), Ok(Pre::New));
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
        start(&mut adds, K, args(&src, &psb, "x"), t(0)).unwrap();
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
        start(&mut adds, K, a.clone(), t(0)).unwrap();
        adds.part(K, 7, 0, &src[..5], t(1)).unwrap();
        // Signed again (a new session): from where it was, never rated.
        assert_eq!(start(&mut adds, K, a.clone(), t(1)), Ok(Pre::Resume { next: 5 }));
        // The card-read rate: the same key's other add within 2 s, any
        // key's within 1 s of the panel's last read.
        let mut b = a.clone();
        b.id = 9;
        assert_eq!(adds.pre(K, &b, t(1)), Err(Refusal::Rate), "the key's 2 s");
        assert_eq!(adds.pre(K2, &args(&src, &psb, "y"), Duration::from_millis(500)), Err(Refusal::Rate), "the panel's 1 s");
        // Another key while it is held: busy.
        assert_eq!(start(&mut adds, K2, args(&src, &psb, "y"), t(3)), Err(Refusal::Busy));
        // The same key's other add replaces it.
        assert_eq!(adds.pre(K, &b, t(6)), Ok(Pre::New));
        assert_eq!(adds.begin(K, b.clone(), witness(&b), "x".into(), t(6)), Ok(Some(7)));
        adds.landed("x-2", t(7));
        assert_eq!(start(&mut adds, K, b, t(8)), Ok(Pre::Landed { stem: "x-2".into() }));
        // A landed add holds nothing: another key may start at once.
        assert_eq!(start(&mut adds, K2, args(&src, &psb, "y"), t(11)), Ok(Pre::New));
        // A stalled one goes after 10 minutes.
        assert_eq!(adds.expire(t(11 + 600)), Some(7));
        assert!(adds.forget_key(&K2).is_none());
    }

    #[test]
    fn red_forget_and_expiry() {
        let (src, psb) = (vec![1u8; 10], vec![2u8; 10]);
        let mut adds = Adds::default();
        start(&mut adds, K, args(&src, &psb, "x"), t(0)).unwrap();
        assert_eq!(adds.forget_key(&K), Some(7), "the key removed: its add goes");
        start(&mut adds, K, args(&src, &psb, "x"), t(10)).unwrap();
        assert_eq!(adds.part(K, 7, 0, &src[..5], t(10 + 600)), Err(Refusal::BadArg), "a stale add takes no part");
    }

    /// 16 adds may start in a day of uptime (in RAM); the 17th is quota,
    /// until the first of them is a day old. Refusals before `begin` (the
    /// rate, the same Formula already there) start nothing.
    #[test]
    fn sixteen_adds_a_day_of_uptime() {
        let (src, psb) = (vec![1u8; 100], vec![2u8; 100]);
        let mut adds = Adds::default();
        let go = |adds: &mut Adds, id: u32, now: u64| {
            let mut a = args(&src, &psb, "x");
            a.id = id;
            start(adds, K, a, t(now))
        };
        for i in 0..DAY_ADDS as u32 {
            assert_eq!(go(&mut adds, i + 1, 10 + 10 * u64::from(i)), Ok(Pre::New), "{i}");
        }
        assert_eq!(go(&mut adds, 99, 200), Err(Refusal::Quota), "the 17th of the day");
        assert_eq!(go(&mut adds, 99, 10 + 24 * 3600 - 3), Err(Refusal::Quota), "just before the first is a day old");
        assert_eq!(go(&mut adds, 100, 10 + 24 * 3600 + 1), Ok(Pre::New), "the first a day old: room for one");
        assert_eq!(go(&mut adds, 101, 10 + 24 * 3600 + 4), Err(Refusal::Quota));
        // `pre` alone counts nothing (a Formula found already there never begins).
        let mut r = Adds::default();
        for i in 0..40u32 {
            assert_eq!(r.pre(K, &AddArgs { id: 500 + i, ..args(&src, &psb, "x") }, t(1000 + u64::from(i) * 3)), Ok(Pre::New));
        }
    }

    /// A whole add's checks tried again: one in 2 s, as a new add.
    #[test]
    fn whole_retries_are_rated() {
        let (src, psb) = (vec![1u8; 4], vec![2u8; 6]);
        let a = args(&src, &psb, "x");
        let mut adds = Adds::default();
        start(&mut adds, K, a.clone(), t(0)).unwrap();
        adds.part(K, 7, 0, &[src.clone(), psb.clone()].concat(), t(1)).unwrap();
        assert_eq!(adds.pre(K, &a, t(3)), Ok(Pre::Resume { next: 10 }));
        assert_eq!(adds.pre(K, &a, t(3)), Err(Refusal::Rate), "again at once");
        assert_eq!(adds.pre(K, &a, t(6)), Ok(Pre::Resume { next: 10 }));
    }
}
