//! Receiving a sent GIF (`docs/connect-protocol.md` 8; P1-PLAN 2.8): the
//! card's half of a transfer, run by [`Client::transfers`] once a turn from
//! `Client::tick`, never from inside the session's read path (`service` →
//! `act` → `frame`), whose stack is the worker's tightest: the SHA-256s
//! and every card touch (the `.part` files, their writes, the head checks,
//! the rename, the copy's move, the play) happen here.
//!
//! - **The read path** only takes things in: a `send` or `send-cancel` (its
//!   signature already checked) is queued, its `ack` deferred to here; a
//!   chunk is checked against the held transfer without hashing
//!   ([`Transfer::accept`]: the next offset, inside the last credit) and
//!   queued; one outside the credit is queued as such and abandons the
//!   transfer here. What can be queued is bounded by the credit (at most
//!   `caps.win` chunks, 2 KB on the classic) and [`CMD_QUEUE`] commands.
//! - **The rules** are the core's [`Slot`]: one transfer at a time; a
//!   resume only for the key that started it, with the same id, file and
//!   hashes, within 10 minutes (one that already landed is answered as
//!   landed again, never written twice); another key's `send` meanwhile
//!   is `busy`, the same key's next replaces its stalled one; `size + copy_size` charged once when a send
//!   is accepted (its first `.part` open), never again on a resume, never
//!   refunded. Before them: sends switched off ([`super::sends_on`]:
//!   `not-allowed`), a LAN upload or an update under way (`busy`: the LAN
//!   always wins), no card (`no-card`).
//! - **The card**: `/gifs/Sent/<name>.<id>.part` for the GIF's bytes,
//!   `….<id>.copy.part` for the copy's (never a LAN upload's own `.part`
//!   name), each ≤ 1 KB write with the cache builder held off, the upload
//!   pacing ([`crate::pace`]) and a yield; a write waits (the chunk stays
//!   queued, no credit) while a LAN upload runs or the heap is under the
//!   reserve. More credit only after the write returned, never past the
//!   window. At the end: both hashes, the heads (a GIF; a copy that is a
//!   prepared copy: [`xfer::copy_head_ok`]), the GIF renamed to
//!   `/gifs/Sent/<name>` (never over another: a taken name gets the web
//!   app's next name), the copy moved to its place in `/cache/` (a stale
//!   one there goes), the FAT index and the cache told, as a LAN upload
//!   does; then played once (`play`). The last `credit` says how it ended:
//!   `done` with the saved `name`, or `err`. `.part` files left by a restart go at the first turn with
//!   the card usable.
//! - **A session that ends** drops what was queued and closes the file;
//!   the transfer stays for a resume (the sender signs `send` again) until
//!   10 minutes pass untouched, or its key is removed.

use super::client::{core_now, Client};
use super::tls::Phase;
use crate::config;
use crate::stream;
use opendmd_core::connect::keys::KeyId;
use opendmd_core::connect::proto::{self, Accepted, Command as Remote, Refusal, SendArgs};
use opendmd_core::connect::xfer::{self, parse_chunk, Held, Landed, Plan, Slot, XferError};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};

/// Where sent GIFs land (open question 3's default: in no playlist until
/// one is made on the LAN).
pub const SENT: &str = "/sdcard/gifs/Sent";
/// `send` / `send-cancel` commands queued at once; more meanwhile: `busy`.
pub(super) const CMD_QUEUE: usize = 4;
/// Credits a second at most (the server's bucket for `ack` and `credit`
/// together is 50 a second: acks are at most 8).
const CREDITS_PER_S: u32 = 40;
/// Names tried for a taken one.
const NAME_TRIES: usize = 20;

/// What the read path queued for the top level.
pub(super) enum XferIn {
    Cmd(Accepted),
    /// A chunk taken in by [`Transfer::accept`]: the whole frame.
    Chunk(Vec<u8>),
    /// A chunk outside the credit for this transfer: it is abandoned.
    Outside(u32),
}

/// The `.part` being written.
pub(super) struct Out {
    file: File,
    /// The copy's (else the GIF's).
    copy: bool,
    pace_debt: u64,
}

