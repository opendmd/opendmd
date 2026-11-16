//! Sending a GIF (P1-PLAN 2.8): the rules of a transfer, without the card.
//!
//! A signed `send` announces the GIF and, optionally, its prepared copy
//! (each ≤ [`MAX_FILE`]). Their bytes then come as one stream, the GIF's
//! first, in binary chunk frames (`0x02`, [`parse_chunk`]) of at most
//! [`MAX_CHUNK`] bytes, strictly in order and only inside the panel's last
//! `credit`. The panel grants more ([`Transfer::written`]) only after the
//! card write returned. At the end both SHA-256s must match, or the
//! `.part` goes. [`Quota`] is the panel's 8 MB per rolling 24 h.
//!
//! [`Slot`] is the panel's one transfer and its rules for a `send`: who may
//! resume it (only the key that started it, the same id, file and hashes,
//! within [`RESUME_FOR`]), what a second `send` does while one is held
//! (`busy`, from any key, until `send-cancel` or 10 minutes untouched), and the quota (`size +
//! copy_size` charged once, when a send is accepted; never again on a
//! resume, never refunded). The card (the `.part` files, the rename, the
//! play) is the firmware's (`firmware/src/connect/xfer.rs`).
//!
//! The credit never runs more than the window past what was written: the
//! server forwards a chunk only inside the last `credit` and trusts
//! `caps.win` for where a resumed stream may start, so [`Transfer::upto`]
//! is always at most `offset + window × MAX_CHUNK`.

use super::codec::hex;
use super::json::quote;
use super::keys::KeyId;
use super::proto::{Refusal, SendArgs};
use core::time::Duration;
use sha2::{Digest, Sha256};

/// A chunk's largest payload.
pub const MAX_CHUNK: usize = 1024;
/// The largest GIF, or copy, one send may carry.
pub const MAX_FILE: u32 = 512 * 1024;
/// A chunk frame: type, transfer id, offset, data.
pub const CHUNK_HEAD: usize = 1 + 4 + 4;
pub const TYPE_CHUNK: u8 = 0x02;
/// The most chunks of credit a panel grants at a time.
pub const MAX_WINDOW: u32 = 64;
/// How long a broken transfer can be resumed.
pub const RESUME_FOR: Duration = Duration::from_secs(10 * 60);
/// The panel's quota.
pub const QUOTA_BYTES: u64 = 8 * 1024 * 1024;
pub const QUOTA_WINDOW: Duration = Duration::from_secs(24 * 3600);

/// A chunk frame, read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk<'a> {
    pub id: u32,
    pub offset: u32,
    pub data: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XferError {
    /// Not a chunk frame, or an empty or oversize chunk.
    BadFrame,
    /// No transfer with that id.
    UnknownTransfer,
    /// Not at the next expected offset, or past the last credit or the end:
    /// the transfer is abandoned.
    OutsideCredit,
    /// The bytes do not hash to what `send` announced: the `.part` goes.
    Tampered,
    /// Not all the bytes are here yet.
    Incomplete,
}

/// Read a binary chunk frame.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn parse_chunk(frame: &[u8]) -> Result<Chunk<'_>, XferError> {
    if frame.len() <= CHUNK_HEAD || frame.len() > CHUNK_HEAD + MAX_CHUNK || frame[0] != TYPE_CHUNK {
        return Err(XferError::BadFrame);
    }
    let id = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]);
    let offset = u32::from_be_bytes([frame[5], frame[6], frame[7], frame[8]]);
    Ok(Chunk { id, offset, data: &frame[CHUNK_HEAD..] })
}

/// A chunk frame, for the sender (and the tests).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn chunk_frame(id: u32, offset: u32, data: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(CHUNK_HEAD + data.len());
    f.push(TYPE_CHUNK);
    f.extend_from_slice(&id.to_be_bytes());
    f.extend_from_slice(&offset.to_be_bytes());
    f.extend_from_slice(data);
    f
}

/// One transfer in progress: where it is, its credit, its hash states.
/// `received` runs ahead of `offset` by the chunks taken in
/// ([`accept`](Self::accept)) and not yet hashed and written
/// ([`commit`](Self::commit)): the panel reads chunks under the session's
/// read path and hashes them at its turn's top level.
#[derive(Clone)]
pub struct Transfer {
    pub send: SendArgs,
    offset: u32,
    received: u32,
    upto: u32,
    window: u32,
    gif: Sha256,
    copy: Sha256,
    /// When it last made progress (for [`RESUME_FOR`]).
    pub touched: Duration,
}

