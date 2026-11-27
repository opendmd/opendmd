//! Where in-order playlists left off ([`opendmd_core::resume`]): the last
//! few playlists' marks, kept in RTC slow memory (`.rtc_noinit`, beside
//! `restarts`' ring and the log ring; plain word loads and stores only), so
//! an engine rebuilt over a playlist — an upload, a reindex, a settings
//! change, a schedule rule or Follow RePlayOS switching back, any software
//! restart — continues it after the entry that last started instead of at
//! the first. A power-on clears them (RTC memory is garbage then); each
//! slot's check word drops anything else that is not a mark.
//!
//! The main task alone reads and writes them: [`building`] when an engine
//! is built, [`pending`] before its first tick (the main loop hands the
//! mark to `Engine::resume_at`), [`started`] at every playlist entry's
//! start (16 word stores, no card or flash write).

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

use opendmd_core::engine::OFF_PLAYLIST;
use opendmd_core::resume::{path_hash, Mark, Marks, SLOTS, WORDS};

#[link_section = ".rtc_noinit"]
static mut MARKS: [[u32; WORDS]; SLOTS] = [[0; WORDS]; SLOTS];

/// The key of the playlist the engine plays ([`path_hash`]; 0: none yet).
static LIST: AtomicU32 = AtomicU32::new(0);
/// An engine was built and has not looked for its place yet.
static PENDING: AtomicBool = AtomicBool::new(false);

fn word(i: usize) -> &'static AtomicU32 {
    debug_assert!(i < SLOTS * WORDS);
    // SAFETY: a word of `MARKS` (`i < SLOTS * WORDS`), only ever accessed
    // through this view (loads and stores).
    unsafe { AtomicU32::from_ptr((&raw mut MARKS).cast::<u32>().add(i)) }
}

fn load() -> Marks {
    let mut w = [[0; WORDS]; SLOTS];
    for (i, x) in w.iter_mut().flatten().enumerate() {
        *x = word(i).load(Relaxed);
    }
    Marks::decode(&w)
}

fn store(m: &Marks) {
    for (i, x) in m.encode().iter().flatten().enumerate() {
        word(i).store(*x, Relaxed);
    }
}

/// First thing at boot: a power-on starts with none; any other reset keeps
/// the valid marks (and writes them back clean).
pub fn boot(power_on: bool) {
    let m = if power_on { Marks::default() } else { load() };
    store(&m);
    if !m.is_empty() {
        log::info!("resume: in-order places kept for the last playlists");
    }
}

/// An engine is being built over the playlist keyed by `key_path` (its
/// manifest's path, or a Follow RePlayOS context's index): the starts from
/// here on are its, and the main loop looks for its place ([`pending`])
/// before the engine's first tick. (Not here: this is the bottom of a
/// rebuild, the main task's deepest path, and the look reads the card.)
pub fn building(key_path: &str) {
    LIST.store(path_hash(key_path), Relaxed);
    PENDING.store(true, Relaxed);
}

/// Once after [`building`]: where the new engine's playlist left off, if
/// anywhere.
#[inline(never)]
pub fn pending() -> Option<Mark> {
    if !PENDING.swap(false, Relaxed) {
        return None;
    }
    let list = LIST.load(Relaxed);
    // Slot by slot; the first valid one of the playlist wins, as in
    // `Marks::decode`.
    (0..SLOTS).find_map(|s| {
        let w: [u32; WORDS] = core::array::from_fn(|k| word(s * WORDS + k).load(Relaxed));
        Mark::decode(&w).filter(|m| m.list == list)
    })
}

/// A file started: a playlist entry (`index`, not [`OFF_PLAYLIST`]: a file
/// played on request, "previous" included, is not the playlist's place).
#[inline(never)]
pub fn started(index: usize, path: &str) {
    let list = LIST.load(Relaxed);
    if index == OFF_PLAYLIST || list == 0 {
        return;
    }
    let mut m = load();
    m.note(Mark {
        list,
        index: index as u32,
        entry: path_hash(path),
    });
    store(&m);
}
