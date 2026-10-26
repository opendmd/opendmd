//! Why the device restarted (firmware `restarts.rs`): the pure part, here so
//! it is tested on the host.
//!
//! **Every restart leaves a reason.** Each planned restart writes an
//! [`Entry`] (the [`Reason`], the uptime, the free heap and its largest
//! block) into a ring of [`SLOTS`] in RTC slow memory just before it resets;
//! a Rust panic and a stack overflow write theirs from their hooks, with
//! where it happened ([`Entry::text`]). The entry is *pending* until the
//! next boot pairs it with the chip's own reset cause ([`Reset`]), in
//! [`boot`]: a pending entry gets that reset; with none, the boot writes one
//! from the reset alone — a brownout, a software restart
//! nobody recorded, or a crash nobody recorded (with the last heartbeat's
//! figures, [`encode_alive`], and the last failed allocation, [`AllocFail`], when
//! there are any). A power-on clears the ring: RTC memory is garbage then.
//!
//! **Encoding.** A slot is [`WORDS`] `u32`s — the firmware touches RTC slow
//! memory with plain word loads and stores only (`sdhealth.rs`) — with a
//! magic word and a checksum, so a slot from an earlier layout, a power-on's
//! garbage or a write cut short by a reset reads as empty. The newest entry
//! is the valid one with the highest sequence number; the next goes in the
//! slot after it (`seq % SLOTS`), overwriting the oldest.
//!
//! Nothing here allocates: the panic hook writes through [`push`].

use core::fmt;

/// Entries kept: the last eight restarts.
pub const SLOTS: usize = 8;
/// Bytes of [`Entry::text`]: a panic's `file:line` and up to
/// [`MSG_MAX`] bytes of its message.
pub const TEXT: usize = 80;
/// The longest panic message kept (the rest is cut, at a character).
pub const MSG_MAX: usize = 48;
/// Words in a slot: magic, seq, meta, uptime, heap free, largest, checksum,
/// then the text.
pub const WORDS: usize = 7 + TEXT / 4;
/// One encoded entry.
pub type Slot = [u32; WORDS];

/// The layout's magic (bump it when the layout changes).
const MAGIC: u32 = 0x5e57_a201;
const ALIVE_MAGIC: u32 = 0xa11f_e001;
const ALLOC_MAGIC: u32 = 0xa110_cf01;
/// `meta` bit: the uptime and heap figures are known.
const KNOWN: u32 = 1 << 31;

/// Why a restart was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Reason {
    /// `POST /api/reboot` (the web app's Restart).
    ApiReboot = 1,
    /// Into the updater to install a firmware update (`PUT /api/firmware`).
    Ota = 2,
    /// The setup network (AP mode) timed out.
    ApTimeout = 3,
    /// `POST /api/card/full-speed` could not change the clock in place.
    FullSpeed = 4,
    /// A `card_speed` change could not be made in place.
    CardClock = 5,
    /// The playlist's index needs a fresh heap.
    FreshHeap = 6,
    /// A panel-timing trial was undone.
    PanelTrial = 7,
    /// The card failed (`sdhealth`'s trip).
    CardFailure = 8,
    /// A different card: its predecessor's slow clock dropped.
    NewCard = 9,
    /// The card's SPI device was lost in a clock change.
    CardLost = 10,
    /// The card-failure window ran out: one restart to try the card again.
    CardRetry = 11,
    /// A Rust panic.
    Panic = 12,
    /// A task's stack overflowed.
    StackOverflow = 13,
    /// A `diag-heap` build's check found the heap or a handler corrupt.
    Diag = 14,
    /// Powered on (the ring starts here).
    PowerOn = 20,
    /// The supply dipped (brownout detector).
    Brownout = 22,
    /// A software restart that recorded nothing (another firmware's, the
    /// updater's, or one this firmware missed).
    Unrecorded = 23,
    /// A crash or watchdog reset that recorded nothing.
    Crash = 24,
}

