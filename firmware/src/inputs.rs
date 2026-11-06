//! The input table's sources (`opendmd_core::inputs`, the built-in inputs
//! Formulas read: opendmd-notes docs/research/psi-inputs.md §2.1, §3.1, §3.2): who
//! writes which key, from where. The table is static in the core; the
//! playing Formula reads it once a frame (`fx_psi::fill_world`).
//!
//! - The sky (`daylight`, `sun.alt`, `moon.phase`, `season`, `yearday`):
//!   the network worker on CPU0 ([`tick`]), once a minute and as soon as
//!   SNTP first sets the clock; the `PUT /api/config` handler (CPU0) at
//!   once when the location or the time zone changes ([`set_place`],
//!   [`refresh_sky`]). Nothing before the clock is set. The sun's keys and
//!   `season` need the location; without one `set_sky` clears them.
//! - `night`, `bright`: the main task, where the schedule's decision is
//!   published (`Scheduling::publish`) and where the panel's brightness is
//!   applied (`Panel::set_brightness`); written only on a change.
//! - `replay.*`: the Follow RePlayOS poller (`follow::Poller::publish`),
//!   from what it already knows: no new traffic.
//!
//! None of this needs Formulas: a build without them fills a table no one
//! reads, for the price of a few small functions.

use opendmd_core::follow::{Game, State};
use opendmd_core::inputs::{self, id, Source};
use opendmd_core::sky::Place;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering::Relaxed};
use std::sync::Mutex;

/// ms since boot, the table's clock (`since`).
fn ms_since_boot() -> u64 {
    // SAFETY: `esp_timer_get_time` has no preconditions.
    (unsafe { esp_idf_sys::esp_timer_get_time() } / 1000) as u64
}

/// Install the table's clock. Called once at boot, before anything writes.
pub fn install_clock() {
    inputs::set_clock(ms_since_boot);
}

/// The location in force (`Settings::location`), for the sky.
static PLACE: Mutex<Option<Place>> = Mutex::new(None);

/// The Unix minute of the last sky write (0: none yet).
static SKY_MINUTE: AtomicU32 = AtomicU32::new(0);

/// A new location (or none): kept for the minute's writes, and the sky's
/// keys recomputed now. CPU0 (boot, the config handler).
#[optimize(size)] // (CPU0: no frame runs it)
#[inline(never)]
pub fn set_place(place: Option<Place>) {
    if let Ok(mut p) = PLACE.lock() {
        *p = place;
    }
    refresh_sky();
}

/// The sky's keys from the clock and the location now; nothing before the
/// clock is set. CPU0 only (the maths is a few hundred integer operations).
#[optimize(size)] // (CPU0: no frame runs it)
#[inline(never)]
pub fn refresh_sky() {
    let Some((unix, yday)) = crate::clock::utc_and_yday() else {
        return;
    };
    let place = PLACE.lock().map(|p| *p).unwrap_or(None);
    inputs::set_sky(unix, yday as u16, place);
    SKY_MINUTE.store((unix / 60) as u32, Relaxed);
}

/// The network worker's loop: the sky once a minute, and at once when the
/// clock is first set.
pub fn tick() {
    let Some((unix, _)) = crate::clock::utc_and_yday() else {
        return;
    };
    if (unix / 60) as u32 != SKY_MINUTE.load(Relaxed) {
        refresh_sky();
    }
}

/// What was last written to `night` and `bright` (−1: nothing yet), so the
/// main task writes only a change.
static NIGHT: AtomicI32 = AtomicI32::new(-1);
static BRIGHT: AtomicI32 = AtomicI32::new(-1);

/// `night`: 1 while the schedule's night window has the panel dimmed.
pub fn night(dimmed: bool) {
    let v = if dimmed { 1 << 16 } else { 0 };
    if NIGHT.swap(v, Relaxed) != v {
        inputs::set(id::NIGHT, v, Source::Panel);
    }
}

/// `bright`: the panel's brightness over its ceiling, 0..1 (Q16.16).
pub fn bright(applied: u8, ceiling: u8) {
    let v = ((i32::from(applied) << 16) / i32::from(ceiling.max(1))).clamp(0, 1 << 16);
    if BRIGHT.swap(v, Relaxed) != v {
        inputs::set(id::BRIGHT, v, Source::Panel);
    }
}

/// `replay.playing`, `replay.platform`, `replay.game` from the poller's
/// state: a game playing (1, its platform's place in
/// `manifest::PLATFORMS` from 1 or 0, its name's hash); the menu between
/// games (`Cooldown`, the game still followed: 0, the game's keys kept);
/// the menu or attract (0, the rest cleared); off or not reached (all
/// cleared). Writing an unchanged value is no change.
#[optimize(size)] // (CPU0: no frame runs it)
#[inline(never)]
pub fn replay(state: State, game: Option<&Game>) {
    let clear_game = || {
        inputs::clear(id::REPLAY_PLATFORM);
        inputs::clear(id::REPLAY_GAME);
    };
    match state {
        State::Off | State::Unreachable => {
            inputs::clear(id::REPLAY_PLAYING);
            clear_game();
        }
        State::Attract => {
            inputs::set(id::REPLAY_PLAYING, 0, Source::Replay);
            clear_game();
        }
        State::Game | State::Cooldown => {
            let playing = if state == State::Game { 1 << 16 } else { 0 };
            inputs::set(id::REPLAY_PLAYING, playing, Source::Replay);
            match game {
                Some(g) => {
                    let platform = g
                        .platform
                        .and_then(|p| opendmd_core::manifest::PLATFORMS.iter().position(|q| *q == p))
                        .map_or(0, |i| (i as i32 + 1) << 16);
                    inputs::set(id::REPLAY_PLATFORM, platform, Source::Replay);
                    inputs::set(id::REPLAY_GAME, inputs::game_hash(&g.name), Source::Replay);
                }
                None => clear_game(),
            }
        }
    }
}