impl Transfer {
    /// Start a transfer for a validated `send`; `window`: chunks of credit
    /// at a time (2 on the classic, 4 or more on the S3).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn start(send: SendArgs, window: u32, now: Duration) -> Transfer {
        let mut t = Transfer { send, offset: 0, received: 0, upto: 0, window: window.clamp(1, MAX_WINDOW), gif: Sha256::new(), copy: Sha256::new(), touched: now };
        t.upto = t.grant();
        t
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn total(&self) -> u32 {
        self.send.size + self.send.copy_size
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn offset(&self) -> u32 {
        self.offset
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn upto(&self) -> u32 {
        self.upto
    }
    /// Bytes taken in (written or waiting to be).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn received(&self) -> u32 {
        self.received
    }
    /// The window, in chunks.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn window(&self) -> u32 {
        self.window
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn complete(&self) -> bool {
        self.offset >= self.total()
    }

    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn grant(&self) -> u32 {
        self.offset.saturating_add(self.window.saturating_mul(MAX_CHUNK as u32)).min(self.total())
    }

    /// The `credit` message for the current credit.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn credit_message(&self) -> String {
        format!(r#"{{"t":"credit","id":{},"upto":{}}}"#, self.send.id, self.upto)
    }

    /// Take one chunk: it must be this transfer's, at the next offset, and
    /// end inside the credit. On success the caller writes `data` to the
    /// card (the GIF's `.part` for the bytes below `send.size`, the copy's
    /// after) and then calls [`written`](Self::written). ([`accept`] and
    /// [`commit`] in one.)
    ///
    /// [`accept`]: Self::accept
    /// [`commit`]: Self::commit
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn chunk(&mut self, c: &Chunk<'_>, now: Duration) -> Result<(), XferError> {
        self.accept(c, now)?;
        self.commit(c.data);
        Ok(())
    }

    /// Take a chunk in, unhashed (cheap: the read path's part): it must be
    /// this transfer's, at the next offset after those taken in, not empty,
    /// and end inside the last credit. Any other chunk abandons the
    /// transfer (the caller's part).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn accept(&mut self, c: &Chunk<'_>, now: Duration) -> Result<(), XferError> {
        if c.id != self.send.id {
            return Err(XferError::UnknownTransfer);
        }
        let end = u64::from(c.offset) + c.data.len() as u64;
        if c.offset != self.received || end > u64::from(self.upto) || c.data.is_empty() {
            return Err(XferError::OutsideCredit);
        }
        self.received = end as u32;
        self.touched = now;
        Ok(())
    }

    /// Hash the next accepted bytes (in order: the caller's queue); how
    /// many of them are the GIF's (the rest are the copy's). The caller
    /// writes them, then asks for more credit ([`grant`](Self::grant_more)).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn commit(&mut self, data: &[u8]) -> usize {
        let n = (self.received.saturating_sub(self.offset) as usize).min(data.len());
        let data = &data[..n];
        let split = (self.send.size.saturating_sub(self.offset) as usize).min(data.len());
        self.gif.update(&data[..split]);
        self.copy.update(&data[split..]);
        self.offset += n as u32;
        split
    }

    /// Chunks taken in but not committed are gone (a session ended with
    /// them unread): the stream starts again from what was written.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn rewind(&mut self) {
        self.received = self.offset;
        self.upto = self.upto.min(self.grant());
    }

    /// More credit after a write, if there is more to give: `true` if
    /// [`upto`](Self::upto) grew (then say [`credit_for`](Self::credit_for)).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn grant_more(&mut self) -> bool {
        let next = self.grant();
        if next > self.upto {
            self.upto = next;
            return true;
        }
        false
    }

    /// The `credit` message, with the key it is for (the server routes a
    /// transfer's credit by key and id).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn credit_for(&self, k: &KeyId) -> String {
        format!(r#"{{"t":"credit","id":{},"upto":{},"k":"{}"}}"#, self.send.id, self.upto, hex(k))
    }

    /// The transfer's last `credit` when it landed: `done` and the file
    /// name it was saved under (after any `_2`), `upto` the total.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn done_for(&self, k: &KeyId, name: &str) -> String {
        done_message(self.send.id, self.total(), k, name)
    }

    /// The transfer's last `credit` when it failed: `err` (`bad-arg` for
    /// bytes that miss the signed hashes, are not a GIF or a prepared copy,
    /// or a chunk outside the credit; `busy` for a card that failed;
    /// `not-allowed` for a key removed meanwhile); `upto` where it was.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn err_for(&self, k: &KeyId, err: &str) -> String {
        format!(r#"{{"t":"credit","id":{},"upto":{},"k":"{}","err":{}}}"#, self.send.id, self.upto, hex(k), quote(err))
    }

    /// The card write returned: more credit if any is left to give. `None`
    /// when the stream is complete (call [`finish`](Self::finish)).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn written(&mut self) -> Option<String> {
        if self.offset >= self.total() {
            return None;
        }
        let next = self.grant();
        if next > self.upto {
            self.upto = next;
        }
        Some(self.credit_message())
    }

    /// All bytes in: both hashes must match.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn finish(&self) -> Result<(), XferError> {
        if self.offset != self.total() {
            return Err(XferError::Incomplete);
        }
        let gif: [u8; 32] = self.gif.clone().finalize().into();
        if gif != self.send.sha256 {
            return Err(XferError::Tampered);
        }
        if self.send.copy_size > 0 {
            let copy: [u8; 32] = self.copy.clone().finalize().into();
            if Some(copy) != self.send.copy_sha256 {
                return Err(XferError::Tampered);
            }
        }
        Ok(())
    }

    /// A `send` signed again after a reconnect resumes this transfer if it
    /// names the same id, file and hashes, within [`RESUME_FOR`]; the
    /// credit then runs from the offset reached.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn resume(&mut self, again: &SendArgs, now: Duration) -> bool {
        if *again != self.send || now.saturating_sub(self.touched) > RESUME_FOR {
            return false;
        }
        self.received = self.offset;
        self.upto = self.grant();
        self.touched = now;
        true
    }
}

