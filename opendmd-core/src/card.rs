//! The microSD card: formatting it from the device (`POST /api/card/format`,
//! firmware `boot.rs` and `sd::SdBus::format`), and telling one card from
//! another (`sd.card_id`, [`identity`]): the pure parts, here so they are
//! tested on the host.
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

// ---- which card this is -------------------------------------------------------

/// What a FAT boot sector says about the volume: its serial number (set
/// by whoever formatted it: a new one on every format) and its label, as
/// the boot sector holds it (11 bytes, space-padded; `NO NAME    ` when
/// none was given). Windows' "rename" changes only the root directory's
/// label entry, not this one, so it changes with a format and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Volume {
    pub serial: u32,
    pub label: [u8; 11],
}

impl Volume {
    /// The label as text, trailing spaces trimmed ("" for none).
    pub fn label(&self) -> String {
        let l = String::from_utf8_lossy(&self.label).trim_end().to_string();
        if l == "NO NAME" {
            String::new()
        } else {
            l
        }
    }
}

/// [`Volume`] from a volume's boot sector (its first sector: on the device
/// FatFs's `volbase`, after any partition table). `None` if it is not a FAT
/// boot sector, or one with no extended boot record (a pre-DOS-4 format:
/// no serial to read). FAT12/16 and FAT32 keep the record at different
/// offsets; a FAT32 boot sector says "16-bit FAT size 0".
pub fn volume(boot: &[u8]) -> Option<Volume> {
    if boot.len() < 512 || boot[510..512] != [0x55, 0xAA] {
        return None;
    }
    let u16_ = |o: usize| u16::from_le_bytes([boot[o], boot[o + 1]]);
    let bps = u16_(11);
    if !(512..=4096).contains(&bps) || !bps.is_power_of_two() || boot[13] == 0 {
        return None;
    }
    let at = if u16_(22) == 0 { 64 } else { 36 };
    // The extended boot record's signature: 0x29 (0x28 has the serial and no label).
    let sig = boot[at + 2];
    if sig != 0x29 && sig != 0x28 {
        return None;
    }
    let serial = u32::from_le_bytes(boot[at + 3..at + 7].try_into().unwrap());
    let mut label = [b' '; 11];
    if sig == 0x29 {
        label.copy_from_slice(&boot[at + 7..at + 18]);
    }
    Some(Volume { serial, label })
}

/// The card's identity for `/api/status` `sd.card_id`: 16 hex digits,
/// FNV-1a 64 over the card's CID serial (which card it is: a card image
/// copied onto many cards has one volume serial), the volume's serial and
/// label (a new one on every format: the same card wiped and copied again)
/// and the capacity. Read once per mount from what the mount has read
/// anyway (the CID, the CSD) plus one sector: no scan. What it cannot see:
/// files changed on the same volume without a format (a computer deleting
/// and copying again); off-card stores keep their own per-file checks for that.
pub fn identity(cid_serial: u32, volume: Option<&Volume>, capacity_bytes: u64) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    eat(&cid_serial.to_le_bytes());
    match volume {
        Some(v) => {
            eat(&[1]);
            eat(&v.serial.to_le_bytes());
            eat(&v.label);
        }
        None => eat(&[0]),
    }
    eat(&capacity_bytes.to_le_bytes());
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boot32(serial: u32, label: &[u8; 11]) -> [u8; 512] {
        let mut b = [0u8; 512];
        b[11..13].copy_from_slice(&512u16.to_le_bytes());
        b[13] = 64;
        b[66] = 0x29;
        b[67..71].copy_from_slice(&serial.to_le_bytes());
        b[71..82].copy_from_slice(label);
        b[82..90].copy_from_slice(b"FAT32   ");
        b[510] = 0x55;
        b[511] = 0xAA;
        b
    }

    #[test]
    fn volume_from_a_boot_sector() {
        let b = boot32(0x1234_ABCD, b"OPENDMD    ");
        let v = volume(&b).unwrap();
        assert_eq!(v.serial, 0x1234_ABCD);
        assert_eq!(v.label(), "OPENDMD");
        assert_eq!(volume(&boot32(1, b"NO NAME    ")).unwrap().label(), "");
        // FAT16: the record at 36, the 16-bit FAT size set.
        let mut f = [0u8; 512];
        f[11..13].copy_from_slice(&512u16.to_le_bytes());
        f[13] = 4;
        f[22] = 9;
        f[38] = 0x29;
        f[39..43].copy_from_slice(&7u32.to_le_bytes());
        f[43..54].copy_from_slice(b"SMALL      ");
        f[510] = 0x55;
        f[511] = 0xAA;
        assert_eq!(
            volume(&f),
            Some(Volume {
                serial: 7,
                label: *b"SMALL      "
            })
        );
    }

    #[test]
    fn not_a_boot_sector() {
        let good = boot32(5, b"A          ");
        let mut b = good;
        b[511] = 0;
        assert_eq!(volume(&b), None, "no signature");
        let mut b = good;
        b[66] = 0;
        assert_eq!(volume(&b), None, "no extended record");
        let mut b = good;
        b[11] = 0;
        b[12] = 3;
        assert_eq!(volume(&b), None, "768 B sectors");
        assert_eq!(volume(&good[..100]), None, "short");
        // 0x28: a serial, no label.
        let mut b = good;
        b[66] = 0x28;
        assert_eq!(volume(&b).unwrap().label, [b' '; 11]);
    }

    #[test]
    fn identity_tells_cards_apart() {
        let v = volume(&boot32(0xAAAA_0001, b"OPENDMD    ")).unwrap();
        let id = identity(0x0102_0304, Some(&v), 31_914_983_424);
        assert_eq!(id.len(), 16);
        assert!(id
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(
            id,
            identity(0x0102_0304, Some(&v), 31_914_983_424),
            "stable"
        );
        // Another card with the same image (another CID), the same card
        // formatted again (another volume serial), relabelled at a format,
        // another size, or a volume with no serial: all differ.
        let w = Volume {
            serial: 0xAAAA_0002,
            ..v
        };
        let l = Volume {
            label: *b"DMDOS      ",
            ..v
        };
        let others = [
            identity(0x0102_0305, Some(&v), 31_914_983_424),
            identity(0x0102_0304, Some(&w), 31_914_983_424),
            identity(0x0102_0304, Some(&l), 31_914_983_424),
            identity(0x0102_0304, Some(&v), 15_931_539_456),
            identity(0x0102_0304, None, 31_914_983_424),
        ];
        for o in &others {
            assert_ne!(*o, id);
        }
    }

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
