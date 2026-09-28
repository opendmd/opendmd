//! The card's health while running: counting card I/O failures, deciding
//! the card (not a file) is failing, and what survives the restart that
//! follows — the slow-clock fallback and the restart budget.
//!
//! **What counts.** A card-class error ([`is_card_error`]: `EIO`,
//! `ETIMEDOUT`, `ENODEV`, which is what FATFS makes of an `sdmmc` command
//! timeout, a CRC error or a card that stopped answering) from
//! - the player's file opens and reads (`sd::open_file`, the
//!   `frametime::Counted` reader, the playlist's `CountedCatalog`), and
//! - the stream task's file serving (`files::serve`).
//!
//! A missing file (`ENOENT`), a bad name (`EINVAL`), a GIF that does not
//! decode (`InvalidData`, raised above the reader) or a truncated one
//! (`UnexpectedEof`) never counts. [`SD_FAIL_TRIP`] errors within
//! [`SD_FAIL_WINDOW`] **trip**, provided at least one of them was on a file
//! that had been fine: a read on a handle that had already read, or an
//! open of one of the last few paths that opened (**proven**). Without that,
//! three errors could be three bad files.
//!
//! **The trip** is acted on by the main task (`main.rs`, `card_failed`): an
//! error line, the slow-clock record, then a restart — or, with the restart
//! budget spent, the player stops and the device stays up.
//!
//! **What lives where.** Everything here is in RTC slow memory
//! (`.rtc_noinit`, beside the log ring): no heap and no DRAM. A software
//! restart keeps it, a power cycle does not (it is garbage then, and the
//! boot treats a power-on reset as "nothing recorded"):
//! - [`Rtc`] (across restarts): the restart budget (count, and the time
//!   since the window's first restart), the reset note for the next boot,
//!   the slow-clock record's copy, a clear request, the I/O test hook.
//! - [`Live`] (this boot, zeroed by [`boot`]): the failure window, the
//!   counts, the recently opened paths, the facts status reports.
//!
//! The slow-clock record itself is in NVS too (`nvs_config::sd_slow_*`,
//! the card's CID serial), so it outlives a power cycle: a card that failed
//! at 40 MHz stays at 20 MHz until a different card is seen. Nothing clears
//! it by time; `POST /api/reboot?sd_fallback=clear` is the bench's way.
//!
//! Only plain word loads and stores touch RTC slow memory (the ESP32's
//! `S32C1I`, behind `fetch_add` and compare-and-swap, does not work
//! there): two tasks recording a failure at the same instant can lose one
//! count, which the next failure makes up.

use core::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::io;

use crate::config::{
    SD_FAIL_TRIP, SD_FAIL_WINDOW, SD_REBOOT_CAP, SD_REBOOT_WINDOW, SD_REMOUNT_INTERVAL,
    SD_SPEED_FALLBACK_KHZ, SD_SPEED_KHZ,
};

const MAGIC: u32 = 0x5d_4e_a1_7b;
/// `Rtc::clear` values: forget the slow-clock record and the restart count
/// (`?sd_fallback=clear`), or the restart count alone (a deliberate reboot).
const CLEAR_ALL: u32 = 1;
const CLEAR_COUNT: u32 = 2;

/// Why the last restart was made, for the next boot's status.
const NOTE_NONE: u32 = 0;
const NOTE_CARD_FAILURE: u32 = 1;
const NOTE_NEW_CARD: u32 = 2;

/// Across restarts.
#[repr(C)]
struct Rtc {
    magic: u32,
    /// Card-failure restarts in the current window.
    reboots: u32,
    /// Milliseconds from the window's first restart to the last restart
    /// (each run's uptime added as it restarts; see [`note_restart`]).
    window_ms: u32,
    /// `NOTE_*` for the next boot; consumed there.
    note: u32,
    /// The trip behind the last card-failure restart.
    trip_failures: u32,
    trip_khz: u32,
    /// 1: the card with `serial` failed at full speed (NVS has the same;
    /// this copy covers an NVS write that failed).
    slow: u32,
    serial: u32,
    /// 1: forget the slow-clock record and the restart budget at boot.
    clear: u32,
    /// The I/O test hook: fail this many player reads on each of the next
    /// `sim_boots` boots.
    sim_per_boot: u32,
    sim_boots: u32,
}

