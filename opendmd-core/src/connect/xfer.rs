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
//! The panel side (card writes, the `.part` file, the stream slot) is F4's.

use super::codec::hex;
use super::proto::SendArgs;
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
#[derive(Clone)]
pub struct Transfer {
    pub send: SendArgs,
    offset: u32,
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
        let mut t = Transfer { send, offset: 0, upto: 0, window: window.clamp(1, MAX_WINDOW), gif: Sha256::new(), copy: Sha256::new(), touched: now };
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
    /// after) and then calls [`written`](Self::written).
    #[cfg_attr(target_arch = "xtensa", optimize(size))]
    pub fn chunk(&mut self, c: &Chunk<'_>, now: Duration) -> Result<(), XferError> {
        if c.id != self.send.id {
            return Err(XferError::UnknownTransfer);
        }
        let end = u64::from(c.offset) + c.data.len() as u64;
        if c.offset != self.offset || end > u64::from(self.upto) || c.data.is_empty() {
            return Err(XferError::OutsideCredit);
        }
        let split = (self.send.size.saturating_sub(c.offset) as usize).min(c.data.len());
        self.gif.update(&c.data[..split]);
        self.copy.update(&c.data[split..]);
        self.offset = end as u32;
        self.touched = now;
        Ok(())
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