impl core::fmt::Debug for Transfer {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Transfer")
            .field("id", &self.send.id)
            .field("sha256", &hex(&self.send.sha256))
            .field("offset", &self.offset)
            .field("received", &self.received)
            .field("upto", &self.upto)
            .finish()
    }
}

/// The panel's quota: [`QUOTA_BYTES`] per rolling [`QUOTA_WINDOW`], kept
/// in RAM as 24 hourly buckets.
#[derive(Debug, Clone, Default)]
pub struct Quota {
    /// (hour since the caller's epoch, bytes) pairs.
    hours: Vec<(u64, u64)>,
}

impl Quota {
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    fn prune(&mut self, now: Duration) {
        let hour = now.as_secs() / 3600;
        let span = QUOTA_WINDOW.as_secs() / 3600;
        self.hours.retain(|&(h, _)| h + span > hour);
    }
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn used(&mut self, now: Duration) -> u64 {
        self.prune(now);
        self.hours.iter().map(|&(_, b)| b).sum()
    }
    /// Would `bytes` fit now?
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn fits(&mut self, bytes: u64, now: Duration) -> bool {
        self.used(now) + bytes <= QUOTA_BYTES
    }
    /// Charge `bytes` if they fit; `false` (and nothing charged) if not.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn charge(&mut self, bytes: u64, now: Duration) -> bool {
        if self.used(now) + bytes > QUOTA_BYTES {
            return false;
        }
        let hour = now.as_secs() / 3600;
        match self.hours.last_mut() {
            Some((h, b)) if *h == hour => *b += bytes,
            _ => self.hours.push((hour, bytes)),
        }
        true
    }
}

/// A transfer held by the panel and the key that started it.
#[derive(Debug, Clone)]
pub struct Held {
    pub t: Transfer,
    pub k: KeyId,
}

/// What a `send` that [`Slot::check`] took is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// The held transfer, signed again by its key (after a reconnect):
    /// credit from where it was, nothing charged.
    Resume,
    /// A new transfer; `replaces`: the same key's held (stalled) one goes,
    /// no refund.
    Start { replaces: bool },
    /// The same `send` as one that already landed (within [`RESUME_FOR`]):
    /// answered as landed again (`r.offset` the total and its `done`
    /// credit), never a second file, nothing charged.
    Landed { total: u32, name: String },
}

/// The last transfer that landed: who sent it, what, under which name.
#[derive(Debug, Clone)]
pub struct Landed {
    pub k: KeyId,
    pub send: SendArgs,
    pub name: String,
    pub at: Duration,
}

/// The panel's one transfer, and its quota (P1-PLAN 2.8 and the F1
/// review's rules).
#[derive(Debug, Clone)]
pub struct Slot {
    pub held: Option<Held>,
    pub landed: Option<Landed>,
    pub quota: Quota,
    window: u32,
}

