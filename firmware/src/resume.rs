//! Where in-order playlists left off ([`opendmd_core::resume`]): the last
//! few playlists' marks and the crash boots in a row, kept in RTC slow
//! memory (`.rtc_noinit`, beside `restarts`' ring and the log ring; plain
//! word loads and stores only), so an engine rebuilt over a playlist — an
//! upload, a reindex, a settings change, a schedule rule or Follow
//! RePlayOS switching back, any software restart — continues it after the
//! entry that last started in order instead of at the first. A power-on
//! clears them (RTC memory is garbage then); each slot's check word drops
//! anything else that is not a mark. A file that crashes the board before
//! its first frame is stepped over at the second crash boot in a row; at
//! the third the marks go (`Tracker::boot`).
//!
//! The main task alone uses them: [`building`] when an engine is built,
//! [`pending`] before its first tick (the main loop hands the mark to
//! `Engine::resume_at`), [`started`] at every playlist entry's start (16
//! word stores, no card or flash write). The logic is the core's
//! `Tracker`, host-tested; this is its memory.

use core::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{Mutex, MutexGuard};

use opendmd_core::resume::{list_key, path_hash, Booted, Mark, Step, Store, Tracker, STORE_WORDS};

#[link_section = ".rtc_noinit"]
static mut WORDS: [u32; STORE_WORDS] = [0; STORE_WORDS];

static TRACKER: Mutex<Tracker> = Mutex::new(Tracker::new());

/// The mounted card's identity (`opendmd_core::card::identity`), hashed
/// (0: none): part of every playlist's key, so another card is never
/// resumed at this one's place.
static CARD: AtomicU32 = AtomicU32::new(0);

/// The words in RTC memory.
struct Rtc;

impl Store for Rtc {
    fn get(&self, i: usize) -> u32 {
        word(i).load(Relaxed)
    }
    fn put(&mut self, i: usize, v: u32) {
        word(i).store(v, Relaxed)
    }
}

fn word(i: usize) -> &'static AtomicU32 {
    assert!(i < STORE_WORDS);
    // SAFETY: a word of `WORDS` (`i < STORE_WORDS`), only ever accessed
    // through this view (loads and stores).
    unsafe { AtomicU32::from_ptr((&raw mut WORDS).cast::<u32>().add(i)) }
}

fn tracker() -> MutexGuard<'static, Tracker> {
    TRACKER.lock().unwrap_or_else(|p| p.into_inner())
}

fn now_ms() -> u64 {
    crate::uclock::now_us() / 1000
}

/// First thing at boot: a power-on starts with none; a crash counts one
/// more crash boot in a row.
pub fn boot(power_on: bool, crashed: bool) {
    match tracker().boot(&mut Rtc, power_on, crashed) {
        Booted::Fresh => {}
        Booted::Kept { crashes: 0 | 1, .. } => log::info!("resume: in-order places kept for the last playlists"),
        Booted::Kept { crashes, .. } => log::warn!(
            "resume: in-order places kept for the last playlists; {crashes} crash boots in a row: the next steps one further on"
        ),
        Booted::Dropped { crashes } => {
            log::warn!("resume: {crashes} crash boots in a row: in-order places dropped, playlists start at the top")
        }
    }
}

/// The mounted card's identity (`sd.card_id`), or none.
pub fn set_card(id: Option<&str>) {
    CARD.store(id.map_or(0, path_hash), Relaxed);
}

/// An engine is being built over the playlist at `path` (its manifest, or
/// a Follow RePlayOS context's index) under `platforms`: the starts from
/// here on are its, and the main loop looks for its place ([`pending`])
/// before the engine's first tick. (Not here: this is the bottom of a
/// rebuild, the main task's deepest path, and the look reads the card.)
pub fn building(path: &str, platforms: &str) {
    let key = list_key(path, platforms, CARD.load(Relaxed));
    tracker().building(key, now_ms());
}

/// Once after [`building`]: where the new engine's playlist continues.
#[inline(never)]
pub fn pending() -> Option<(Mark, Step)> {
    tracker().pending(&Rtc)
}

/// A file started (`index`: its catalog index, or `OFF_PLAYLIST`);
/// marked only `in_order`.
#[inline(never)]
pub fn started(index: usize, path: &str, in_order: bool) {
    tracker().started(&mut Rtc, index, path, in_order, now_ms());
}
