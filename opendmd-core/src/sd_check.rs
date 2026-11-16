//! Why a card will not mount, in plain words: a raw check of the card at
//! the slow clock before the driver's own start-up (firmware `sd.rs`), and
//! the record of the last card transfer that failed (firmware
//! `cardspeed.rs` / `sdhealth.rs`, kept in RTC memory across a restart).
//! The pure part, here so it is tested on the host.
//!
//! **The check** ([`run`], over a [`Wire`]: the firmware's SPI bus at
//! 400 kHz, CS driven by hand) is what a person with a logic analyser
//! would look at first:
//! - MISO while the card is not selected (CS high, 10 bytes of clocks:
//!   the 74 the card needs to wake) and selected before any command;
//! - the 8 bytes after `CMD0` (`GO_IDLE_STATE`), up to [`CMD0_TRIES`]
//!   times, as the driver tries it;
//! - `CMD8`'s R7 (voltage and check pattern), when CMD0 answered idle;
//! - how many `ACMD41`s (`CMD55` + `SD_SEND_OP_COND`, 10 ms apart, at most
//!   [`ACMD41_MAX`]: the spec's 1 s) the card took to leave the idle state.
//!
//! [`Check::class`] sorts it: no answer at all ([`Class::Silent`]: no card,
//! or one that has stopped answering until its power goes), MISO held low
//! ([`Class::StuckLow`]), idle for ever ([`Class::StuckIdle`]), garbage
//! ([`Class::Garbled`]), or a card that answers the slow check, so the
//! failure is elsewhere ([`Class::Answers`]).
//!
//! **The card that has stopped answering.** A card left mid-transfer by a
//! reset can stay silent to every CMD0 until its power is removed (seen
//! twice on a test board's passive SD module): no software retry
//! helps, so after [`SILENT_STREAK`] silent checks in a row on a boot that
//! did not cut the card's power (not a power-on or a brownout), status says
//! so ([`NOT_ANSWERING`]), worded for both cases: on the wire a wedged card
//! and an empty socket (a card pulled from a running display) look the
//! same. After a power-on a silent socket is most likely empty, which the
//! old words still say.

use core::fmt::Write as _;

/// CMD0 sent at most this many times (the driver's `sdmmc_init_idle`
/// retries it too).
pub const CMD0_TRIES: u8 = 3;
/// ACMD41s at most, [`ACMD41_PAUSE_MS`] apart: the spec's 1 s.
pub const ACMD41_MAX: u16 = 100;
pub const ACMD41_PAUSE_MS: u32 = 10;
/// Silent checks in a row before status says to cut the power (each
/// check already sends [`CMD0_TRIES`] CMD0s).
pub const SILENT_STREAK: u32 = 3;
/// `sd.error` once [`SILENT_STREAK`] is reached (not after a power-on):
/// for an empty socket and a card that has stopped answering alike.
pub const NOT_ANSWERING: &str =
    "No card found. If a card is in, unplug the power for 10 seconds, then plug it back in.";

/// The bus the check runs on: full duplex, MSB first, mode 0.
pub trait Wire {
    /// CS low (`true`, selected) or high.
    fn select(&mut self, on: bool);
    /// Clock `buf` out and replace it with what came in. `false`: the SPI
    /// driver failed (nothing is known about the card then).
    fn xfer(&mut self, buf: &mut [u8]) -> bool;
    fn pause_ms(&mut self, ms: u32);
}

/// What MISO did over a run of clocks with MOSI high.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Level {
    /// Not measured (the bus failed first).
    #[default]
    Unknown,
    /// Every byte 0xFF: released (pulled up) or driven high.
    High,
    /// Every byte 0x00.
    Low,
    /// Anything else.
    Mixed,
}