impl Slot {
    /// `window`: the panel's `caps.win`, the most chunks of credit at once.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn new(window: u32) -> Slot {
        Slot { held: None, landed: None, quota: Quota::default(), window: window.clamp(1, MAX_WINDOW) }
    }

    /// A held transfer untouched for [`RESUME_FOR`] goes: the caller removes
    /// its `.part` files.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn expire(&mut self, now: Duration) -> Option<Held> {
        if self.held.as_ref().is_some_and(|h| now.saturating_sub(h.t.touched) > RESUME_FOR) {
            return self.held.take();
        }
        None
    }

    /// What a validated `send` from `k` would do, or why not (nothing
    /// changes; an expired transfer is dropped first, as [`expire`]):
    ///
    /// - the held transfer's id from its own key with the same arguments:
    ///   [`Plan::Resume`]; with others: `bad-arg`;
    /// - the same `send` as the last one that landed, from its key, within
    ///   [`RESUME_FOR`]: [`Plan::Landed`];
    /// - anything while another key's transfer is held: `busy` (resume is
    ///   only for the key that started it);
    /// - another id from the held transfer's own key: a new start that
    ///   replaces the stalled one (no refund);
    /// - a new start whose `size + copy_size` does not fit the quota:
    ///   `quota`.
    ///
    /// [`expire`]: Self::expire
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn check(&mut self, k: &KeyId, s: &SendArgs, now: Duration) -> Result<(Plan, Option<Held>), Refusal> {
        let expired = self.expire(now);
        if let Some(l) = self.landed.as_ref().filter(|l| l.k == *k && l.send == *s && now.saturating_sub(l.at) <= RESUME_FOR) {
            return Ok((Plan::Landed { total: l.send.size + l.send.copy_size, name: l.name.clone() }, expired));
        }
        let replaces = match &self.held {
            Some(h) if h.k != *k => return Err(Refusal::Busy),
            Some(h) if h.t.send.id == s.id => {
                return if h.t.send == *s { Ok((Plan::Resume, expired)) } else { Err(Refusal::BadArg) };
            }
            Some(_) => true,
            None => false,
        };
        if !self.quota.fits(u64::from(s.size) + u64::from(s.copy_size), now) {
            return Err(Refusal::Quota);
        }
        Ok((Plan::Start { replaces }, expired))
    }

    /// Resume the held transfer (after [`Plan::Resume`]): the credit from
    /// the offset written. Nothing is charged.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn resume(&mut self, s: &SendArgs, now: Duration) -> bool {
        self.held.as_mut().is_some_and(|h| h.t.resume(s, now))
    }

    /// Start a transfer (after [`Plan::Start`], once its first `.part` is
    /// open): `size + copy_size` charged now, once. Any held one (a caller
    /// that skipped `check`), to clean up. `Err(quota)` if the quota no longer fits (nothing changed).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn start(&mut self, k: KeyId, s: SendArgs, now: Duration) -> Result<Option<Held>, Refusal> {
        if !self.quota.charge(u64::from(s.size) + u64::from(s.copy_size), now) {
            return Err(Refusal::Quota);
        }
        Ok(self.held.replace(Held { t: Transfer::start(s, self.window, now), k }))
    }

    /// `send-cancel {id}` from `k`: the held transfer goes if it is that
    /// id's and `k`'s (returned, to clean up; no refund). Another key's:
    /// `not-allowed`. No such transfer: nothing to do.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn cancel(&mut self, k: &KeyId, id: u32) -> Result<Option<Held>, Refusal> {
        match &self.held {
            Some(h) if h.t.send.id == id && h.k != *k => Err(Refusal::NotAllowed),
            Some(h) if h.t.send.id == id => Ok(self.held.take()),
            _ => Ok(None),
        }
    }

    /// The held transfer goes if its key is no longer on the list.
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn drop_unless(&mut self, listed: impl Fn(&KeyId) -> bool) -> Option<Held> {
        if self.held.as_ref().is_some_and(|h| !listed(&h.k)) {
            return self.held.take();
        }
        None
    }
}

/// A landed transfer's last `credit` (also said again for a `send` signed
/// again after it landed: [`Plan::Landed`]).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn done_message(id: u32, total: u32, k: &KeyId, name: &str) -> String {
    format!(r#"{{"t":"credit","id":{id},"upto":{total},"k":"{}","done":true,"name":{}}}"#, hex(k), quote(name))
}

/// The next name to try when one is taken (the web app's `nextName`): the
/// GIF's number up by one, keeping its width (`x_09.gif` → `x_10.gif`,
/// `x_01_foo.gif` → `x_02_foo.gif`), else a `_2` before `.gif`.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn next_name(name: &str) -> String {
    let Some(stem) = name.len().checked_sub(4).filter(|&i| name.get(i..).is_some_and(|e| e.eq_ignore_ascii_case(".gif"))).map(|i| &name[..i]) else {
        return name.to_string();
    };
    let ext = &name[stem.len()..];
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let bump = |head: &str, num: &str, tail: &str| -> Option<String> {
        let v: u64 = num.parse().ok()?;
        Some(format!("{head}{:0w$}{tail}", v.checked_add(1)?, w = num.len()))
    };
    if let Some(i) = stem.rfind('_') {
        let after = &stem[i + 1..];
        if digits(after) {
            if let Some(n) = bump(&stem[..=i], after, ext) {
                return n;
            }
        } else if let Some(j) = stem[..i].rfind('_') {
            let mid = &stem[j + 1..i];
            if digits(mid) {
                if let Some(n) = bump(&stem[..=j], mid, &format!("{}{ext}", &stem[i..])) {
                    return n;
                }
            }
        }
    }
    format!("{stem}_2{ext}")
}

