//! microSD over SPI, mounted as FAT at `/sdcard`.
//!
//! The DMDos carrier wires the card to a plain SPI socket, not the SDMMC
//! peripheral: `SCK=18 MISO=19 MOSI=23 CS=5` (measured, see
//! `hardware/PINOUT.md`). Card detect and write protect are not wired.
//!
//! Those are VSPI's IO_MUX pins, and VSPI is `SPI3` (`SPI3_HOST`); `SPI2` is
//! HSPI. Driven from `SPI2` they went through the GPIO matrix, whose input
//! delay caps a full-duplex read at 26.7 MHz (`spi_hal: ... frequency >
//! 26.7MHz, device cannot read correct data`). On `SPI3` they are IO_MUX
//! and the driver allows 40 MHz. See [`config::SD_SPEED_KHZ`].
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
use esp_idf_hal::spi::{Dma, SpiDriver, SPI3};
use esp_idf_svc::fs::fatfs::Fatfs;
use esp_idf_svc::io::vfs::MountedFatfs;
use opendmd_core::engine::{ReadSeek, Source};
use opendmd_core::layout;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU32, Ordering::{Relaxed, SeqCst}};
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
/// timeout for a while — seen on a test panel: `ESP_ERR_TIMEOUT` twice, then a
/// mount 10 s later — and a retry must not inherit a bus in whatever state
/// the failed attempt left it. Attempts after the first also give each
/// command longer ([`config::SD_RETRY_CMD_TIMEOUT_MS`]).
pub struct SdBus {
    spi: SPI3,
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
    /// The SPI clock the bus really runs at: 80 MHz over a whole divider
    /// (40, 26.7, 20, 16 … MHz), the nearest to the clock asked for
    /// ([`config::SD_SPEED_KHZ`], or [`config::SD_SPEED_FALLBACK_KHZ`]).
    pub real_freq_khz: i32,
    /// The card's CID serial number: which card this is, for the
    /// slow-clock record ([`crate::sdhealth`]).
    pub serial: u32,
}

impl Drop for SdCard {
    fn drop(&mut self) {
        // Before the fields go: no clock change or raw read from here on.
        crate::cardspeed::detach();
        crate::fatidx::unmounted();
    }
}

// SAFETY: the card is mounted on the boot task and handed to the main task
// once (`boot::Handover`), which only keeps it (dropping it unmounts). What
// it holds is C state for the driver and the FATFS volume — raw pointers
// with no tie to the task that made them: FATFS is reentrant (its own lock)
// and every other task reaches the volume through the VFS, never through
// this value.
unsafe impl Send for SdCard {}