/// The transfer's state on the client.
pub(super) struct Xfer {
    pub slot: Slot,
    pub inbox: Vec<XferIn>,
    pub out: Option<Out>,
    /// A credit to say (the window grew, or a start or a resume).
    pub credit_due: bool,
    pub credits: proto::Bucket,
    /// The `.part` files a restart left were removed.
    pub swept: bool,
    /// A text to say right after the `ack` being answered (a landed
    /// send's `done`, said again).
    pub after_ack: Option<String>,
    /// The command just answered was a resume.
    pub resumed: bool,
}

impl Xfer {
    #[optimize(size)]
    #[inline(never)]
    pub fn new() -> Xfer {
        Xfer {
            slot: Slot::new(window()),
            inbox: Vec::new(),
            out: None,
            credit_due: false,
            credits: proto::Bucket::new(CREDITS_PER_S, core_now()),
            swept: false,
            after_ack: None,
            resumed: false,
        }
    }

    /// How soon the next turn is wanted: at once (5 ms) for queued work,
    /// a while (75 ms) when the head of the queue is a chunk waiting for a
    /// LAN upload, an update or the heap; `None`: nothing to do.
    pub fn due(&self) -> Option<std::time::Duration> {
        if matches!(self.inbox.first(), Some(XferIn::Chunk(_))) && writes_wait() {
            return Some(std::time::Duration::from_millis(75));
        }
        (!self.inbox.is_empty() || self.credit_due).then_some(std::time::Duration::from_millis(5))
    }
}

/// The chunks of credit this panel grants at once (`hello`'s `caps.win`):
/// 2 on the classic (its heap), 4 on the S3. The server trusts it.
pub const fn window() -> u32 {
    if cfg!(esp32s3) {
        4
    } else {
        2
    }
}

/// The two `.part` files of a send.
#[optimize(size)]
#[inline(never)]
fn parts(s: &SendArgs) -> (String, String) {
    (format!("{SENT}/{}.{:08x}.part", s.name, s.id), format!("{SENT}/{}.{:08x}.copy.part", s.name, s.id))
}

/// Is the card there to write to (an emulator image: the lane's switch)?
#[optimize(size)]
#[inline(never)]
fn card_usable(c: &Client) -> bool {
    #[cfg(feature = "connect-test")]
    if super::TEST_NO_CARD.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    c.link.status.lock().unwrap().sd_usable()
}

/// Card writes wait: a LAN upload (it always wins), an update, the heap
/// under the reserve.
#[optimize(size)]
#[inline(never)]
fn writes_wait() -> bool {
    stream::uploads() > 0 || super::updating() || crate::state::free_heap() < config::HEAP_RESERVE + 4096
}

impl Client {
    /// From the read path: a chunk frame, checked against the held
    /// transfer without hashing, and queued. Anything else is dropped.
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn chunk_in(&mut self, f: Vec<u8>) {
        let Ok(c) = parse_chunk(&f) else { return };
        let id = c.id;
        let Some(h) = self.xfer.slot.held.as_mut() else { return };
        match h.t.accept(&c, core_now()) {
            Ok(()) => self.xfer.inbox.push(XferIn::Chunk(f)),
            Err(XferError::OutsideCredit) => {
                if !matches!(self.xfer.inbox.last(), Some(XferIn::Outside(_))) {
                    self.xfer.inbox.push(XferIn::Outside(id));
                }
            }
            // Another transfer's (one that ended): nothing.
            Err(_) => {}
        }
    }

    /// From the read path: a `send` or `send-cancel`, its `ack` from the
    /// top level; `Some(ack)` now only if too many are queued.
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn xfer_cmd_in(&mut self, acc: Accepted) -> Option<String> {
        if self.xfer.inbox.iter().filter(|i| matches!(i, XferIn::Cmd(_))).count() >= CMD_QUEUE {
            return Some(proto::ack_err(&acc.k, Some(acc.n), Refusal::Busy));
        }
        self.xfer.inbox.push(XferIn::Cmd(acc));
        None
    }