/// The most of a file's head [`gif_head_ok`] and [`copy_head_ok`] read: the
/// header, the screen descriptor, a 256-colour palette and the hint.
pub const HEAD_MAX: usize = 13 + 3 * 256 + 19;

/// A sent GIF's head: `GIF87a` or `GIF89a` and a whole screen descriptor
/// (what the player's open checks before it plays one, `GIF8`, and more).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn gif_head_ok(head: &[u8]) -> bool {
    head.len() >= 13 && (head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a"))
}

/// A sent prepared copy's head (`docs/cache-format.md`): a GIF89a with its
/// `OPENDMD1LZW` hint right after the global palette, at a cap no wider
/// than a prepared copy's ([`crate::gifslim::PREPARED`]): anything else
/// would make the player play a copy in more heap than the GIF it
/// stands for, so it is refused and the send fails.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn copy_head_ok(head: &[u8]) -> bool {
    use crate::gifslim::{HINT_APP_ID, MIN_CAP, PREPARED};
    if head.len() < 13 || !head.starts_with(b"GIF89a") {
        return false;
    }
    let flags = head[10];
    let at = 13 + if flags & 0x80 != 0 { 3usize << ((flags & 7) + 1) } else { 0 };
    let Some(h) = head.get(at..at + 19) else { return false };
    h[..3] == [0x21, 0xFF, 11] && h[3..14].eq_ignore_ascii_case(HINT_APP_ID) && h[14] >= 2 && h[15] == 1 && (MIN_CAP..=PREPARED.cap).contains(&h[16])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect::codec::sha256;

    fn send(gif: &[u8], copy: &[u8]) -> SendArgs {
        SendArgs {
            id: 77,
            name: "cat.gif".into(),
            size: gif.len() as u32,
            sha256: sha256(&[gif]),
            copy_size: copy.len() as u32,
            copy_sha256: (!copy.is_empty()).then(|| sha256(&[copy])),
            play: true,
        }
    }

    fn run(t: &mut Transfer, bytes: &[u8]) -> Result<(), XferError> {
        while t.offset() < t.total() {
            let at = t.offset() as usize;
            let n = (t.upto() as usize - at).min(MAX_CHUNK);
            let f = chunk_frame(t.send.id, at as u32, &bytes[at..at + n]);
            t.chunk(&parse_chunk(&f)?, Duration::ZERO)?;
            t.written();
        }
        t.finish()
    }

    #[test]
    fn a_whole_send_with_its_copy() {
        let gif: Vec<u8> = (0..5000u32).map(|i| i as u8).collect();
        let copy: Vec<u8> = (0..2500u32).map(|i| (i * 7) as u8).collect();
        let mut t = Transfer::start(send(&gif, &copy), 2, Duration::ZERO);
        assert_eq!(t.upto(), 2048);
        let all = [gif.as_slice(), copy.as_slice()].concat();
        assert_eq!(run(&mut t, &all), Ok(()));
    }

    #[test]
    fn a_tampered_chunk_fails_at_the_end() {
        let gif = vec![1u8; 3000];
        let mut t = Transfer::start(send(&gif, &[]), 2, Duration::ZERO);
        let mut bad = gif.clone();
        bad[1500] ^= 1;
        assert_eq!(run(&mut t, &bad), Err(XferError::Tampered));
    }

    #[test]
    fn chunks_outside_the_credit_are_refused() {
        let gif = vec![2u8; 8000];
        let mut t = Transfer::start(send(&gif, &[]), 2, Duration::ZERO);
        let c = |off: u32, n: usize| chunk_frame(77, off, &gif[off as usize..off as usize + n]);
        // Ahead of the offset, behind it, past the credit, another id.
        assert_eq!(t.chunk(&parse_chunk(&c(1024, 1024)).unwrap(), Duration::ZERO), Err(XferError::OutsideCredit));
        t.chunk(&parse_chunk(&c(0, 1024)).unwrap(), Duration::ZERO).unwrap();
        assert_eq!(t.chunk(&parse_chunk(&c(0, 1024)).unwrap(), Duration::ZERO), Err(XferError::OutsideCredit));
        t.chunk(&parse_chunk(&c(1024, 1024)).unwrap(), Duration::ZERO).unwrap();
        // No credit left until the write returns.
        assert_eq!(t.chunk(&parse_chunk(&c(2048, 1)).unwrap(), Duration::ZERO), Err(XferError::OutsideCredit));
        assert_eq!(t.written().as_deref(), Some(r#"{"t":"credit","id":77,"upto":4096}"#));
        assert_eq!(t.chunk(&parse_chunk(&chunk_frame(78, 2048, &[2])).unwrap(), Duration::ZERO), Err(XferError::UnknownTransfer));
    }

    #[test]
    fn chunk_frames() {
        assert_eq!(parse_chunk(&chunk_frame(1, 0, &[])), Err(XferError::BadFrame), "empty");
        assert_eq!(parse_chunk(&chunk_frame(1, 0, &[0; MAX_CHUNK + 1])), Err(XferError::BadFrame), "oversize");
        assert!(parse_chunk(&chunk_frame(1, 0, &[0; MAX_CHUNK])).is_ok());
        let mut f = chunk_frame(1, 0, &[0]);
        f[0] = 0x03;
        assert_eq!(parse_chunk(&f), Err(XferError::BadFrame));
        assert_eq!(parse_chunk(&[0x02, 0, 0]), Err(XferError::BadFrame));
    }

    #[test]
    fn resume_needs_the_same_send_in_time() {
        let gif = vec![3u8; 4000];
        let s = send(&gif, &[]);
        let mut t = Transfer::start(s.clone(), 2, Duration::ZERO);
        t.chunk(&parse_chunk(&chunk_frame(77, 0, &gif[..1024])).unwrap(), Duration::from_secs(1)).unwrap();
        let mut other = s.clone();
        other.sha256[0] ^= 1;
        assert!(!t.resume(&other, Duration::from_secs(2)));
        assert!(!t.resume(&s, Duration::from_secs(1) + RESUME_FOR + Duration::from_secs(1)));
        assert!(t.resume(&s, Duration::from_secs(60)));
        assert_eq!(t.offset(), 1024);
        assert_eq!(t.upto(), 3072);
    }

    #[test]
    fn credit_never_runs_past_the_window() {
        // Odd-sized chunks, queued ahead of their writes: what is credited
        // is never more than the window past what was written.
        let gif: Vec<u8> = (0..20_000u32).map(|i| (i * 13) as u8).collect();
        for win in [1u32, 2, 4] {
            let mut t = Transfer::start(send(&gif, &[]), win, Duration::ZERO);
            let cap = win * MAX_CHUNK as u32;
            let mut queue: Vec<Vec<u8>> = Vec::new();
            let mut sizes = [700usize, 1024, 1, 333, 1024].iter().cycle();
            while !t.complete() {
                // Take in all the credit allows, in odd sizes.
                while t.received() < t.upto() {
                    let at = t.received() as usize;
                    let n = (*sizes.next().unwrap()).min(t.upto() as usize - at);
                    let f = chunk_frame(77, at as u32, &gif[at..at + n]);
                    t.accept(&parse_chunk(&f).unwrap(), Duration::ZERO).unwrap();
                    queue.push(gif[at..at + n].to_vec());
                    assert!(t.upto() - t.offset() <= cap);
                }
                // One past it is refused.
                let at = t.received() as usize;
                if at < gif.len() {
                    let f = chunk_frame(77, at as u32, &gif[at..at + 1]);
                    assert_eq!(t.accept(&parse_chunk(&f).unwrap(), Duration::ZERO), Err(XferError::OutsideCredit));
                }
                // One write, then more credit.
                let d = queue.remove(0);
                t.commit(&d);
                t.grant_more();
                assert!(t.upto() - t.offset() <= cap, "win {win}: upto {} offset {}", t.upto(), t.offset());
                assert!(t.upto() <= t.total());
            }
            assert_eq!(t.finish(), Ok(()));
        }
    }

    #[test]
    fn rewind_drops_what_was_not_written() {
        let gif = vec![5u8; 5000];
        let mut t = Transfer::start(send(&gif, &[]), 2, Duration::ZERO);
        t.accept(&parse_chunk(&chunk_frame(77, 0, &gif[..1024])).unwrap(), Duration::ZERO).unwrap();
        t.accept(&parse_chunk(&chunk_frame(77, 1024, &gif[1024..2048])).unwrap(), Duration::ZERO).unwrap();
        t.commit(&gif[..1024]);
        // The session ends with the second chunk unwritten.
        t.rewind();
        assert_eq!((t.offset(), t.received()), (1024, 1024));
        assert!(t.resume(&send(&gif, &[]), Duration::from_secs(5)));
        assert_eq!(t.upto(), 1024 + 2048);
        assert_eq!(t.accept(&parse_chunk(&chunk_frame(77, 2048, &gif[2048..3072])).unwrap(), Duration::ZERO), Err(XferError::OutsideCredit));
        assert!(t.accept(&parse_chunk(&chunk_frame(77, 1024, &gif[1024..2048])).unwrap(), Duration::ZERO).is_ok());
    }

    #[test]
    fn commit_splits_gif_and_copy() {
        let gif = vec![1u8; 1500];
        let copy = vec![2u8; 700];
        let mut t = Transfer::start(send(&gif, &copy), 4, Duration::ZERO);
        let all = [gif.as_slice(), copy.as_slice()].concat();
        t.accept(&parse_chunk(&chunk_frame(77, 0, &all[..1024])).unwrap(), Duration::ZERO).unwrap();
        t.accept(&parse_chunk(&chunk_frame(77, 1024, &all[1024..2048])).unwrap(), Duration::ZERO).unwrap();
        t.accept(&parse_chunk(&chunk_frame(77, 2048, &all[2048..])).unwrap(), Duration::ZERO).unwrap();
        assert_eq!(t.commit(&all[..1024]), 1024);
        assert_eq!(t.commit(&all[1024..2048]), 476);
        assert_eq!(t.commit(&all[2048..]), 0);
        assert!(t.complete());
        assert_eq!(t.finish(), Ok(()));
        assert!(!t.grant_more());
    }

    #[test]
    fn messages() {
        let gif = vec![1u8; 3000];
        let t = Transfer::start(send(&gif, &[]), 2, Duration::ZERO);
        assert_eq!(t.credit_for(&[0xab; 8]), r#"{"t":"credit","id":77,"upto":2048,"k":"abababababababab"}"#);
        assert_eq!(t.done_for(&[1; 8], "cat \"2\".gif"), r#"{"t":"credit","id":77,"upto":3000,"k":"0101010101010101","done":true,"name":"cat \"2\".gif"}"#);
        assert_eq!(t.err_for(&[1; 8], "bad-arg"), r#"{"t":"credit","id":77,"upto":2048,"k":"0101010101010101","err":"bad-arg"}"#);
    }

    const A: KeyId = [0xa; 8];
    const B: KeyId = [0xb; 8];

    #[test]
    fn slot_resume_only_for_the_key_that_started_it() {
        let gif = vec![3u8; 4000];
        let s = send(&gif, &[]);
        let mut slot = Slot::new(2);
        let t0 = Duration::from_secs(10);
        assert_eq!(slot.check(&A, &s, t0).unwrap().0, Plan::Start { replaces: false });
        assert!(slot.start(A, s.clone(), t0).unwrap().is_none());
        let used = slot.quota.used(t0);
        assert_eq!(used, 4000);
        // Another key, the same send: busy; its own other send: busy.
        assert_eq!(slot.check(&B, &s, t0).unwrap_err(), Refusal::Busy);
        let mut other = s.clone();
        other.id = 78;
        assert_eq!(slot.check(&B, &other, t0).unwrap_err(), Refusal::Busy);
        // The same key, the same id, other arguments: bad-arg.
        let mut changed = s.clone();
        changed.size += 1;
        assert_eq!(slot.check(&A, &changed, t0).unwrap_err(), Refusal::BadArg);
        // The same key again: a resume, no second charge.
        assert_eq!(slot.check(&A, &s, t0).unwrap().0, Plan::Resume);
        assert!(slot.resume(&s, t0 + Duration::from_secs(1)));
        assert_eq!(slot.quota.used(t0), used);
        // The same key, another id: replaces its stalled one (no refund).
        assert_eq!(slot.check(&A, &other, t0).unwrap().0, Plan::Start { replaces: true });
        let old = slot.start(A, other.clone(), t0).unwrap();
        assert_eq!(old.map(|h| h.t.send.id), Some(77));
        assert_eq!(slot.quota.used(t0), used * 2);
        // A cancel names the held one by id, from its key.
        assert!(slot.cancel(&A, 78).unwrap().is_some());
    }

    #[test]
    fn slot_expiry_cancel_and_removed_keys() {
        let gif = vec![3u8; 4000];
        let s = send(&gif, &[]);
        let mut slot = Slot::new(2);
        let t0 = Duration::from_secs(10);
        slot.start(A, s.clone(), t0).unwrap();
        // Past the resume window another key may start; the old one is
        // handed back to clean up.
        let later = t0 + RESUME_FOR + Duration::from_secs(1);
        let (plan, gone) = slot.check(&B, &s, later).unwrap();
        assert_eq!((plan, gone.map(|h| h.k)), (Plan::Start { replaces: false }, Some(A)));
        slot.start(B, s.clone(), later).unwrap();
        assert_eq!(slot.cancel(&A, 77).unwrap_err(), Refusal::NotAllowed);
        assert!(slot.cancel(&B, 99).unwrap().is_none());
        assert!(slot.drop_unless(|k| *k == B).is_none());
        assert_eq!(slot.drop_unless(|k| *k == A).map(|h| h.k), Some(B));
        assert!(slot.held.is_none());
        slot.start(A, s.clone(), later).unwrap();
        assert_eq!(slot.cancel(&A, 77).unwrap().map(|h| h.k), Some(A));
    }

    #[test]
    fn slot_quota_is_charged_at_acceptance() {
        let mut slot = Slot::new(2);
        let t0 = Duration::from_secs(10);
        let big = SendArgs { id: 1, name: "a.gif".into(), size: MAX_FILE, sha256: [0; 32], copy_size: MAX_FILE, copy_sha256: Some([0; 32]), play: false };
        let mut n = 0;
        loop {
            let mut s = big.clone();
            s.id = n + 1;
            match slot.check(&A, &s, t0) {
                Ok((Plan::Start { .. }, _)) => {
                    slot.start(A, s, t0).unwrap();
                    n += 1;
                }
                Err(e) => {
                    assert_eq!(e, Refusal::Quota);
                    break;
                }
                Ok(p) => panic!("{p:?}"),
            }
        }
        assert_eq!(u64::from(n), QUOTA_BYTES / (2 * u64::from(MAX_FILE)));
        // A resume of a held one is never refused for the quota.
        let mut s = big.clone();
        s.id = n;
        slot.held = Some(Held { t: Transfer::start(s.clone(), 2, t0), k: A });
        assert_eq!(slot.check(&A, &s, t0).unwrap().0, Plan::Resume);
    }

    #[test]
    fn a_landed_send_signed_again_is_answered_as_landed() {
        let gif = vec![3u8; 4000];
        let s = send(&gif, &[]);
        let mut slot = Slot::new(2);
        let t0 = Duration::from_secs(10);
        slot.start(A, s.clone(), t0).unwrap();
        slot.held = None;
        slot.landed = Some(Landed { k: A, send: s.clone(), name: "cat_2.gif".into(), at: t0 });
        let used = slot.quota.used(t0);
        assert_eq!(slot.check(&A, &s, t0 + Duration::from_secs(5)).unwrap().0, Plan::Landed { total: 4000, name: "cat_2.gif".into() });
        assert_eq!(slot.quota.used(t0), used);
        // Not for another key, nor past the window: a new transfer.
        assert_eq!(slot.check(&B, &s, t0).unwrap().0, Plan::Start { replaces: false });
        assert_eq!(slot.check(&A, &s, t0 + RESUME_FOR + Duration::from_secs(1)).unwrap().0, Plan::Start { replaces: false });
        assert_eq!(done_message(77, 4000, &A, "cat_2.gif"), r#"{"t":"credit","id":77,"upto":4000,"k":"0a0a0a0a0a0a0a0a","done":true,"name":"cat_2.gif"}"#);
    }

    #[test]
    fn next_names() {
        assert_eq!(next_name("cat.gif"), "cat_2.gif");
        assert_eq!(next_name("cat_2.gif"), "cat_3.gif");
        assert_eq!(next_name("SNES_x_09.GIF"), "SNES_x_10.GIF");
        assert_eq!(next_name("MD_Game_01_sephirot68.gif"), "MD_Game_02_sephirot68.gif");
        assert_eq!(next_name("a_b.gif"), "a_b_2.gif");
        assert_eq!(next_name("x_99999999999999999999.gif"), "x_99999999999999999999_2.gif");
        assert_eq!(next_name("noext"), "noext");
    }

    #[test]
    fn heads() {
        assert!(gif_head_ok(b"GIF89a\x80\x00\x20\x00\x00\x00\x00"));
        assert!(gif_head_ok(b"GIF87a\x80\x00\x20\x00\x00\x00\x00"));
        assert!(!gif_head_ok(b"GIF89a\x80\x00"));
        assert!(!gif_head_ok(b"PNG\x89a\x80\x00\x20\x00\x00\x00\x00"));
        // A 4-colour palette, then the hint.
        let mut c = b"GIF89a\x80\x00\x20\x00\x81\x00\x00".to_vec();
        c.extend_from_slice(&[0; 12]);
        let hint = |cap: u8| [&[0x21u8, 0xFF, 11][..], b"OPENDMD1LZW", &[3, 1, cap, 1, 0]].concat();
        assert!(copy_head_ok(&[c.as_slice(), &hint(9)].concat()));
        let mut low = [c.as_slice(), &hint(9)].concat();
        low[c.len() + 3..c.len() + 14].make_ascii_lowercase();
        assert!(copy_head_ok(&low), "ImageMagick lowercases the id");
        assert!(!copy_head_ok(&[c.as_slice(), &hint(12)].concat()), "a cap wider than a copy's");
        assert!(!copy_head_ok(&c), "no hint");
        assert!(!copy_head_ok(&[b"GIF87a".as_slice(), &c[6..], &hint(9)].concat()), "87a");
    }

    #[test]
    fn quota_rolls() {
        let mut q = Quota::default();
        let h = |n: u64| Duration::from_secs(n * 3600);
        assert!(q.charge(QUOTA_BYTES - 10, h(0)));
        assert!(!q.charge(11, h(5)));
        assert!(q.charge(10, h(5)));
        assert!(!q.charge(1, h(23)));
        // The first hour's bytes leave the window.
        assert!(q.charge(QUOTA_BYTES - 10, h(24)));
    }
}
