//! The firmware's log ring (`firmware/src/logring.rs`): its record format,
//! and reading back what a crashed run left in it (`GET /api/log/last`).
//!
//! The ring is [`RING`] bytes of RTC slow memory, which a panic or a
//! watchdog reset leaves as it was. Records are reserved one after another
//! from offset 0, each an 8-byte header — state (byte 0: [`WRITING`],
//! [`COMMITTED`] or [`WRAP`]), kind (byte 1: [`TEXT`] or [`RAW`]), the
//! reserved payload capacity (bytes 2-3, LE), the written length (bytes
//! 4-5, LE) — then the payload, the whole padded to 8 bytes ([`padded`]).
//! A record never wraps: when the space to the ring's end is too small, a
//! `WRAP` filler marks it unused and the record goes at 0. The drain
//! releases records but never clears them, so the bytes stay until they
//! are reserved again.
//!
//! The run's `TAIL` (bytes ever reserved, a `u32`) says where it stopped:
//! the newest records are the chain from 0 up to `tail % RING`. If the run
//! had gone round the ring at least once (`tail >= RING`), the older lap's
//! records fill the rest, up to the ring's end, from the first record
//! boundary at or past `tail % RING` (found as the first offset from which
//! the headers chain to the end; [`newest_text`]). A run that had not
//! gone round leaves an earlier boot's bytes there, which are never read.
//!
//! Nothing here allocates.

/// The ring's size in bytes (a power of two: `TAIL` wraps at 2^32, a
/// multiple of it).
pub const RING: usize = 4096;
/// The longest record payload (a longer line is cut).
pub const MAX_LINE: usize = 1536;
/// A record's header bytes.
pub const HDR: usize = 8;
/// Record states (header byte 0).
pub const WRITING: u8 = 0;
pub const COMMITTED: u8 = 1;
/// Filler: the rest of the ring up to its end is unused; go to 0.
pub const WRAP: u8 = 2;
/// Record kinds (header byte 1): a log line, raw bytes (Improv frames).
pub const TEXT: u8 = 0;
pub const RAW: u8 = 1;

/// A record's size in the ring for a `len`-byte payload.
pub const fn padded(len: usize) -> usize {
    (HDR + len + 7) & !7
}

fn u16_at(ring: &[u8], at: usize) -> usize {
    u16::from_le_bytes([ring[at], ring[at + 1]]) as usize
}

/// One record, as a chain step: `None` if the header at `at` is not one a
/// writer could have left; otherwise the next offset and the text it
/// carries (committed text only).
fn step(ring: &[u8], at: usize, older: bool) -> Option<(usize, Option<&[u8]>)> {
    if at + HDR > ring.len() {
        return None;
    }
    let state = ring[at];
    if state == WRAP {
        return Some((ring.len(), None));
    }
    // A record still being written is only ever among the newest: in the
    // older lap a WRITING header is taken for garbage.
    if !(state == COMMITTED || state == WRITING && !older) {
        return None;
    }
    let kind = ring[at + 1];
    let cap = u16_at(ring, at + 2);
    if kind > RAW || cap > MAX_LINE {
        return None;
    }
    let next = at + padded(cap);
    if next > ring.len() {
        return None;
    }
    let len = u16_at(ring, at + 4);
    if state == COMMITTED && len > cap {
        return None;
    }
    let text = (state == COMMITTED && kind == TEXT).then(|| &ring[at + HDR..at + HDR + len]);
    Some((next, text))
}

/// Walk the chain from `from` to `to` (exactly), calling `each` with every
/// committed line's text. `false` (having called `each` for the valid
/// prefix) if the chain breaks or overshoots.
fn walk<'a>(ring: &'a [u8], from: usize, to: usize, older: bool, each: &mut dyn FnMut(&'a [u8])) -> bool {
    let mut at = from;
    while at < to {
        match step(ring, at, older) {
            Some((next, text)) if next <= to || ring[at] == WRAP => {
                if let Some(t) = text {
                    each(t);
                }
                if ring[at] == WRAP {
                    return to == ring.len();
                }
                at = next;
            }
            _ => return false,
        }
    }
    at == to
}

/// The first offset at or past `t` from which the older lap's records
/// chain to the ring's end, if any.
fn older_start(ring: &[u8], t: usize) -> Option<usize> {
    let mut s = (t + 7) & !7;
    while s < ring.len() {
        let mut lines = 0;
        if walk(ring, s, ring.len(), true, &mut |_| lines += 1) && lines > 0 {
            return Some(s);
        }
        s += 8;
    }
    None
}