impl Reason {
    const ALL: [Reason; 18] = [
        Reason::Diag,
        Reason::ApiReboot,
        Reason::Ota,
        Reason::ApTimeout,
        Reason::FullSpeed,
        Reason::CardClock,
        Reason::FreshHeap,
        Reason::PanelTrial,
        Reason::CardFailure,
        Reason::NewCard,
        Reason::CardLost,
        Reason::CardRetry,
        Reason::Panic,
        Reason::StackOverflow,
        Reason::PowerOn,
        Reason::Brownout,
        Reason::Unrecorded,
        Reason::Crash,
    ];

    pub fn from_u8(v: u8) -> Option<Reason> {
        Self::ALL.into_iter().find(|r| *r as u8 == v)
    }

    /// The API's word (`/api/status` `restarts[].reason`).
    pub fn name(self) -> &'static str {
        match self {
            Reason::ApiReboot => "api_reboot",
            Reason::Ota => "firmware_update",
            Reason::ApTimeout => "ap_timeout",
            Reason::FullSpeed => "card_full_speed",
            Reason::CardClock => "card_speed",
            Reason::FreshHeap => "fresh_heap",
            Reason::PanelTrial => "panel_trial",
            Reason::CardFailure => "card_failure",
            Reason::NewCard => "new_card",
            Reason::CardLost => "card_lost",
            Reason::CardRetry => "card_retry",
            Reason::Panic => "panic",
            Reason::StackOverflow => "stack_overflow",
            Reason::PowerOn => "power_on",
            Reason::Diag => "diag_abort",
            Reason::Brownout => "brownout",
            Reason::Unrecorded => "unrecorded",
            Reason::Crash => "unrecorded_crash",
        }
    }

    /// In plain words (`/api/status` `reset_note`).
    pub fn note(self) -> &'static str {
        match self {
            Reason::ApiReboot => "restarted on request",
            Reason::Ota => "restarted to install a firmware update",
            Reason::ApTimeout => "restarted after the setup network timed out",
            Reason::FullSpeed => "restarted to try the card at full speed",
            Reason::CardClock => "restarted to apply a card speed change",
            Reason::FreshHeap => "restarted to build the playlist with more free memory",
            Reason::PanelTrial => "restarted to undo a panel timing trial",
            Reason::CardFailure => "card failure",
            Reason::NewCard => "different card",
            Reason::CardLost => "restarted after losing contact with the card",
            Reason::CardRetry => "restarted to try the card again",
            Reason::Panic => "crashed (panic)",
            Reason::StackOverflow => "crashed (stack overflow)",
            Reason::PowerOn => "powered on",
            Reason::Diag => "stopped by a heap check (diagnostic build)",
            Reason::Brownout => "the power supply dipped",
            Reason::Unrecorded => "restarted without a recorded reason",
            Reason::Crash => "crashed without a recorded cause",
        }
    }
}

/// The chip's reset cause (`esp_reset_reason`), as the boot saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Reset {
    /// Not yet paired: the entry was written before a reset no boot has
    /// seen yet.
    Pending = 0,
    PowerOn = 1,
    External = 2,
    Software = 3,
    Panic = 4,
    IntWdt = 5,
    TaskWdt = 6,
    Wdt = 7,
    DeepSleep = 8,
    Brownout = 9,
    Sdio = 10,
    Unknown = 11,
}

impl Reset {
    pub fn from_u8(v: u8) -> Reset {
        use Reset::*;
        [
            Pending, PowerOn, External, Software, Panic, IntWdt, TaskWdt, Wdt, DeepSleep, Brownout,
            Sdio,
        ]
        .get(v as usize)
        .copied()
        .unwrap_or(Unknown)
    }

    /// `/api/status`'s word (`reset_reason`, `restarts[].reset`).
    pub fn name(self) -> &'static str {
        match self {
            Reset::Pending => "pending",
            Reset::PowerOn => "poweron",
            Reset::External => "external",
            Reset::Software => "software",
            Reset::Panic => "panic",
            Reset::IntWdt => "interrupt watchdog",
            Reset::TaskWdt => "task watchdog",
            Reset::Wdt => "watchdog",
            Reset::DeepSleep => "deep sleep",
            Reset::Brownout => "brownout",
            Reset::Sdio => "sdio",
            Reset::Unknown => "unknown",
        }
    }

    fn is_crash(self) -> bool {
        matches!(
            self,
            Reset::Panic | Reset::IntWdt | Reset::TaskWdt | Reset::Wdt
        )
    }
}

