//! The card's SPI clock while it is mounted (`card_speed`,
//! `opendmd_core::card_speed`): slowing a card that cannot keep up with
//! 40 MHz down to 20 MHz **in place**, with no restart, and the probe that
//! finds such a card soon after it mounts.
//!
//! **How: a disk I/O layer of our own under FatFs.** `SdBus::mount`
//! registers the card with FatFs (`ff_diskio_register_sdmmc`, through
//! esp-idf-svc), then [`attach`] registers this module's functions for the
//! same drive in its place. Each one calls the IDF's own (`ff_sdmmc_read`
//! and friends, which still find the card they were given) under [`IO`], a
//! mutex of ours. Every sector the card moves for anyone (the player, the
//! stream task, the cache builder, uploads, listings, the playlist index)
//! passes through here, one transfer at a time: FatFs already holds its
//! volume lock around each of these calls, and [`IO`] also covers the two
//! things that do not come through FatFs — the probe's raw reads and a
//! clock change.
//!
//! **The clock change** is `sdmmc_host_t::set_card_clk`, i.e.
//! `sdspi_host_set_card_clk`: it removes the card's SPI device from the bus
//! and adds it again at the new clock (`spi_bus_remove_device`,
//! `spi_bus_add_device`; no interrupt is allocated, the bus and its
//! interrupt stay CPU0's). Under [`IO`] no transfer is in flight, which is
//! all the remove needs. The card itself does not care: in SPI mode the
//! host's clock is the only clock, and both 40 and 20 MHz are 80 MHz over a
//! whole divider, so the real clock is the one asked for.
//!
//! **At the first card error at full speed in `fast`**
//! ([`card_speed::drop_on_error`]) the transfer that failed is followed,
//! still under [`IO`], by: the card taken out of a failed read
//! ([`stop_locked`]: CMD12; not a failed multi-block write, see there),
//! the slow-clock record (`sdhealth::slowed`: RTC memory at once,
//! NVS from the network worker), the clock to 20 MHz, and the same transfer
//! once more. If that works nothing above sees an error: the GIF plays on,
//! the download or upload goes on. If it fails too, the error goes up as
//! before and is counted (`sdhealth`): the trip and its restart budget
//! still guard against a card that fails at 20 MHz as well.
//!
//! **When the clock cannot be changed** there are two cases. *Refused*
//! (under 2 KB of largest free block, [`config::SD_CLOCK_HEAP`]): nothing
//! was touched, the card keeps its clock, the error goes up, and the old
//! path — the trip at 40 MHz, the record, a restart at 20 MHz — is the
//! fallback. *Lost* (`set_card_clk` removed the SPI device and failed to
//! add it back, twice): the card has no device, and the driver would panic
//! on the next transfer (`spi_device_acquire_bus` on a null handle), so
//! [`DEAD`] stops every transfer here from reaching it and the main task
//! restarts at once ([`dead`]; in `fast` the record is already in
//! RTC memory, so that boot mounts at 20 MHz). A clock change asked for
//! over HTTP that fails either way answers `restart` and restarts.
//!
//! **The probe** ([`probe_soon`]): a card mounted at full speed in
//! `fast` with no record is read for [`config::SD_PROBE_BYTES`]
//! (4 MB) from its FAT, in 2 KB reads with a pause after each, on a small
//! CPU0 task that starts [`config::SD_PROBE_DELAY`] after the handover. The
//! reads go through the same path, so an error there drops the clock
//! exactly as the player's would, before the cache builder or a transfer
//! leans on the card.

use core::ffi::c_void;
use core::sync::atomic::{
    AtomicBool, AtomicPtr, AtomicU32, AtomicU8,
    Ordering::{AcqRel, Relaxed, Release},
};
use std::sync::{Mutex, MutexGuard};