/// Every committed line the run left, oldest first: the older lap's (when
/// the run had gone round) then the newest chain.
fn each_line<'a>(ring: &'a [u8], tail: u32, each: &mut dyn FnMut(&'a [u8])) {
    let t = tail as usize % ring.len();
    if tail as usize >= ring.len() {
        if let Some(s) = older_start(ring, t) {
            walk(ring, s, ring.len(), true, each);
        }
    }
    walk(ring, 0, t, false, each);
}

/// Copies text into `out`, dropping ANSI escape sequences (the log's
/// colours) and ASCII control bytes but for newlines and tabs; counts what
/// it would write (`seen`) when `out` is empty. The first `skip` bytes are
/// dropped; a line they cut is kept from the cut on, after [`CUT`].
struct Clean<'o> {
    out: &'o mut [u8],
    len: usize,
    /// Kept bytes seen so far (whether written or skipped).
    seen: usize,
    skip: usize,
    /// Inside an escape sequence.
    esc: bool,
    /// The last byte skipped ended a line (or nothing was skipped).
    at_eol: bool,
}

/// What starts a line kept from part way through (its start did not fit).
pub const CUT: &[u8] = b"... ";

impl Clean<'_> {
    fn push(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.esc {
                if b.is_ascii_alphabetic() {
                    self.esc = false;
                }
                continue;
            }
            if b == 0x1b {
                self.esc = true;
                continue;
            }
            if b < 0x20 && b != b'\n' && b != b'\t' || b == 0x7f {
                continue;
            }
            self.seen += 1;
            if self.seen <= self.skip {
                self.at_eol = b == b'\n';
                continue;
            }
            if self.len == 0 && !self.at_eol {
                // A line cut by the skip: said so, then the rest of it.
                let n = CUT.len().min(self.out.len());
                self.out[..n].copy_from_slice(&CUT[..n]);
                self.len = n;
            }
            if self.len < self.out.len() {
                self.out[self.len] = b;
                self.len += 1;
            }
        }
    }
}