impl Level {
    pub fn of(bytes: &[u8]) -> Level {
        if bytes.iter().all(|&b| b == 0xff) {
            Level::High
        } else if bytes.iter().all(|&b| b == 0) {
            Level::Low
        } else {
            Level::Mixed
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Level::Unknown => "unknown",
            Level::High => "high",
            Level::Low => "low",
            Level::Mixed => "mixed",
        }
    }
}

/// The ACMD41 loop's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Acmd41 {
    /// ACMD41s sent.
    pub tries: u16,
    /// The last one's R1 (`0x00`: ready; `0x01`: still idle; `0xFF`: none).
    pub r1: u8,
}

/// One raw check's findings.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Check {
    /// MISO with CS high (the wake-up clocks).
    pub cs_high: Level,
    /// MISO with CS low, before CMD0.
    pub cs_low: Level,
    /// The 8 bytes after the last CMD0 sent.
    pub cmd0: [u8; 8],
    pub cmd0_tries: u8,
    /// The 8 bytes after CMD8 (R1, then the R7's four), when CMD0 answered idle.
    pub cmd8: Option<[u8; 8]>,
    pub acmd41: Option<Acmd41>,
    /// The SPI driver failed part way.
    pub bus_failed: bool,
}

/// What a [`Check`] says about the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// No reply to CMD0 at all (MISO high throughout).
    Silent,
    /// MISO held low ("DO stuck low").
    StuckLow,
    /// CMD0 answered, but ACMD41 never left the idle state.
    StuckIdle,
    /// Replies that make no sense.
    Garbled,
    /// The card answers the slow check: the failure is past it.
    Answers,
    /// The SPI bus could not be used.
    NoBus,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::Silent => "silent",
            Class::StuckLow => "do_stuck_low",
            Class::StuckIdle => "stuck_idle",
            Class::Garbled => "garbled",
            Class::Answers => "answers",
            Class::NoBus => "no_bus",
        }
    }

    /// In plain words, for `sd.error`.
    pub fn words(self) -> &'static str {
        match self {
            Class::Silent => "the card gives no answer at all (no card in the socket, or one that has stopped answering)",
            Class::StuckLow => "the card's data line is held low (a card stuck part way through something, or a wiring fault)",
            Class::StuckIdle => "the card answers but never finishes starting up",
            Class::Garbled => "the card's answers are garbled (a loose contact, or a failing card)",
            Class::Answers => "the card answers a slow check, but the driver's start-up failed",
            Class::NoBus => "the card's SPI bus could not be started",
        }
    }
}

const CMD0: [u8; 6] = [0x40, 0, 0, 0, 0, 0x95];
const CMD8: [u8; 6] = [0x48, 0, 0, 0x01, 0xaa, 0x87];
const CMD55: [u8; 6] = [0x77, 0, 0, 0, 0, 0x65];
const ACMD41_HCS: [u8; 6] = [0x69, 0x40, 0, 0, 0, 0x77];
const ACMD41_V1: [u8; 6] = [0x69, 0, 0, 0, 0, 0xe5];

/// The R1 in `bytes`: the first byte with bit 7 clear.
pub fn r1_of(bytes: &[u8]) -> Option<u8> {
    bytes.iter().copied().find(|b| b & 0x80 == 0)
}

/// One command, selected: its 6 bytes, then 8 clocked in.
fn command<W: Wire>(w: &mut W, cmd: [u8; 6]) -> Option<[u8; 8]> {
    let mut buf = [0xffu8; 14];
    buf[..6].copy_from_slice(&cmd);
    if !w.xfer(&mut buf) {
        return None;
    }
    let mut out = [0u8; 8];
    out.copy_from_slice(&buf[6..]);
    Some(out)
}