use esp_idf_sys::{
    ff_diskio_impl_t, sdmmc_card_t, DRESULT, DRESULT_RES_ERROR, DRESULT_RES_OK, DSTATUS,
};
use opendmd_core::card_speed::{self, CardSpeed, FULL_KHZ, SAFE_KHZ};

use crate::{config, sdhealth};

extern "C" {
    // The IDF's own disk I/O for an SD card (`diskio_sdmmc.c`): not in the
    // bindings, but exported.
    fn ff_sdmmc_initialize(pdrv: u8) -> DSTATUS;
    fn ff_sdmmc_status(pdrv: u8) -> DSTATUS;
    fn ff_sdmmc_read(pdrv: u8, buff: *mut u8, sector: u32, count: u32) -> DRESULT;
    fn ff_sdmmc_write(pdrv: u8, buff: *const u8, sector: u32, count: u32) -> DRESULT;
    fn ff_sdmmc_ioctl(pdrv: u8, cmd: u8, buff: *mut c_void) -> DRESULT;
}

/// One card transfer (or clock change) at a time; see the module docs.
static IO: Mutex<()> = Mutex::new(());
/// The mounted card (null when none), for its host's `set_card_clk` and the
/// probe's raw reads. Only dereferenced under [`IO`].
static CARD: AtomicPtr<sdmmc_card_t> = AtomicPtr::new(core::ptr::null_mut());
/// The clock the card runs at now (0: none mounted).
static KHZ: AtomicU32 = AtomicU32::new(0);
static SERIAL: AtomicU32 = AtomicU32::new(0);
static SECTORS: AtomicU32 = AtomicU32::new(0);
/// Bumped by every [`attach`]: the probe stops if the card changes under it.
static GEN: AtomicU32 = AtomicU32::new(0);
/// A clock change removed the card's SPI device and could not add it back
/// (`set_card_clk` failed twice): no transfer may reach the driver again
/// (`spi_device_acquire_bus` on the missing device would panic), and the
/// main task restarts ([`dead`]).
static DEAD: AtomicBool = AtomicBool::new(false);
/// The card-failure restart budget was spent when it mounted: the safe
/// clock whatever the setting (`card_speed::clock_for`).
static CAPPED: AtomicBool = AtomicBool::new(false);
/// `CardSpeed::as_u8`.
static MODE: AtomicU8 = AtomicU8::new(0);
/// In-place slow-downs this boot.
static SLOWDOWNS: AtomicU32 = AtomicU32::new(0);
/// Recoveries after a failed transfer this boot ([`stop_locked`]).
static STOPS: AtomicU32 = AtomicU32::new(0);
/// The test hook (`bench` builds): transfers at full speed still to fail
/// on purpose.
#[cfg(feature = "bench")]
static SIM_ERRORS: AtomicU32 = AtomicU32::new(0);

/// The probe, for status (an index into [`PROBE_NAMES`]).
static PROBE: AtomicU8 = AtomicU8::new(NONE);
/// A probe task exists (set by compare-and-swap, cleared by that task).
static PROBE_TASK: AtomicBool = AtomicBool::new(false);
const NONE: u8 = 0;
const WAITING: u8 = 1;
const RUNNING: u8 = 2;
const CLEAN: u8 = 3;
const SLOWED: u8 = 4;
const SKIPPED: u8 = 5;
const PROBE_NAMES: [&str; 6] = ["null", "\"waiting\"", "\"running\"", "\"clean\"", "\"slowed\"", "\"skipped\""];

fn io() -> MutexGuard<'static, ()> {
    IO.lock().unwrap_or_else(|p| p.into_inner())
}

fn bump(a: &AtomicU32) {
    a.store(a.load(Relaxed).wrapping_add(1), Relaxed);
}

/// The setting in effect.
pub fn mode() -> CardSpeed {
    CardSpeed::from_u8(MODE.load(Relaxed))
}

/// The setting, at boot (before any mount).
pub fn init(mode: CardSpeed) {
    MODE.store(mode.as_u8(), Relaxed);
}