/// One restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub seq: u32,
    pub reason: Reason,
    pub reset: Reset,
    /// Uptime (ms), free heap and largest free block when it was written;
    /// `None` when nothing was known (a power-on, an unrecorded restart with
    /// no heartbeat).
    pub figures: Option<Figures>,
    text: [u8; TEXT],
    len: u8,
}

/// Uptime and heap at a moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Figures {
    pub uptime_ms: u32,
    pub heap_free: u32,
    pub largest: u32,
}

impl Entry {
    /// A pending entry (seq is set by [`push`]).
    pub fn new(reason: Reason, figures: Option<Figures>) -> Entry {
        Entry {
            seq: 0,
            reason,
            reset: Reset::Pending,
            figures,
            text: [0; TEXT],
            len: 0,
        }
    }

    /// Where it happened (a panic's `file:line message`), or `""`.
    pub fn text(&self) -> &str {
        core::str::from_utf8(&self.text[..self.len as usize]).unwrap_or("")
    }

    /// Set the text from formatting arguments, cut at [`TEXT`] bytes (on a
    /// character). No allocation.
    pub fn set_text(&mut self, args: fmt::Arguments) {
        let mut w = Text {
            buf: &mut self.text,
            len: 0,
            room: TEXT,
        };
        let _ = fmt::write(&mut w, args);
        self.len = w.len as u8;
    }

    /// A panic's text: the file's last component, the line, then at most
    /// [`MSG_MAX`] bytes of the message (on one line).
    pub fn set_panic(&mut self, file: &str, line: u32, msg: &str) {
        let file = file.rsplit(['/', '\\']).next().unwrap_or(file);
        let mut w = Text {
            buf: &mut self.text,
            len: 0,
            room: TEXT,
        };
        let _ = fmt::write(&mut w, format_args!("{file}:{line}"));
        // The message on one line, cut at MSG_MAX bytes.
        let mut parts = msg.split(['\n', '\r']).filter(|p| !p.is_empty());
        if let Some(first) = parts.next() {
            w.room = (w.len + 1 + MSG_MAX).min(TEXT);
            let _ = fmt::Write::write_str(&mut w, " ");
            let _ = fmt::Write::write_str(&mut w, first);
            for part in parts {
                let _ = fmt::Write::write_str(&mut w, " ");
                let _ = fmt::Write::write_str(&mut w, part);
            }
        }
        self.len = w.len as u8;
    }

    /// The reset note for status: the reason in plain words, `None` for a
    /// power-on.
    pub fn note(&self) -> Option<&'static str> {
        (self.reason != Reason::PowerOn).then(|| self.reason.note())
    }
}

/// A fixed-buffer writer that cuts at `room` bytes, on a character.
struct Text<'a> {
    buf: &'a mut [u8; TEXT],
    len: usize,
    room: usize,
}