/// The check, start to end; the card is left deselected with 8 clocks.
pub fn run<W: Wire>(w: &mut W) -> Check {
    let mut c = Check::default();
    let done = |w: &mut W, mut c: Check, failed: bool| {
        w.select(false);
        let _ = w.xfer(&mut [0xff]);
        c.bus_failed |= failed;
        c
    };
    w.select(false);
    let mut wake = [0xffu8; 10];
    if !w.xfer(&mut wake) {
        return done(w, c, true);
    }
    c.cs_high = Level::of(&wake);
    w.select(true);
    let mut sel = [0xffu8; 2];
    if !w.xfer(&mut sel) {
        return done(w, c, true);
    }
    c.cs_low = Level::of(&sel);
    let mut r1 = None;
    while c.cmd0_tries < CMD0_TRIES {
        c.cmd0_tries += 1;
        let Some(rx) = command(w, CMD0) else {
            return done(w, c, true);
        };
        c.cmd0 = rx;
        r1 = r1_of(&rx);
        if r1 == Some(0x01) {
            break;
        }
        // Between tries: deselected, a byte of clocks (as the driver does).
        w.select(false);
        let _ = w.xfer(&mut [0xff]);
        w.select(true);
    }
    if r1 != Some(0x01) {
        return done(w, c, false);
    }
    let Some(r7) = command(w, CMD8) else {
        return done(w, c, true);
    };
    c.cmd8 = Some(r7);
    // A v2 card echoes the pattern; a v1 card calls CMD8 illegal (0x05).
    let v2 = r1_of(&r7) == Some(0x01);
    let mut a = Acmd41 { tries: 0, r1: 0xff };
    while a.tries < ACMD41_MAX {
        a.tries += 1;
        if command(w, CMD55).is_none() {
            c.acmd41 = Some(a);
            return done(w, c, true);
        }
        let Some(rx) = command(w, if v2 { ACMD41_HCS } else { ACMD41_V1 }) else {
            c.acmd41 = Some(a);
            return done(w, c, true);
        };
        a.r1 = r1_of(&rx).unwrap_or(0xff);
        if a.r1 != 0x01 {
            break;
        }
        w.pause_ms(ACMD41_PAUSE_MS);
    }
    c.acmd41 = Some(a);
    done(w, c, false)
}

impl Check {
    pub fn class(&self) -> Class {
        if self.bus_failed && self.cmd0_tries == 0 {
            return Class::NoBus;
        }
        let r1 = r1_of(&self.cmd0);
        if self.cmd0 == [0; 8] || (self.cs_high == Level::Low && r1 != Some(0x01)) {
            return Class::StuckLow;
        }
        match r1 {
            None if self.cmd0 == [0xff; 8] => Class::Silent,
            None => Class::Garbled,
            Some(0x01) => match self.acmd41 {
                Some(Acmd41 { r1: 0x00, .. }) => Class::Answers,
                Some(Acmd41 { r1: 0x01, .. }) => Class::StuckIdle,
                Some(_) => Class::Garbled,
                // The bus failed between CMD0 and ACMD41.
                None => Class::NoBus,
            },
            Some(_) => Class::Garbled,
        }
    }