impl SdBus {
    pub fn new(
        spi: SPI3,
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
    /// to erase. (Erasing is [`SdBus::format`], only ever asked for.)
    ///
    /// `speed_khz` is the clock to ask for after the card's 400 kHz init.
    /// Only a clock the driver refuses fails *at* the clock switch, and that
    /// failure's teardown crashes (CLAUDE.md, "When the card will not
    /// mount"); 40 MHz on `SPI3` and the 20 MHz fallback are both allowed.
    ///
    /// The mounted card's disk I/O goes through `cardspeed` (registered
    /// before the first access), which can change the clock in place;
    /// `capped` (the card-failure restart budget is spent) keeps it at the
    /// safe clock whatever the setting.
    pub fn mount(&mut self, speed_khz: u32, capped: bool) -> Result<SdCard> {
        self.attempts += 1;
        if take_simulated_failure() {
            anyhow::bail!("simulated failure (POST /api/reboot?sd_fail=N)");
        }
        let card = self.init(speed_khz)?;

        let raw = card.card();
        let capacity_bytes = (raw.csd.capacity as u64) * (raw.csd.sector_size as u64);
        let real_freq_khz = raw.real_freq_khz;
        // SAFETY: `cid` is the union's decoded view, which the SD init
        // fills (`raw_cid` is MMC's).
        let serial = unsafe { raw.__bindgen_anon_1.cid.serial } as u32;
        let sectors = raw.csd.capacity as u32;
        // The driver keeps the card boxed: this address lives as long as
        // the mount (`cardspeed::detach` in our drop, before it goes).
        let raw_ptr = raw as *const esp_idf_sys::sdmmc_card_t as *mut esp_idf_sys::sdmmc_card_t;

        let fatfs = Fatfs::new_sdcard(0, card).context("FAT driver registration failed")?;
        // Ours in place of the IDF's disk I/O before the VFS can reach the
        // drive (FatFs mounts lazily: nothing has been read yet).
        crate::cardspeed::attach(0, raw_ptr, real_freq_khz as u32, serial, sectors, capped);
        let mounted = match MountedFatfs::mount(fatfs, config::SD_MOUNT, config::SD_MAX_FILES) {
            Ok(m) => m,
            Err(e) => {
                // `fatfs` (and the drive's registration) went with it.
                crate::cardspeed::detach();
                return Err(e).context("FAT mount failed");
            }
        };

        Ok(SdCard {
            _fs: mounted,
            capacity_bytes,
            real_freq_khz,
            serial,
        })
    }

    /// The bus, the SD SPI host and the card's init sequence, built afresh
    /// (see the struct's doc); dropping the result tears all three down.
    fn init(&mut self, speed_khz: u32) -> Result<Card> {
        // SAFETY: the peripheral and the pins are this struct's alone, and
        // the drivers built on them here are either kept (inside the
        // returned `SdCard`, which is dropped — unmounted — before `mount`
        // is called again: the boot task's remount of a card that cannot be
        // read, and otherwise never; or by `format`, until it returns) or
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
        cfg.speed_khz = speed_khz;
        if self.attempts > 1 {
            cfg.command_timeout_ms = config::SD_RETRY_CMD_TIMEOUT_MS;
        }

        SdCardDriver::new_spi(host, &cfg).context("no card, or card init failed")
    }

    /// Format the card in the socket FAT32 (FAT16 if it is too small for
    /// FAT32's cluster count): an MBR with one partition, two FATs, 32 KB
    /// clusters, the data area aligned to 4 MiB (`opendmd_core::card`).
    /// Everything on the card is lost. The caller has unmounted it (the
    /// unreadable `SdCard` dropped), so `/sdcard` is not in the VFS while
    /// this runs and no other task can reach the card; afterwards it is
    /// mounted as any card is ([`SdBus::mount`]).
    ///
    /// One blocking `f_mkfs`: on a 64 GB card ~15 MB of FAT written in
    /// [`config::SD_FORMAT_BUF`] pieces, ~63 s. The SD SPI driver polls (no
    /// interrupt, no yield), so the calling task drops to the idle
    /// priority for the duration: CPU0's idle task then shares the core
    /// with it and keeps feeding the task watchdog, and everything else on
    /// CPU0 (WiFi, the httpd answering status polls) comes first. CPU1 is
    /// not involved: the bus is `SPI3`, its interrupt is CPU0's, and the
    /// display never touches an unreadable card.
    pub fn format(&mut self, speed_khz: u32) -> Result<Formatted> {
        let card = self.init(speed_khz)?;
        let capacity_bytes = {
            let raw = card.card();
            (raw.csd.capacity as u64) * (raw.csd.sector_size as u64)
        };
        // `u8` spelled out: the pointer goes out as `c_void`, and an
        // inferred element type was `i32` — four times the heap.
        let mut buf: Vec<u8> = Vec::new();
        buf.try_reserve_exact(config::SD_FORMAT_BUF)
            .map_err(|_| anyhow::anyhow!("no memory for the format's work buffer"))?;
        buf.resize(config::SD_FORMAT_BUF, 0);
        let fatfs = Fatfs::new_sdcard(0, card).context("FAT driver registration failed")?;
        let opt = esp_idf_sys::MKFS_PARM {
            fmt: esp_idf_sys::FM_ANY as u8,
            n_fat: 2,
            align: opendmd_core::card::ALIGN_SECTORS,
            n_root: 0,
            au_size: opendmd_core::card::ALLOCATION_UNIT,
        };
        // SAFETY: lowering, then restoring, the calling task's own priority.
        let prio = unsafe { esp_idf_sys::uxTaskPriorityGet(std::ptr::null_mut()) };
        unsafe { esp_idf_sys::vTaskPrioritySet(std::ptr::null_mut(), 0) };
        let t = Instant::now();
        // SAFETY: drive 0 is registered just above (`fatfs`, unregistered
        // when it drops) and mounted nowhere; `buf` outlives the call.
        let res = unsafe {
            esp_idf_sys::f_mkfs(
                c"0:".as_ptr(),
                &opt,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
            )
        };
        let ms = t.elapsed().as_millis() as u32;
        unsafe { esp_idf_sys::vTaskPrioritySet(std::ptr::null_mut(), prio) };
        drop(fatfs);
        if res != esp_idf_sys::FRESULT_FR_OK {
            anyhow::bail!("{} (FatFs {res}) after {ms} ms", mkfs_error(res));
        }
        Ok(Formatted { capacity_bytes, ms })
    }
}

/// What [`SdBus::format`] did.
pub struct Formatted {
    pub capacity_bytes: u64,
    /// The `f_mkfs` call's duration.
    pub ms: u32,
}

/// A failed `f_mkfs` in words.
fn mkfs_error(res: esp_idf_sys::FRESULT) -> &'static str {
    match res {
        esp_idf_sys::FRESULT_FR_DISK_ERR => "a card write failed",
        esp_idf_sys::FRESULT_FR_NOT_READY => "the card did not answer",
        esp_idf_sys::FRESULT_FR_WRITE_PROTECTED => "the card is write-protected",
        esp_idf_sys::FRESULT_FR_MKFS_ABORTED => "the card's size does not suit FAT",
        esp_idf_sys::FRESULT_FR_NOT_ENOUGH_CORE => "not enough memory",
        _ => "the format failed",
    }
}