/// The card lost its SPI device in a failed clock change: the main task
/// restarts (in `fast` the slow-clock record is already in RTC
/// memory, so the next boot mounts at the safe clock).
pub fn dead() -> bool {
    DEAD.load(Relaxed)
}

static IMPL: ff_diskio_impl_t = ff_diskio_impl_t {
    init: Some(d_init),
    status: Some(d_status),
    read: Some(d_read),
    write: Some(d_write),
    ioctl: Some(d_ioctl),
};

/// The card just registered with FatFs as `drive` (`SdBus::mount`, before
/// the VFS mount): this module's disk I/O in place of the IDF's. `card` is
/// the driver's boxed `sdmmc_card_t`, alive until [`detach`].
pub fn attach(drive: u8, card: *mut sdmmc_card_t, khz: u32, serial: u32, sectors: u32, capped: bool) {
    let _io = io();
    CARD.store(card, Relaxed);
    KHZ.store(khz, Relaxed);
    SERIAL.store(serial, Relaxed);
    SECTORS.store(sectors, Relaxed);
    CAPPED.store(capped, Relaxed);
    bump(&GEN);
    PROBE.store(NONE, Relaxed);
    // SAFETY: `IMPL` is static; FatFs copies it. The drive was registered
    // just now by `ff_diskio_register_sdmmc`, whose functions ours call.
    unsafe { esp_idf_sys::ff_diskio_register(drive, &IMPL) };
}

/// The card is being unmounted (`SdCard`'s drop, or a failed VFS mount):
/// no more clock changes or raw reads. FatFs's own calls during the
/// unmount still reach the IDF's functions through ours.
pub fn detach() {
    let _io = io();
    CARD.store(core::ptr::null_mut(), Relaxed);
    KHZ.store(0, Relaxed);
}

unsafe extern "C" fn d_init(pdrv: u8) -> DSTATUS {
    let _io = io();
    if dead() {
        return esp_idf_sys::STA_NOINIT as DSTATUS;
    }
    ff_sdmmc_initialize(pdrv)
}

unsafe extern "C" fn d_status(pdrv: u8) -> DSTATUS {
    let _io = io();
    if dead() {
        return esp_idf_sys::STA_NOINIT as DSTATUS;
    }
    ff_sdmmc_status(pdrv)
}

/// Reads waiting for (or doing) a transfer: a long write steps aside for
/// them between its chunks ([`d_write`]).
static READERS: AtomicU32 = AtomicU32::new(0);
/// The most sectors one write transfer carries: a save's multi-sector write
/// (a whole cluster, 64 sectors: ~60 ms with the card's busy time) held the
/// I/O lock that long, and the player's raw read waited it out.
const WRITE_CHUNK: u32 = 8;
/// How long a write waits between chunks for the reads that want the card
/// (each a few sectors): at most this much a chunk, so a stream of reads can
/// slow a save but never stop it.
const YIELD_MAX_US: i64 = 5_000;
/// Writes that stepped aside, and the µs they waited (`/api/status`).
static YIELDS: AtomicU32 = AtomicU32::new(0);
static YIELD_US: AtomicU32 = AtomicU32::new(0);

unsafe extern "C" fn d_read(pdrv: u8, buff: *mut u8, sector: u32, count: u32) -> DRESULT {
    READERS.fetch_add(1, Relaxed);
    let r = transfer(false, &mut || ff_sdmmc_read(pdrv, buff, sector, count) == DRESULT_RES_OK);
    READERS.fetch_sub(1, Relaxed);
    r
}

unsafe extern "C" fn d_write(pdrv: u8, buff: *const u8, sector: u32, count: u32) -> DRESULT {
    let mut done = 0;
    while done < count {
        if done > 0 {
            step_aside();
        }
        let n = (count - done).min(WRITE_CHUNK);
        // SAFETY: within the caller's `count` sectors.
        let (b, at) = (buff.add(done as usize * 512), sector + done);
        let r = transfer(true, &mut || ff_sdmmc_write(pdrv, b, at, n) == DRESULT_RES_OK);
        if r != DRESULT_RES_OK {
            return r;
        }
        done += n;
    }
    DRESULT_RES_OK
}