    /// `sd.check`'s fields after `class` (no braces): what was seen.
    pub fn json_fields(&self, out: &mut String) {
        let _ = write!(
            out,
            r#""miso_cs_high":"{}","miso_cs_low":"{}","cmd0":"{}","cmd0_tries":{},"cmd8":"#,
            self.cs_high.name(),
            self.cs_low.name(),
            Hex(&self.cmd0),
            self.cmd0_tries
        );
        match &self.cmd8 {
            Some(b) => {
                let _ = write!(out, r#""{}""#, Hex(b));
            }
            None => out.push_str("null"),
        }
        match self.acmd41 {
            Some(a) => {
                let _ = write!(out, r#","acmd41":{{"tries":{},"r1":{}}}"#, a.tries, a.r1);
            }
            None => out.push_str(r#","acmd41":null"#),
        }
    }
}

/// Bytes as `ff ff 01 …`.
pub struct Hex<'a>(pub &'a [u8]);

impl core::fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// `sd.error` for a mount that failed after a check: [`NOT_ANSWERING`]
/// once `silent` checks in a row reach [`SILENT_STREAK`] on a boot that was
/// not a power-on (`poweron`: or a brownout: the card's power was cut), else
/// the class in words, then the driver's error.
pub fn mount_error(class: Class, silent: u32, poweron: bool, driver: &str) -> String {
    if stuck(class, silent, poweron) {
        return NOT_ANSWERING.to_string();
    }
    format!("{}; {driver}", class.words())
}

/// Has the card stopped answering for good (until its power goes)?
pub fn stuck(class: Class, silent: u32, poweron: bool) -> bool {
    class == Class::Silent && silent >= SILENT_STREAK && !poweron
}

// ---------------------------------------------------------------------------
// The failed transfer
// ---------------------------------------------------------------------------

/// A card transfer that failed: what the driver was asked and what it said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoFault {
    pub write: bool,
    /// The SD command the driver uses for it (17/18 read, 24/25 write).
    pub cmd: u8,
    pub sector: u32,
    pub count: u32,
    /// The `esp_err_t`.
    pub err: i32,
    /// The clock (kHz) it ran at.
    pub khz: u32,
}

/// Words in an encoded [`IoFault`]: magic, meta, sector, count, err, check.
pub const FAULT_WORDS: usize = 6;
const FAULT_MAGIC: u32 = 0x10fa_0c71;

fn fault_check(w: &[u32]) -> u32 {
    w.iter().fold(0x811c_9dc5u32, |h, &v| (h ^ v).wrapping_mul(0x0100_0193))
}

/// The command the IDF's `sdmmc_{read,write}_sectors` sends: a multi-block
/// one for more than one sector from a DMA-capable buffer (`direct`), else
/// one sector at a time through its bounce buffer.
pub fn cmd_for(write: bool, count: u32, direct: bool) -> u8 {
    match (write, count > 1 && direct) {
        (false, false) => 17,
        (false, true) => 18,
        (true, false) => 24,
        (true, true) => 25,
    }
}

impl IoFault {
    pub fn encode(&self) -> [u32; FAULT_WORDS] {
        let meta = u32::from(self.write) | u32::from(self.cmd) << 8 | self.khz.min(0xffff) << 16;
        let mut w = [FAULT_MAGIC, meta, self.sector, self.count, self.err as u32, 0];
        w[5] = fault_check(&w[..5]);
        w
    }

    /// `None` for anything that is not a record (a power-on's garbage,
    /// zeroes, another layout's words).
    pub fn decode(w: &[u32; FAULT_WORDS]) -> Option<IoFault> {
        if w[0] != FAULT_MAGIC || w[5] != fault_check(&w[..5]) {
            return None;
        }
        Some(IoFault {
            write: w[1] & 1 != 0,
            cmd: (w[1] >> 8) as u8,
            sector: w[2],
            count: w[3],
            err: w[4] as i32,
            khz: w[1] >> 16,
        })
    }

    pub fn op(&self) -> &'static str {
        if self.write {
            "write"
        } else {
            "read"
        }
    }

    /// `{"op":"write","cmd":25,"sector":N,"count":N,"err":"ESP_ERR_TIMEOUT","code":263,"khz":40000}`.
    pub fn json(&self) -> String {
        format!(
            r#"{{"op":"{}","cmd":{},"sector":{},"count":{},"err":"{}","code":{},"khz":{}}}"#,
            self.op(),
            self.cmd,
            self.sector,
            self.count,
            ErrName(self.err),
            self.err,
            self.khz
        )
    }

    /// One line, at most 80 bytes (the restart ring's note).
    pub fn note(&self) -> String {
        format!(
            "{} CMD{} sector {} x{} at {} kHz: {}",
            self.op(),
            self.cmd,
            self.sector,
            self.count,
            self.khz,
            ErrName(self.err)
        )
    }
}

/// An `esp_err_t` by its name (the ones the SD driver returns), else hex.
pub struct ErrName(pub i32);