/// The filesystem of the card just mounted, probed once: a `read_dir` of
/// the root, which is when FATFS first reads the volume (`MountedFatfs`
/// mounts lazily, so a card whose SD init answered mounts even when its
/// filesystem is one FATFS cannot read). `Err` only for a card-class error
/// (`sdhealth::is_card_error`: `ENODEV` for exFAT or no filesystem, `EIO`):
/// that card is never handed to the player.
pub fn probe() -> io::Result<()> {
    match std::fs::read_dir(config::SD_MOUNT) {
        Ok(_) => Ok(()),
        Err(e) if crate::sdhealth::is_card_error(&e) => Err(e),
        Err(e) => {
            log::warn!(
                "microSD: listing {}: {e} (not a card error; carrying on)",
                config::SD_MOUNT
            );
            Ok(())
        }
    }
}

/// Test hook: mount attempts to fail on purpose after the next restart, so
/// the no-card boot (network up, mount retried) can be exercised without
/// pulling the card. Kept in RTC slow memory, which a software restart
/// leaves alone and a power-on (or another firmware's layout, after an
/// update) fills with garbage — hence the magic word.
///
/// **Every access is volatile.** The static's initializer is all the
/// compiler knows of it, so with plain accesses LLVM may prove the word
/// only ever holds 0 or the magic and shrink it to a one-byte flag
/// (GlobalOpt): then any non-zero garbage byte "matches". It did, with
/// `codegen-units = 1`: after an update from a build whose log ring sat
/// there, a boot failed ~180 mounts (9 min without the card) on a test board.
#[link_section = ".rtc_noinit"]
static mut SIM_FAIL: [u32; 2] = [0; 2];
const SIM_MAGIC: u32 = 0x5d_fa_11_ed;

fn sim_word(i: usize) -> *mut u32 {
    // SAFETY: an element of the static; only reached through volatile ops.
    unsafe { (&raw mut SIM_FAIL).cast::<u32>().add(i) }
}

