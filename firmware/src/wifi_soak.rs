//! The emulator's WiFi soak lane (`make emu-wifi-soak`, feature
//! `emu-wifi-soak`, tools/emu/wifi_soak_lane.py): never in a release. Run
//! from the network worker's loop on CPU0 once the WiFi has been up a while
//! and the card's first chores are done, it keeps the network away from the
//! station for long stretches, the way a test panel met it on 2026-11-21
//! (offline for hours with its network up, 1356 reconnect tries, the heap at
//! 11.7 KB free in pieces of 2.3 KB at the end), each step logged as
//! `wifi-soak: …` for the lane to judge:
//!
//! 1. `start`: playback paused (the display's own allocations would hide a
//!    few bytes a try), the heap noted.
//! 2. `direct`: [`DIRECT_TRIES`] connects pinned to an access point and
//!    channel that are not there (the learned ones spoiled), one after
//!    another: the heap after each ten, so a try that keeps some of it
//!    shows (the driver's own allocations included).
//! 3. `away`: the network the reconnect looks for renamed to one that is
//!    not there: the worker's own reconnect runs its rounds (scans that
//!    find nothing, the backoff, the driver restarts every
//!    `RESET_ROUNDS`) for [`AWAY_TRIES`] tries; its `reconnect failed`
//!    lines carry the heap after each.
//! 4. `refused`: the name given back with a wrong password: the scan
//!    finds the network and every rung of the connect ladder fails its
//!    handshake, for [`REFUSED_TRIES`] rounds (ten attempts each), the heap
//!    after each round on its `reconnect failed` line.
//! 5. `back`: the password given back: the next round joins.
//! 6. `tight`: the heap taken in 1 KB blocks, every third given back, so
//!    ~12 KB is free in pieces just big enough for the reconnect's scan
//!    (the test panel's heap at its restart, a block bigger), the learned
//!    access point spoiled and the association dropped: for [`TIGHT_MS`]
//!    the reconnect has that heap to rejoin in (whether it can is the
//!    finding), then `release` gives it all back and it must rejoin.
//!    Playback resumed (`done`).
//! 7. `stuck 1`: the name taken away again, the hour's restart made
//!    [`STUCK_MS`] (`Reconnect::set_down_stuck_ms`), and the next boot told
//!    (RTC memory) to look for the name nobody answers to: the `wifi_stuck`
//!    restart; boot 2 does not find the network, so it reconnects without
//!    the setup network, and with the short hour set from its start it is
//!    not restarted again (the outage spent); after [`BOOT2_TRIES`] tries,
//!    past the short hour, the name is given back and it rejoins
//!    (`boot 2 back`).
//! 8. `stuck 2`: a wrong password and a short hour again, the next boot
//!    told to join with it: the `wifi_stuck` restart; boot 3 finds the
//!    network and is refused, so it raises the setup network, as any boot
//!    with a changed password.
//!
//! The emulator models no access point going away: a spoiled pin and a
//! name nobody answers to are the station's side of one.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use crate::chan as mpsc;
use crate::state::Command;
use crate::wifi::WifiManager;
use opendmd_core::wifi_reconnect::Reconnect;

/// Pinned connects in the `direct` step.
pub const DIRECT_TRIES: u32 = 40;
/// Reconnect tries in the `away` step (rounds 6, 12 and 18 restart the
/// driver).
pub const AWAY_TRIES: u32 = 20;
const AWAY: &str = "-away";
/// Reconnect rounds in the `refused` step.
pub const REFUSED_TRIES: u32 = 3;
const WRONG: &str = "-wrong";
/// How long the `tight` step keeps the heap squeezed.
pub const TIGHT_MS: u32 = 150_000;
/// The hour's restart in the `stuck` steps.
pub const STUCK_MS: u32 = 150_000;
/// Boot 2's reconnect tries before the name is given back.
/// Past [`STUCK_MS`] (~17 s a try): the short hour set from boot 2's start
/// would have restarted it, were the outage not spent.
pub const BOOT2_TRIES: u32 = 12;

/// What the next boot joins with (`[MAGIC, stage]`, RTC slow memory: kept
/// across a restart, lost at a power-on; plain word loads and stores).
#[link_section = ".rtc_noinit"]
static mut NEXT_BOOT: [u32; 2] = [0; 2];
const MAGIC: u32 = 0x5753_4f4b;
const BOOT_AWAY: u32 = 1;
const BOOT_WRONG: u32 = 2;
/// This boot's stage (0: the first boot).
static BOOT: AtomicU32 = AtomicU32::new(0);

fn next_boot(i: usize) -> &'static AtomicU32 {
    // SAFETY: a word of `NEXT_BOOT`, only ever accessed through this view.
    unsafe { AtomicU32::from_ptr((&raw mut NEXT_BOOT).cast::<u32>().add(i)) }
}

fn tell_next_boot(stage: u32) {
    next_boot(1).store(stage, Relaxed);
    next_boot(0).store(MAGIC, Relaxed);
}