impl core::fmt::Display for ErrName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = match self.0 {
            -1 => "ESP_FAIL",
            0x101 => "ESP_ERR_NO_MEM",
            0x102 => "ESP_ERR_INVALID_ARG",
            0x103 => "ESP_ERR_INVALID_STATE",
            0x104 => "ESP_ERR_INVALID_SIZE",
            0x105 => "ESP_ERR_NOT_FOUND",
            0x106 => "ESP_ERR_NOT_SUPPORTED",
            0x107 => "ESP_ERR_TIMEOUT",
            0x108 => "ESP_ERR_INVALID_RESPONSE",
            0x109 => "ESP_ERR_INVALID_CRC",
            0x10a => "ESP_ERR_INVALID_VERSION",
            0x10b => "ESP_ERR_INVALID_MAC",
            0x10c => "ESP_ERR_NOT_FINISHED",
            0x10d => "ESP_ERR_NOT_ALLOWED",
            e => return write!(f, "0x{e:x}"),
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card in SPI mode, as far as the check goes: `kind` picks how it
    /// misbehaves. Records what the host sent.
    #[derive(Default)]
    struct Card {
        kind: Kind,
        selected: bool,
        /// Bytes of the command being received.
        cmd: Vec<u8>,
        out: std::collections::VecDeque<u8>,
        app: bool,
        acmd41s: u16,
        /// ACMD41s before it is ready.
        ready_after: u16,
        pauses: u32,
        fail_after: Option<usize>,
        xfers: usize,
        v1: bool,
    }

    #[derive(Default, Clone, Copy, PartialEq)]
    enum Kind {
        #[default]
        Good,
        /// Nothing on MISO ever: pulled up.
        Silent,
        /// MISO held at 0.
        Low,
        /// Idle for ever.
        Idle,
        /// Answers CMD0 with noise.
        Noise,
        /// Silent for the first CMD0, then fine.
        SlowWake,
    }

    impl Card {
        fn of(kind: Kind) -> Card {
            Card { kind, ready_after: 3, ..Card::default() }
        }

        fn byte(&mut self, mosi: u8) -> u8 {
            if !self.selected {
                return if self.kind == Kind::Low { 0 } else { 0xff };
            }
            let miso = match self.kind {
                Kind::Silent => 0xff,
                Kind::Low => 0,
                _ => self.out.pop_front().unwrap_or(0xff),
            };
            if self.cmd.is_empty() && mosi & 0xc0 != 0x40 {
                return miso;
            }
            self.cmd.push(mosi);
            if self.cmd.len() == 6 {
                let index = self.cmd[0] & 0x3f;
                self.cmd.clear();
                self.respond(index);
            }
            miso
        }

        fn respond(&mut self, index: u8) {
            let app = std::mem::take(&mut self.app);
            let r: Vec<u8> = match (index, app) {
                (0, _) => match self.kind {
                    Kind::Noise => vec![0xff, 0xfe, 0xf0, 0xbf],
                    Kind::SlowWake if self.xfers < 6 => vec![],
                    _ => vec![0xff, 0x01],
                },
                (8, _) if self.v1 => vec![0xff, 0x05],
                (8, _) => vec![0xff, 0x01, 0, 0, 0x01, 0xaa],
                (55, _) => {
                    self.app = true;
                    vec![0xff, 0x01]
                }
                (41, true) => {
                    self.acmd41s += 1;
                    let ready = self.kind != Kind::Idle && self.acmd41s >= self.ready_after;
                    vec![0xff, if ready { 0x00 } else { 0x01 }]
                }
                _ => vec![0xff, 0x05],
            };
            self.out.extend(r);
        }
    }

    impl Wire for Card {
        fn select(&mut self, on: bool) {
            self.selected = on;
            if !on {
                self.cmd.clear();
                self.out.clear();
            }
        }
        fn xfer(&mut self, buf: &mut [u8]) -> bool {
            self.xfers += 1;
            if self.fail_after.is_some_and(|n| self.xfers > n) {
                return false;
            }
            for b in buf.iter_mut() {
                *b = self.byte(*b);
            }
            true
        }
        fn pause_ms(&mut self, ms: u32) {
            self.pauses += ms;
        }
    }

