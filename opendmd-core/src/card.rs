//! Formatting the microSD card from the device (`POST /api/card/format`,
//! firmware `boot.rs` and `sd::SdBus::format`): the pure part, here so it
//! is tested on the host.
//!
//! Most cards over 32 GB come formatted exFAT, which the firmware's FATFS
//! cannot read: such a card mounts and then answers nothing (`sd.readable`
//! false). Formatting it FAT32 on the device lets a user fix that with no
//! computer. **The safety rule**: only a card that is mounted and *not*
//! readable may be formatted. A readable card holds the user's GIFs and
//! playlists and is never erased through the API, however the request is
//! worded; no card means nothing to format.

/// The FAT allocation unit (cluster) a format asks for: 32 KB. `f_mkfs`
/// picks the same on its own for a FAT32 volume of 32 GB and up (and keeps
/// a smaller card's FAT16 valid by halving it), but asking keeps the
/// layout independent of the FATFS version. On a 64 GB card: ~1.9 M
/// clusters, each FAT 7.6 MB, so the format writes ~15 MB of FAT.
pub const ALLOCATION_UNIT: u32 = 32 * 1024;

/// The data area's alignment, in 512 B sectors: 4 MiB, the SD
/// specification's allocation unit for an SDHC/SDXC card of this size, so
/// every 32 KB cluster lies inside one of the card's erase blocks.
pub const ALIGN_SECTORS: u32 = 4 * 1024 * 1024 / 512;

/// Why a format request is refused (HTTP 409).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No card mounted.
    NoCard,
    /// The card can be read: it may hold GIFs; never formatted.
    Readable,
    /// A format is already running.
    Busy,
}

impl Refusal {
    /// The reply's `error`, in words.
    pub fn reason(self) -> &'static str {
        match self {
            Refusal::NoCard => "no card to format",
            Refusal::Readable => "the card can be read; only an unreadable card is formatted",
            Refusal::Busy => "the card is being formatted already",
        }
    }
}

/// May the card be formatted now? `mounted`/`readable` as status reports
/// them (`sd.mounted`, `sd.readable`), `formatting` a format under way.
pub fn may_format(mounted: bool, readable: bool, formatting: bool) -> Result<(), Refusal> {
    if formatting {
        Err(Refusal::Busy)
    } else if !mounted {
        Err(Refusal::NoCard)
    } else if readable {
        Err(Refusal::Readable)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_mounted_unreadable_card() {
        assert_eq!(may_format(true, false, false), Ok(()));
        assert_eq!(may_format(true, true, false), Err(Refusal::Readable));
        assert_eq!(may_format(false, false, false), Err(Refusal::NoCard));
        // `readable` without `mounted` cannot happen; still no card.
        assert_eq!(may_format(false, true, false), Err(Refusal::NoCard));
    }

    #[test]
    fn one_at_a_time() {
        for (m, r) in [(true, false), (true, true), (false, false)] {
            assert_eq!(may_format(m, r, true), Err(Refusal::Busy));
        }
    }

    #[test]
    fn reasons_are_distinct_words() {
        let all = [Refusal::NoCard, Refusal::Readable, Refusal::Busy].map(Refusal::reason);
        assert!(all.iter().all(|r| !r.is_empty()));
        assert!(all[0] != all[1] && all[1] != all[2] && all[0] != all[2]);
    }

    #[test]
    fn fat_parameters() {
        // FATFS takes a power of two of at most 128 sectors for FAT/FAT32,
        // and an alignment of at most 0x8000 sectors, also a power of two.
        assert!(ALLOCATION_UNIT.is_power_of_two() && ALLOCATION_UNIT / 512 <= 128);
        assert!(ALIGN_SECTORS.is_power_of_two() && ALIGN_SECTORS <= 0x8000);
        assert_eq!(ALIGN_SECTORS % (ALLOCATION_UNIT / 512), 0);
        // 64 GB: well inside FAT32's cluster range.
        let clusters = 62_534_975_488u64 / u64::from(ALLOCATION_UNIT);
        assert!(clusters > 65_525 && clusters < 0x0FFF_FFF5);
    }
}