/// Between a write's chunks, the I/O lock free: let the reads waiting for
/// it go first, for at most [`YIELD_MAX_US`] (a FreeRTOS mutex is not
/// fair: the writer, at a higher priority, would take it straight back).
fn step_aside() {
    if READERS.load(Relaxed) == 0 {
        return;
    }
    // SAFETY: a plain clock read.
    let t0 = unsafe { esp_idf_sys::esp_timer_get_time() };
    YIELDS.fetch_add(1, Relaxed);
    loop {
        std::thread::yield_now();
        // SAFETY: as above.
        let waited = unsafe { esp_idf_sys::esp_timer_get_time() } - t0;
        if READERS.load(Relaxed) == 0 || waited >= YIELD_MAX_US {
            YIELD_US.fetch_add(waited as u32, Relaxed);
            return;
        }
        if waited > 200 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

/// `"write_yields":N,"write_yield_us":N` for the status's card part.
pub fn yields_json() -> String {
    format!(r#","write_yields":{},"write_yield_us":{}"#, YIELDS.load(Relaxed), YIELD_US.load(Relaxed))
}

unsafe extern "C" fn d_ioctl(pdrv: u8, cmd: u8, buff: *mut c_void) -> DRESULT {
    let _io = io();
    if dead() {
        return DRESULT_RES_ERROR;
    }
    ff_sdmmc_ioctl(pdrv, cmd, buff)
}

/// One transfer under [`IO`], and at an error at full speed in `fast`
/// mode the in-place slow-down and one more try (see the module docs).
/// One copy for every caller (`dyn`).
fn transfer(write: bool, op: &mut dyn FnMut() -> bool) -> DRESULT {
    let _io = io();
    if dead() {
        return DRESULT_RES_ERROR;
    }
    let khz = KHZ.load(Relaxed);
    if !take_sim(khz) && op() {
        return DRESULT_RES_OK;
    }
    // Whatever happens next, the card first leaves the transfer that
    // failed (see `stop_locked`).
    stop_locked();
    if !card_speed::drop_on_error(mode(), khz) {
        return DRESULT_RES_ERROR;
    }
    let ok = match slow_down_locked() {
        Err(e) => Err(e.why()),
        Ok(()) => Ok(op()),
    };
    if ok != Ok(true) {
        stop_locked();
    }
    log::warn!(
        "microSD: {} error at 40 MHz: {}",
        if write { "write" } else { "read" },
        match ok {
            Ok(true) => "20 MHz, retry ok",
            Ok(false) => "20 MHz, retry failed",
            Err(why) => why,
        }
    );
    if ok == Ok(true) {
        DRESULT_RES_OK
    } else {
        DRESULT_RES_ERROR
    }
}

/// After a failed transfer, under [`IO`]: `STOP_TRANSMISSION` (CMD12),
/// then a status read (CMD13), results ignored. The driver returns from a
/// multi-block read at the first bad block's CRC without stopping the
/// card, which keeps sending data and answers nothing else: on the spare's
/// 64 GB card the retry at the safe clock and the transfers after it
/// failed that way (three errors in a row, a card-failure restart) until
/// this was added. A card that is not mid-transfer refuses CMD12.
///
/// **This does not recover a failed multi-block write.** A card in one
/// ignores command frames and waits for the Stop Tran token (0xFD), which
/// the driver sends only when the write succeeds. So a write error still
/// drops the clock and the write is tried again, but a card left
/// mid-write fails that retry and what follows until the card-failure
/// trip restarts the device (the old path). Sending the token ourselves
/// (a raw frame through `sdspi_host_start_command`) was written and not
/// kept: 60 uploads at 40 MHz on the spare's card never failed a write,
/// so it could not be shown to work.
fn stop_locked() {
    let card = CARD.load(Relaxed);
    if card.is_null() {
        return;
    }
    // SAFETY: the mounted card (non-null under `IO`); no transfer in flight.
    unsafe {
        let host = &(*card).host;
        if let Some(run) = host.do_transaction {
            let mut cmd = esp_idf_sys::sdmmc_command_t {
                opcode: esp_idf_sys::MMC_STOP_TRANSMISSION,
                flags: (esp_idf_sys::SCF_CMD_AC | esp_idf_sys::SCF_RSP_R1B) as i32,
                timeout_ms: 1000,
                ..Default::default()
            };
            let _ = run(host.slot, &mut cmd);
        }
        let _ = esp_idf_sys::sdmmc_get_status(card);
    }
    bump(&STOPS);
}

/// The test hook (`bench` builds only): a transfer at full speed that
/// fails on purpose.
#[cfg(feature = "bench")]
fn take_sim(khz: u32) -> bool {
    let n = SIM_ERRORS.load(Relaxed);
    if n == 0 || khz <= SAFE_KHZ {
        return false;
    }
    SIM_ERRORS.store(n - 1, Relaxed);
    true
}

#[cfg(not(feature = "bench"))]
fn take_sim(_khz: u32) -> bool {
    false
}

/// Why the clock was not changed.
enum ClockErr {
    /// Nothing was touched: the card runs at the clock it had.
    Refused(&'static str),
    /// The SPI device was removed and could not be added back: the card is
    /// unusable until a restart ([`DEAD`]).
    Dead,
}

impl ClockErr {
    fn why(&self) -> &'static str {
        match self {
            ClockErr::Refused(r) => r,
            ClockErr::Dead => "SPI device lost; restarting",
        }
    }
}

/// The automatic slow-down, under [`IO`]: the record first (so a crash in
/// the driver, or a lost device, still boots slow), then the clock.
fn slow_down_locked() -> Result<(), ClockErr> {
    sdhealth::slowed(SERIAL.load(Relaxed));
    set_clock_locked(SAFE_KHZ)?;
    bump(&SLOWDOWNS);
    if PROBE.load(Relaxed) == RUNNING {
        PROBE.store(SLOWED, Relaxed);
    }
    sdhealth::mounted(SAFE_KHZ, true);
    Ok(())
}

/// The card's clock, changed in place; the caller holds [`IO`].
fn set_clock_locked(khz: u32) -> Result<(), ClockErr> {
    let card = CARD.load(Relaxed);
    if card.is_null() {
        return Err(ClockErr::Refused("no card"));
    }
    if KHZ.load(Relaxed) == khz {
        return Ok(());
    }
    // SAFETY: an argument-free read of the heap's accounting.
    let block = unsafe {
        esp_idf_sys::heap_caps_get_largest_free_block(esp_idf_sys::MALLOC_CAP_INTERNAL)
    };
    if block < config::SD_CLOCK_HEAP {
        return Err(ClockErr::Refused("low memory"));
    }
    // SAFETY: `card` is the mounted card's (non-null under `IO`, see
    // `detach`), and no transfer is in flight while `IO` is held.
    unsafe {
        let host = &(*card).host;
        let set = host.set_card_clk.ok_or(ClockErr::Refused("no clock control"))?;
        // It removes the device, then adds it at the new clock: a failure
        // leaves no device. Once more, then the card is given up.
        if set(host.slot, khz) != esp_idf_sys::ESP_OK && set(host.slot, khz) != esp_idf_sys::ESP_OK {
            DEAD.store(true, Relaxed);
            CARD.store(core::ptr::null_mut(), Relaxed);
            KHZ.store(0, Relaxed);
            return Err(ClockErr::Dead);
        }
        (*card).real_freq_khz = khz as i32;
    }
    KHZ.store(khz, Relaxed);
    Ok(())
}

/// The clock now (0: no card), for the trip and status.
pub fn khz() -> u32 {
    KHZ.load(Relaxed)
}

/// What a request to change the clock did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Applied {
    /// In effect now.
    Now,
    /// No card mounted: the next mount uses it.
    AtMount,
    /// The clock could not be changed in place: the caller restarts, which
    /// applies it (the setting and the record are saved first).
    Restart,
}

impl Applied {
    pub fn name(self) -> &'static str {
        match self {
            Applied::Now => "now",
            Applied::AtMount => "at_mount",
            Applied::Restart => "restart",
        }
    }
}

/// Bring the mounted card to the clock the setting and its record call
/// for (and, with `probe`, probe it if that is full speed in `fast`). Also
/// `check_card`'s way back to full speed for a different card than the
/// record's (on the boot task before the handover, the record forgotten
/// first). [`Applied::Restart`]: the caller restarts.
pub fn settle(probe: bool) -> Applied {
    let applied = {
        let _io = io();
        if CARD.load(Relaxed).is_null() {
            return if dead() { Applied::Restart } else { Applied::AtMount };
        }
        let want = card_speed::clock_for(mode(), sdhealth::is_slow(SERIAL.load(Relaxed)), CAPPED.load(Relaxed));
        match set_clock_locked(want.khz) {
            Ok(()) => {
                sdhealth::mounted(want.khz, want.fallback);
                Applied::Now
            }
            Err(e) => {
                log::error!("microSD: clock change: {}", e.why());
                Applied::Restart
            }
        }
    };
    if applied == Applied::Now && probe {
        probe_soon(std::time::Duration::ZERO);
    }
    applied
}

/// `card_speed` changed (`PUT /api/config`, saved): in effect at once when
/// a card is mounted; [`Applied::Restart`]: the caller restarts.
pub fn set_mode(new: CardSpeed) -> Applied {
    let old = mode();
    MODE.store(new.as_u8(), Relaxed);
    if old == new {
        return if CARD.load(Relaxed).is_null() { Applied::AtMount } else { Applied::Now };
    }
    settle(true)
}

/// "Try full speed again" (`POST /api/card/full-speed`): forget this
/// card's slow-clock record and go back to full speed, at once when a card
/// is mounted (and probe it). `test_errors`: the test hook, that many
/// transfers at full speed fail on purpose (the probe is skipped, so the
/// player's or a download's transfer meets them). [`Applied::Restart`]:
/// the caller restarts (the record is cleared at that boot too).
pub fn full_speed_again(test_errors: u32) -> Result<Applied, &'static str> {
    if mode() == CardSpeed::Safe {
        return Err("card_speed is safe: the card always runs at the safe speed");
    }
    sdhealth::full_again();
    CAPPED.store(false, Relaxed);
    // A hook's failures left from before (they fire at full speed only)
    // go: this one arms its own or none.
    #[cfg(feature = "bench")]
    SIM_ERRORS.store(test_errors.min(20), Relaxed);
    if test_errors > 0 {
        PROBE.store(SKIPPED, Relaxed);
    }
    let a = settle(test_errors == 0);
    if a == Applied::Restart {
        // NVS may still hold the record (the network worker copies it):
        // the next boot forgets it.
        sdhealth::request_clear();
    }
    Ok(a)
}