/// Arm [`SIM_FAIL`]: the next boot's first `n` mount attempts fail.
/// (`bench` builds only: a release has no way to arm it.)
#[cfg(feature = "bench")]
pub fn simulate_mount_failures(n: u32) {
    // SAFETY: written once, from the httpd task, just before a restart;
    // read only by the boot task (`boot.rs`), which makes every mount.
    unsafe {
        sim_word(0).write_volatile(SIM_MAGIC);
        sim_word(1).write_volatile(n);
    }
}

fn take_simulated_failure() -> bool {
    // A chain of simulated card failures (`sd_io_fail`) goes first: these
    // then fail the remounts after it.
    if crate::sdhealth::sim_pending() {
        return false;
    }
    // SAFETY: the boot task is the only one mounting, see above.
    unsafe {
        let left = sim_word(1).read_volatile();
        if sim_word(0).read_volatile() != SIM_MAGIC || left == 0 || left > 20 {
            sim_word(0).write_volatile(0);
            return false;
        }
        sim_word(1).write_volatile(left - 1);
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
    /// The prepared copy, opened ahead while the previous file played
    /// ([`SdSource::open_ahead`]) after `try_us` µs then.
    Ahead { try_us: u32 },
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
        Served::Ahead { try_us } => {
            P_HITS.fetch_add(1, Relaxed);
            B_HITS.fetch_add(1, Relaxed);
            (4, try_us)
        }
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
        4 => Some(Served::Ahead { try_us: us }),
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

/// One `File::open`, through the probe when it is built in; its reads and
/// seeks are timed for the display loop's miss attribution
/// ([`crate::frametime::Counted`]).
fn open_file(full: &str) -> io::Result<Box<dyn ReadSeek>> {
    open_file_keyed(full, 0)
}

/// [`open_file`], the handle carrying `ahead_key` (`frametime::Counted`).
fn open_file_keyed(full: &str, ahead_key: u32) -> io::Result<Box<dyn ReadSeek>> {
    // No `BufReader` here: gifslim wraps whatever it is handed in a 2 KiB
    // `BufReader` of its own (the `gif` crate, the fallback, in 8 KiB), so
    // a second one buys nothing but heap.
    //
    // This open is a linear scan of the file's FAT directory (~0.65 ms
    // per 512 B sector: ~330 ms in the 3,781-entry `gifs/Consoles`), which
    // is why the engine loops a file by seeking this handle back to 0
    // instead of opening it again, and why prepared copies live in
    // ~15-entry bucket directories.
    use crate::frametime::Counted;
    let file = if crate::probe::ENABLED {
        crate::probe::open(full)
    } else {
        // Read raw: never behind FatFs's volume lock (`fatidx` "raw reads").
        crate::fatidx::open_player(full)
    };
    let file = match file {
        Ok(f) => f,
        Err(e) => {
            crate::sdhealth::open_failed(crate::sdhealth::Kind::Open, full, &e);
            return Err(e);
        }
    };
    crate::sdhealth::opened(full);
    if crate::probe::ENABLED {
        return Ok(Box::new(Counted(crate::probe::TimedFile(file), false, ahead_key)));
    }
    Ok(Box::new(Counted(file, false, ahead_key)))
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
    /// All of it is the display loop's `open` time ([`crate::frametime::open`]).
    fn open(&self, path: &str) -> io::Result<Box<dyn ReadSeek>> {
        // A switch: the read-ahead's ring goes before the new decoder and
        // the transition ask the heap (`readahead::release`).
        crate::readahead::release();
        crate::frametime::open(|| open_resolved(path))
    }

    /// The copy alone (a path already under `cache/` as it is), never the
    /// original; `Ok(None)` when there is no copy, it is not a GIF, or a
    /// transfer has just started (`cache_builder::busy()`). A
    /// card-class error is returned (and counted by `sdhealth`, in
    /// [`open_file`], exactly as the switch's open of the copy would be);
    /// any other failure is a miss. No builder release wait here: the main
    /// loop only steps ahead while the builder's task is down. Open time.
    fn open_ahead(&self, path: &str) -> io::Result<Option<Box<dyn ReadSeek>>> {
        crate::frametime::open(|| {
            let copy = match layout::classify(path) {
                layout::Prepared::Direct => path.to_string(),
                layout::Prepared::Candidate { .. } => match layout::cache_path(path) {
                    Some(c) => c,
                    None => return Ok(None),
                },
                layout::Prepared::Never => return Ok(None),
            };
            let t = Instant::now();
            let key = layout::prepared_key(&copy);
            let full = format!("{}{copy}", config::SD_MOUNT);
            // The key goes up before the open, and a transfer that started
            // since the main loop looked (its handler may already have
            // checked the key) means no open at all: see `state::AHEAD_KEY`.
            // Once the handle exists, its drop clears the key.
            use crate::state::AHEAD_KEY;
            let unkey = || {
                let _ = AHEAD_KEY.compare_exchange(key, 0, SeqCst, SeqCst);
            };
            AHEAD_KEY.store(key, SeqCst);
            if crate::cache_builder::transfer_in_flight() {
                unkey();
                return Ok(None);
            }
            AHEAD_ORIGINAL.store(false, Relaxed);
            let mut f = match open_file_keyed(&full, key) {
                Ok(f) => f,
                Err(e) if crate::sdhealth::is_card_error(&e) => {
                    unkey();
                    return Err(e);
                }
                // No copy: the original, if the FAT index opens it (never
                // a directory scan here). `ahead` holds its decoder only
                // with the heap for cap 12.
                Err(_) => match (copy != path)
                    .then(|| format!("{}{path}", config::SD_MOUNT))
                    .and_then(|orig| crate::fatidx::indexed_player(&orig).map(|r| (orig, r)))
                {
                    Some((orig, Ok(o))) => {
                        crate::sdhealth::opened(&orig);
                        AHEAD_ORIGINAL.store(true, Relaxed);
                        AHEAD_US.store(t.elapsed().as_micros() as u32, Relaxed);
                        return Ok(Some(Box::new(crate::frametime::Counted(o, false, key)) as Box<dyn ReadSeek>));
                    }
                    Some((_, Err(e))) if crate::sdhealth::is_card_error(&e) => {
                        unkey();
                        return Err(e);
                    }
                    _ => {
                        unkey();
                        return Ok(None);
                    }
                },
            };
            let mut magic = [0u8; 4];
            match f.read_exact(&mut magic) {
                Ok(()) if &magic == b"GIF8" => {}
                Err(e) if crate::sdhealth::is_card_error(&e) => return Err(e),
                _ => {
                    log::warn!("{full}: not a GIF; the original plays instead");
                    return Ok(None);
                }
            }
            f.seek(SeekFrom::Start(0))?;
            AHEAD_US.store(t.elapsed().as_micros() as u32, Relaxed);
            Ok(Some(f))
        })
    }

    /// The copy opened ahead takes the screen: what the open at the switch
    /// would have done besides opening — the builder lets go of its heap
    /// (if it started meanwhile), and the GIF-start line says how it was
    /// served.
    fn adopt(&self, _path: &str) {
        // A switch to the copy opened ahead: as in `open`.
        crate::readahead::release();
        crate::frametime::open(|| {
            if crate::cache_builder::task_up() {
                crate::cache_builder::player_opening();
            }
            record(Served::Ahead {
                try_us: AHEAD_US.load(Relaxed),
            });
        });
    }
}

/// The open time of the copy opened ahead last (µs), for its GIF-start line.
static AHEAD_US: AtomicU32 = AtomicU32::new(0);
/// What was opened ahead last is an original (through the FAT index).
static AHEAD_ORIGINAL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn ahead_is_original() -> bool {
    AHEAD_ORIGINAL.load(Relaxed)
}

fn open_resolved(path: &str) -> io::Result<Box<dyn ReadSeek>> {
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