/// This boot.
#[repr(C)]
struct Live {
    /// When the last [`SD_FAIL_TRIP`] failures happened (ms since boot, +1;
    /// 0 = empty), a ring.
    times: [u32; SD_FAIL_TRIP],
    next: u32,
    /// The last proven failure (ms since boot, +1).
    proven_at: u32,
    opens: u32,
    reads: u32,
    serves: u32,
    proven: u32,
    tripped: u32,
    /// Simulated read failures still to come (the test hook).
    sim_left: u32,
    /// FNV hashes of the last paths that opened.
    good: [u32; 4],
    good_next: u32,
    /// For status: the mounted clock (0 unmounted), whether it is the
    /// fallback, this boot's reset note.
    clock_khz: u32,
    fallback: u32,
    note: u32,
}

#[link_section = ".rtc_noinit"]
static mut RTC: Rtc = Rtc {
    magic: 0,
    reboots: 0,
    window_ms: 0,
    note: 0,
    trip_failures: 0,
    trip_khz: 0,
    slow: 0,
    serial: 0,
    clear: 0,
    sim_per_boot: 0,
    sim_boots: 0,
};

#[link_section = ".rtc_noinit"]
static mut LIVE: Live = Live {
    times: [0; SD_FAIL_TRIP],
    next: 0,
    proven_at: 0,
    opens: 0,
    reads: 0,
    serves: 0,
    proven: 0,
    tripped: 0,
    sim_left: 0,
    good: [0; 4],
    good_next: 0,
    clock_khz: 0,
    fallback: 0,
    note: 0,
};

/// A word of [`RTC`] or [`LIVE`] as an atomic, for plain `Relaxed` loads and
/// stores only (see the module note).
macro_rules! w {
    ($s:ident . $($f:tt)+) => {
        // SAFETY: a 4-aligned word of a static that is only ever accessed
        // through this atomic view (loads and stores, never RMW).
        unsafe { AtomicU32::from_ptr(&raw mut $s.$($f)+) }
    };
}

fn get(a: &AtomicU32) -> u32 {
    a.load(Relaxed)
}

fn set(a: &AtomicU32, v: u32) {
    a.store(v, Relaxed)
}

fn bump(a: &AtomicU32) {
    a.store(a.load(Relaxed).wrapping_add(1), Relaxed)
}

/// Milliseconds since boot, +1 so that 0 means "never".
fn now_ms() -> u32 {
    // SAFETY: argument-free read of the IDF's microsecond clock.
    ((unsafe { esp_idf_sys::esp_timer_get_time() } / 1000) as u32).wrapping_add(1)
}

const WINDOW_MS: u32 = SD_FAIL_WINDOW.as_millis() as u32;
const REBOOT_WINDOW_MS: u32 = SD_REBOOT_WINDOW.as_millis() as u32;

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

/// What the boot does with the card, from the records.
pub struct Boot {
    /// The clock to mount at.
    pub speed_khz: u32,
    /// The card that must be seen for [`Boot::speed_khz`] to stay slow.
    pub slow_serial: Option<u32>,
    /// The restart budget is spent: no mount at boot, a remount every
    /// [`SD_REMOUNT_INTERVAL`] instead, and no restart on another trip.
    pub capped: bool,
    /// NVS's slow-clock record is to be removed (a clear was asked for).
    pub clear_nvs: bool,
}