// ---------------------------------------------------------------------------
// The probe
// ---------------------------------------------------------------------------

/// Start the probe on a task of its own after `delay`, if the card is at
/// full speed in `fast`, the player has it (not an unreadable card
/// the boot task holds), one is not running already and the heap has room
/// for the task ([`config::SD_PROBE_HEAP`]).
pub fn probe_soon(delay: std::time::Duration) {
    if !card_speed::should_probe(mode(), khz()) || !crate::CARD_READY.load(Relaxed) {
        return;
    }
    // SAFETY: an argument-free read of the heap's accounting.
    let block = unsafe {
        esp_idf_sys::heap_caps_get_largest_free_block(esp_idf_sys::MALLOC_CAP_INTERNAL)
    };
    if block < config::SD_PROBE_HEAP {
        PROBE.store(SKIPPED, Relaxed);
        return;
    }
    if PROBE_TASK.compare_exchange(false, true, AcqRel, Relaxed).is_err() {
        return;
    }
    PROBE.store(WAITING, Relaxed);
    let spawned = crate::task::spawn_core0(
        b"sd-probe\0",
        config::SD_PROBE_STACK,
        1,
        Box::new(move || {
            std::thread::sleep(delay);
            probe();
            PROBE_TASK.store(false, Release);
        }),
    )
    .is_ok();
    if !spawned {
        PROBE.store(SKIPPED, Relaxed);
        PROBE_TASK.store(false, Release);
    }
}

