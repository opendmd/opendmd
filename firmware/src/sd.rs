//! microSD over SPI, mounted as FAT at `/sdcard`.
//!
//! The DMDos carrier wires the card to a plain SPI socket, not the SDMMC
//! peripheral: VSPI/SPI2, `SCK=18 MISO=19 MOSI=23 CS=5` (measured, see
//! `hardware/PINOUT.md`). Card detect and write protect are not wired.
//!
//! Note that long filenames are a build-time concern, not a runtime one:
//! `CONFIG_FATFS_LFN_HEAP=y` and `CONFIG_FATFS_MAX_LFN=255` in
//! `sdkconfig.defaults` are what make `/gifs/NEW 2021/...` openable at all.

use anyhow::{Context, Result};
use esp_idf_hal::gpio::{AnyIOPin, AnyInputPin, AnyOutputPin};
use esp_idf_hal::peripheral::Peripheral;
use esp_idf_hal::sd::spi::SdSpiHostDriver;
use esp_idf_hal::sd::{SdCardConfiguration, SdCardDriver};
use esp_idf_hal::spi::config::DriverConfig;
use esp_idf_hal::spi::{Dma, SpiDriver, SPI2};
use esp_idf_svc::fs::fatfs::Fatfs;
use esp_idf_svc::io::vfs::MountedFatfs;
use opendmd_core::engine::{ReadSeek, Source};
use opendmd_core::layout;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::time::Instant;

use crate::config;

type Host = SdSpiHostDriver<'static, SpiDriver<'static>>;
type Card = SdCardDriver<Host>;

/// The SD socket's SPI peripheral and pins, owned for the life of the
/// program; every mount attempt builds the whole stack on them afresh.
///
/// **Each attempt is a full re-initialisation**: the SPI bus
/// (`spi_bus_initialize`), the SD SPI host and device, and the card's init
/// sequence (CMD0 onwards) — and a failed attempt tears all three down again
/// (`sdspi_host_remove_device`, `sdspi_host_deinit`, `spi_bus_free`, in the
/// drivers' drops). A card left mid-transfer by a reset (a crash, a flash, a
/// reboot while the cache builder was writing) can answer CMD0 with a
/// timeout for a while — seen on the rig: `ESP_ERR_TIMEOUT` twice, then a
/// mount 10 s later — and a retry must not inherit a bus in whatever state
/// the failed attempt left it. Attempts after the first also give each
/// command longer ([`config::SD_RETRY_CMD_TIMEOUT_MS`]).
pub struct SdBus {
    spi: SPI2,
    sck: AnyOutputPin,
    mosi: AnyOutputPin,
    miso: AnyIOPin,
    cs: AnyOutputPin,
    attempts: u32,
}

/// A mounted card. Dropping it unmounts, so keep it alive for the whole run.
pub struct SdCard {
    _fs: MountedFatfs<Fatfs<Card>>,
    /// Usable capacity in bytes, as reported by the card's CSD.
    pub capacity_bytes: u64,
}

impl SdBus {
    pub fn new(
        spi: SPI2,
        sck: AnyOutputPin,
        mosi: AnyOutputPin,
        miso: AnyIOPin,
        cs: AnyOutputPin,
    ) -> SdBus {
        SdBus {
            spi,
            sck,
            mosi,
            miso,
            cs,
            attempts: 0,
        }
    }

    /// Mount attempts made so far (the one in progress included).
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// One mount attempt. Fails (rather than formatting) when no card is
    /// present, the card is unreadable, or it is not FAT — the Rust `Fatfs`
    /// API never formats, which is the `format_if_mount_failed = false` the
    /// brief asks for: a card that won't mount is a card to look at, not one
    /// to erase.
    pub fn mount(&mut self) -> Result<SdCard> {
        self.attempts += 1;
        if take_simulated_failure() {
            anyhow::bail!("simulated failure (POST /api/reboot?sd_fail=N)");
        }
        // SAFETY: the peripheral and the pins are this struct's alone, and
        // the drivers built on them here are either kept (inside the
        // returned `SdCard`, after which `mount` is never called again) or
        // dropped before this returns — never two at once.
        let (spi, sck, mosi, miso, cs) = unsafe {
            (
                self.spi.clone_unchecked(),
                self.sck.clone_unchecked(),
                self.mosi.clone_unchecked(),
                self.miso.clone_unchecked(),
                self.cs.clone_unchecked(),
            )
        };
        let driver = SpiDriver::new(
            spi,
            sck,
            mosi,
            Some(miso),
            &DriverConfig::default().dma(Dma::Auto(4096)),
        )
        .context("SPI bus init failed")?;
        let host = SdSpiHostDriver::new(
            driver,
            Some(cs),
            AnyInputPin::none(),
            AnyInputPin::none(),
            AnyInputPin::none(),
            None,
        )
        .context("SD SPI host init failed")?;

        let mut cfg = SdCardConfiguration::new();
        cfg.speed_khz = config::SD_SPEED_KHZ;
        if self.attempts > 1 {
            cfg.command_timeout_ms = config::SD_RETRY_CMD_TIMEOUT_MS;
        }

        let card = SdCardDriver::new_spi(host, &cfg).context("no card, or card init failed")?;

        let raw = card.card();
        let capacity_bytes = (raw.csd.capacity as u64) * (raw.csd.sector_size as u64);

        let fatfs = Fatfs::new_sdcard(0, card).context("FAT driver registration failed")?;
        let mounted = MountedFatfs::mount(fatfs, config::SD_MOUNT, config::SD_MAX_FILES)
            .context("FAT mount failed")?;

        Ok(SdCard {
            _fs: mounted,
            capacity_bytes,
        })
    }
}