/// The login the boot joins with: as saved, or as the previous boot's step
/// told it (boot 2 the name nobody answers to, boot 3 a wrong password).
pub fn boot_login(mut ssid: String, mut password: String) -> (String, String) {
    let stage = if next_boot(0).load(Relaxed) == MAGIC { next_boot(1).load(Relaxed) } else { 0 };
    next_boot(0).store(0, Relaxed);
    match stage {
        BOOT_AWAY => ssid.push_str(AWAY),
        BOOT_WRONG => password.push_str(WRONG),
        _ => return (ssid, password),
    }
    BOOT.store(stage, Relaxed);
    log::info!("wifi-soak: boot {}: joining '{ssid}'{}", stage + 1, if stage == BOOT_WRONG { " with a wrong password" } else { "" });
    (ssid, password)
}

static STEP: AtomicU32 = AtomicU32::new(0);
static AT: AtomicU32 = AtomicU32::new(0);
static UP_SINCE: AtomicU32 = AtomicU32::new(0);
/// The reconnect tries when `away` began.
static TRIES_AT: AtomicU32 = AtomicU32::new(0);

fn now_ms() -> u32 {
    // SAFETY: an argument-free read of the IDF's microsecond clock.
    (unsafe { esp_idf_sys::esp_timer_get_time() } / 1000) as u32
}

fn next(step: u32) {
    STEP.store(step, Relaxed);
    AT.store(now_ms(), Relaxed);
}

fn since() -> u32 {
    now_ms().wrapping_sub(AT.load(Relaxed))
}

fn heap() -> (u32, usize) {
    (crate::state::free_heap(), crate::state::largest_free_block())
}

/// The heap the `tight` step holds: 1 KB blocks, some given back.
static HELD: std::sync::Mutex<Vec<Option<Vec<u8>>>> = std::sync::Mutex::new(Vec::new());
const BLOCKS: usize = 320;

/// Every free piece of the heap kept under `limit` with about `target`
/// free (as `oom_test`'s squeeze): 1 KB blocks taken while the largest
/// free block is `limit` or more, then every third of those just taken
/// given back while less than `target` is free. Run at every tick of the
/// step, so what others give back is crumbled in turn.
fn squeeze(limit: usize, target: u32) -> (usize, u32, usize) {
    let mut held = HELD.lock().unwrap();
    if held.capacity() < BLOCKS && held.try_reserve_exact(BLOCKS).is_err() {
        return (0, crate::state::free_heap(), crate::state::largest_free_block());
    }
    let start = held.len();
    while crate::state::largest_free_block() >= limit && held.len() < BLOCKS {
        let mut b = Vec::new();
        if b.try_reserve_exact(1024).is_err() {
            break;
        }
        b.resize(1024, 0x5a);
        held.push(Some(b));
    }
    let mut k = start;
    while crate::state::free_heap() < target && k < held.len() {
        held[k] = None;
        k += 3;
    }
    let n = held.iter().filter(|b| b.is_some()).count();
    (n, crate::state::free_heap(), crate::state::largest_free_block())
}

fn disconnect() {
    // SAFETY: an argument-free driver call on a started driver.
    unsafe { esp_idf_sys::esp_wifi_disconnect() };
}

fn pause(cmd: &mpsc::Sender<Command>) {
    let (tx, rx) = mpsc::channel();
    if cmd.send(Command::TogglePause(tx)).is_ok() {
        let _ = rx.recv_timeout(std::time::Duration::from_secs(5));
    }
}