    /// The session ended: what was queued goes, the file closes, the
    /// transfer waits for a resume.
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn xfer_session_end(&mut self) {
        self.xfer.inbox.clear();
        self.xfer.out = None;
        self.xfer.credit_due = false;
        if let Some(h) = self.xfer.slot.held.as_mut() {
            h.t.rewind();
        }
    }

    /// The turn's transfer work (see the module's notes).
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn transfers(&mut self) {
        let now = core_now();
        // Once a boot, and again each time the card comes back.
        if !card_usable(self) {
            self.xfer.swept = false;
        } else if !self.xfer.swept && self.xfer.slot.held.is_none() && stream::uploads() == 0 {
            self.xfer.swept = true;
            sweep();
        }
        if let Some(h) = self.xfer.slot.expire(now) {
            log::info!("connect: a send untouched for 10 minutes abandoned");
            self.abandon(h, None);
        }
        let keys = &self.keys;
        if let Some(h) = self.xfer.slot.drop_unless(|k| keys.find(k).is_some()) {
            log::info!("connect: a send's controller was removed: abandoned");
            self.abandon(h, Some("not-allowed"));
        }
        let mut rest = Vec::new();
        // Chunks queued behind a resume were taken in before it rewound the
        // stream: none of them is written (the sender sends again from the
        // offset the resume's credit gives).
        let mut after_resume = false;
        for item in std::mem::take(&mut self.xfer.inbox) {
            if !rest.is_empty() {
                rest.push(item);
                continue;
            }
            if after_resume && matches!(item, XferIn::Chunk(_) | XferIn::Outside(_)) {
                continue;
            }
            match item {
                XferIn::Cmd(acc) => {
                    let ack = self.xfer_cmd(acc);
                    after_resume |= std::mem::take(&mut self.xfer.resumed);
                    self.say(&ack);
                    if let Some(m) = self.xfer.after_ack.take() {
                        self.say(&m);
                    }
                    // The credit right behind its ack.
                    self.say_credit();
                }
                XferIn::Outside(id) => {
                    if self.xfer.slot.held.as_ref().is_some_and(|h| h.t.send.id == id) {
                        let h = self.xfer.slot.held.take().expect("held");
                        error_line("a chunk outside the credit: the send abandoned");
                        self.abandon(h, Some("bad-arg"));
                    }
                }
                XferIn::Chunk(f) if writes_wait() => rest.push(XferIn::Chunk(f)),
                XferIn::Chunk(f) => self.write(&f),
            }
            if self.conn.is_none() {
                // The session ended meanwhile (a write failed): nothing more.
                rest.clear();
                break;
            }
        }
        self.xfer.inbox = rest;
        if self.xfer.slot.held.as_ref().is_some_and(|h| h.t.complete()) {
            self.finish();
        }
        self.say_credit();
        super::SENDING.store(self.xfer.slot.held.is_some(), std::sync::atomic::Ordering::Relaxed);
    }

    /// A text to the server, if the session is open (a failure ends it).
    #[optimize(size)]
    #[inline(never)]
    fn say(&mut self, s: &str) {
        let Some(conn) = self.conn.as_mut().filter(|c| c.phase == Phase::Open) else { return };
        if let Err(e) = conn.text(s) {
            self.end(e);
        }
    }

    /// The held transfer's credit, if one is due and the budget allows.
    #[optimize(size)]
    #[inline(never)]
    fn say_credit(&mut self) {
        if !self.xfer.credit_due || !self.conn.as_ref().is_some_and(|c| c.phase == Phase::Open) {
            return;
        }
        let Some(h) = self.xfer.slot.held.as_ref() else {
            self.xfer.credit_due = false;
            return;
        };
        if h.t.complete() || !self.xfer.credits.take(core_now()) {
            return;
        }
        let m = h.t.credit_for(&h.k);
        self.xfer.credit_due = false;
        self.say(&m);
    }