/// Read and settle the records, once, before the first mount. `nvs_slow`
/// is NVS's slow-clock record (a CID serial); `poweron` says RTC slow
/// memory is garbage.
pub fn boot(poweron: bool, nvs_slow: Option<u32>) -> Boot {
    // SAFETY: before any other task exists; nothing else touches LIVE yet.
    unsafe {
        (&raw mut LIVE).write_bytes(0, 1);
    }
    if poweron || get(w!(RTC.magic)) != MAGIC {
        // SAFETY: as above, and nothing else touches RTC before this.
        unsafe { (&raw mut RTC).write_bytes(0, 1) };
        set(w!(RTC.magic), MAGIC);
    }
    let mut clear_nvs = false;
    if get(w!(RTC.clear)) == CLEAR_COUNT {
        set(w!(RTC.clear), 0);
        if get(w!(RTC.reboots)) > 0 {
            log::warn!(
                "microSD: card-failure restart count cleared ({} in this window) by a deliberate reboot; the slow-clock record is kept",
                get(w!(RTC.reboots))
            );
        }
        set(w!(RTC.reboots), 0);
        set(w!(RTC.window_ms), 0);
    }
    if get(w!(RTC.clear)) == CLEAR_ALL {
        for f in [
            w!(RTC.clear),
            w!(RTC.slow),
            w!(RTC.serial),
            w!(RTC.reboots),
            w!(RTC.window_ms),
        ] {
            set(f, 0);
        }
        clear_nvs = nvs_slow.is_some();
        log::warn!(
            "microSD: slow-clock record{} and card-failure restart count cleared (POST /api/reboot?sd_fallback=clear)",
            if clear_nvs { "" } else { " (there was none)" }
        );
    }
    let note = get(w!(RTC.note));
    set(w!(RTC.note), NOTE_NONE);
    set(w!(LIVE.note), note);
    let reboots = get(w!(RTC.reboots));
    let window_ms = get(w!(RTC.window_ms));
    match note {
        NOTE_CARD_FAILURE => log::warn!(
            "reset note: card failure ({} card I/O errors within {} s at {} kHz); card-failure restart {reboots} of {SD_REBOOT_CAP} in {} min",
            get(w!(RTC.trip_failures)),
            SD_FAIL_WINDOW.as_secs(),
            get(w!(RTC.trip_khz)),
            SD_REBOOT_WINDOW.as_secs() / 60,
        ),
        NOTE_NEW_CARD => log::warn!("reset note: a different card; its predecessor's slow clock no longer applies"),
        _ => {}
    }
    if reboots > 0 && window_ms >= REBOOT_WINDOW_MS {
        set(w!(RTC.reboots), 0);
        set(w!(RTC.window_ms), 0);
    }
    let capped = get(w!(RTC.reboots)) >= SD_REBOOT_CAP;

    let slow_serial = if clear_nvs {
        None
    } else {
        nvs_slow.or((get(w!(RTC.slow)) == 1).then(|| get(w!(RTC.serial))))
    };
    let speed_khz = if slow_serial.is_some() || capped {
        SD_SPEED_FALLBACK_KHZ
    } else {
        SD_SPEED_KHZ
    };
    if let Some(serial) = slow_serial {
        log::warn!(
            "microSD: card {serial:#010x} failed at {SD_SPEED_KHZ} kHz before; mounting at {SD_SPEED_FALLBACK_KHZ} kHz (a different card goes back to {SD_SPEED_KHZ} kHz)"
        );
    }
    if capped {
        log::error!(
            "microSD: {} card-failure restarts within {} s (the cap is {SD_REBOOT_CAP} in {} min): not mounting at boot and not restarting again; remounting every {} s at {SD_SPEED_FALLBACK_KHZ} kHz",
            get(w!(RTC.reboots)),
            window_ms / 1000,
            SD_REBOOT_WINDOW.as_secs() / 60,
            SD_REMOUNT_INTERVAL.as_secs()
        );
    }

    // The I/O test hook, re-armed per boot.
    let boots = get(w!(RTC.sim_boots));
    if boots > 0 {
        set(w!(RTC.sim_boots), boots - 1);
        let n = get(w!(RTC.sim_per_boot));
        set(w!(LIVE.sim_left), n);
        log::warn!(
            "test hook: from 10 s after boot, the player's next {n} reads of files that had read fail (EIO); {} more boot(s) armed",
            boots - 1
        );
    }
    Boot {
        speed_khz,
        slow_serial,
        capped,
        clear_nvs,
    }
}