impl fmt::Write for Text<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let left = self.room.saturating_sub(self.len);
        let mut n = s.len().min(left);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        if n < s.len() {
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

const FNV_SEED: u32 = 0x811c_9dc5;

/// FNV-1a, one word's bytes further.
fn fnv(mut h: u32, w: u32) -> u32 {
    for b in w.to_le_bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn checksum(words: &[u32]) -> u32 {
    words.iter().fold(FNV_SEED, |h, w| fnv(h, *w))
}

/// An entry as a slot.
pub fn encode(e: &Entry) -> Slot {
    let mut s = [0u32; WORDS];
    let f = e.figures.unwrap_or_default();
    s[0] = MAGIC;
    s[1] = e.seq;
    s[2] = e.reason as u32
        | (e.reset as u32) << 8
        | (e.len as u32) << 16
        | if e.figures.is_some() { KNOWN } else { 0 };
    s[3] = f.uptime_ms;
    s[4] = f.heap_free;
    s[5] = f.largest;
    for (i, c) in e.text.as_chunks::<4>().0.iter().enumerate() {
        s[7 + i] = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
    }
    s[6] = checksum_of(&s);
    s
}

fn checksum_of(s: &Slot) -> u32 {
    combine(checksum(&s[1..6]), checksum(&s[7..]))
}

/// The slot's checksum from its header's (words 1-5) and its text's.
fn combine(header: u32, text: u32) -> u32 {
    header ^ text.rotate_left(7)
}

/// A slot as an entry: `None` if it is empty, garbage or another layout's.
pub fn decode(s: &Slot) -> Option<Entry> {
    if s[0] != MAGIC || s[6] != checksum_of(s) {
        return None;
    }
    let reason = Reason::from_u8(s[2] as u8)?;
    let len = ((s[2] >> 16) & 0xff) as u8;
    if len as usize > TEXT {
        return None;
    }
    let mut text = [0u8; TEXT];
    for (i, w) in s[7..].iter().enumerate() {
        text[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    core::str::from_utf8(&text[..len as usize]).ok()?;
    Some(Entry {
        seq: s[1],
        reason,
        reset: Reset::from_u8((s[2] >> 8) as u8),
        figures: (s[2] & KNOWN != 0).then_some(Figures {
            uptime_ms: s[3],
            heap_free: s[4],
            largest: s[5],
        }),
        text,
        len,
    })
}

/// Where the slots live: RTC memory on the device, an array in the tests.
/// Word by word, so the stack-overflow hook's [`push_minimal`] never holds
/// a whole slot.
pub trait Store {
    fn get(&self, slot: usize, word: usize) -> u32;
    fn put(&mut self, slot: usize, word: usize, v: u32);
}

impl Store for [Slot; SLOTS] {
    fn get(&self, slot: usize, word: usize) -> u32 {
        self[slot][word]
    }
    fn put(&mut self, slot: usize, word: usize, v: u32) {
        self[slot][word] = v;
    }
}

fn read(store: &dyn Store, i: usize) -> Slot {
    let mut s = [0; WORDS];
    for (k, w) in s.iter_mut().enumerate() {
        *w = store.get(i, k);
    }
    s
}

fn write(store: &mut dyn Store, i: usize, s: &Slot) {
    for (k, w) in s.iter().enumerate() {
        store.put(i, k, *w);
    }
}

/// `a` is newer than `b` (sequence numbers wrap at 2^32).
fn newer(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}

/// The newest valid entry.
pub fn newest(store: &dyn Store) -> Option<Entry> {
    (0..SLOTS)
        .filter_map(|i| decode(&read(store, i)))
        .fold(None, |best: Option<Entry>, e| match best {
            Some(b) if !newer(e.seq, b.seq) => Some(b),
            _ => Some(e),
        })
}

/// Write `e` as the newest entry (its `seq` is the newest's + 1) over the
/// oldest. Returns the entry as written. No allocation.
pub fn push(store: &mut dyn Store, mut e: Entry) -> Entry {
    e.seq = newest(store).map_or(1, |n| n.seq.wrapping_add(1));
    write(store, e.seq as usize % SLOTS, &encode(&e));
    e
}

/// [`push`] for the stack-overflow hook: reads only each slot's magic and
/// seq words (an unchecked slot can only push the seq further on), builds
/// the words one at a time — no slot on the stack, no `core::fmt` — from
/// `parts` concatenated (cut at [`TEXT`]; bytes past ASCII become `?`).
pub fn push_minimal(store: &mut dyn Store, reason: Reason, f: Figures, parts: &[&[u8]]) {
    let mut seq = None;
    for i in 0..SLOTS {
        if store.get(i, 0) == MAGIC {
            let s = store.get(i, 1);
            if seq.is_none_or(|b| newer(s, b)) {
                seq = Some(s);
            }
        }
    }
    let seq = seq.map_or(1, |s: u32| s.wrapping_add(1));
    let len = parts.iter().map(|p| p.len()).sum::<usize>().min(TEXT);
    let byte = |n: usize| -> u8 {
        let mut n = n;
        for p in parts {
            if n < p.len() {
                return if p[n].is_ascii() { p[n] } else { b'?' };
            }
            n -= p.len();
        }
        0
    };
    let text_word = |k: usize| -> u32 {
        let at = |j: usize| if k * 4 + j < len { byte(k * 4 + j) } else { 0 };
        u32::from_le_bytes([at(0), at(1), at(2), at(3)])
    };
    let header = [
        seq,
        reason as u32 | (len as u32) << 16 | KNOWN,
        f.uptime_ms,
        f.heap_free,
        f.largest,
    ];
    let text = (0..TEXT / 4).fold(FNV_SEED, |h, k| fnv(h, text_word(k)));
    let slot = seq as usize % SLOTS;
    store.put(slot, 0, 0);
    for (k, w) in header.iter().enumerate() {
        store.put(slot, 1 + k, *w);
    }
    store.put(slot, 6, combine(checksum(&header), text));
    for k in 0..TEXT / 4 {
        store.put(slot, 7 + k, text_word(k));
    }
    store.put(slot, 0, MAGIC);
}

/// Every valid entry, newest first: decoded one at a time as it is
/// iterated (the order is a few bytes), so a caller on a small stack (the
/// HTTP server's) never holds all eight.
pub fn newest_first(store: &dyn Store) -> NewestFirst<'_> {
    let mut seqs = [(0u32, 0u8); SLOTS];
    let mut n = 0;
    let mut top = None;
    for i in 0..SLOTS {
        if let Some(e) = decode(&read(store, i)) {
            seqs[n] = (e.seq, i as u8);
            n += 1;
            if top.is_none_or(|t| newer(e.seq, t)) {
                top = Some(e.seq);
            }
        }
    }
    let top = top.unwrap_or(0);
    // Distance back from the newest, which wraps with the seq.
    seqs[..n].sort_unstable_by_key(|(seq, _)| top.wrapping_sub(*seq));
    let mut order = [0u8; SLOTS];
    for (o, (_, i)) in order.iter_mut().zip(seqs) {
        *o = i;
    }
    NewestFirst {
        store,
        order,
        n: n as u8,
        at: 0,
    }
}

/// [`newest_first`]'s iterator.
pub struct NewestFirst<'a> {
    store: &'a dyn Store,
    order: [u8; SLOTS],
    n: u8,
    at: u8,
}

impl Iterator for NewestFirst<'_> {
    type Item = Entry;
    fn next(&mut self) -> Option<Entry> {
        while self.at < self.n {
            let i = self.order[self.at as usize] as usize;
            self.at += 1;
            // Still valid unless rewritten meanwhile (then skipped).
            if let Some(e) = decode(&read(self.store, i)) {
                return Some(e);
            }
        }
        None
    }
}

/// Empty every slot.
pub fn clear(store: &mut dyn Store) {
    for i in 0..SLOTS {
        write(store, i, &[0; WORDS]);
    }
}

/// The last heartbeat's figures, kept in RTC memory for a restart that
/// records nothing (a crash, a watchdog).
pub fn encode_alive(f: &Figures) -> [u32; 5] {
    let mut s = [ALIVE_MAGIC, f.uptime_ms, f.heap_free, f.largest, 0];
    s[4] = checksum(&s[..4]);
    s
}

pub fn decode_alive(s: &[u32; 5]) -> Option<Figures> {
    (s[0] == ALIVE_MAGIC && s[4] == checksum(&s[..4])).then_some(Figures {
        uptime_ms: s[1],
        heap_free: s[2],
        largest: s[3],
    })
}

/// The last failed allocation (the IDF heap's failed-allocation callback):
/// a crash right after one is most likely the out-of-memory abort, which
/// no Rust panic hook sees. `at.largest` is 0: the callback does not walk
/// the heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocFail {
    pub size: u32,
    pub at: Figures,
}