    #[test]
    fn a_good_card_answers_and_counts_its_acmd41s() {
        let mut card = Card::of(Kind::Good);
        let c = run(&mut card);
        assert_eq!(c.class(), Class::Answers);
        assert_eq!((c.cs_high, c.cs_low), (Level::High, Level::High));
        assert_eq!(r1_of(&c.cmd0), Some(0x01));
        assert_eq!(c.cmd0_tries, 1);
        assert_eq!(c.cmd8.map(|b| b[1..6].to_vec()), Some(vec![0x01, 0, 0, 0x01, 0xaa]));
        assert_eq!(c.acmd41, Some(Acmd41 { tries: 3, r1: 0 }));
        // Paused between the idle answers only.
        assert_eq!(card.pauses, 2 * ACMD41_PAUSE_MS);
        assert!(!card.selected, "left deselected");
    }

    #[test]
    fn a_v1_card_is_asked_without_hcs_and_answers() {
        let mut card = Card::of(Kind::Good);
        card.v1 = true;
        let c = run(&mut card);
        assert_eq!(r1_of(&c.cmd8.unwrap()), Some(0x05));
        assert_eq!(c.class(), Class::Answers);
    }

    #[test]
    fn a_silent_card_is_silent_after_three_cmd0s() {
        let c = run(&mut Card::of(Kind::Silent));
        assert_eq!(c.class(), Class::Silent);
        assert_eq!(c.cmd0, [0xff; 8]);
        assert_eq!(c.cmd0_tries, CMD0_TRIES);
        assert_eq!((c.cmd8, c.acmd41), (None, None));
    }

    #[test]
    fn a_card_slow_to_wake_answers_a_later_cmd0() {
        let c = run(&mut Card::of(Kind::SlowWake));
        assert!(c.cmd0_tries > 1, "{c:?}");
        assert_eq!(c.class(), Class::Answers);
    }

    #[test]
    fn miso_held_low_is_stuck_low() {
        let c = run(&mut Card::of(Kind::Low));
        assert_eq!(c.cs_high, Level::Low);
        assert_eq!(c.class(), Class::StuckLow);
    }

    #[test]
    fn a_card_idle_for_ever_is_stuck_idle_after_a_second() {
        let mut card = Card::of(Kind::Idle);
        let c = run(&mut card);
        assert_eq!(c.class(), Class::StuckIdle);
        assert_eq!(c.acmd41, Some(Acmd41 { tries: ACMD41_MAX, r1: 1 }));
        assert_eq!(card.pauses, u32::from(ACMD41_MAX) * ACMD41_PAUSE_MS);
    }

    #[test]
    fn noise_is_garbled() {
        assert_eq!(run(&mut Card::of(Kind::Noise)).class(), Class::Garbled);
        // An R1 with error bits to CMD0 is not idle either.
        let c = Check { cs_high: Level::High, cs_low: Level::High, cmd0: [0xff, 0x05, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff], cmd0_tries: 3, ..Check::default() };
        assert_eq!(c.class(), Class::Garbled);
        // MISO low only while deselected (another device on the bus?), no idle answer.
        let c = Check { cs_high: Level::Low, cmd0: [0xff; 8], cmd0_tries: 3, ..Check::default() };
        assert_eq!(c.class(), Class::StuckLow);
    }

    #[test]
    fn a_bus_that_fails_is_no_bus() {
        let mut card = Card::of(Kind::Good);
        card.fail_after = Some(0);
        let c = run(&mut card);
        assert!(c.bus_failed);
        assert_eq!(c.class(), Class::NoBus);
        // Failing between CMD0 and ACMD41.
        let mut card = Card::of(Kind::Good);
        card.fail_after = Some(3);
        let c = run(&mut card);
        assert_eq!(c.class(), Class::NoBus, "{c:?}");
    }

