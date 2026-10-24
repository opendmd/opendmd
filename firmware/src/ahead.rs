//! The next file, opened ahead: the main loop's half of
//! `opendmd_core::engine`'s `ahead_due` / `ahead_step` (see "The next file,
//! ahead" in CLAUDE.md).
//!
//! The engine says which step is due — the pick (scheduler + catalog
//! lookup), the open of the copy, its header — and this decides whether
//! there is time and heap for it now: it runs on the display task, between
//! a frame going up and the next one being due, only with
//! `config::AHEAD_*_SLACK` to spare, no transfer on the card and the cache
//! builder's task down, and only with `config::AHEAD_HEAP_RESERVE` left
//! after the step's bytes. A copy held ahead is let go (its pick kept) when
//! the heap runs under `AHEAD_YIELD_HEAP` or any card error is counted.
//!
//! Counters are statics touched only by the main task (`Relaxed`), reported
//! in the heartbeat's `ahead:` part.

use opendmd_core::engine::{AheadOutcome, AheadStage, Engine};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::time::{Duration, Instant};

use crate::config;
use crate::state::{free_heap, largest_free_block};

macro_rules! counters {
    ($($name:ident),* $(,)?) => { $(static $name: AtomicU32 = AtomicU32::new(0);)* };
}

counters!(
    // Steps skipped for the heap (the gate refused), since boot.
    HEAP_SKIPS,
    // Copies let go: low heap, a card error, a handler about to touch it.
    YIELDS,
    CARD_DROPS,
    ASKED_DROPS,
    // Steps run, and the worst time of each kind since boot (µs).
    STEPS,
    PICK_MAX,
    OPEN_MAX,
    PARSE_MAX,
    // Steps that ended after the next frame was due (should stay 0).
    OVERRUNS,
    // Heap a ready copy holds (open + header steps' drop in free heap):
    // samples, sum, min, max.
    HELD_N,
    HELD_SUM,
    HELD_MIN,
    HELD_MAX,
    // The open step's drop in free heap, kept for the copy being readied.
    OPEN_HELD,
    // Card errors seen at the last check.
    ERRORS,
);

/// What a step of `stage` takes from the heap, and the time it needs.
fn needs(stage: AheadStage) -> (usize, Duration) {
    match stage {
        AheadStage::Pick => (0, config::AHEAD_PICK_SLACK),
        AheadStage::Open => (config::AHEAD_OPEN_BYTES, config::AHEAD_OPEN_SLACK),
        AheadStage::Parse => (config::AHEAD_PARSE_BYTES, config::AHEAD_PARSE_SLACK),
    }
}

/// Before anything else this iteration: let a held copy go if a card error
/// was counted since the last look (a card pulled or failing: the handle is
/// no good, and the switch's own open then counts as it always did), or if
/// the heap is low.
pub fn guard(eng: &mut Engine) {
    let held = eng.ahead_path().is_some_and(|(_, open)| open);
    let errors = crate::sdhealth::errors_seen();
    if ERRORS.swap(errors, Relaxed) != errors && held {
        eng.release_ahead_file();
        CARD_DROPS.fetch_add(1, Relaxed);
        log::info!("ahead: card error seen, the next file's copy let go");
        return;
    }
    if held && (free_heap() as usize) < config::AHEAD_YIELD_HEAP {
        eng.release_ahead_file();
        YIELDS.fetch_add(1, Relaxed);
        log::info!(
            "ahead: heap low ({} B free), the next file's copy let go",
            free_heap()
        );
    }
}

/// A handler is about to replace or delete the copy held ahead
/// (`Command::ReleaseAhead`).
pub fn release(eng: &mut Engine) {
    if eng.ahead_path().is_some_and(|(_, open)| open) {
        eng.release_ahead_file();
        ASKED_DROPS.fetch_add(1, Relaxed);
    }
}