/// A card mounted: its clock, for status. `fallback`: below full speed
/// because of the record (or the spent budget).
pub fn mounted(real_khz: u32, fallback: bool) {
    set(w!(LIVE.clock_khz), real_khz);
    set(w!(LIVE.fallback), fallback as u32);
}

/// Forget the slow-clock record's RTC copy and the restart budget (a
/// different card), leaving a note for the next boot.
pub fn forget_for_new_card() {
    for f in [
        w!(RTC.slow),
        w!(RTC.serial),
        w!(RTC.reboots),
        w!(RTC.window_ms),
    ] {
        set(f, 0);
    }
    set(w!(RTC.note), NOTE_NEW_CARD);
}

/// Ask the next boot to forget the slow-clock record and the restart
/// budget (`POST /api/reboot?sd_fallback=clear`).
pub fn request_clear() {
    set(w!(RTC.clear), CLEAR_ALL);
}

/// A deliberate reboot (`POST /api/reboot` with no test hook, i.e. the
/// UI's button): the next boot forgets the card-failure restart count and
/// its window, so a person's own restart mounts the card at once instead
/// of inheriting the cap's 60 s wait. The slow-clock record is kept: the
/// card is still the card that failed.
pub fn request_fresh_start() {
    if get(w!(RTC.clear)) == 0 {
        set(w!(RTC.clear), CLEAR_COUNT);
    }
}

/// Every planned restart: this run's uptime goes into the budget's window,
/// which is measured across restarts in uptimes (a crash's run is lost,
/// which only makes the window look shorter, i.e. the cap stricter).
pub fn note_restart() {
    if get(w!(RTC.reboots)) > 0 {
        let w = get(w!(RTC.window_ms)).saturating_add(now_ms());
        set(w!(RTC.window_ms), w);
    }
}

/// Milliseconds until the restart budget's window ends (0: it has).
pub fn window_left_ms() -> u32 {
    if get(w!(RTC.reboots)) == 0 {
        return 0;
    }
    let elapsed = get(w!(RTC.window_ms)).saturating_add(now_ms());
    REBOOT_WINDOW_MS.saturating_sub(elapsed)
}

/// Card-failure restarts in the current window.
pub fn reboots() -> u32 {
    if window_left_ms() == 0 {
        0
    } else {
        get(w!(RTC.reboots))
    }
}

// ---------------------------------------------------------------------------
// Counting
// ---------------------------------------------------------------------------

/// Where a failure was seen.
#[derive(Clone, Copy)]
pub enum Kind {
    /// The player opening a file.
    Open,
    /// The player reading or seeking a file (the GIF, the playlist).
    Read,
    /// The stream task serving a file.
    Serve,
}

/// An I/O error that means the card, not the file: what FATFS returns when
/// the `sdmmc` layer fails (`EIO`: `FR_DISK_ERR`, a command timeout or a
/// CRC error; `ENODEV`: `FR_NOT_READY`) or when the volume stays locked by
/// a card operation that will not finish (`ETIMEDOUT`: `FR_TIMEOUT`).
pub fn is_card_error(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(c) if c == esp_idf_sys::EIO as i32
        || c == esp_idf_sys::ETIMEDOUT as i32
        || c == esp_idf_sys::ENODEV as i32)
}

fn hash(path: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in path.bytes() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    h | 1
}

/// `path` opened: remember it, so a later failure to open it is proven.
pub fn opened(path: &str) {
    let i = get(w!(LIVE.good_next));
    set(w!(LIVE.good[(i as usize) % 4]), hash(path));
    set(w!(LIVE.good_next), i.wrapping_add(1));
}

/// An open of `path` failed with `e`.
pub fn open_failed(kind: Kind, path: &str, e: &io::Error) {
    if !is_card_error(e) {
        return;
    }
    let h = hash(path);
    let proven = (0..4).any(|i| get(w!(LIVE.good[i])) == h);
    record(kind, proven, path, e);
}

/// A read or seek failed with `e`; `proven` if the handle had read before.
pub fn read_failed(kind: Kind, proven: bool, what: &str, e: &io::Error) {
    if is_card_error(e) {
        record(kind, proven, what, e);
    }
}