/// One look, from the network worker's loop (about once a second).
/// `ssid`, `password`: what the reconnect joins with (changed while away
/// and refused).
pub fn tick(
    wifi: &mut WifiManager,
    ssid: &mut String,
    password: &mut String,
    again: &mut Reconnect,
    cmd: &mpsc::Sender<Command>,
) {
    let joined = wifi.state() == crate::wifi::WifiState::Connected && wifi.is_associated();
    if !joined {
        UP_SINCE.store(0, Relaxed);
    } else if UP_SINCE.load(Relaxed) == 0 {
        UP_SINCE.store(now_ms().max(1), Relaxed);
    }
    let up = joined && now_ms().wrapping_sub(UP_SINCE.load(Relaxed)) > 10_000;
    match BOOT.load(Relaxed) {
        BOOT_AWAY => return boot2(ssid, password, again, up),
        BOOT_WRONG => return, // boot 3: the boot decision is the step
        _ => {}
    }
    match STEP.load(Relaxed) {
        // The card builder's pass and the FAT index are done by ~120 s.
        0 if up && now_ms() > 150_000 => {
            pause(cmd);
            next(1);
        }
        1 if since() > 5_000 => {
            let (free, largest) = heap();
            log::info!("wifi-soak: start: playback paused, heap {free} B free, largest block {largest} B");
            wifi.spoil_learned();
            let mut least = free;
            for n in 1..=DIRECT_TRIES {
                let joined = wifi.connect_direct(ssid, "", None).unwrap_or(false);
                let (f, l) = heap();
                least = least.min(f);
                if joined {
                    log::warn!("wifi-soak: direct {n}: joined a spoiled pin");
                }
                if n % 10 == 0 {
                    log::info!(
                        "wifi-soak: direct {n}/{DIRECT_TRIES}: heap {f} B free, largest block {l} B, least {least} B, last disconnect reason {}",
                        crate::wifi::last_reason()
                    );
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            let (f, l) = heap();
            log::info!(
                "wifi-soak: direct done: {DIRECT_TRIES} tries, heap {f} B free (from {free} B), largest block {l} B (from {largest} B)"
            );
            TRIES_AT.store(crate::wifi::reconnect_tries(), Relaxed);
            ssid.push_str(AWAY);
            log::info!("wifi-soak: away: the reconnect looks for '{ssid}' for {AWAY_TRIES} tries, heap {f} B free, largest block {l} B");
            next(2);
        }
        2 if crate::wifi::reconnect_tries().wrapping_sub(TRIES_AT.load(Relaxed)) >= AWAY_TRIES => {
            let n = ssid.len() - AWAY.len();
            ssid.truncate(n);
            password.push_str(WRONG);
            TRIES_AT.store(crate::wifi::reconnect_tries(), Relaxed);
            let (f, l) = heap();
            log::info!(
                "wifi-soak: refused: the reconnect joins '{ssid}' with a wrong password for {REFUSED_TRIES} tries, heap {f} B free, largest block {l} B"
            );
            next(3);
        }
        3 if crate::wifi::reconnect_tries().wrapping_sub(TRIES_AT.load(Relaxed)) >= REFUSED_TRIES => {
            let n = password.len() - WRONG.len();
            password.truncate(n);
            let (f, l) = heap();
            log::info!("wifi-soak: back: the reconnect joins '{ssid}' with its password again, heap {f} B free, largest block {l} B");
            next(4);
        }
        4 if up => {
            let (f, l) = heap();
            log::info!("wifi-soak: rejoined: heap {f} B free, largest block {l} B");
            next(5);
        }
        5 if up => {
            let (n, f, l) = squeeze(3 * 1024, 12 * 1024);
            log::info!(
                "wifi-soak: tight: {n} blocks held, heap {f} B free, largest block {l} B; the learned access point spoiled, the association dropped for {} s",
                TIGHT_MS / 1000
            );
            wifi.spoil_learned();
            disconnect();
            next(6);
        }
        6 if since() < TIGHT_MS => {
            squeeze(3 * 1024, 12 * 1024);
        }
        6 => {
            let joined = wifi.state() == crate::wifi::WifiState::Connected && wifi.is_associated();
            let (f, l) = heap();
            *HELD.lock().unwrap() = Vec::new();
            let (rf, rl) = heap();
            log::info!(
                "wifi-soak: release: {} in the squeezed heap ({f} B free, largest block {l} B at the end), heap {rf} B free, largest block {rl} B now",
                if joined { "joined" } else { "not joined" }
            );
            next(7);
        }
        7 if up => {
            pause(cmd);
            log::info!("wifi-soak: done: playback resumed");
            next(8);
        }
        // The lane reads the status first.
        8 if up && since() > 30_000 => {
            tell_next_boot(BOOT_AWAY);
            ssid.push_str(AWAY);
            log::info!(
                "wifi-soak: stuck 1: the reconnect looks for '{ssid}', the hour's restart after {} s; the next boot looks for it too",
                STUCK_MS / 1000
            );
            disconnect();
            next(9);
        }
        9 => again.set_down_stuck_ms(STUCK_MS),
        _ => {}
    }
}

/// Boot 2 (out of `stuck 1`'s restart, looking for the name nobody answers
/// to): the name given back after [`BOOT2_TRIES`] tries, then `stuck 2`.
fn boot2(ssid: &mut String, password: &mut String, again: &mut Reconnect, up: bool) {
    match STEP.load(Relaxed) {
        0 if crate::wifi::reconnect_tries() < BOOT2_TRIES => again.set_down_stuck_ms(STUCK_MS),
        0 => {
            let n = ssid.len() - AWAY.len();
            ssid.truncate(n);
            log::info!("wifi-soak: boot 2: {BOOT2_TRIES} tries; the reconnect looks for '{ssid}' again");
            next(1);
        }
        1 if up => {
            log::info!("wifi-soak: boot 2 back: joined");
            next(2);
        }
        // The lane reads the status first.
        2 if since() > 30_000 => {
            tell_next_boot(BOOT_WRONG);
            password.push_str(WRONG);
            log::info!(
                "wifi-soak: stuck 2: the reconnect joins '{ssid}' with a wrong password, the hour's restart after {} s; the next boot joins with it too",
                STUCK_MS / 1000
            );
            disconnect();
            next(3);
        }
        3 => again.set_down_stuck_ms(STUCK_MS),
        _ => {}
    }
}