/// Between frames: take the steps that are due while the next frame
/// (`deadline`, on `epoch`'s clock) leaves time for each. True if any ran
/// (the caller then sleeps only what is left).
pub fn between_frames(eng: &mut Engine, deadline: Duration, epoch: &Instant) -> bool {
    // The A/B switch for measurements: a probe build with `ahead-off`
    // never steps ahead (everything else identical).
    if cfg!(feature = "ahead-off") || crate::cache_builder::task_up() || crate::cache_builder::transfer_in_flight() {
        return false;
    }
    let mut ran = false;
    loop {
        let now = epoch.elapsed();
        let Some(stage) = eng.ahead_due(now) else {
            break;
        };
        // An original opened ahead (through the FAT index) is held only with
        // the heap for its cap-12 decoder.
        let wide = stage == AheadStage::Parse && crate::sd::ahead_is_original();
        eng.set_ahead_wide(wide);
        let (mut bytes, slack) = needs(stage);
        if wide {
            bytes += config::AHEAD_ORIGINAL_BYTES;
        }
        if deadline.saturating_sub(now) < slack {
            break;
        }
        let before = free_heap() as usize;
        if bytes > 0
            && (before < bytes + config::AHEAD_HEAP_RESERVE || largest_free_block() < 16 * 1024)
        {
            HEAP_SKIPS.fetch_add(1, Relaxed);
            crate::probe::ahead_skip(stage, before);
            // Not again this turn: the heap rarely recovers in a dwell, and
            // a skip a frame would only fill the counter.
            eng.release_ahead_file();
            break;
        }
        let t = Instant::now();
        let outcome = eng.ahead_step();
        let us = t.elapsed().as_micros() as u32;
        let after = free_heap() as usize;
        ran = true;
        STEPS.fetch_add(1, Relaxed);
        match stage {
            AheadStage::Pick => PICK_MAX.fetch_max(us, Relaxed),
            AheadStage::Open => OPEN_MAX.fetch_max(us, Relaxed),
            AheadStage::Parse => PARSE_MAX.fetch_max(us, Relaxed),
        };
        let late = epoch.elapsed() > deadline;
        if late {
            OVERRUNS.fetch_add(1, Relaxed);
        }
        let took = before.saturating_sub(after) as u32;
        match outcome {
            Some(AheadOutcome::Opened) => OPEN_HELD.store(took, Relaxed),
            Some(AheadOutcome::Ready) => {
                let held = OPEN_HELD.swap(0, Relaxed) + took;
                HELD_N.fetch_add(1, Relaxed);
                HELD_SUM.fetch_add(held, Relaxed);
                HELD_MAX.fetch_max(held, Relaxed);
                if HELD_MIN.load(Relaxed) == 0 || held < HELD_MIN.load(Relaxed) {
                    HELD_MIN.store(held, Relaxed);
                }
            }
            _ => {}
        }
        crate::probe::ahead_step(stage, outcome, us, took, late, eng.ahead_path());
        if outcome.is_none() || late {
            break;
        }
    }
    ran
}

/// The heartbeat's `ahead:` part: the engine's counts (ready, used,
/// dropped, picks used) and this side's.
pub fn summary(eng: Option<&Engine>) -> String {
    let st = eng.map(Engine::ahead_stats).unwrap_or_default();
    let n = HELD_N.load(Relaxed);
    format!(
        ", ahead: {} ready, {} used, {} dropped (heap {}, card {}, asked {}), {} heap skips, picks used {}, \
         held avg {} B (min {} max {}), steps {} max pick/open/parse {}/{}/{} us, overruns {}",
        st.ready,
        st.used,
        st.dropped,
        YIELDS.load(Relaxed),
        CARD_DROPS.load(Relaxed),
        ASKED_DROPS.load(Relaxed),
        HEAP_SKIPS.load(Relaxed),
        st.picks_used,
        HELD_SUM.load(Relaxed).checked_div(n).unwrap_or(0),
        HELD_MIN.load(Relaxed),
        HELD_MAX.load(Relaxed),
        STEPS.load(Relaxed),
        PICK_MAX.load(Relaxed),
        OPEN_MAX.load(Relaxed),
        PARSE_MAX.load(Relaxed),
        OVERRUNS.load(Relaxed),
    )
}