pub fn encode_alloc(a: &AllocFail) -> [u32; 6] {
    let mut s = [
        ALLOC_MAGIC,
        a.size,
        a.at.uptime_ms,
        a.at.heap_free,
        a.at.largest,
        0,
    ];
    s[5] = checksum(&s[..5]);
    s
}

pub fn decode_alloc(s: &[u32; 6]) -> Option<AllocFail> {
    (s[0] == ALLOC_MAGIC && s[5] == checksum(&s[..5])).then_some(AllocFail {
        size: s[1],
        at: Figures {
            uptime_ms: s[2],
            heap_free: s[3],
            largest: s[4],
        },
    })
}

/// A failed allocation at most this long before the last heartbeat, or
/// after it, is named beside a crash. The heartbeat is every 30 s, so that
/// is up to ~65 s before the crash.
const ALLOC_RECENT_MS: u32 = 35_000;

/// The boot's pairing: the pending entry gets `reset`; with none, an entry
/// from the reset alone (`alive`: the previous run's last heartbeat;
/// `alloc`: its last failed allocation). A power-on clears the ring first.
/// Returns this boot's entry (already in the ring).
pub fn boot(
    store: &mut dyn Store,
    reset: Reset,
    alive: Option<Figures>,
    alloc: Option<AllocFail>,
) -> Entry {
    if reset == Reset::PowerOn {
        clear(store);
        let mut e = Entry::new(Reason::PowerOn, None);
        e.reset = reset;
        return push(store, e);
    }
    if let Some(mut e) = newest(store).filter(|e| e.reset == Reset::Pending) {
        e.reset = reset;
        write(store, e.seq as usize % SLOTS, &encode(&e));
        return e;
    }
    let reason = match reset {
        // The ESP32 reports its EN button as a power-on (the ring is
        // cleared then); `External` is for chips that tell them apart.
        Reset::Brownout => Reason::Brownout,
        r if r.is_crash() => Reason::Crash,
        _ => Reason::Unrecorded,
    };
    let mut e = Entry::new(reason, alive);
    e.reset = reset;
    if reason == Reason::Crash {
        if let Some(a) = alloc.filter(|a| {
            alive.is_none_or(|f| a.at.uptime_ms.saturating_add(ALLOC_RECENT_MS) >= f.uptime_ms)
        }) {
            e.set_text(format_args!(
                "last failed alloc: {} B at {} s, {} B free (maybe a probe)",
                a.size,
                a.at.uptime_ms / 1000,
                a.at.heap_free
            ));
        }
    }
    push(store, e)
}