fn record(kind: Kind, proven: bool, what: &str, e: &io::Error) {
    let now = now_ms();
    let i = get(w!(LIVE.next));
    set(w!(LIVE.times[(i as usize) % SD_FAIL_TRIP]), now);
    set(w!(LIVE.next), i.wrapping_add(1));
    bump(match kind {
        Kind::Open => w!(LIVE.opens),
        Kind::Read => w!(LIVE.reads),
        Kind::Serve => w!(LIVE.serves),
    });
    if proven {
        set(w!(LIVE.proven_at), now);
        bump(w!(LIVE.proven));
    }
    let n = in_window(now);
    let proven_at = get(w!(LIVE.proven_at));
    let trip = n >= SD_FAIL_TRIP && proven_at != 0 && now.wrapping_sub(proven_at) <= WINDOW_MS;
    log::warn!(
        "microSD: card I/O error {} {what}{}: {e}; {n} in the last {} s{}",
        match kind {
            Kind::Open => "opening",
            Kind::Read => "reading",
            Kind::Serve => "serving",
        },
        if proven { " (it had been fine)" } else { "" },
        SD_FAIL_WINDOW.as_secs(),
        if trip { ": the card is failing" } else { "" }
    );
    if trip {
        set(w!(LIVE.tripped), 1);
    }
}

fn in_window(now: u32) -> usize {
    (0..SD_FAIL_TRIP)
        .filter(|&i| {
            let t = get(w!(LIVE.times[i]));
            t != 0 && now.wrapping_sub(t) <= WINDOW_MS
        })
        .count()
}

/// Failures in the last [`SD_FAIL_WINDOW`] (at most [`SD_FAIL_TRIP`]).
pub fn failures() -> usize {
    in_window(now_ms())
}

/// The card has tripped (and the main task has not acted on it yet).
pub fn tripped() -> bool {
    get(w!(LIVE.tripped)) == 1
}

/// What the main task does about a trip.
pub struct Trip {
    /// Restart now (budget left); else stay up without the card.
    pub restart: bool,
    /// The failure happened at full speed: record the card as slow.
    pub slow_down: bool,
    /// The line for the log and for `sd.error`.
    pub summary: String,
}

/// Act on a trip: counts, the restart budget, the slow-clock record's RTC
/// copy, the note for the next boot. `serial`/`khz`: the mounted card.
pub fn trip(serial: u32, khz: u32) -> Trip {
    set(w!(LIVE.tripped), 0);
    let failures = failures() as u32;
    let slow_down = khz > SD_SPEED_FALLBACK_KHZ;
    if slow_down {
        set(w!(RTC.slow), 1);
        set(w!(RTC.serial), serial);
    }
    // A window that has run out starts again with this restart.
    if window_left_ms() == 0 {
        set(w!(RTC.reboots), 0);
        set(w!(RTC.window_ms), 0);
    }
    let reboots = get(w!(RTC.reboots));
    let restart = reboots < SD_REBOOT_CAP;
    // For the next boot's note (this one's, or the window's end).
    set(w!(RTC.trip_failures), failures);
    set(w!(RTC.trip_khz), khz);
    let counts = format!(
        "{failures} card I/O errors within {} s (opens {}, reads {}, served {}; {} on files that had been fine) at {khz} kHz",
        SD_FAIL_WINDOW.as_secs(),
        get(w!(LIVE.opens)),
        get(w!(LIVE.reads)),
        get(w!(LIVE.serves)),
        get(w!(LIVE.proven)),
    );
    if restart {
        // This run's uptime into the window (from its second restart on).
        note_restart();
        set(w!(RTC.reboots), reboots + 1);
        set(w!(RTC.note), NOTE_CARD_FAILURE);
        Trip {
            restart,
            slow_down,
            summary: format!(
                "card failing: {counts}; restarting (card-failure restart {} of {SD_REBOOT_CAP} in {} min){}",
                reboots + 1,
                SD_REBOOT_WINDOW.as_secs() / 60,
                if slow_down {
                    format!(", next boot at {SD_SPEED_FALLBACK_KHZ} kHz for card {serial:#010x}")
                } else {
                    String::new()
                }
            ),
        }
    } else {
        Trip {
            restart,
            slow_down,
            summary: format!(
                "card failing again: {counts}; not restarting ({reboots} card-failure restarts in {} min); the player has stopped, one restart when that window ends in {} min",
                SD_REBOOT_WINDOW.as_secs() / 60,
                window_left_ms().div_ceil(60_000)
            ),
        }
    }
}