/// Test hook: mount attempts to fail on purpose after the next restart, so
/// the no-card boot (network up, mount retried) can be exercised without
/// pulling the card. Kept in RTC slow memory, which a software restart
/// leaves alone and a power-on fills with garbage — hence the magic word.
#[link_section = ".rtc_noinit"]
static mut SIM_FAIL: [u32; 2] = [0; 2];
const SIM_MAGIC: u32 = 0x5d_fa_11_ed;

/// Arm [`SIM_FAIL`]: the next boot's first `n` mount attempts fail.
pub fn simulate_mount_failures(n: u32) {
    // SAFETY: written once, from the httpd task, just before a restart;
    // read only by the main task during boot.
    unsafe {
        let p = &raw mut SIM_FAIL;
        (*p)[0] = SIM_MAGIC;
        (*p)[1] = n;
    }
}

fn take_simulated_failure() -> bool {
    // SAFETY: the main task is the only one mounting, see above.
    unsafe {
        let p = &raw mut SIM_FAIL;
        if (*p)[0] != SIM_MAGIC || (*p)[1] == 0 {
            (*p)[0] = 0;
            return false;
        }
        (*p)[1] -= 1;
        true
    }
}

/// `opendmd_core::engine::Source` over the mounted card, with the playback
/// cache resolved underneath it.
///
/// Manifest paths are card-absolute (`/gifs/Consoles/X.gif`); joining them
/// onto the mount point is this type's job, and so is **transparent
/// resolution**: for a `/gifs/…/X.gif` path it first tries the prepared copy
/// at `opendmd_core::layout::cache_path` (`/cache/Consoles/<xx>/X.gif`) and
/// plays that if it opens and starts with `GIF8` (the decoder then finds its
/// `OPENDMD1LZW` hint and plays it at cap 9); otherwise the GIF. A playlist
/// therefore only ever names GIFs, and the engine, the status and the web
/// app all see the GIF's path. A path under `cache/` is opened as it is.
///
/// The miss costs one failed open in a small directory: the lookup walks
/// `cache/` and `cache/<Category>` (≤256 bucket entries, 8.3 names, one
/// dirent each) and then fails in the ~15-entry bucket, or earlier if the
/// bucket or category does not exist. It is measured per open ([`take_last`],
/// [`period_summary`]) and reported in the GIF-start line and the heartbeat.
pub struct SdSource;

/// How the last [`SdSource::open`] was served — read by the main loop when it
/// prints the deferred GIF-start line. Only the main task opens through
/// `SdSource`, so `Relaxed` is plenty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Served {
    /// The prepared copy, after `try_us` µs spent opening and checking it.
    Prepared { try_us: u32 },
    /// The original GIF, after `miss_us` µs spent failing to find a copy.
    Original { miss_us: u32 },
    /// A path with no prepared form (already a copy, or not under `gifs/`).
    Direct,
}

static LAST_KIND: AtomicU32 = AtomicU32::new(0);
static LAST_US: AtomicU32 = AtomicU32::new(0);
// Per heartbeat period: hits, misses, and the miss time (sum, max).
static P_HITS: AtomicU32 = AtomicU32::new(0);
static P_MISSES: AtomicU32 = AtomicU32::new(0);
static P_MISS_US: AtomicU32 = AtomicU32::new(0);
static P_MISS_MAX: AtomicU32 = AtomicU32::new(0);
// Since boot.
static B_HITS: AtomicU32 = AtomicU32::new(0);
static B_MISSES: AtomicU32 = AtomicU32::new(0);
static B_MISS_MAX: AtomicU32 = AtomicU32::new(0);

