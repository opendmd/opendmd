//! The microSD card's SPI clock (firmware `cardspeed.rs`, `sdhealth.rs`):
//! the pure part, here so it is tested on the host.
//!
//! **20 MHz is the default for every card; 40 MHz is opt-in.** The SD
//! specification only promises SPI mode up to the default speed (25 MHz);
//! 40 MHz works on most cards but not all, and on a marginal card it can
//! fail *silently*: the spare's 64 GB card (2026-10-25) served 2 of ~25
//! downloads at 40 MHz with the right length, a passing CRC and wrong
//! bytes, besides the errors it did report. At 20 MHz the card still reads
//! several times faster than playback (a prepared copy is tens of KB a
//! second) or a WiFi download (~300 KB/s) uses it. The `card_speed`
//! setting:
//!
//! - **`safe`** (the default): always 20 MHz.
//! - **`fast`**: 40 MHz, and at the first card I/O error the clock drops to
//!   20 MHz *in place* (the failed transfer tried again at 20 MHz, so
//!   nothing above sees the error), the card remembered as slow (by its CID
//!   serial) until a different card is seen or a person asks to try fast
//!   again. A card mounted at 40 MHz with no such record is read for a few
//!   MB soon after boot ([`plan`]) so one that cannot keep up is found
//!   early.
//! - **`full`** (API and bench only, not in the web app): always 40 MHz,
//!   never slowed, no record; its errors go to the card-failure restart.
//!
//! Older firmware's `auto` (40 MHz with the slow-down, then the default)
//! reads as `safe` ([`CardSpeed::from_nvs`]).
//!
//! In every mode the card-failure trip (3 errors in 60 s) and its restart
//! budget still guard against a card that fails at 20 MHz too.

/// Full speed: 80 MHz / 2, allowed on VSPI's IO_MUX pins.
pub const FULL_KHZ: u32 = 40_000;
/// The safe clock: 80 MHz / 4, under the 25 MHz default speed every card
/// supports and the GPIO matrix's 26.7 MHz, so the driver never refuses it.
pub const SAFE_KHZ: u32 = 20_000;

/// The `card_speed` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CardSpeed {
    /// Always the safe clock. The default.
    #[default]
    Safe,
    /// Full speed, slowing down by itself after an error.
    Fast,
    /// Always full speed, never slowed down (API and bench only).
    Full,
}

/// The 400 for a bad `card_speed`.
pub const INVALID: &str = "card_speed must be safe, fast or full";

impl CardSpeed {
    /// A value `PUT /api/config` accepts.
    pub fn parse(s: &str) -> Option<CardSpeed> {
        match s {
            "safe" => Some(CardSpeed::Safe),
            "fast" => Some(CardSpeed::Fast),
            "full" => Some(CardSpeed::Full),
            _ => None,
        }
    }

    /// What NVS holds: anything else, older firmware's `auto` included,
    /// and no value at all, is `safe`.
    pub fn from_nvs(s: Option<&str>) -> CardSpeed {
        s.and_then(CardSpeed::parse).unwrap_or_default()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CardSpeed::Safe => "safe",
            CardSpeed::Fast => "fast",
            CardSpeed::Full => "full",
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(v: u8) -> CardSpeed {
        match v {
            1 => CardSpeed::Fast,
            2 => CardSpeed::Full,
            _ => CardSpeed::Safe,
        }
    }
}

/// A clock for the card, and whether it is below full speed *for safety*
/// (`sd.fallback` in status: after errors, or with the restart budget
/// spent) rather than by choice (`safe`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    pub khz: u32,
    pub fallback: bool,
}

/// The clock a card runs at: `recorded`, this card has failed at full
/// speed before (its slow-clock record); `capped`, the card-failure
/// restart budget is spent (the remounts are at the safe clock whatever
/// the setting, as before).
pub fn clock_for(mode: CardSpeed, recorded: bool, capped: bool) -> Clock {
    let safe = |fallback| Clock {
        khz: SAFE_KHZ,
        fallback,
    };
    let full = Clock {
        khz: FULL_KHZ,
        fallback: false,
    };
    match mode {
        CardSpeed::Safe => safe(false),
        _ if capped => safe(true),
        CardSpeed::Full => full,
        CardSpeed::Fast if recorded => safe(true),
        CardSpeed::Fast => full,
    }
}

/// A card I/O error at `khz`: drop to [`SAFE_KHZ`] in place, record the
/// card, and try the transfer again? Only `fast` above the safe
/// clock.
pub fn drop_on_error(mode: CardSpeed, khz: u32) -> bool {
    mode == CardSpeed::Fast && khz > SAFE_KHZ
}