    /// A queued `send` or `send-cancel`: its `ack`.
    #[optimize(size)]
    #[inline(never)]
    fn xfer_cmd(&mut self, acc: Accepted) -> String {
        let Accepted { k, n, cmd } = acc;
        let r = match cmd {
            Remote::Send(s) => self.send_start(k, s),
            Remote::SendCancel { id } => match self.xfer.slot.cancel(&k, id) {
                Ok(Some(h)) => {
                    log::info!("connect: a send cancelled by its controller");
                    self.abandon(h, None);
                    Ok(None)
                }
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            },
            _ => Err(Refusal::NotAllowed),
        };
        log::info!("connect: command n={n} from {}: {}", opendmd_core::connect::codec::hex(&k[..4]), if r.is_ok() { "ok" } else { "refused" });
        match r {
            Ok(r) => proto::ack_ok(&k, n, r.as_deref()),
            Err(e) => {
                log::info!("connect: send refused: {}", e.code());
                proto::ack_err(&k, Some(n), e)
            }
        }
    }

    /// A `send` from `k`: refused, resumed or started (its first `.part`
    /// open, then charged).
    #[optimize(size)]
    #[inline(never)]
    fn send_start(&mut self, k: KeyId, s: SendArgs) -> Result<Option<String>, Refusal> {
        if !super::sends_on() {
            return Err(Refusal::NotAllowed);
        }
        if stream::uploads() > 0 || super::updating() {
            return Err(Refusal::Busy);
        }
        if !card_usable(self) {
            return Err(Refusal::NoCard);
        }
        let now = core_now();
        if let Some(h) = self.xfer.slot.expire(now) {
            self.abandon(h, None);
        }
        let (plan, _) = self.xfer.slot.check(&k, &s, now)?;
        match plan {
            Plan::Resume => {
                self.xfer.slot.resume(&s, now);
                self.xfer.resumed = true;
                self.xfer.out = None;
                self.xfer.credit_due = true;
                let off = self.xfer.slot.held.as_ref().map_or(0, |h| h.t.offset());
                log::info!("connect: a send resumed at {off} B");
                Ok(Some(format!(r#"{{"offset":{off},"resumed":true}}"#)))
            }
            Plan::Landed { total, name } => {
                // Signed again after it landed (its `done` was lost): the
                // same answer, never a second file.
                self.xfer.after_ack = Some(xfer::done_message(s.id, total, &k, &name));
                Ok(Some(format!(r#"{{"offset":{total}}}"#)))
            }
            Plan::Start { .. } => {
                // The stalled one it replaces stops writing first.
                self.xfer.out = None;
                let (gif, _) = parts(&s);
                let _ = fs::create_dir_all(SENT);
                let file = {
                    let _held = crate::cache_builder::busy();
                    File::create(&gif)
                };
                let file = match file {
                    Ok(f) => f,
                    Err(e) => {
                        error_line(&format!("a send's file could not be made: {e}"));
                        return Err(Refusal::Busy);
                    }
                };
                let (size, copy) = (s.size, s.copy_size);
                match self.xfer.slot.start(k, s, now) {
                    Ok(old) => {
                        if let Some(old) = old {
                            log::info!("connect: a stalled send replaced by its controller's next");
                            self.abandon(old, None);
                        }
                    }
                    Err(e) => {
                        drop(file);
                        let _ = fs::remove_file(&gif);
                        return Err(e);
                    }
                }
                self.xfer.out = Some(Out { file, copy: false, pace_debt: 0 });
                self.xfer.credit_due = true;
                log::info!("connect: a send started ({size} B, copy {copy} B)");
                Ok(Some(r#"{"offset":0}"#.into()))
            }
        }
    }

    /// One queued chunk: hashed, written, more credit.
    #[optimize(size)]
    #[inline(never)]
    fn write(&mut self, f: &[u8]) {
        let Ok(c) = parse_chunk(f) else { return };
        // (A chunk taken in before a resume rewound the stream is not the
        // next one any more: dropped, its bytes asked for again.)
        let Some(h) = self.xfer.slot.held.as_mut().filter(|h| h.t.send.id == c.id && h.t.offset() == c.offset) else { return };
        let at = h.t.offset();
        let size = h.t.send.size;
        // Hashed whole or not at all: one not wholly among the bytes taken
        // in (queued behind a resume, which rewound the stream) is dropped,
        // never written.
        let Some(split) = h.t.commit(c.data) else { return };
        let (gif, copy) = (&c.data[..split], &c.data[split..]);
        let s = h.t.send.clone();
        let r = {
            let _held = crate::cache_builder::busy();
            let mut r = Ok(());
            if !gif.is_empty() {
                r = self.part_write(&s, false, at, gif);
            }
            if r.is_ok() && !copy.is_empty() {
                r = self.part_write(&s, true, at.max(size), copy);
            }
            r
        };
        if let Err(e) = r {
            error_line(&format!("a send's write failed: {e}"));
            if let Some(h) = self.xfer.slot.held.take() {
                self.abandon(h, Some("busy"));
            }
            return;
        }
        if let Some(out) = self.xfer.out.as_mut() {
            crate::pace::after_read(c.data.len(), &mut out.pace_debt);
        }
        std::thread::yield_now();
        if self.xfer.slot.held.as_ref().is_some_and(|h| h.t.complete()) {
            // At once: a `send` queued behind the last chunk must not find
            // it unfinished.
            return self.finish();
        }
        if self.xfer.slot.held.as_mut().is_some_and(|h| h.t.grant_more()) {
            self.xfer.credit_due = true;
        }
    }

    /// Append `data` (at stream offset `at`) to the GIF's or the copy's
    /// `.part`, opening it if it is not the one open (a resume: the file
    /// must hold exactly the bytes before `at`).
    #[optimize(size)]
    #[inline(never)]
    fn part_write(&mut self, s: &SendArgs, copy: bool, at: u32, data: &[u8]) -> std::io::Result<()> {
        #[cfg(feature = "connect-test")]
        if super::TEST_FAIL_WRITE.swap(false, std::sync::atomic::Ordering::Relaxed) {
            return Err(std::io::Error::other("a write failed (the lane's switch)"));
        }
        if self.xfer.out.as_ref().is_some_and(|o| o.copy != copy) {
            self.xfer.out = None;
        }
        if self.xfer.out.is_none() {
            let (gif_part, copy_part) = parts(s);
            let (path, before) = if copy { (copy_part, at - s.size) } else { (gif_part, at) };
            let file = if before == 0 { File::create(&path)? } else { OpenOptions::new().append(true).open(&path)? };
            if file.metadata()?.len() != u64::from(before) {
                return Err(std::io::Error::other("the .part does not hold what was written"));
            }
            self.xfer.out = Some(Out { file, copy, pace_debt: 0 });
        }
        self.xfer.out.as_mut().expect("open").file.write_all(data)
    }

    /// Abandon a transfer: its files go; `end`: say so to its controller.
    #[optimize(size)]
    #[inline(never)]
    fn abandon(&mut self, h: Held, end: Option<&str>) {
        self.xfer.out = None;
        self.xfer.inbox.retain(|i| !matches!(i, XferIn::Chunk(_) | XferIn::Outside(_)));
        let (gif, copy) = parts(&h.t.send);
        {
            let _held = crate::cache_builder::busy();
            let _ = fs::remove_file(&gif);
            let _ = fs::remove_file(&copy);
        }
        if let Some(e) = end {
            let m = h.t.err_for(&h.k, e);
            self.say(&m);
        }
    }

    /// All the bytes are in: the hashes, the heads, the rename, the copy,
    /// the play.
    #[optimize(size)]
    #[inline(never)]
    fn finish(&mut self) {
        self.xfer.out = None;
        let Some(h) = self.xfer.slot.held.take() else { return };
        if h.t.finish().is_err() {
            error_line("a send's bytes do not match its signed hashes: refused, the file removed");
            return self.abandon(h, Some("bad-arg"));
        }
        let (gif_part, copy_part) = parts(&h.t.send);
        let heads_ok = {
            let _held = crate::cache_builder::busy();
            head(&gif_part).is_some_and(|b| xfer::gif_head_ok(&b)) && (h.t.send.copy_size == 0 || head(&copy_part).is_some_and(|b| xfer::copy_head_ok(&b)))
        };
        if !heads_ok {
            error_line("a sent file is not a GIF, or its copy not a prepared copy: refused, removed");
            return self.abandon(h, Some("bad-arg"));
        }
        let Some(name) = place(&gif_part, &h.t.send.name, h.t.send.id) else {
            return self.abandon(h, Some("busy"));
        };
        let card = format!("/gifs/Sent/{name}");
        let full = format!("{}{card}", config::SD_MOUNT);
        crate::fatidx::note_written(&full);
        crate::cache_builder::drop_failed(&card);
        if let Some(dest) = opendmd_core::layout::cache_path(&card).map(|p| format!("{}{p}", config::SD_MOUNT)) {
            let _held = crate::cache_builder::busy();
            if h.t.send.copy_size > 0 {
                let moved = std::path::Path::new(&dest).parent().map_or(Ok(()), fs::create_dir_all).map_err(anyhow::Error::from).and_then(|_| crate::files::replace(&copy_part, &dest));
                match moved {
                    Ok(()) => crate::fatidx::note_written(&dest),
                    // The GIF plays as it is (the builder may make a copy later).
                    Err(e) => error_line(&format!("a sent copy could not be put in place: {e:#}")),
                }
            } else {
                // A copy for an earlier GIF of that name would play instead.
                let _ = fs::remove_file(&dest);
            }
            crate::cache::invalidate();
        }
        let _ = fs::remove_file(&copy_part);
        log::info!("connect: a send landed: {card} ({} B, copy {} B)", h.t.send.size, h.t.send.copy_size);
        let m = h.t.done_for(&h.k, &name);
        self.say(&m);
        self.xfer.slot.landed = Some(Landed { k: h.k, send: h.t.send.clone(), name: name.clone(), at: core_now() });
        if h.t.send.play {
            // Once, as a LAN "Show on panel" (the same validation and play).
            match self.play(card) {
                Ok(_) => {}
                Err(e) => log::info!("connect: a sent GIF did not play: {}", e.code()),
            }
        }
    }
}

/// The first bytes of a file (up to [`xfer::HEAD_MAX`]).
#[optimize(size)]
#[inline(never)]
fn head(path: &str) -> Option<Vec<u8>> {
    let mut f = File::open(path).ok()?;
    let mut b = vec![0u8; xfer::HEAD_MAX];
    let mut n = 0;
    while n < b.len() {
        match f.read(&mut b[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(_) => return None,
        }
    }
    b.truncate(n);
    Some(b)
}

/// Rename `part` to `/gifs/Sent/<name>`, never over another file: a taken
/// name gets the next one (FAT's rename refuses an existing name, so the
/// rename is the check). The name used.
#[optimize(size)]
#[inline(never)]
fn place(part: &str, name: &str, id: u32) -> Option<String> {
    let _held = crate::cache_builder::busy();
    let mut name = name.to_string();
    for k in 0..=NAME_TRIES {
        if k == NAME_TRIES {
            // That many taken: the stem and the transfer's id.
            name = xfer::fallback_name(&name, id);
        }
        let dest = format!("{SENT}/{name}");
        crate::fatidx::touched(&dest);
        match fs::rename(part, &dest) {
            Ok(()) => return Some(name),
            Err(e) if e.kind() == ErrorKind::AlreadyExists && k < NAME_TRIES => name = xfer::next_name(&name),
            Err(e) => {
                error_line(&format!("a sent GIF could not be put in place: {e}"));
                return None;
            }
        }
    }
    error_line(&format!("a sent GIF: {} names taken", NAME_TRIES + 1));
    None
}

/// The `.part` files a restart left in `/gifs/Sent/` go.
#[optimize(size)]
#[inline(never)]
fn sweep() {
    let Ok(dir) = fs::read_dir(SENT) else { return };
    let names: Vec<String> = dir.filter_map(|e| e.ok()).filter_map(|e| e.file_name().to_str().map(str::to_string)).filter(|n| xfer::is_send_part(n)).collect();
    let _held = crate::cache_builder::busy();
    for n in names {
        let _ = fs::remove_file(format!("{SENT}/{n}"));
        log::info!("connect: a send's {n} left by a restart: removed");
    }
}

#[optimize(size)]
#[inline(never)]
fn error_line(e: &str) {
    super::error(e.to_string());
}
