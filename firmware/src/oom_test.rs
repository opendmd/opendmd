//! The emulator's low-heap lane (`make emu-oom`, feature `emu-oom-test`,
//! tools/emu/oom_lane.py): never in a release. Run from the network
//! worker's loop on CPU0, once the WiFi is up and a few GIFs have played,
//! it walks the firmware through what a test panel on 1.1.0-beta.2 met (a
//! heap in crumbs, perhaps the WiFi lost), each step logged as `oom-test: …` for
//! the lane to judge:
//!
//! 1. `drop 1`: the association dropped (`esp_wifi_disconnect`): the
//!    reconnect goes straight back to the access point, no scan.
//! 2. `drop 2`: the learned channel and access point spoiled first: the
//!    pinned connect fails, the scan for the name finds it, the ladder joins.
//! 3. `squeeze`: playback paused, then the heap taken in 1 KB blocks and
//!    every third one given back, so ~12 KB is free in pieces under 3 KB,
//!    kept so at every look for 35 s: the heartbeat in that time must be the short one,
//!    with no abort. Then `drop 3` with the heap squeezed harder (pieces of
//!    1 KB): the reconnect is held back for the heap, until `release` gives
//!    it all back and it goes straight back. Playback resumed.
//! 4. `abort`: an allocation of 1 MB that cannot be made, the way Rust's
//!    out-of-memory abort happens: the next boot must name it (the restart
//!    record's `where` and `activity`) and keep the run's last lines
//!    (`GET /api/log/last`), which the lane reads.
//!
//! The emulator models no access point going away, so the drops are the
//! station's own disconnects; the scans meet the emulator's one AP.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Mutex;

use crate::chan as mpsc;
use crate::state::Command;
use crate::wifi::WifiManager;

/// The step reached (see the module note), and when it began (uptime ms).
static STEP: AtomicU32 = AtomicU32::new(0);
static AT: AtomicU32 = AtomicU32::new(0);
/// Since when the WiFi has been up (uptime ms; 0: down).
static UP_SINCE: AtomicU32 = AtomicU32::new(0);

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

/// The heap the squeeze holds: 1 KB blocks, some given back.
struct Squeeze(Vec<Option<Vec<u8>>>);

static HELD: Mutex<Option<Squeeze>> = Mutex::new(None);
/// The most blocks held (the list's room is taken once, fallibly).
const BLOCKS: usize = 320;

/// Keep every free piece of the heap under `limit` with about `target`
/// free: 1 KB blocks taken while the largest free block is `limit` or more,
/// then every third of those just taken given back while less than
/// `target` is free (holes of a block between blocks still held). Run at
/// every tick of the step: what others give back meanwhile (the read-ahead,
/// the glow, a task's stack) is crumbled in turn. Blocks held, free heap,
/// largest block.
fn squeeze(limit: usize, target: u32) -> (usize, u32, usize) {
    let mut g = HELD.lock().unwrap();
    let held = g.get_or_insert_with(|| Squeeze(Vec::new()));
    if held.0.capacity() < BLOCKS && held.0.try_reserve_exact(BLOCKS).is_err() {
        return (0, crate::state::free_heap(), crate::state::largest_free_block());
    }
    let start = held.0.len();
    while crate::state::largest_free_block() >= limit && held.0.len() < BLOCKS {
        let mut b = Vec::new();
        if b.try_reserve_exact(1024).is_err() {
            break;
        }
        b.resize(1024, 0x5a);
        held.0.push(Some(b));
    }
    let mut k = start;
    while crate::state::free_heap() < target && k < held.0.len() {
        held.0[k] = None;
        k += 3;
    }
    let n = held.0.iter().filter(|b| b.is_some()).count();
    (n, crate::state::free_heap(), crate::state::largest_free_block())
}

fn release() {
    *HELD.lock().unwrap() = None;
}

fn pause(cmd: &mpsc::Sender<Command>) {
    let (tx, rx) = mpsc::channel();
    if cmd.send(Command::TogglePause(tx)).is_ok() {
        let _ = rx.recv_timeout(std::time::Duration::from_secs(5));
    }
}

fn disconnect() {
    // SAFETY: an argument-free driver call on a started driver.
    unsafe { esp_idf_sys::esp_wifi_disconnect() };
}

/// One look, from the network worker's loop (about once a second).
pub fn tick(wifi: &mut WifiManager, cmd: &mpsc::Sender<Command>) {
    let joined = wifi.state() == crate::wifi::WifiState::Connected && wifi.is_associated();
    if !joined {
        UP_SINCE.store(0, Relaxed);
    } else if UP_SINCE.load(Relaxed) == 0 {
        UP_SINCE.store(now_ms().max(1), Relaxed);
    }
    // Up for 10 s: the lane's own requests after a reconnect come first.
    let up = joined && now_ms().wrapping_sub(UP_SINCE.load(Relaxed)) > 10_000;
    match STEP.load(Relaxed) {
        0 if up && now_ms() > 40_000 => {
            log::info!("oom-test: drop 1: the association dropped (straight back expected)");
            next(1);
            disconnect();
        }
        1 if up => {
            log::info!("oom-test: drop 2: the learned channel spoiled (a scan expected)");
            next(2);
            wifi.spoil_learned();
            disconnect();
        }
        2 if up => {
            pause(cmd);
            let (n, free, largest) = squeeze(3 * 1024, 12 * 1024);
            log::info!("oom-test: squeeze: playback paused, {n} blocks held, heap {free} B free, largest block {largest} B");
            next(3);
        }
        3 if since() > 35_000 => {
            let (n, free, largest) = squeeze(2 * 1024, 11 * 1024);
            log::info!("oom-test: drop 3: squeezed harder ({n} blocks held, heap {free} B free, largest block {largest} B): held back expected");
            next(4);
            disconnect();
        }
        3 => {
            squeeze(3 * 1024, 12 * 1024);
        }
        4 if since() < 12_000 => {
            squeeze(2 * 1024, 11 * 1024);
        }
        4 => {
            release();
            log::info!(
                "oom-test: release: heap {} B free, largest block {} B",
                crate::state::free_heap(),
                crate::state::largest_free_block()
            );
            next(5);
        }
        5 if up => {
            pause(cmd);
            log::info!("oom-test: playback resumed");
            next(6);
        }
        6 if since() > 20_000 => {
            log::info!("oom-test: abort: 1048576 B asked of the heap, which has nothing like it");
            next(7);
            crate::logring::flush(std::time::Duration::from_secs(1));
            let v: Vec<u8> = Vec::with_capacity(std::hint::black_box(1 << 20));
            std::hint::black_box(&v);
        }
        _ => {}
    }
}