    #[test]
    fn json_names_what_was_seen() {
        let c = run(&mut Card::of(Kind::Silent));
        let mut s = String::new();
        c.json_fields(&mut s);
        assert_eq!(
            s,
            r#""miso_cs_high":"high","miso_cs_low":"high","cmd0":"ff ff ff ff ff ff ff ff","cmd0_tries":3,"cmd8":null,"acmd41":null"#
        );
        let c = run(&mut Card::of(Kind::Good));
        let mut s = String::new();
        c.json_fields(&mut s);
        assert!(s.ends_with(r#""cmd8":"ff 01 00 00 01 aa ff ff","acmd41":{"tries":3,"r1":0}"#), "{s}");
    }

    #[test]
    fn the_power_advice_needs_a_streak_of_silence_after_a_restart() {
        let driver = "no card, or card init failed: ESP_ERR_TIMEOUT";
        for n in 0..SILENT_STREAK {
            let e = mount_error(Class::Silent, n, false, driver);
            assert!(e.starts_with("the card gives no answer") && e.ends_with(driver), "{e}");
        }
        assert_eq!(mount_error(Class::Silent, SILENT_STREAK, false, driver), NOT_ANSWERING);
        assert_eq!(mount_error(Class::Silent, 40, false, driver), NOT_ANSWERING);
        // After a power-on: most likely no card at all.
        assert_ne!(mount_error(Class::Silent, 40, true, driver), NOT_ANSWERING);
        // Only silence: a card stuck idle is not one the power advice is for.
        assert_ne!(mount_error(Class::StuckIdle, 40, false, driver), NOT_ANSWERING);
    }

    #[test]
    fn a_fault_round_trips_and_garbage_does_not_decode() {
        let f = IoFault { write: true, cmd: 25, sector: 123_456_789, count: 8, err: 0x107, khz: 40_000 };
        assert_eq!(IoFault::decode(&f.encode()), Some(f));
        let r = IoFault { write: false, cmd: 17, sector: 0, count: 1, err: -1, khz: 20_000 };
        assert_eq!(IoFault::decode(&r.encode()), Some(r));
        assert_eq!(IoFault::decode(&[0; FAULT_WORDS]), None);
        assert_eq!(IoFault::decode(&[0xdead_beef; FAULT_WORDS]), None);
        for i in 0..FAULT_WORDS {
            let mut w = f.encode();
            w[i] ^= 1 << 7;
            assert_eq!(IoFault::decode(&w), None, "word {i} flipped");
        }
    }

    #[test]
    fn a_fault_in_words_and_json() {
        let f = IoFault { write: true, cmd: 25, sector: 4_294_967_295, count: 4_294_967_295, err: 0x10d, khz: 40_000 };
        let n = f.note();
        assert!(n.len() <= 80, "{} bytes: {n}", n.len());
        assert_eq!(n, "write CMD25 sector 4294967295 x4294967295 at 40000 kHz: ESP_ERR_NOT_ALLOWED");
        let f = IoFault { write: false, cmd: 18, sector: 8192, count: 4, err: 0x109, khz: 20_000 };
        assert_eq!(f.json(), r#"{"op":"read","cmd":18,"sector":8192,"count":4,"err":"ESP_ERR_INVALID_CRC","code":265,"khz":20000}"#);
        assert_eq!(ErrName(0x1234).to_string(), "0x1234");
    }

    #[test]
    fn the_command_is_the_one_the_driver_sends() {
        assert_eq!(cmd_for(false, 1, true), 17);
        assert_eq!(cmd_for(false, 4, true), 18);
        assert_eq!(cmd_for(false, 4, false), 17);
        assert_eq!(cmd_for(true, 1, true), 24);
        assert_eq!(cmd_for(true, 8, true), 25);
        assert_eq!(cmd_for(true, 8, false), 24);
    }
}