/// Rewrite this boot's entry as a firmware update (`ota::after_boot` found
/// the updater's mark) when it recorded nothing: the restart out of the
/// updater, after an update from a firmware that kept no ring.
pub fn mark_update(store: &mut dyn Store) -> bool {
    match newest(store) {
        Some(mut e) if e.reason == Reason::Unrecorded && e.reset == Reset::Software => {
            e.reason = Reason::Ota;
            write(store, e.seq as usize % SLOTS, &encode(&e));
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fig(uptime_ms: u32) -> Option<Figures> {
        Some(Figures {
            uptime_ms,
            heap_free: 90_000,
            largest: 40_000,
        })
    }

    fn ring() -> [Slot; SLOTS] {
        [[0; WORDS]; SLOTS]
    }

    #[test]
    fn round_trip() {
        let mut e = Entry::new(Reason::CardClock, fig(1234));
        e.seq = 7;
        e.reset = Reset::Software;
        e.set_text(format_args!("x={}", 5));
        let d = decode(&encode(&e)).unwrap();
        assert_eq!(d, e);
        assert_eq!(d.text(), "x=5");
        let e = Entry::new(Reason::PowerOn, None);
        assert_eq!(decode(&encode(&e)).unwrap().figures, None);
        for r in Reason::ALL {
            assert_eq!(Reason::from_u8(r as u8), Some(r));
            assert!(!r.name().is_empty() && !r.note().is_empty());
        }
        for v in 0..12 {
            assert_eq!(Reset::from_u8(v) as u8, v);
        }
        assert_eq!(Reset::from_u8(200), Reset::Unknown);
    }

    #[test]
    fn checksum_and_magic_reject() {
        let mut e = Entry::new(Reason::ApiReboot, fig(5));
        e.seq = 3;
        let good = encode(&e);
        assert!(decode(&[0; WORDS]).is_none());
        for w in 1..WORDS {
            let mut bad = good;
            bad[w] ^= 1 << (w % 32);
            assert!(decode(&bad).is_none(), "word {w} flipped");
        }
        let mut bad = good;
        bad[0] = MAGIC ^ 1;
        assert!(decode(&bad).is_none());
        // Garbage that happens to carry the magic.
        let mut g = [0xdead_beefu32; WORDS];
        g[0] = MAGIC;
        assert!(decode(&g).is_none());
    }

    #[test]
    fn wraps_around_newest_first() {
        let mut r = ring();
        for i in 0..20u32 {
            let e = push(&mut r, Entry::new(Reason::ApiReboot, fig(i)));
            assert_eq!(e.seq, i + 1);
        }
        let all: Vec<Entry> = newest_first(&r).collect();
        assert_eq!(all.len(), SLOTS);
        let seqs: Vec<u32> = all.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, (13..=20).rev().collect::<Vec<_>>());
        assert_eq!(all[0].figures.unwrap().uptime_ms, 19);
        // A corrupt slot is skipped; the rest carry on.
        r[20 % SLOTS][3] ^= 1;
        assert_eq!(newest(&r).unwrap().seq, 19);
        assert_eq!(push(&mut r, Entry::new(Reason::Ota, None)).seq, 20);
    }

    #[test]
    fn panic_text_is_cut() {
        let mut e = Entry::new(Reason::Panic, fig(1));
        e.set_panic(
            "/home/x/.cargo/src/http.rs",
            1270,
            "test panic\nsecond line",
        );
        assert_eq!(e.text(), "http.rs:1270 test panic second line");
        let long = "é".repeat(100);
        e.set_panic("src\\main.rs", 9, &long);
        let t = e.text();
        assert!(t.starts_with("main.rs:9 é"));
        assert!(t.len() <= "main.rs:9 ".len() + MSG_MAX, "{}", t.len());
        assert!(t.len() <= TEXT);
        let mut e = Entry::new(Reason::Panic, None);
        e.set_panic(&"f".repeat(200), 1, "msg");
        assert_eq!(e.text().len(), TEXT);
        e.set_panic("a.rs", 3, "");
        assert_eq!(e.text(), "a.rs:3");
        assert_eq!(decode(&encode(&e)).unwrap().text(), "a.rs:3");
    }

    #[test]
    fn boot_pairs_a_pending_entry() {
        let mut r = ring();
        boot(&mut r, Reset::PowerOn, None, None);
        push(&mut r, Entry::new(Reason::CardClock, fig(60_000)));
        let e = boot(&mut r, Reset::Software, fig(55_000), None);
        assert_eq!((e.reason, e.reset), (Reason::CardClock, Reset::Software));
        assert_eq!(e.note(), Some("restarted to apply a card speed change"));
        assert_eq!(e.figures.unwrap().uptime_ms, 60_000);
        // The next boot with nothing written: unrecorded, from the
        // heartbeat.
        let e = boot(&mut r, Reset::Software, fig(30_000), None);
        assert_eq!((e.reason, e.reset), (Reason::Unrecorded, Reset::Software));
        assert_eq!(e.figures.unwrap().uptime_ms, 30_000);
        let all: Vec<_> = newest_first(&r).map(|e| e.reason).collect();
        assert_eq!(
            all,
            [Reason::Unrecorded, Reason::CardClock, Reason::PowerOn]
        );
    }

    #[test]
    fn boot_after_a_crash() {
        let mut r = ring();
        boot(&mut r, Reset::PowerOn, None, None);
        let mut p = Entry::new(Reason::Panic, fig(9_000));
        p.set_panic("main.rs", 10, "boom");
        push(&mut r, p);
        let e = boot(&mut r, Reset::Panic, fig(1), None);
        assert_eq!(
            (e.reason, e.reset, e.text()),
            (Reason::Panic, Reset::Panic, "main.rs:10 boom")
        );
        // A watchdog with no entry: an unrecorded crash; a failed
        // allocation just before it is named.
        let a = AllocFail {
            size: 4096,
            at: fig(100_000).unwrap(),
        };
        let e = boot(&mut r, Reset::IntWdt, fig(90_000), Some(a));
        assert_eq!(e.reason, Reason::Crash);
        assert!(
            e.text().contains("failed alloc: 4096 B at 100 s"),
            "{}",
            e.text()
        );
        // An old one is not.
        let e = boot(&mut r, Reset::Panic, fig(900_000), Some(a));
        assert_eq!((e.reason, e.text()), (Reason::Crash, ""));
        let e = boot(&mut r, Reset::Brownout, None, None);
        assert_eq!(e.reason, Reason::Brownout);
        let e = boot(&mut r, Reset::External, None, None);
        assert_eq!(e.reason, Reason::Unrecorded);
    }

    #[test]
    fn power_on_clears() {
        let mut r = ring();
        for _ in 0..5 {
            push(&mut r, Entry::new(Reason::ApiReboot, fig(1)));
        }
        let e = boot(&mut r, Reset::PowerOn, None, None);
        assert_eq!((e.reason, e.seq, e.note()), (Reason::PowerOn, 1, None));
        assert_eq!(newest_first(&r).count(), 1);
        // Garbage (a power-on's RTC memory) reads as nothing.
        let mut g = [[0x5a5a_5a5au32; WORDS]; SLOTS];
        assert_eq!(newest(&g), None);
        let e = boot(&mut g, Reset::Software, None, None);
        assert_eq!((e.reason, e.seq), (Reason::Unrecorded, 1));
    }

    #[test]
    fn a_restart_out_of_the_updater() {
        let mut r = ring();
        boot(&mut r, Reset::Software, None, None);
        assert!(mark_update(&mut r));
        assert_eq!(newest(&r).unwrap().reason, Reason::Ota);
        assert!(!mark_update(&mut r));
    }

    #[test]
    fn side_records() {
        let f = fig(77).unwrap();
        assert_eq!(decode_alive(&encode_alive(&f)), Some(f));
        let mut bad = encode_alive(&f);
        bad[2] += 1;
        assert_eq!(decode_alive(&bad), None);
        let a = AllocFail { size: 12, at: f };
        assert_eq!(decode_alloc(&encode_alloc(&a)), Some(a));
        assert_eq!(decode_alloc(&[0; 6]), None);
    }

    #[test]
    fn minimal_push_matches_push() {
        let mut r = ring();
        boot(&mut r, Reset::PowerOn, None, None);
        let f = fig(4242).unwrap();
        push_minimal(&mut r, Reason::StackOverflow, f, &[b"task ", b"httpd"]);
        let e = newest(&r).unwrap();
        assert_eq!(
            (e.seq, e.reason, e.reset, e.text()),
            (2, Reason::StackOverflow, Reset::Pending, "task httpd")
        );
        assert_eq!(e.figures, Some(f));
        let mut want = Entry::new(Reason::StackOverflow, Some(f));
        want.seq = 2;
        want.set_text(format_args!("task httpd"));
        assert_eq!(read(&r, 2), encode(&want));
        // Non-ASCII and overlong text.
        let long = [b'x'; 100];
        push_minimal(&mut r, Reason::StackOverflow, f, &[b"\xff", &long]);
        let e = newest(&r).unwrap();
        assert_eq!(e.text().len(), TEXT);
        assert!(e.text().starts_with("?x"));
        // Into an empty (garbage) ring.
        let mut g = [[0x5a5a_5a5au32; WORDS]; SLOTS];
        push_minimal(&mut g, Reason::StackOverflow, f, &[b"task a"]);
        assert_eq!(newest(&g).unwrap().seq, 1);
    }

    #[test]
    fn seq_wraps() {
        let mut r = ring();
        let mut e = Entry::new(Reason::ApiReboot, fig(1));
        e.seq = u32::MAX - 2;
        r[e.seq as usize % SLOTS] = encode(&e);
        for _ in 0..5 {
            push(&mut r, Entry::new(Reason::Ota, fig(2)));
        }
        let seqs: Vec<u32> = newest_first(&r).map(|e| e.seq).collect();
        assert_eq!(seqs, [2, 1, 0, u32::MAX, u32::MAX - 1, u32::MAX - 2]);
        push_minimal(&mut r, Reason::Panic, fig(3).unwrap(), &[]);
        assert_eq!(newest(&r).unwrap().seq, 3);
    }
}