/// The newest of what a crashed run left in its log `ring` (the run's
/// `tail`): at most `out.len()` bytes of its lines, oldest first, without
/// colour codes or control bytes (raw records, Improv frames, are left
/// out); the oldest kept is cut at its start when it does not fit whole
/// (then it starts with [`CUT`]). Returns the bytes written.
pub fn newest_text(ring: &[u8], tail: u32, out: &mut [u8]) -> usize {
    if ring.is_empty() || !ring.len().is_power_of_two() || out.len() <= CUT.len() {
        return 0;
    }
    // Pass one: how much there is; pass two: the last of it that fits
    // (room left for the cut mark).
    let mut count = Clean { out: &mut [], len: 0, seen: 0, skip: 0, esc: false, at_eol: true };
    each_line(ring, tail, &mut |t| count.push(t));
    let skip = if count.seen > out.len() { count.seen - (out.len() - CUT.len()) } else { 0 };
    let mut keep = Clean { out, len: 0, seen: 0, skip, esc: false, at_eol: true };
    each_line(ring, tail, &mut |t| keep.push(t));
    keep.len
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer over a test ring, as `logring::reserve` + commit.
    struct W {
        ring: Vec<u8>,
        tail: u32,
    }

    impl W {
        fn new(fill: u8) -> W {
            W { ring: vec![fill; RING], tail: 0 }
        }

        /// Reserve `cap` bytes; the record left WRITING (its len stale).
        fn reserve(&mut self, cap: usize) -> usize {
            let need = padded(cap);
            let at = self.tail as usize % RING;
            let (skip, at2) = if RING - at < need { (RING - at, 0) } else { (0, at) };
            if skip > 0 {
                self.ring[at] = WRAP;
            }
            self.ring[at2] = WRITING;
            self.ring[at2 + 1] = TEXT;
            self.ring[at2 + 2..at2 + 4].copy_from_slice(&(cap as u16).to_le_bytes());
            self.tail = self.tail.wrapping_add((skip + need) as u32);
            at2
        }

        fn line(&mut self, kind: u8, s: &[u8]) {
            let at = self.reserve(s.len());
            self.ring[at + 1] = kind;
            self.ring[at + HDR..at + HDR + s.len()].copy_from_slice(s);
            self.ring[at + 4..at + 6].copy_from_slice(&(s.len() as u16).to_le_bytes());
            self.ring[at] = COMMITTED;
        }

        fn text(&mut self, s: &str) {
            self.line(TEXT, s.as_bytes());
        }

        fn read(&self, room: usize) -> String {
            let mut out = vec![0u8; room];
            let n = newest_text(&self.ring, self.tail, &mut out);
            String::from_utf8(out[..n].to_vec()).unwrap()
        }
    }

    fn numbered(i: usize) -> String {
        format!("\x1b[0;32mI ({i}) opendmd: line {i} {}\x1b[0m\n", "x".repeat(i % 37))
    }

    #[test]
    fn a_run_that_never_went_round() {
        // An earlier boot's bytes beyond the tail are never read.
        let mut w = W::new(0);
        for i in 0..40 {
            w.text(&numbered(i));
        }
        let earlier_tail = w.tail;
        let mut w2 = W { ring: w.ring.clone(), tail: 0 };
        w2.text("first\n");
        w2.text("\x1b[0;31mE (5) x: second\x1b[0m\n");
        assert!(w2.tail < earlier_tail);
        assert_eq!(w2.read(4096), "first\nE (5) x: second\n");
        assert_eq!(W::new(0).read(100), "");
    }

    #[test]
    fn a_run_that_went_round_keeps_its_older_lap() {
        let mut w = W::new(0xa5);
        let mut all = String::new();
        for i in 0..400 {
            let l = numbered(i);
            w.text(&l);
            all.push_str(&l.replace("\x1b[0;32m", "").replace("\x1b[0m", ""));
        }
        assert!(w.tail as usize > 4 * RING);
        let got = w.read(8192);
        // Whole lines, the newest last, and every one of them is the run's.
        let plain = |i| numbered(i).replace("\x1b[0;32m", "").replace("\x1b[0m", "");
        assert!(got.ends_with(&plain(399)), "{got}");
        assert!(all.ends_with(&got), "a contiguous tail of what was written");
        // The older lap is read too: lines from before the one at offset 0.
        let at0 = String::from_utf8_lossy(&w.ring[HDR + 10..HDR + 20]).to_string();
        let first: usize = got[3..].split(')').next().unwrap().parse().unwrap();
        let zero: usize = at0.split(')').next().unwrap().parse().unwrap();
        assert!(first < zero, "{first} {zero} {at0}");
        // Most of a ring's worth (less headers, padding and colour codes).
        assert!(got.len() > RING * 3 / 5, "{}", got.len());
        // Less room: the newest that fit, the oldest line cut at its start
        // (and said so) unless the cut fell between lines.
        let small = w.read(300);
        assert!(small.len() <= 300 && small.len() >= 300 - CUT.len(), "{}", small.len());
        let rest = small.strip_prefix("... ").unwrap_or(&small);
        assert!(all.ends_with(rest));
        assert!(small.starts_with("I (") || small.starts_with("... "), "{small}");
        assert!(small.ends_with(&plain(399)));
        // A cut exactly between lines: no mark.
        let last = plain(399);
        let exact = w.read(last.len() + CUT.len());
        assert_eq!(exact, last);
    }

    #[test]
    fn raw_records_unprintables_and_a_line_in_flight() {
        let mut w = W::new(0);
        w.text("one\n");
        w.line(RAW, b"IMPROV\x01\x02\x03");
        w.text("two \x07bell\n");
        // A line being written when the run stopped: its length is stale.
        let at = w.reserve(40);
        w.ring[at + 4] = 0xff;
        w.ring[at + 5] = 0xff;
        assert_eq!(w.read(100), "one\ntwo bell\n");
        // The chain carries on past it.
        w.text("three\n");
        assert_eq!(w.read(100), "one\ntwo bell\nthree\n");
    }

    #[test]
    fn garbage_never_panics() {
        // A power-on's RTC memory and odd tails: nothing, or whatever
        // chains, never a panic or an overrun.
        let mut seed = 0x1234_5678u32;
        for round in 0..200 {
            let mut ring = vec![0u8; RING];
            for b in ring.iter_mut() {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                *b = (seed >> 16) as u8 % if round % 2 == 0 { 3 } else { 255 };
            }
            let tail = seed.rotate_left(round);
            let mut out = [0u8; 64];
            let n = newest_text(&ring, tail, &mut out);
            assert!(n <= out.len());
        }
        assert_eq!(newest_text(&[0u8; 100], 5, &mut [0u8; 10]), 0, "not a power of two");
        assert_eq!(newest_text(&[0u8; RING], 5, &mut []), 0);
    }

    #[test]
    fn a_wrap_filler_ends_the_older_lap() {
        let mut w = W::new(0);
        // Lines sized so the reservations leave a filler at the end.
        for i in 0..30 {
            w.text(&format!("{i:03} {}\n", "y".repeat(300)));
        }
        let got = w.read(8192);
        assert!(got.ends_with(&format!("029 {}\n", "y".repeat(300))));
        let firsts: Vec<&str> = got.lines().map(|l| &l[..3]).collect();
        let n: Vec<u32> = firsts.iter().map(|s| s.parse().unwrap()).collect();
        assert!(n.windows(2).all(|p| p[1] == p[0] + 1), "{n:?}");
        assert!(n.len() >= 10, "{n:?}");
    }
}