/// The restart a stopped card (budget spent, failed again) gets once the
/// window has run out: a fresh window's first.
pub fn restart_after_window() {
    set(w!(RTC.reboots), 1);
    set(w!(RTC.window_ms), 0);
    set(w!(RTC.note), NOTE_CARD_FAILURE);
}

/// `sd.error` while the budget is spent and the card is not mounted.
pub fn capped_error(attempt: u32, last: Option<&str>) -> String {
    format!(
        "card failed repeatedly: {} card-failure restarts in {} min, so no more; remounting every {} s at {SD_SPEED_FALLBACK_KHZ} kHz (remount {attempt}{}{})",
        get(w!(RTC.reboots)),
        SD_REBOOT_WINDOW.as_secs() / 60,
        SD_REMOUNT_INTERVAL.as_secs(),
        if last.is_some() { ": " } else { "" },
        last.unwrap_or("")
    )
}

// ---------------------------------------------------------------------------
// The test hook
// ---------------------------------------------------------------------------

/// Arm the I/O test hook (`POST /api/reboot?sd_io_fail=N&sd_io_boots=B`):
/// on each of the next `boots` boots, from 10 s after boot, the player's
/// next `n` card reads of files that had read fail with `EIO`.
pub fn simulate_io_failures(n: u32, boots: u32) {
    set(w!(RTC.sim_per_boot), n);
    set(w!(RTC.sim_boots), boots);
}

/// The hook's chain is still running: a simulated mount failure
/// (`sd::take_simulated_failure`) waits until it is over, so one request
/// can set up restarts first and failing remounts after.
pub fn sim_pending() -> bool {
    get(w!(RTC.sim_boots)) > 0 || get(w!(LIVE.sim_left)) > 0
}

/// The I/O test hook's failures start this long after boot.
const SIM_AFTER_MS: u32 = 10_000;

/// One simulated read failure, if the hook has one left.
pub fn take_sim_read() -> Option<io::Error> {
    let left = get(w!(LIVE.sim_left));
    // Not before the player has been going a while, so the demonstration
    // shows it playing first.
    if left == 0 || now_ms() < SIM_AFTER_MS {
        return None;
    }
    set(w!(LIVE.sim_left), left - 1);
    Some(io::Error::from_raw_os_error(esp_idf_sys::EIO as i32))
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// `sd`'s extra fields (after `attempts`), and the top-level `reset_note`.
pub fn status_json() -> String {
    let khz = get(w!(LIVE.clock_khz));
    format!(
        r#""clock_khz":{},"fallback":{},"failures":{},"reboots":{}"#,
        if khz == 0 {
            "null".to_string()
        } else {
            khz.to_string()
        },
        get(w!(LIVE.fallback)) == 1,
        failures(),
        reboots(),
    )
}

/// Why this boot's restart was made, when it was this module's doing.
pub fn reset_note() -> Option<&'static str> {
    match get(w!(LIVE.note)) {
        NOTE_CARD_FAILURE => Some("card failure"),
        NOTE_NEW_CARD => Some("different card"),
        _ => None,
    }
}

/// The heartbeat's part, only once something has failed.
pub fn heartbeat() -> String {
    let (o, r, s) = (
        get(w!(LIVE.opens)),
        get(w!(LIVE.reads)),
        get(w!(LIVE.serves)),
    );
    if o + r + s == 0 {
        return String::new();
    }
    format!(
        ", card errors: opens {o} reads {r} served {s} ({} proven; {} in the last {} s)",
        get(w!(LIVE.proven)),
        failures(),
        SD_FAIL_WINDOW.as_secs()
    )
}