/// One raw read of `count` sectors from `sector` into `buf`, through
/// [`transfer`] (so an error slows the card down as any would).
fn read_raw(buf: &mut [u32], sector: u32, count: u32) -> bool {
    transfer(false, &mut || {
        let card = CARD.load(Relaxed);
        // SAFETY: under `IO` (held by `transfer`), so `card` is the
        // mounted card or null; `buf` holds `count` sectors.
        !card.is_null()
            && unsafe {
                esp_idf_sys::sdmmc_read_sectors(card, buf.as_mut_ptr().cast(), sector as _, count as _)
            } == esp_idf_sys::ESP_OK
    }) == DRESULT_RES_OK
}

/// Still the card the probe started on, at full speed in `fast`;
/// waits (up to [`config::SD_PROBE_WAIT`] in all) while the cache builder's
/// task is up or a transfer shares the card, so the probe never adds to
/// their load.
fn may_read(gen: u32, waited: &mut std::time::Duration) -> bool {
    loop {
        if GEN.load(Relaxed) != gen || KHZ.load(Relaxed) != FULL_KHZ || mode() != CardSpeed::Fast {
            return false;
        }
        if !crate::cache_builder::task_up() && !crate::cache_builder::transfer_in_flight() {
            return true;
        }
        if *waited >= config::SD_PROBE_WAIT {
            return false;
        }
        std::thread::sleep(config::SD_PROBE_BUSY_POLL);
        *waited += config::SD_PROBE_BUSY_POLL;
    }
}