/// Read the card for a while after it mounts ([`plan`])? The same rule:
/// only `fast` at full speed (a card with a record is already
/// slow; `full` would only fail, `safe` cannot be slowed further).
pub fn should_probe(mode: CardSpeed, khz: u32) -> bool {
    drop_on_error(mode, khz)
}

/// A card-failure trip (3 errors in 60 s) at `khz`: remember the card as
/// slow for the next boot? In `fast` above the safe clock (the
/// in-place drop did not happen or did not help). `full` never slows down.
pub fn record_on_trip(mode: CardSpeed, khz: u32) -> bool {
    drop_on_error(mode, khz)
}

/// The probe's reads: `bytes` from sector `first`, in reads of `chunk`
/// sectors, never past `capacity` sectors. Nothing is written. Where it
/// reads hardly matters (the errors are the bus's, not the data's): the
/// firmware starts at 4 MiB, past the partition table's gap, where a card
/// formatted for SD (or by the device) has its volume and FAT.
pub fn plan(first: u32, capacity: u32, bytes: u32, chunk: u32) -> impl Iterator<Item = (u32, u32)> {
    let chunk = chunk.max(1);
    let end = first.saturating_add(bytes / 512).min(capacity);
    (first..end)
        .step_by(chunk as usize)
        .map(move |s| (s, chunk.min(end - s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for m in [CardSpeed::Safe, CardSpeed::Fast, CardSpeed::Full] {
            assert_eq!(CardSpeed::parse(m.as_str()), Some(m));
            assert_eq!(CardSpeed::from_u8(m.as_u8()), m);
            assert_eq!(CardSpeed::from_nvs(Some(m.as_str())), m);
        }
        assert_eq!(CardSpeed::parse("Fast"), None);
        assert_eq!(CardSpeed::parse("auto"), None, "auto is gone from the API");
        assert_eq!(CardSpeed::parse(""), None);
        assert_eq!(CardSpeed::from_u8(9), CardSpeed::Safe);
        assert_eq!(CardSpeed::default(), CardSpeed::Safe);
    }

    #[test]
    fn older_values_read_as_safe() {
        assert_eq!(CardSpeed::from_nvs(Some("auto")), CardSpeed::Safe);
        assert_eq!(CardSpeed::from_nvs(None), CardSpeed::Safe);
        assert_eq!(CardSpeed::from_nvs(Some("bogus")), CardSpeed::Safe);
    }

    #[test]
    fn clocks() {
        use CardSpeed::*;
        let c = |khz, fallback| Clock { khz, fallback };
        assert_eq!(clock_for(Fast, false, false), c(FULL_KHZ, false));
        assert_eq!(clock_for(Fast, true, false), c(SAFE_KHZ, true));
        assert_eq!(clock_for(Fast, false, true), c(SAFE_KHZ, true));
        // Safe is a choice, never "slowed for safety".
        assert_eq!(clock_for(Safe, false, false), c(SAFE_KHZ, false));
        assert_eq!(clock_for(Safe, true, true), c(SAFE_KHZ, false));
        // Full ignores the record, but not a spent restart budget.
        assert_eq!(clock_for(Full, true, false), c(FULL_KHZ, false));
        assert_eq!(clock_for(Full, false, true), c(SAFE_KHZ, true));
    }

    #[test]
    fn only_fast_at_full_speed_slows_down() {
        use CardSpeed::*;
        assert!(drop_on_error(Fast, FULL_KHZ));
        assert!(!drop_on_error(Fast, SAFE_KHZ));
        assert!(!drop_on_error(Full, FULL_KHZ));
        assert!(!drop_on_error(Safe, SAFE_KHZ));
        assert!(should_probe(Fast, FULL_KHZ));
        assert!(!should_probe(Fast, SAFE_KHZ));
        assert!(!should_probe(Full, FULL_KHZ));
        assert!(!should_probe(Safe, SAFE_KHZ));
        assert!(record_on_trip(Fast, FULL_KHZ));
        assert!(!record_on_trip(Full, FULL_KHZ));
        assert!(!record_on_trip(Fast, SAFE_KHZ));
    }

    #[test]
    fn plans_reads() {
        let reads: Vec<_> = plan(100, 1_000_000, 4096 * 3 + 1024, 8).collect();
        assert_eq!(reads, vec![(100, 8), (108, 8), (116, 8), (124, 2)]);
        // Never past the card's end.
        let reads: Vec<_> = plan(990, 1000, 1 << 20, 8).collect();
        assert_eq!(reads, vec![(990, 8), (998, 2)]);
        assert_eq!(plan(2000, 1000, 1 << 20, 8).count(), 0);
        // 4 MiB in 4 KB reads.
        assert_eq!(plan(0, u32::MAX, 4 << 20, 8).count(), 1024);
    }
}