fn record(served: Served) {
    let (kind, us) = match served {
        Served::Prepared { try_us } => {
            P_HITS.fetch_add(1, Relaxed);
            B_HITS.fetch_add(1, Relaxed);
            (1, try_us)
        }
        Served::Original { miss_us } => {
            P_MISSES.fetch_add(1, Relaxed);
            B_MISSES.fetch_add(1, Relaxed);
            P_MISS_US.fetch_add(miss_us, Relaxed);
            P_MISS_MAX.fetch_max(miss_us, Relaxed);
            B_MISS_MAX.fetch_max(miss_us, Relaxed);
            (2, miss_us)
        }
        Served::Direct => (3, 0),
    };
    LAST_KIND.store(kind, Relaxed);
    LAST_US.store(us, Relaxed);
}

/// How the most recent open was served (`None` before the first).
pub fn take_last() -> Option<Served> {
    let us = LAST_US.load(Relaxed);
    match LAST_KIND.load(Relaxed) {
        1 => Some(Served::Prepared { try_us: us }),
        2 => Some(Served::Original { miss_us: us }),
        3 => Some(Served::Direct),
        _ => None,
    }
}

/// The heartbeat's cache part: this period's prepared/original opens and the
/// miss cost, plus the boot totals. Resets the period counters.
pub fn period_summary() -> String {
    let hits = P_HITS.swap(0, Relaxed);
    let misses = P_MISSES.swap(0, Relaxed);
    let miss_us = P_MISS_US.swap(0, Relaxed);
    let miss_max = P_MISS_MAX.swap(0, Relaxed);
    let avg = miss_us.checked_div(misses).unwrap_or(0);
    format!(
        ", cache: {hits} prepared / {misses} original (miss avg {:.1} ms, max {:.1} ms; boot {} / {}, max {:.1} ms)",
        avg as f32 / 1000.0,
        miss_max as f32 / 1000.0,
        B_HITS.load(Relaxed),
        B_MISSES.load(Relaxed),
        B_MISS_MAX.load(Relaxed) as f32 / 1000.0,
    )
}

/// One `File::open`, through the probe when it is built in.
fn open_file(full: &str) -> io::Result<Box<dyn ReadSeek>> {
    // No `BufReader` here: gifslim wraps whatever it is handed in a 2 KiB
    // `BufReader` of its own (the `gif` crate, the fallback, in 8 KiB), so
    // a second one buys nothing but heap.
    //
    // This open is a linear scan of the file's FAT directory (~0.65 ms
    // per 512 B sector: ~330 ms in the 3,781-entry `gifs/Consoles`), which
    // is why the engine loops a file by seeking this handle back to 0
    // instead of opening it again, and why prepared copies live in
    // ~15-entry bucket directories.
    if crate::probe::ENABLED {
        return Ok(Box::new(crate::probe::TimedFile(crate::probe::open(full)?)));
    }
    Ok(Box::new(File::open(full)?))
}

/// The prepared copy at `full`, if it opens and is a GIF. The magic is read
/// here (the decoder would read the same sector anyway, and FATFS keeps it
/// in the file's own buffer) so a truncated or foreign file falls back to
/// the original instead of costing the playlist an entry.
fn open_prepared(full: &str) -> Option<Box<dyn ReadSeek>> {
    let mut f = open_file(full).ok()?;
    let mut magic = [0u8; 4];
    let ok = f.read_exact(&mut magic).is_ok() && &magic == b"GIF8";
    if !ok {
        log::warn!("{full}: not a GIF; playing the original instead");
        return None;
    }
    f.seek(SeekFrom::Start(0)).ok()?;
    Some(f)
}

impl Source for SdSource {
    fn open(&self, path: &str) -> io::Result<Box<dyn ReadSeek>> {
        // The cache builder lets go of its heap first: this may be an
        // original, whose decoder needs more than a copy's, and the open's
        // directory scan should not queue behind its card I/O either.
        crate::cache_builder::player_opening();
        let Some(prepared) = layout::cache_path(path) else {
            let f = open_file(&format!("{}{path}", config::SD_MOUNT))?;
            record(Served::Direct);
            return Ok(f);
        };
        let t = Instant::now();
        if let Some(f) = open_prepared(&format!("{}{prepared}", config::SD_MOUNT)) {
            record(Served::Prepared {
                try_us: t.elapsed().as_micros() as u32,
            });
            return Ok(f);
        }
        let miss_us = t.elapsed().as_micros() as u32;
        let f = open_file(&format!("{}{path}", config::SD_MOUNT))?;
        record(Served::Original { miss_us });
        Ok(f)
    }
}