fn probe() {
    let gen = GEN.load(Relaxed);
    let mut waited = std::time::Duration::ZERO;
    let chunk = config::SD_PROBE_CHUNK;
    let mut buf: Vec<u32> = Vec::new();
    if !may_read(gen, &mut waited) || buf.try_reserve_exact(chunk as usize * 128).is_err() {
        PROBE.store(SKIPPED, Relaxed);
        return;
    }
    buf.resize(chunk as usize * 128, 0);
    PROBE.store(RUNNING, Relaxed);
    let mut done = true;
    for (sector, count) in card_speed::plan(config::SD_PROBE_START, SECTORS.load(Relaxed), config::SD_PROBE_BYTES, chunk) {
        if !may_read(gen, &mut waited) || !read_raw(&mut buf, sector, count) {
            done = false;
            break;
        }
        std::thread::sleep(config::SD_PROBE_PAUSE);
    }
    let state = if PROBE.load(Relaxed) == SLOWED {
        SLOWED
    } else if done {
        CLEAN
    } else {
        SKIPPED
    };
    PROBE.store(state, Relaxed);
}

/// `sd`'s fields from here (after `sdhealth`'s): the probe, this boot's
/// in-place slow-downs and recoveries. Written straight into the status
/// reply's `format!` (no `String` of its own).
pub struct Status;

impl core::fmt::Display for Status {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            r#""probe":{},"slowdowns":{},"recoveries":{}"#,
            PROBE_NAMES[PROBE.load(Relaxed) as usize % PROBE_NAMES.len()],
            SLOWDOWNS.load(Relaxed),
            STOPS.load(Relaxed)
        )
    }
}
