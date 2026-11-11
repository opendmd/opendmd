//! `@formula?src=<formula>`: a Psi formula ([`opendmd_psi`]) as a live
//! effect. Behind the `psi` feature (spike; nothing user-facing yet).
//!
//! `@formula?src=<url-encoded source>[&a=0.5&b=..&c=..][&fps=50|100|25]`:
//! the formula compiled when the path is opened (an error makes the path
//! fail to open, as an unknown effect does); `a b c` are its sliders
//! (decimal, default 0); the frame rate is `fps=`, else the formula's
//! `fps N` line, else 50 (a look: `fps=`, else 50). A loop is the formula's `loop
//! N` seconds; `t` is the frame's time into it, so the loop is seamless
//! when everything the formula moves completes whole cycles in it. A
//! formula that never repeats (`loop forever`) is played in passes of
//! [`PASS_MS`] (the engine's loop: the dwell is looked at after each), its
//! `t` the time since it opened, going on across passes, and its `seed` a
//! new one at each open.
//!
//! **Memory.** The program (2,876 B) and the VM's working memory
//! ([`opendmd_psi::Scratch`], 8,204 B at 128 lanes) are one allocation made
//! when the path is opened, zeroed and filled in place (never built on the
//! main task's stack); a formula that draws (`draw { }`) also has its
//! canvas ([`opendmd_psi::Ink`], 4 KB), a second allocation made then. No
//! heap per frame, and [`Fx`](crate::fx::Fx) itself holds only the pointer. The sine and the palettes are lent from
//! [`crate::fx`] (`SIN`, `PALETTES`), so the device carries one copy.
//!
//! **Cost.** A formula whose estimated render time
//! ([`psi::Program::esp32_us`]) is over [`MAX_EST_US`] does not open. One
//! that opens is held to the effect budget by its [`Governor`] once a
//! [`set_clock`] is installed (the firmware): it shows every 2nd, 4th or
//! 8th frame of its own sequence, never a different picture, so a frame on
//! the panel is still the browser's frame for the same `t`.

use crate::canvas::{Canvas, Rgb, PIXELS};
use crate::fx::{Effect, PALETTES, SIN};
pub use alloc_box::PsiBox;
use opendmd_psi as psi;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;

/// The lane width: one panel row.
pub const LANES: usize = psi::LANES;

/// The compiled formula, its working memory and its options.
pub struct PsiFx {
    /// Its program, with the store its formula needs (none: a zeroed
    /// `PsiFx`'s, before its open).
    prog: ProgBox,
    scratch: psi::Scratch<LANES>,
    /// The draw stage's canvas (4 KB, 8 KB when it paints), held only
    /// while the formula draws (`draw { }`); `None` is all-zero, so the
    /// zeroed box is valid.
    ink: Option<Box<psi::Ink>>,
    /// When the canvas was last asked for and refused (the installed
    /// clock's µs; 0: not refused): asked again a second later (a clone
    /// made without one; see `frame`).
    ink_tried: u32,
    /// The still cache ([`psi::Still`]: 16 KB a value the formula works out
    /// from the pixel's position alone), taken in `frame` when the
    /// [`set_still_gate`] gate allows and let go when it says to: the same
    /// pictures either way, only faster. All-zero is none held.
    still: psi::Still,
    /// When the still cache was last refused (as `ink_tried`).
    still_tried: u32,
    /// The still cache has been held since the formula opened: until it
    /// has, it is asked for every frame (after a GIF -> Formula switch the
    /// transition's buffers hold the heap for a moment), then once a second
    /// after a refusal.
    still_once: bool,
    /// A `rate` formula's slow layers' picture ([`psi::Held`], ~12.6 KB),
    /// taken in `frame` when the still cache's gate allows (and when it was
    /// last refused, as `ink_tried`): without it each frame draws the slow
    /// layers from nothing (the same bytes, slower). `None` is all-zero.
    held: Option<Box<psi::Held>>,
    held_tried: u32,
    /// It reads the wall clock (`hour`, `minute`, `second`): only then is
    /// the clock read each frame (a `gettimeofday` and a `localtime_r` on
    /// the device).
    clock: bool,
    /// Frames in a row whose changes marked half the panel or more;
    /// from [`FULL_RUN`] on the frames are not compared (`mark_all`) but
    /// every [`RECHECK`]th, counted in `uncompared` (all-zero: compare).
    full: u8,
    uncompared: u8,
    delay_ms: u8,
    inputs: psi::Inputs,
    gov: Governor,
    /// The formula's own time since it opened (the frame times it
    /// returned): a `loop forever` formula's `t`, which a new pass (the
    /// engine's rewind) leaves running and a live edit keeps.
    played_ms: u64,
    /// The µs of `played_ms`'s next millisecond (a raced frame is 7,98x µs).
    played_us: u32,
    /// Frames drawn since the open: the first is never raced (the canvas
    /// may not be the panel's picture yet).
    drawn: u32,
    /// This frame is raced ([`set_beam`]): decided in `frame_inner`.
    racing: bool,
    /// The JIT's kernel for the pixel stage (the ESP32 with `psi-jit`).
    #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
    jit: jit::Jit,
}

/// A pass of a formula that never repeats (`loop forever`): what the engine
/// counts as its loop, so the dwell is looked at once a second (as a
/// clock's); its `t` goes on across them.
pub const PASS_MS: u32 = 1000;

/// A `seed` for a formula that never repeats, at each open: a hash of the
/// opens so far and the clock (the firmware's µs since boot), 0..1.
fn next_seed() -> i32 {
    static OPENS: AtomicU32 = AtomicU32::new(0);
    let n = OPENS.fetch_add(1, Ordering::Relaxed);
    let salt = clock().map_or(0, |c| c());
    (psi::fixed::mix32(n.wrapping_mul(0x9e37_79b9) ^ salt) >> 16) as i32
}

/// How long a formula that draws waits to ask for its canvas again after
/// the heap refused it (the look's `refused` convention).
const CANVAS_RETRY_US: u32 = 1_000_000;

/// May a formula's still cache take `bytes` more of the heap now (any
/// `bytes` above 0), or keep what it holds (0)? The firmware's keeps room
/// for a transition and gives way to a look ([`set_still_gate`]).
pub type StillGate = fn(usize) -> bool;

#[cfg(not(test))]
static STILL_GATE: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
thread_local! {
    static STILL_GATE: AtomicUsize = const { AtomicUsize::new(0) };
}

/// The heap the playing formula's still cache holds (B), for status.
#[cfg(not(test))]
static STILL_HELD: AtomicU32 = AtomicU32::new(0);
// Tests run in parallel: each thread its own.
#[cfg(test)]
thread_local! {
    static STILL_HELD_T: AtomicU32 = const { AtomicU32::new(0) };
}

fn set_still_held(b: u32) {
    #[cfg(not(test))]
    STILL_HELD.store(b, Ordering::Relaxed);
    #[cfg(test)]
    STILL_HELD_T.with(|h| h.store(b, Ordering::Relaxed));
}

/// Install the gate a formula's still cache asks before it takes the heap
/// and every frame while it holds it (`None`: no gate — taken whenever
/// the allocation succeeds: the simulator, the browser, the tests).
pub fn set_still_gate(gate: Option<StillGate>) {
    let a = gate.map_or(0, |f| f as usize);
    #[cfg(not(test))]
    STILL_GATE.store(a, Ordering::Relaxed);
    #[cfg(test)]
    STILL_GATE.with(|g| g.store(a, Ordering::Relaxed));
}

/// May a formula's JIT kernel be built now, its lowering and code taking
/// about `bytes` of the heap for a moment ([`jit_build_bytes`])? The
/// firmware's says whether the heap has that and its floor besides; none
/// installed (the host): yes. Refused, the formula stays on the VM and asks
/// again a second later: the build's working memory is small allocations
/// that would abort the player, not refuse, on a short heap.
pub type JitGate = fn(usize) -> bool;

static JIT_GATE: AtomicUsize = AtomicUsize::new(0);

/// Install the [`JitGate`].
pub fn set_jit_gate(gate: Option<JitGate>) {
    JIT_GATE.store(gate.map_or(0, |f| f as usize), Ordering::Relaxed);
}

#[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
fn jit_build_allowed(bytes: usize) -> bool {
    match JIT_GATE.load(Ordering::Relaxed) {
        0 => true,
        // SAFETY: stored by `set_jit_gate` from a `JitGate` (a fn pointer,
        // never null, the size of a `usize`).
        a => {
            let gate = unsafe { std::mem::transmute::<usize, JitGate>(a) };
            gate(bytes)
        }
    }
}

/// What a kernel's build takes of the heap at its peak, about: the
/// lowering's tables (measured on the host at ~20 B a step with 64-bit
/// pointers, Endless road V2's 751 steps 14.7 KB; less on the ESP32) and
/// the code (a few KB).
pub fn jit_build_bytes(prog: &psi::Prog) -> usize {
    prog.len() * 16 + 6 * 1024
}

fn still_allowed(bytes: usize) -> bool {
    #[cfg(not(test))]
    let a = STILL_GATE.load(Ordering::Relaxed);
    #[cfg(test)]
    let a = STILL_GATE.with(|g| g.load(Ordering::Relaxed));
    match a {
        0 => true,
        // SAFETY: stored by `set_still_gate` from a `StillGate` (a fn
        // pointer, never null, the size of a `usize`).
        a => {
            let gate = unsafe { std::mem::transmute::<usize, StillGate>(a) };
            gate(bytes)
        }
    }
}

/// The heap the playing formula's still cache holds now, bytes (0: none,
/// or the formula has no values for one).
pub fn still_held() -> u32 {
    #[cfg(not(test))]
    return STILL_HELD.load(Ordering::Relaxed);
    #[cfg(test)]
    STILL_HELD_T.with(|h| h.load(Ordering::Relaxed))
}

/// A formula estimated to render slower than this (µs a frame on the
/// ESP32) is refused: ~3.5 × the ring clock sketch, ~5 × the heaviest
/// example. The [`Governor`]'s slowest step (8 frames held, 160 ms at
/// 50 fps) still keeps it under 40 % of the main core.
pub const MAX_EST_US: u32 = psi::PANEL_MAX_US;

/// How another program takes the place of the one a Formula plays
/// ([`PsiFx::retune_from`], [`FormulaLook::retune`]): what its
/// [`Governor`] keeps ([`Governor::retune`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retune {
    /// A live edit of the Formula on screen (the editor's changes,
    /// `live=1` / `"live":true`): the level goes on, slower where the new
    /// estimate says so.
    Edit,
    /// A different Formula played over the one on screen (`/api/play`,
    /// "Show on panel"): it starts from its own estimate's level, slower
    /// or faster, as it would after a GIF.
    Play,
}

/// Keeps a formula inside the effect budget (CLAUDE.md, "Live effects":
/// ≤ 5 ms a frame at 100 fps, else ≤ 40 % of the frame — 8 ms at 50 fps)
/// by holding each frame for 1, 2, 4 or 8 of its frame times. Fed each
/// frame's measured render time; starts from the compiler's estimate, so a
/// heavy formula is slowed before its first frame can be late. Slows down
/// at once (the running mean over budget, or one frame longer than its
/// own slot); speeds up after [`Governor::CALM`] frames under 31/32 of the
/// faster step's budget. (Renders run ~3% slower while a transition or
/// heavy traffic shares the flash cache: on a test board a 7.7 ms formula
/// read 7.98 ms through a dissolve; a ¾ margin left one that crossed 8 ms
/// once at 25 fps for good.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Governor {
    /// Frames held: 1 << `level`.
    level: u8,
    calm: u16,
    /// Running mean of the render time, µs (1/8 per frame).
    avg_us: u32,
    /// Frames left out of the mean: the first after an open or a live
    /// edit runs from a cold flash cache (the compiler just ran).
    skip: u8,
    /// Racing the beam: late raced frames in the current window of
    /// [`Governor::LATE_WINDOW`] (and the frames seen in it); over
    /// [`Governor::LATE_MAX`] there, every 2nd refresh at least (`floor`)
    /// for [`Governor::LATE_HOLD`] frames, then tried again.
    late: u16,
    seen: u16,
    floor: u8,
    floor_left: u16,
    /// Raced frames after an open or a retune whose lateness is not
    /// counted ([`Governor::LATE_GRACE`]): they run from cold caches.
    grace: u8,
}

impl Governor {
    pub const MAX_LEVEL: u8 = 3;
    pub const CALM: u16 = 100;
    pub const SKIP: u8 = 2;

    /// A live edit: the next frames run cold, leave them out.
    pub fn edited(&mut self) {
        self.skip = Self::SKIP;
    }

    /// Another program in place: `start`, its own governor
    /// ([`Governor::new`] from its estimate), and `how` it came.
    ///
    /// - [`Retune::Edit`], a live edit of the Formula on screen: the level
    ///   goes on (the frames measured are evidence for the edited program
    ///   too), but where `start` is slower it starts there (a heavier
    ///   formula starts at least as slow as its estimate says); it speeds up
    ///   after [`Governor::CALM`] calm frames as ever.
    /// - [`Retune::Play`], a different Formula played over the one on
    ///   screen: `start` entire, slower or faster. The old level described
    ///   another program's frames: kept, a light Formula (~1-2 ms a frame)
    ///   played over a `smooth` one with 9 noises (~35.5 ms, level 3)
    ///   kept its level, raced at every 8th refresh, then one level faster per
    ///   [`Governor::CALM`] frames: seconds at a fraction of its rate, often
    ///   its whole turn (1.1.0-beta.2), where after a GIF it opens at every
    ///   refresh.
    ///
    /// Either way the mean is the new program's estimate (the old one's
    /// mean was another program's frames), and the raced frames' record
    /// (late frames and the every-2nd-refresh hold they bring) starts
    /// again: it was the old program's against the scan. Kept, a hold one
    /// heavy Formula earned held every Formula played over it to half its
    /// frame rate for ~4,096 frames (on a test board after Endless road:
    /// Scanlines at 1.6 ms a frame on 32 ms frames).
    pub fn retune(&mut self, start: Governor, how: Retune) {
        if how == Retune::Play || start.level > self.level {
            *self = start;
        }
        self.avg_us = start.avg_us;
        (self.late, self.seen, self.floor, self.floor_left) = (0, 0, 0, 0);
        self.grace = Self::LATE_GRACE;
        self.edited();
    }

    /// The budget at the slowest step on a `grid` ms grid: the heaviest
    /// formula the governor can hold.
    pub fn slowest_budget_us(grid: u32) -> u32 {
        Self::budget_us(grid << Self::MAX_LEVEL)
    }

    /// The render budget for a frame of `ms`.
    pub fn budget_us(ms: u32) -> u32 {
        if ms <= 10 {
            5_000
        } else {
            ms * 400
        }
    }

    /// Starting from an estimate of `est_us` a frame on a `grid` ms grid.
    pub fn new(est_us: u32, grid: u32) -> Governor {
        let mut g = Governor {
            level: 0,
            calm: 0,
            avg_us: est_us,
            skip: Self::SKIP,
            grace: Self::LATE_GRACE,
            ..Governor::default()
        };
        while g.level < Self::MAX_LEVEL && est_us > Self::budget_us(grid << g.level) {
            g.level += 1;
        }
        g
    }

    /// The frame time now, on a `grid` ms grid.
    pub fn delay_ms(&self, grid: u32) -> u32 {
        grid << self.level
    }

    pub fn avg_us(&self) -> u32 {
        self.avg_us
    }

    /// A render that does not steer it (a formula look's frame drawn for
    /// the picture under it): the mean only.
    pub fn note(&mut self, us: u32) {
        self.avg_us = self.avg_us - self.avg_us / 8 + us / 8;
    }

    /// One frame took `us` to render; `true` if the frame time changed.
    pub fn observe(&mut self, us: u32, grid: u32) -> bool {
        let ms = self.delay_ms(grid);
        if self.skip > 0 && us < ms * 1000 {
            self.skip -= 1;
            return false;
        }
        self.avg_us = self.avg_us - self.avg_us / 8 + us / 8;
        if self.level < Self::MAX_LEVEL && (self.avg_us > Self::budget_us(ms) || us >= ms * 1000) {
            self.level += 1;
            self.calm = 0;
            return true;
        }
        if self.level > 0 && self.avg_us < Self::budget_us(ms >> 1) / 32 * 31 {
            self.calm += 1;
            if self.calm >= Self::CALM {
                self.level -= 1;
                self.calm = 0;
                return true;
            }
        } else {
            self.calm = 0;
        }
        false
    }
}

/// What a raced frame may take (render and writes, µs) at a frame time
/// of `frame_us` on a refresh of `cycle_us`: a refresh less
/// [`BEAM_SLACK_US`] when every refresh is a frame (~6.5 ms at 125 Hz), else
/// half the frame (8 ms at 62.6 Hz, as 20 ms frames had 8).
pub fn beam_budget_us(frame_us: u32, cycle_us: u32) -> u32 {
    if frame_us <= cycle_us {
        cycle_us.saturating_sub(BEAM_SLACK_US)
    } else {
        frame_us / 2
    }
}

/// A raced frame's time left out of a refresh: the loop's own work around
/// it (the wake, the publishing) and the margin of its window.
pub const BEAM_SLACK_US: u32 = 1_500;

impl Governor {
    /// [`Governor::observe`] for a raced frame: `unit_us` the frame time
    /// its `fps` gets ([`crate::scan::beam_frames`] refreshes), each level
    /// doubling it; the budget [`beam_budget_us`].
    pub fn observe_beam(&mut self, us: u32, unit_us: u32, cycle_us: u32) -> bool {
        let frame = unit_us << self.level;
        if self.skip > 0 && us < frame {
            self.skip -= 1;
            return false;
        }
        self.avg_us = self.avg_us - self.avg_us / 8 + us / 8;
        if self.level < Self::MAX_LEVEL && (self.avg_us > beam_budget_us(frame, cycle_us) || us >= frame) {
            self.level += 1;
            self.calm = 0;
            return true;
        }
        if self.level > 0 && self.avg_us < beam_budget_us(frame >> 1, cycle_us) / 32 * 31 {
            self.calm += 1;
            if self.calm >= Self::CALM {
                self.level -= 1;
                self.calm = 0;
                return true;
            }
        } else {
            self.calm = 0;
        }
        false
    }

    /// Raced frames after an open or a retune left out of the late count:
    /// the first frames of a Formula run from cold caches (its code and
    /// data just loaded, the flash cache holding the last one's), and two
    /// late ones among them held a Formula that keeps every refresh after
    /// that to every 2nd for [`Governor::LATE_HOLD`] frames: Starfield,
    /// opened after a reboot or over Glitter, at 16 ms frames for a minute
    /// and over Old photo at 8 (psi-perf, 2026-11-02).
    pub const LATE_GRACE: u8 = 16;

    /// The frames (refreshes) a raced picture is held for.
    pub fn beam_frames(&self, unit: u32) -> u32 {
        unit << self.level.max(self.floor)
    }

    /// Raced frames in a window (~8 s at 125 Hz).
    pub const LATE_WINDOW: u16 = 1024;
    /// More late frames than this in a window (over 0.1 %) …
    pub const LATE_MAX: u16 = 1;
    /// … hold every 2nd refresh for this many frames (~65 s), then try
    /// every refresh again. (Held for two, a start too late for its window
    /// waits for the next refresh instead: none are late.)
    pub const LATE_HOLD: u16 = 4096;

    /// A late frame counts towards the hold only while the formula's own
    /// mean render (render and writes, waits left out) is over this share
    /// of its frame's budget, %. Under it, a late frame is a stall from
    /// outside the formula: the loop's work before the tick, the wake, a
    /// flash write on CPU0 (which stops the flash cache on both cores),
    /// the web app's polls. A test board (psi-perf, 2026-11-02): Starfield at
    /// ~3,300 µs of a 6,468 µs budget had 1-2 late frames in ~200 while
    /// being measured over HTTP, ~1 in 1,000 in all (7 in 6,269 raced
    /// frames since boot), and was held to 50 fps for a minute or not by
    /// luck. The hold stays for a formula near the edge, where a late
    /// frame says its own time does not fit (console-lessons §8.11: it
    /// was the fallback for Starfield's narrow window, which `late_share`
    /// and `retarget` absorbed since).
    pub const LATE_NEAR_PCT: u32 = 75;

    /// A raced frame was (`late`) or wasn't late, `budget_us` its frame's
    /// render budget ([`beam_budget_us`]); `true` when that just moved the
    /// formula to every 2nd refresh.
    pub fn note_late(&mut self, late: bool, budget_us: u32) -> bool {
        let late = late && u64::from(self.avg_us) * 100 > u64::from(budget_us) * u64::from(Self::LATE_NEAR_PCT);
        if self.grace > 0 {
            self.grace -= 1;
            return false;
        }
        if self.floor > 0 {
            self.floor_left = self.floor_left.saturating_sub(1);
            if self.floor_left == 0 {
                self.floor = 0;
            }
            return false;
        }
        self.late += u16::from(late);
        self.seen += 1;
        if self.late > Self::LATE_MAX {
            (self.late, self.seen) = (0, 0);
            self.floor = 1;
            self.floor_left = Self::LATE_HOLD;
            return self.level == 0;
        }
        if self.seen >= Self::LATE_WINDOW {
            (self.late, self.seen) = (0, 0);
        }
        false
    }
}

/// A clock in µs (wrapping) the governor times frames with.
pub type Clock = fn() -> u32;

/// The installed [`Clock`] as an address (0: none, no governing — the
/// simulator, the browser and the tests play every frame). Per thread in
/// tests, so one test's fake clock does not govern another's formula.
#[cfg(not(test))]
static CLOCK: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
thread_local! {
    static CLOCK: AtomicUsize = const { AtomicUsize::new(0) };
}
/// The formula on screen: its mean render time (µs), and its frame time
/// and the one it asked for (ms, high and low half).
#[cfg(not(test))]
static STATS: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
// (Per thread in tests, as the clock: a raced test's frames are governed
// beside the governor's tests.)
#[cfg(test)]
thread_local! {
    static STATS_T: [AtomicU32; 3] = const { [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)] };
}

fn stat(i: usize) -> u32 {
    #[cfg(not(test))]
    return STATS[i].load(Ordering::Relaxed);
    #[cfg(test)]
    STATS_T.with(|s| s[i].load(Ordering::Relaxed))
}

fn set_stat(i: usize, v: u32) {
    #[cfg(not(test))]
    STATS[i].store(v, Ordering::Relaxed);
    #[cfg(test)]
    STATS_T.with(|s| s[i].store(v, Ordering::Relaxed));
}

// Tests: refuse the next N loads on this thread with [`psi::NO_MEMORY`]
// (the player's heap, without a heap to make short).
#[cfg(test)]
thread_local! {
    static REFUSE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Tests: the next `n` loads on this thread are refused for the heap.
#[cfg(test)]
pub(crate) fn refuse_loads(n: u32) {
    REFUSE.with(|r| r.set(n));
}

/// What a Formula on the card says when its compiled file is not there
/// (the display prepares a card's Formulas from their source when it
/// mounts the card: this one is missing, or did not compile).
pub const NOT_PREPARED: &str =
    "this Formula is not on the card, or could not be prepared for the panel";

/// What a Formula that reads the clock (`hour`, `minute`, `second`) says
/// when the display has no time and never will this boot ([`set_no_clock`]).
pub const NEEDS_TIME: &str =
    "this Formula shows the time, and the display has no clock with WiFi off";

/// The display has no clock this boot (WiFi off: no time sync), so a
/// Formula that reads it does not open ([`NEEDS_TIME`]): a playlist moves
/// past it, a look or start-up show of it falls back as for any Formula
/// that does not open. Per thread in tests.
#[cfg(not(test))]
static NO_CLOCK: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
thread_local! {
    static NO_CLOCK: AtomicBool = const { AtomicBool::new(false) };
}

/// Whether the display has no clock this boot (the firmware sets it when
/// the network is off): Formulas that read the time are refused.
pub fn set_no_clock(none: bool) {
    #[cfg(not(test))]
    NO_CLOCK.store(none, Ordering::Relaxed);
    #[cfg(test)]
    NO_CLOCK.with(|c| c.store(none, Ordering::Relaxed));
}

fn no_clock() -> bool {
    #[cfg(not(test))]
    return NO_CLOCK.load(Ordering::Relaxed);
    #[cfg(test)]
    NO_CLOCK.with(|c| c.load(Ordering::Relaxed))
}

/// [`NEEDS_TIME`] for a program that reads the clock while there is none.
fn clock_check(prog: &psi::Prog) -> Result<(), psi::Error> {
    if no_clock() && reads_clock(prog) {
        return Err(psi::Error {
            start: 0,
            end: 0,
            msg: NEEDS_TIME.to_string(),
        });
    }
    Ok(())
}

/// A formula's program on the heap, with the store its formula needs
/// ([`psi::Prog`]: its tables and sprites, a power of two of numbers, 4 for
/// a formula with none), or none: an all-zero one (a zeroed [`PsiFx`]'s,
/// before its open gives it one; nothing reads it then). (No static empty
/// program to read instead: it would be 6.4 KB of the app's flash.)
pub struct ProgBox(Option<Box<psi::Prog>>);

impl ProgBox {
    /// A copy in a box of its own, or `None` when the heap has none.
    fn try_clone(&self) -> Option<ProgBox> {
        match &self.0 {
            Some(p) => Some(ProgBox(Some(p.try_boxed_copy()?))),
            None => Some(ProgBox(None)),
        }
    }

    /// The heap it holds (B).
    pub fn bytes(&self) -> usize {
        self.0.as_deref().map_or(0, std::mem::size_of_val)
    }
}

impl std::ops::Deref for ProgBox {
    type Target = psi::Prog;
    fn deref(&self) -> &psi::Prog {
        self.0.as_deref().expect("a formula's program, given at its open")
    }
}

impl std::fmt::Debug for ProgBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(f)
    }
}

impl PartialEq for ProgBox {
    fn eq(&self, o: &ProgBox) -> bool {
        **self == **o
    }
}

/// Load the compiled Formula `psb` (program `which`) into a program of the
/// store it needs: checked and finished for the player
/// ([`psi::psb::load_into`]).
fn load_boxed(psb: &[u8], which: psi::psb::Which) -> Result<ProgBox, psi::Error> {
    #[cfg(test)]
    if REFUSE.with(|r| r.replace(r.get().saturating_sub(1)) > 0) {
        return Err(no_memory());
    }
    let err = |m: &str| psi::Error {
        start: 0,
        end: 0,
        msg: m.to_string(),
    };
    let words = psi::psb::store_needed(psb, which).map_err(err)?;
    let mut p = hinted(ARENA_PROG, || psi::Prog::try_boxed(words)).ok_or_else(no_memory)?;
    psi::psb::load_into(psb, which, &mut p).map_err(err)?;
    Ok(ProgBox(Some(p)))
}

/// Install the clock that turns the governor on (the firmware's
/// `esp_timer`).
pub fn set_clock(clock: Option<Clock>) {
    let a = clock.map_or(0, |f| f as usize);
    #[cfg(not(test))]
    CLOCK.store(a, Ordering::Relaxed);
    #[cfg(test)]
    CLOCK.with(|c| c.store(a, Ordering::Relaxed));
}

fn clock() -> Option<Clock> {
    #[cfg(not(test))]
    let a = CLOCK.load(Ordering::Relaxed);
    #[cfg(test)]
    let a = CLOCK.with(|c| c.load(Ordering::Relaxed));
    match a {
        0 => None,
        // SAFETY: stored by `set_clock` from a `Clock` (a fn pointer, never
        // null, the size of a `usize`).
        a => Some(unsafe { std::mem::transmute::<usize, Clock>(a) }),
    }
}

/// Racing the beam (console-lessons §2.6, §8): the way Formulas play on the
/// panel; the player's side, installed by the firmware. A Formula's frame drawn in
/// the panel's pair order, each pair written to the panel as it is drawn
/// (`pair`), behind the scan, instead of a whole frame then a paced blit.
#[derive(Clone, Copy, Debug)]
pub struct BeamHooks {
    /// A frame is about to be raced: the refresh (µs), or `None` when it
    /// can't be raced now (the scan unreadable).
    pub begin: fn() -> Option<u32>,
    /// Its rows start (after the frame's steps and draw stage:
    /// [`psi::BEAM_START`]), the frame expected to take `est_us` (render
    /// and writes) and held for `frames` refreshes: plans the race from
    /// here (and may wait for its window; with two refreshes or more, a
    /// start too late for its window waits for the next refresh).
    pub start: fn(est_us: u32, frames: u32),
    /// Pair `p` (rows `p`, `p + 16`) is drawn in `bytes` (the canvas, packed
    /// RGB888) and `dirty` marks what of it to write.
    pub pair: fn(p: usize, bytes: &[u8], dirty: &crate::canvas::DirtyMap),
    /// The frame is written: `(err, waited, late)` — how late (+) or early
    /// (−) its start was for its window (µs; the next delay corrects it),
    /// the µs it waited for the scan (left out of the governor's time), and
    /// whether a pair was written past its window.
    pub end: fn() -> (i32, u32, bool),
}

static BEAM: std::sync::Mutex<Option<BeamHooks>> = std::sync::Mutex::new(None);
#[cfg(not(test))]
static BEAM_ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
thread_local! {
    static BEAM_ARMED: std::sync::atomic::AtomicBool = const { std::sync::atomic::AtomicBool::new(false) };
}
/// The raced frame time (µs) of the formula on screen, 0 when not racing:
/// `/api/status` `now_playing.formula.beam_us`.
static BEAM_US: AtomicU32 = AtomicU32::new(0);

/// Install (or remove) the beam's hooks: the firmware, when the panel's
/// scan can be read.
pub fn set_beam(hooks: Option<BeamHooks>) {
    *BEAM.lock().unwrap() = hooks;
    if hooks.is_none() {
        BEAM_US.store(0, Ordering::Relaxed);
    }
}

/// The player's say for the next tick: a Formula's frame may be raced
/// (no glow, no formula look, no transition running).
pub fn arm_beam(on: bool) {
    #[cfg(not(test))]
    BEAM_ARMED.store(on, Ordering::Relaxed);
    #[cfg(test)]
    BEAM_ARMED.with(|a| a.store(on, Ordering::Relaxed));
}

fn beam_hooks() -> Option<BeamHooks> {
    #[cfg(not(test))]
    let armed = BEAM_ARMED.load(Ordering::Relaxed);
    #[cfg(test)]
    let armed = BEAM_ARMED.with(|a| a.load(Ordering::Relaxed));
    if !armed {
        return None;
    }
    *BEAM.lock().unwrap()
}

/// The raced frame time now (µs; 0: not racing).
pub fn beam_us() -> u32 {
    BEAM_US.load(Ordering::Relaxed)
}

/// Whether `prog` reads a built-in input (`daylight`, `since(night)` …).
fn reads_world(prog: &psi::Prog) -> bool {
    prog.inputs() & 128 != 0
}

/// The built-in inputs from the panel's table ([`crate::inputs`]): each
/// key's value, the one before and the seconds since it changed; a key
/// with no value its default (what previews show).
pub fn fill_world(world: &mut psi::World) {
    let (t, now) = crate::inputs::snapshot();
    for k in 1..psi::world::MAX_KEYS.min(crate::inputs::KEYS) {
        let r = t.read(k as u8, now);
        let def = psi::world::key_by_id(k as u8).map_or(0, |key| key.default);
        world.value[k] = r.value.unwrap_or(def);
        world.was[k] = r.was.unwrap_or(def);
        world.since[k] = r.since;
    }
}

/// Whether `prog` reads the wall clock (`hour`, `minute`, `second`).
fn reads_clock(prog: &psi::Prog) -> bool {
    prog.inputs() & 16 != 0
}

/// The playing formula's renders since boot: µs in all (wrapping) and how
/// many — what a measurement over a stretch of time averages
/// (`/api/status` `formula.render_us_total`, `renders`; the governor's
/// `frame_us` is a running mean of the last few frames).
static RENDER_US: AtomicU32 = AtomicU32::new(0);
static RENDERS: AtomicU32 = AtomicU32::new(0);

fn note_render(us: u32) {
    RENDER_US.fetch_add(us, Ordering::Relaxed);
    RENDERS.fetch_add(1, Ordering::Relaxed);
}

/// ([`RENDER_US`], [`RENDERS`]).
pub fn render_totals() -> (u32, u32) {
    (RENDER_US.load(Ordering::Relaxed), RENDERS.load(Ordering::Relaxed))
}

/// The last formula drawn: (mean render µs, frame time ms, asked ms, the
/// last compile's µs) — for `/api/status`. Zeros before any.
pub fn stats() -> (u32, u32, u32, u32) {
    let d = stat(1);
    (
        stat(0),
        d >> 16,
        d & 0xffff,
        stat(2),
    )
}

/// The tables Psi reads, lent from `fx` (the sine, the palettes) and
/// `text` (the fonts, for `digit` and `glyph`).
pub fn tables() -> psi::Tables<'static> {
    // `Rgb` is `#[repr(C)]` with three `u8` fields (asserted in `canvas`),
    // so a `[Rgb; 32]` is a `[[u8; 3]; 32]`.
    const _: () = assert!(std::mem::size_of::<[Rgb; 32]>() == 96);
    let pal = |i: usize| -> &'static [[u8; 3]; 32] {
        // SAFETY: same size and alignment 1, every bit pattern valid.
        unsafe { &*PALETTES[i].as_ptr().cast::<[[u8; 3]; 32]>() }
    };
    psi::Tables {
        sin: &SIN,
        palettes: [pal(0), pal(1), pal(2), pal(3), pal(4)],
        small: &crate::text::FONT_4X6,
        normal: &crate::text::FONT_5X7,
        bold: &crate::text::FONT_BOLD,
    }
}

/// The `@formula?psb=…` path for a compiled Formula, with `opts`
/// (`a=0.5&fps=100`) after it. May be longer than a virtual path may be
/// (512 B): check.
pub fn path_of(psb: &[u8], opts: &str) -> String {
    let mut p = format!("{}?psb={}", crate::synth::FORMULA, crate::synth::to_base64(psb));
    if !opts.is_empty() {
        p.push('&');
        p.push_str(opts);
    }
    p
}

/// The path for a formula's source, compiled here (the host's tools and
/// tests; the player never compiles): a source that does not compile gives
/// a path that does not open.
#[cfg(any(test, feature = "psi-compiler"))]
pub fn path(src: &str, opts: &str) -> String {
    path_of(&psi::build(src).unwrap_or_else(|_| b"not compiled".to_vec()), opts)
}

/// An `@formula` query: the compiled Formula, the frame time asked (`fps=`; `None`:
/// the formula's `fps N`, else 20 ms), the sliders (`None`: not on the
/// path, so the formula's declared default: `psi::Meta::values`).
struct Query<'a> {
    psb: std::borrow::Cow<'a, [u8]>,
    /// The sent Formula's bytes ([`crate::synth::SENT`]), in place of `psb`.
    held: Option<std::sync::Arc<Vec<u8>>>,
    delay_ms: Option<u8>,
    sliders: [Option<i32>; 3],
}

impl Query<'_> {
    fn bytes(&self) -> &[u8] {
        self.held.as_deref().map_or(&self.psb, |v| v)
    }
}

/// The sentence for `@formula:<name>` when no bundled Formula has that name
/// ([`io_error`] makes it `NotFound`: `/api/play` answers 404).
pub const NO_SUCH_FORMULA: &str = "there is no bundled Formula by that name";
/// The sentence for [`crate::synth::SENT`] once its Formula was let go
/// (`NotFound` too).
pub const SENT_GONE: &str = "that Formula is no longer on the panel: show it again";

/// What follows `@formula` on a path: `?psb=…&…` (or the query alone,
/// `psb=…`), or `:<name>?<opts>` for a bundled Formula (any `psb=` there is
/// ignored: the name says what plays).
fn resolve(rest: &str) -> Result<Query<'static>, psi::Error> {
    let Some(named) = rest.strip_prefix(':') else {
        return Ok(query(rest.trim_start_matches('?')));
    };
    let (name, opts) = named.split_once('?').unwrap_or((named, ""));
    let err = |msg: &str| psi::Error {
        start: 0,
        end: 0,
        msg: msg.to_string(),
    };
    if name == crate::synth::SENT_NAME {
        let mut q = query(opts);
        q.held = Some(crate::synth::sent().ok_or_else(|| err(SENT_GONE))?);
        return Ok(q);
    }
    let psb = crate::synth::valid_formula_name(name)
        .then(|| crate::synth::bundled(name))
        .flatten()
        .ok_or_else(|| err(NO_SUCH_FORMULA))?;
    let mut q = query(opts);
    q.psb = std::borrow::Cow::Borrowed(psb);
    Ok(q)
}

/// A compiled Formula (read from its file) with `opts`.
fn with_bytes<'a>(psb: &'a [u8], opts: &str) -> Query<'a> {
    let mut q = query(opts);
    q.psb = std::borrow::Cow::Borrowed(psb);
    q
}

/// A compiled Formula's file (`.psb`), read whole; `InvalidInput` ("too
/// long for the panel") past [`MAX_FORMULA_FILE`](crate::synth::MAX_FORMULA_FILE).
pub fn read_psb<R: std::io::Read>(r: R) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let max = crate::synth::MAX_FORMULA_FILE;
    let mut b = Vec::new();
    r.take(max as u64 + 1).read_to_end(&mut b)?;
    if b.len() > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "this Formula is too long for the panel",
        ));
    }
    Ok(b)
}

fn query(query: &str) -> Query<'static> {
    let mut q = Query {
        psb: std::borrow::Cow::Borrowed(&[]),
        held: None,
        delay_ms: None,
        sliders: [None; 3],
    };
    for kv in query.split('&') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        match k {
            "psb" => q.psb = std::borrow::Cow::Owned(crate::synth::from_base64(v.as_bytes()).unwrap_or_default()),
            "fps" => {
                q.delay_ms = match v {
                    "100" => Some(10),
                    "50" => Some(20),
                    "25" => Some(40),
                    _ => q.delay_ms,
                }
            }
            "a" | "b" | "c" => {
                // A number, maybe below zero (a control's range may be);
                // anything else — empty, junk, 32,768 or more — leaves the
                // slot unset: the formula's default (js/psikit.js `num`
                // reads a link the same way).
                let (neg, digits) = v.strip_prefix('-').map_or((false, v), |d| (true, d));
                if let Some(n) = psi::number(digits) {
                    q.sliders[usize::from(k.as_bytes()[0] - b'a')] = Some(if neg { -n } else { n });
                }
            }
            _ => {}
        }
    }
    q
}

/// What the [`Governor`] can keep in budget at a frame time asked (ms):
/// [`MAX_EST_US`], and every refresh (grid 10 ms, slowest step 80 ms) 32
/// ms (`psi::tier_limit_us`).
fn tier_limit(delay_ms: u8) -> u32 {
    MAX_EST_US.min(Governor::slowest_budget_us(grid(delay_ms)))
}

/// The frame time a Formula of `est` µs asking for `delay_ms` opens at:
/// that, or 20 (every 2nd refresh) when it is too heavy to keep a faster
/// one but not that (`psi::opens_ms`): played, not refused. By the rate
/// it asks, not the glow's floor ([`grid`]): one opened under the glow
/// keeps a rate it can keep once the glow is off.
fn opens_at(delay_ms: u8, est: u32) -> u8 {
    let asked = MAX_EST_US.min(Governor::slowest_budget_us(u32::from(delay_ms)));
    if delay_ms < 20 && est > asked {
        20
    } else {
        delay_ms
    }
}

/// Load `q`'s program and hold it to what the [`Governor`] can keep in
/// budget ([`tier_limit`]) at the frame rate it opens at ([`opens_at`]).
/// The program, the estimate and that frame time (ms: the path's `fps=`,
/// else the formula's rate line, else 20; 20 for a fast one too heavy for it).
fn compile(q: &Query) -> Result<(ProgBox, u32, u8), psi::Error> {
    let t0 = clock().map(|c| (c, c()));
    let prog = load_boxed(q.bytes(), psi::psb::Which::Play)?;
    if let Some((c, t0)) = t0 {
        set_stat(2, c().wrapping_sub(t0));
    }
    let est = prog.esp32_us();
    let delay = q
        .delay_ms
        .or(prog.frame_ms().map(|ms| ms as u8))
        .unwrap_or(20);
    let delay = opens_at(delay, est);
    let limit = tier_limit(delay);
    if est > limit {
        return Err(psi::Error {
            start: 0,
            end: 0,
            msg: format!(
                "this formula is too heavy for the panel (about {} ms a frame; the limit is {} ms) — simplify it",
                est.div_ceil(1000),
                limit / 1000
            ),
        });
    }
    Ok((prog, est, delay))
}

/// The ms into its loop a frame at `ms` (the clock) draws `t` at. Raced,
/// the refresh's own time wrapped at exactly the loop's length (`loop
/// 4.85` at 4,850 ms, as the browser's preview does; a `loop forever`
/// one's passes, its `t` unread); else the frame of the grid at or before
/// it, the loop `n` whole frames of `grid` ms (held frames skip some,
/// never draw between them).
fn loop_ms_at(ms: u32, raced: bool, loop_ms: u32, n: u32, grid: u32) -> u32 {
    match (raced, loop_ms) {
        (true, 0) => ms % (n * grid),
        (true, lp) => ms % lp,
        (false, _) => ms / grid % n * grid,
    }
}

/// What the governor starts from: the estimate with the still cache when
/// the formula has values for one and the gate would let it take it now
/// (it is taken on the first frame, which the governor leaves out), else
/// `est` (every step every frame). The refusal limit is always held to
/// `est`: the cache may be refused or let go at any time.
fn start_estimate(prog: &psi::Prog, est: u32) -> u32 {
    let still = prog.still_lanes() > 0 && still_allowed(psi::Still::bytes_for(prog));
    match (still, jit_built()) {
        (true, true) => prog.esp32_us_jit_still(),
        (true, false) => prog.esp32_us_still(),
        (false, true) => est.min(prog.esp32_us_jit()),
        (false, false) => est,
    }
}

/// Whether formulas here may be drawn by the JIT (built for the ESP32 with
/// `psi-jit`, and the setting on): the governor starts from its estimate.
fn jit_built() -> bool {
    cfg!(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf")) && jit_allowed()
}

/// The frame grid for a frame time asked (ms), floored like [`PsiFx::ms`].
fn grid(delay_ms: u8) -> u32 {
    u32::from(delay_ms.max(crate::fx::floor()))
}

/// What a formula that draws gets when the heap hasn't its canvas (4 KB,
/// 8 KB for one that paints): as the compiler's own thread does when it
/// can't start.
pub const NO_CANVAS: &str = "the display is short of memory just now: try again in a moment";

/// A canvas for `prog` if it draws, with the planes it uses (taken from
/// the heap only if it may fail: `None` for one that doesn't draw, `Err`
/// when the heap hasn't them). `held`, the canvas on hand, is taken and
/// fitted (the paint plane taken or let go); refused, it is left as it was
/// for the program it serves.
fn canvas_for(
    prog: &psi::Prog,
    held: &mut Option<Box<psi::Ink>>,
) -> Result<Option<Box<psi::Ink>>, psi::Error> {
    if !prog.draws() {
        return Ok(None);
    }
    let fitted = hinted(ARENA_CANVAS, || match held.as_deref_mut() {
        Some(k) => k.try_fit(prog).then(|| held.take()).flatten(),
        None => psi::Ink::try_boxed_for(prog),
    });
    match fitted {
        Some(k) => Ok(Some(k)),
        None => Err(psi::Error {
            start: 0,
            end: 0,
            msg: NO_CANVAS.to_string(),
        }),
    }
}

/// A formula's compile error as an open error: `InvalidInput` with the
/// compiler's sentence (the firmware's `/api/play` makes it a 400; an
/// unknown `@` path is `InvalidData`, a 415).
pub fn io_error(e: psi::Error) -> std::io::Error {
    let kind = if e.msg == NO_SUCH_FORMULA || e.msg == SENT_GONE {
        std::io::ErrorKind::NotFound
    } else {
        std::io::ErrorKind::InvalidInput
    };
    std::io::Error::new(kind, e.msg)
}

/// Whether an open failed only for the heap ([`psi::NO_MEMORY`], as
/// [`io_error`] makes it): it may open once the memory something else holds
/// is let go (the Formula on screen, `Engine::retune`).
pub fn is_no_memory(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::InvalidInput && e.to_string() == psi::NO_MEMORY
}

/// A zeroed `Box<T>`, or `None` when the heap has no block for it.
///
/// # Safety
/// All-zero must be a valid `T`.
unsafe fn try_zeroed<T>() -> Option<Box<T>> {
    let layout = std::alloc::Layout::new::<T>();
    // SAFETY: `T` is not zero-sized (the callers' are KBs); a non-null
    // block of `layout` from the global allocator is what `Box` frees.
    let p = unsafe { std::alloc::alloc_zeroed(layout) } as *mut T;
    (!p.is_null()).then(|| unsafe { Box::from_raw(p) })
}

/// The compile error for an open the heap can't hold just now.
fn no_memory() -> psi::Error {
    psi::Error {
        start: 0,
        end: 0,
        msg: psi::NO_MEMORY.to_string(),
    }
}

/// Why a formula's path does not open (`None` if it is not a formula, or
/// it does).
pub fn open_error(path: &str) -> Option<std::io::Error> {
    if !crate::synth::is_formula(path) {
        return None;
    }
    PsiFx::from_query(&path[crate::synth::FORMULA.len()..]).err().map(io_error)
}

impl PsiFx {
    /// The effect for what follows `@formula` on a path (`?src=…`, the
    /// query alone, or `:<name>?…` for a bundled one), or why not.
    pub fn from_query(q: &str) -> Result<PsiBox, psi::Error> {
        Self::open(&resolve(q)?)
    }

    /// The effect for a compiled Formula (read from its file) with `opts`.
    pub fn from_bytes(psb: &[u8], opts: &str) -> Result<PsiBox, psi::Error> {
        Self::open(&with_bytes(psb, opts))
    }

    fn open(q: &Query) -> Result<PsiBox, psi::Error> {
        // (The box first, as before the store was sized per Formula: on a
        // short heap it is the one refused, before the load's own working
        // memory is asked for.)
        let mut fx = PsiBox::try_new().ok_or_else(no_memory)?;
        let (prog, est, delay) = compile(q)?;
        clock_check(&prog)?;
        fx.prog = prog;
        fx.ink = canvas_for(&fx.prog, &mut None)?;
        fx.clock = reads_clock(&fx.prog);
        fx.delay_ms = delay;
        let s = fx.prog.meta().values(q.sliders);
        fx.set_sliders(s);
        fx.inputs.seed = next_seed();
        fx.gov = Governor::new(start_estimate(&fx.prog, est), fx.ms());
        // Where its working memory landed (the scratch's lanes and the
        // canvas): a frame's cost can depend on which SRAM bank they share
        // with the panel's DMA (the firmware logs its rows' range at boot).
        log::info!(
            "formula: scratch at {:p}, canvas at {:?}",
            &fx.scratch as *const _,
            fx.ink.as_deref().map(|k| k as *const psi::Ink)
        );
        Ok(fx)
    }

    /// The `seed` a formula that never repeats plays with (a new one at
    /// each open): tests, and the simulator's `--seed`.
    pub fn set_seed(&mut self, seed: i32) {
        self.inputs.seed = seed;
    }

    fn set_sliders(&mut self, s: [i32; 3]) {
        self.inputs.a = s[0];
        self.inputs.b = s[1];
        self.inputs.c = s[2];
    }

    /// Take another query's formula, sliders and frame rate in place (a
    /// live edit on the panel, [`Retune::Edit`], or `/api/play` of another
    /// Formula over this one, [`Retune::Play`]): the clock and the working
    /// memory stay. The governor's level goes on for an edit (a heavier
    /// formula starting at least as slow as its estimate says) and starts
    /// from the new estimate for a play; the raced frames' hold never stays
    /// ([`Governor::retune`]). Only a program's worth (3.4 KB) is
    /// allocated, and only while loading.
    pub fn retune_from(&mut self, q: &str, how: Retune) -> Result<(), psi::Error> {
        let q = resolve(q)?;
        let (prog, est, delay) = compile(&q)?;
        clock_check(&prog)?;
        // A formula that starts drawing takes its canvas here, between
        // frames (as an open does); one that stops gives it back.
        self.ink = canvas_for(&prog, &mut self.ink)?;
        // The still cache keeps its lanes for a formula with as many values
        // (the next full frame keeps the new formula's: it checks), else
        // lets them go and the next frame asks again.
        if self.still.held_bytes() != psi::Still::bytes_for(&prog) {
            self.still.release();
            set_still_held(0);
        }
        self.prog = prog;
        // Another program: its slow layers are drawn again (or their
        // picture let go).
        if self.prog.rate().is_none() {
            self.held = None;
        } else if let Some(h) = self.held.as_deref_mut() {
            h.forget();
        } else {
            self.held_tried = 0;
        }
        // Another program: its kernel (if any) is made once the edits pause
        // (`Jit::settling`), the VM drawing meanwhile.
        #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
        {
            self.jit = jit::Jit::settling(0);
        }
        self.clock = reads_clock(&self.prog);
        self.full = 0;
        self.delay_ms = delay;
        let s = self.prog.meta().values(q.sliders);
        self.set_sliders(s);
        self.gov.retune(Governor::new(start_estimate(&self.prog, est), self.ms()), how);
        // (The emulator lane reads the start from this line.)
        log::info!(
            "formula: {} in place: {} ms frames (asked {})",
            match how {
                Retune::Edit => "edited",
                Retune::Play => "played",
            },
            self.gov.delay_ms(self.ms()),
            self.ms()
        );
        Ok(())
    }

    fn ms(&self) -> u32 {
        u32::from(self.delay_ms.max(crate::fx::floor()))
    }

    /// Tests: its governor (the level a retune left, and so on).
    #[cfg(test)]
    pub(crate) fn governor(&self) -> &Governor {
        &self.gov
    }

    /// The nominal frame time (the governor may hold frames longer).
    pub fn delay(&self) -> Duration {
        Duration::from_millis(u64::from(self.ms()))
    }

    /// The loop as played: raced, exactly its `loop` (`t` the refresh's
    /// time in it: [`loop_ms_at`]); else its frames on the nominal grid
    /// (and a `loop forever` one's passes).
    pub fn loop_time(&self) -> Duration {
        match self.prog.loop_ms() {
            lp if self.racing && lp > 0 => Duration::from_millis(u64::from(lp)),
            _ => Duration::from_millis(u64::from(self.loop_frames() * self.ms())),
        }
    }

    pub fn program(&self) -> &psi::Prog {
        &self.prog
    }

    /// The last frame's `t`, ms into the loop, and whether it was raced
    /// (tests).
    #[cfg(test)]
    fn last_t(&self) -> (u32, bool) {
        (((i64::from(self.inputs.t) * 1000 + 32_768) >> 16) as u32, self.racing)
    }
}

impl PsiFx {
    /// Before a frame: let the still cache go when the gate says (a
    /// transition or a look needs the heap), or take it when it allows (at
    /// most once a second after a refusal; every frame without a clock).
    fn still_step(&mut self, clocked: bool, now: u32) {
        let need = psi::Still::bytes_for(&self.prog);
        if need == 0 {
            return;
        }
        if self.still.is_held() {
            if !still_allowed(0) {
                self.still.release();
                set_still_held(0);
                log::info!("formula: still cache ({need} B) given back to the heap");
            }
            return;
        }
        let due = !clocked
            || !self.still_once
            || self.still_tried == 0
            || now.wrapping_sub(self.still_tried) >= CANVAS_RETRY_US;
        if !due {
            return;
        }
        if still_allowed(need) && self.still.take(&self.prog) {
            self.still_tried = 0;
            self.still_once = true;
            set_still_held(need as u32);
            // This frame keeps the values (every step runs): left out.
            self.gov.edited();
            log::info!("formula: still cache taken ({need} B)");
        } else {
            self.still_tried = now | 1;
        }
    }
}

impl PsiFx {
    /// Before a frame of a `rate` formula: take the held picture when the
    /// still cache's gate allows (at most once a second after a refusal;
    /// every frame without a clock), let it go when the gate says.
    fn held_step(&mut self, clocked: bool, now: u32) {
        if self.prog.rate().is_none() {
            return;
        }
        if self.held.is_some() {
            if !still_allowed(0) {
                self.held = None;
            }
            return;
        }
        let due = !clocked || self.held_tried == 0 || now.wrapping_sub(self.held_tried) >= CANVAS_RETRY_US;
        if !due {
            return;
        }
        if still_allowed(psi::Held::BYTES) {
            self.held = psi::Held::try_boxed();
        }
        if self.held.is_some() {
            self.held_tried = 0;
            // This frame draws every slow row: left out.
            self.gov.edited();
        } else {
            self.held_tried = now | 1;
        }
    }
}

/// Where the playing formula's working memory is (`display.buffers` in
/// the firmware's `/api/status`: which SRAM pool each landed in, against
/// CPU0's DMA traffic, display-isolation 2026-11-02): the formula's one
/// allocation (program and VM scratch), its ink plane, its paint plane and
/// its JIT kernel; 0 for none. Noted at each of its frames, cleared when
/// it goes.
pub static BUFFERS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
/// [`BUFFERS`]' entries.
pub const BUF_FX: usize = 0;
pub const BUF_INK: usize = 1;
pub const BUF_PLANE: usize = 2;
pub const BUF_KERNEL: usize = 3;
/// Bytes of the formula's one allocation.
pub const FX_BYTES: usize = std::mem::size_of::<PsiFx>();
/// Bytes of an ink plane (with its paint plane's pointer), and of a paint
/// plane.
pub const INK_BYTES: usize = std::mem::size_of::<psi::Ink>();
pub const PLANE_BYTES: usize = std::mem::size_of::<psi::Plane>();

/// The firmware's `formula_arena` (a dev setting): while this names an
/// allocation the formula is about to make ([`ARENA_FX`], [`ARENA_CANVAS`]),
/// the firmware's allocator serves it from an arena it set aside at boot,
/// so the formula's memory lands in the same place every time. 0: none
/// (and nothing reads it off the device).
pub static ARENA_HINT: AtomicUsize = AtomicUsize::new(0);
pub const ARENA_FX: usize = 1;
pub const ARENA_CANVAS: usize = 2;
/// The formula's program (its store sized by the formula: the arena's slot
/// holds the biggest).
pub const ARENA_PROG: usize = 3;

/// Run `f` with [`ARENA_HINT`] at `hint`.
fn hinted<T>(hint: usize, f: impl FnOnce() -> T) -> T {
    ARENA_HINT.store(hint, Ordering::Relaxed);
    let r = f();
    ARENA_HINT.store(0, Ordering::Relaxed);
    r
}

impl PsiFx {
    /// Note where this formula's memory is ([`BUFFERS`]).
    fn note_buffers(&self) {
        BUFFERS[BUF_FX].store(self as *const PsiFx as usize, Ordering::Relaxed);
        let ink = self.ink.as_deref();
        BUFFERS[BUF_INK].store(ink.map_or(0, |k| k as *const psi::Ink as usize), Ordering::Relaxed);
        let plane = ink.and_then(|k| k.1.as_deref());
        BUFFERS[BUF_PLANE].store(plane.map_or(0, |p| p as *const psi::Plane as usize), Ordering::Relaxed);
        #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
        BUFFERS[BUF_KERNEL].store(self.jit.kernel_addr(), Ordering::Relaxed);
    }
}

impl Drop for PsiFx {
    fn drop(&mut self) {
        if self.still.is_held() {
            set_still_held(0);
        }
        if BUFFERS[BUF_FX].load(Ordering::Relaxed) == self as *const PsiFx as usize {
            for b in &BUFFERS {
                b.store(0, Ordering::Relaxed);
            }
        }
    }
}

impl Effect for PsiFx {
    fn name(&self) -> &'static str {
        "psi"
    }

    fn loop_frames(&self) -> u32 {
        let lp = if self.prog.forever() {
            PASS_MS
        } else {
            self.prog.loop_ms()
        };
        (lp / self.ms()).max(1)
    }

    fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration {
        #[cfg(feature = "psi-stats")]
        psi::stats::stamp(0);
        let d = self.frame_inner(t, canvas);
        #[cfg(feature = "psi-stats")]
        psi::stats::stamp(4);
        d
    }
}

impl PsiFx {
    fn frame_inner(&mut self, t: Duration, canvas: &mut Canvas) -> Duration {
        let n = self.loop_frames();
        let grid = self.ms();
        let ms = (t.as_secs() as u32)
            .wrapping_mul(1000)
            .wrapping_add(t.subsec_millis());
        // The frame of the grid at or before `t`: held frames skip some, never
        // draw between them. A raced frame is on the panel's refresh grid
        // instead: its own `t` (one `t` a frame, the picture the browser
        // draws for it).
        // (Not a `rate` formula: on the refresh grid its held slow rows are
        // due 0, 1 or 2 bands a frame before its rows start, which moved
        // the start by milliseconds — Storm front raced went from 20 to
        // 32 ms frames with 213 misses a minute on a test board, beam-rate-fix.)
        self.racing = self.drawn > 0
            && self.prog.rate().is_none()
            && clock().is_some()
            && !self.trialing()
            && beam_hooks().is_some();
        let i = loop_ms_at(ms, self.racing, self.prog.loop_ms(), n, grid);
        self.inputs.t = psi::Inputs::at_ms(i);
        // (One that never repeats: its own time since it opened.)
        self.inputs.tw = psi::Inputs::wide_ms(self.played_ms);
        if let Some(now) = self.clock.then(crate::synth::wall_now).flatten() {
            self.inputs.hour = now.hour;
            self.inputs.minute = now.minute;
            self.inputs.second = now.second;
        }
        if reads_world(&self.prog) {
            fill_world(&mut self.inputs.world);
        }
        let d = self.render(canvas);
        self.drawn = self.drawn.saturating_add(1);
        let us = d.as_micros() as u64 + u64::from(self.played_us);
        self.played_ms += us / 1000;
        self.played_us = (us % 1000) as u32;
        d
    }

    /// A JIT trial frame now (kernel against VM, timed): never raced.
    fn trialing(&self) -> bool {
        #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
        return self.jit.trialing();
        #[cfg(not(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf")))]
        false
    }

    /// Draw the frame of `self.inputs` and say how long it shows.
    fn render(&mut self, canvas: &mut Canvas) -> Duration {
        let grid = self.ms();
        let clock = clock();
        let t0 = clock.map_or(0, |c| c());
        // A formula that draws but has no canvas (a clone made when the heap
        // hadn't 4 KB) draws without it, `ink` 0, and asks again once a
        // second (every frame without a clock: the simulator, the tests).
        if self.prog.draws() && self.ink.is_none() {
            let due = clock.is_none()
                || self.ink_tried == 0
                || t0.wrapping_sub(self.ink_tried) >= CANVAS_RETRY_US;
            if due {
                self.ink = psi::Ink::try_boxed_for(&self.prog);
                self.ink_tried = if self.ink.is_some() { 0 } else { t0 | 1 };
            }
        }
        self.still_step(clock.is_some(), t0);
        self.held_step(clock.is_some(), t0);
        // Racing the beam: planned (any wait for its window left out of
        // the governor's time), else drawn as before.
        let hooks = beam_hooks().filter(|_| self.racing);
        let cycle = hooks.and_then(|h| (h.begin)());
        let est = self.gov.avg_us();
        let frames = self.gov.beam_frames(crate::scan::beam_frames(grid * 1000, cycle.unwrap_or(0)));
        let hooks = hooks.filter(|_| cycle.is_some());
        let t0 = match (hooks, clock) {
            (Some(_), Some(c)) => c(),
            _ => t0,
        };
        let (bytes, dirty) = canvas.bytes_and_dirty_mut();
        // Only what changed is marked (the canvas holds the frame before:
        // the `Effect` contract; what differs from it is what the panel
        // lacks, the map marking the rest). A Formula that keeps changing
        // half the panel or more (Tunnel, Time gate, Plasma, Big clock) is
        // not compared, only checked again now and then: there the compare
        // costs more than the blit saves (`FULL_PX`). A kernel's frame
        // compares as the VM's does.
        let compare = !old() && (self.full < FULL_RUN || self.uncompared >= RECHECK);
        let mut changed = [0u32; psi::H];
        // A `rate` formula with its slow layers held: the slow rows due
        // first (the VM's, for the kernel's frames and the VM's alike).
        // Timed apart: the trial times the rest (the part kernel and VM
        // draw differently), and the governor counts slow rows beyond a
        // band's at a band's share — on a slower grid more bands are due a
        // frame, and that must not hold the formula there.
        let mut slow_us = 0u32;
        let mut slow_fair = 0u32;
        if let (Some(h), Some(band)) = (self.held.as_deref_mut(), self.prog.rate_rows()) {
            let ta = clock.map_or(0, |c| c());
            let rows = h.update::<LANES>(&self.prog, &mut self.scratch, &self.inputs, &tables());
            slow_us = clock.map_or(0, |c| c()).wrapping_sub(ta);
            slow_fair = if band > 0 && rows > band {
                (u64::from(slow_us) * u64::from(band) / u64::from(rows)) as u32
            } else {
                slow_us
            };
        }
        #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
        let (trial, t1) = (self.jit.trialing(), clock.map_or(0, |c| c()));
        let mut marked = 0u32;
        let drawn = match hooks {
            Some(h) => {
                // Racing the beam: each pair marked and written as it is
                // drawn, then forgotten: the blit after the tick finds
                // nothing (a mark from before, a new tint's, is written
                // with its pair).
                let mut tell = |p: usize, out: &[u8], a: u32, b: u32| {
                    const HALF: usize = psi::H / 2;
                    if p == psi::BEAM_START {
                        return (h.start)(est, frames);
                    }
                    marked += if compare {
                        mark_pair(dirty, p, a, b)
                    } else {
                        dirty.mark_span(p, 0, crate::canvas::WIDTH);
                        dirty.mark_span(p + HALF, 0, crate::canvas::WIDTH);
                        2 * crate::canvas::WIDTH as u32
                    };
                    (h.pair)(p, out, dirty);
                    dirty.clear_row(p);
                    dirty.clear_row(p + HALF);
                };
                self.draw(bytes, compare.then_some(&mut changed), Some(&mut tell))
            }
            None => {
                BEAM_US.store(0, Ordering::Relaxed);
                self.draw(bytes, compare.then_some(&mut changed), None)
            }
        };
        #[cfg(feature = "psi-stats")]
        if self.prog.rate().is_some() {
            let k = if self.held.is_none() { 2 } else { 0 };
            RATE_FRAMES[k].fetch_add(1, Ordering::Relaxed);
            if drawn {
                RATE_FRAMES[1].fetch_add(1, Ordering::Relaxed);
            }
        }
        #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
        let t2 = clock.map_or(0, |c| c());
        if hooks.is_some() {
            self.counted(compare, marked);
        } else if compare {
            let marked = mark_changed(dirty, &changed);
            self.counted(true, marked);
        } else {
            dirty.mark_all();
            self.counted(false, 0);
        }
        // After a VM frame: a trial's frame is timed, else a kernel not yet
        // checked is checked against it; a kernel's trial frame is timed.
        // (A raced frame is never a trial's.)
        #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
        if trial {
            if clock.is_some() {
                self.jit.timed(0, drawn, t2.wrapping_sub(t1));
            }
        } else if !drawn {
            self.jit_check(bytes);
        }
        #[cfg(not(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf")))]
        let _ = drawn;
        if let Some(h) = hooks {
            return self.beam_end(h, cycle.unwrap_or(0), clock, t0, grid);
        }
        let Some(c) = clock else {
            return self.delay();
        };
        let us = c().wrapping_sub(t0).wrapping_sub(slow_us).wrapping_add(slow_fair);
        note_render(us);
        if self.gov.observe(us, grid) {
            log::info!(
                "formula: {} us a frame -> {} ms frames (asked {grid})",
                self.gov.avg_us(),
                self.gov.delay_ms(grid)
            );
        }
        let ms = self.gov.delay_ms(grid);
        #[cfg(feature = "psi-stats")]
        if self.prog.rate().is_some() && ms > grid {
            RATE_FRAMES[3].fetch_add(1, Ordering::Relaxed);
        }
        set_stat(0, self.gov.avg_us());
        set_stat(1, ms << 16 | grid);
        Duration::from_millis(u64::from(ms))
    }

    /// Draw the frame of `self.inputs` into `bytes`, what changed into
    /// `changed` when given: the kernel if it draws this frame (`true`),
    /// else the VM — racing the beam with `sink`, over a `rate` formula's
    /// held slow rows, or whole. One call for every way (out of line:
    /// each way's ~10-argument call written once, not per path in
    /// `render`).
    #[inline(never)]
    fn draw(&mut self, bytes: &mut [u8], mut changed: Option<&mut [u32; psi::H]>, sink: Option<psi::PairSink<'_>>) -> bool {
        self.note_buffers();
        #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
        let mut sink = sink;
        #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
        if self.jit_render(bytes, changed.as_deref_mut(), sink.as_deref_mut().map(|s| s as psi::PairSink<'_>)) {
            return true;
        }
        let tables = tables();
        let ink = self.ink.as_deref_mut();
        let still = Some(&mut self.still);
        match (sink, self.held.as_deref().filter(|_| self.prog.rate().is_some())) {
            // (Never a `rate` formula's frame: see `frame_inner`.)
            (Some(s), _) => {
                psi::render_rows_beam::<LANES>(&self.prog, &mut self.scratch, ink, still, &self.inputs, &tables, None, bytes, changed, s)
            }
            // A `rate` formula: the rest over the slow rows held.
            (None, Some(h)) => psi::render_fast::<LANES>(
                &self.prog,
                &mut self.scratch,
                ink,
                still,
                &self.inputs,
                &tables,
                Some(h.picture()),
                u32::MAX,
                bytes,
                changed,
            ),
            (None, None) => match changed.take() {
                Some(c) => psi::render_rows_changed::<LANES>(
                    &self.prog,
                    &mut self.scratch,
                    ink,
                    still,
                    &self.inputs,
                    &tables,
                    None,
                    u32::MAX,
                    bytes,
                    c,
                ),
                None => psi::render_rows_still::<LANES>(
                    &self.prog,
                    &mut self.scratch,
                    ink,
                    still,
                    &self.inputs,
                    &tables,
                    None,
                    u32::MAX,
                    bytes,
                ),
            },
        }
        false
    }

    /// The compare's bookkeeping after a frame that marked `marked` pixels
    /// (`compare` false: not compared, all marked).
    fn counted(&mut self, compare: bool, marked: u32) {
        if compare {
            self.full = if marked >= FULL_PX {
                self.full.saturating_add(1)
            } else {
                0
            };
            self.uncompared = 0;
        } else {
            self.uncompared = self.uncompared.saturating_add(1);
        }
    }

    /// A raced frame written: the governor told its time (render and
    /// writes, less any wait for the scan) against its refresh grid, and
    /// the delay to the next, locked onto the scan
    /// ([`crate::scan::beam_delay_us`]).
    fn beam_end(&mut self, h: BeamHooks, cycle: u32, clock: Option<Clock>, t0: u32, grid: u32) -> Duration {
        let (err, waited, late) = (h.end)();
        let unit = crate::scan::beam_frames(grid * 1000, cycle);
        let budget = beam_budget_us(self.gov.beam_frames(unit) * cycle, cycle);
        if self.gov.note_late(late, budget) {
            log::info!(
                "formula: over 1 in 1,000 raced frames late near its budget -> every {} refreshes for a while",
                self.gov.beam_frames(unit)
            );
        }
        if let Some(c) = clock {
            let us = c().wrapping_sub(t0).saturating_sub(waited);
            note_render(us);
            if self.gov.observe_beam(us, unit * cycle, cycle) {
                log::info!(
                    "formula: {} us a frame raced -> every {} refreshes (asked {grid} ms)",
                    self.gov.avg_us(),
                    self.gov.beam_frames(unit)
                );
            }
        }
        let frames = self.gov.beam_frames(unit);
        let frame_us = frames * cycle;
        set_stat(0, self.gov.avg_us());
        set_stat(1, (((frame_us + 500) / 1000) << 16) | grid);
        BEAM_US.store(frame_us, Ordering::Relaxed);
        Duration::from_micros(u64::from(crate::scan::beam_delay_us(frames, cycle, err)))
    }
}

/// psi-probe's counts of a `rate` formula's frames (feature `psi-stats`):
/// drawn with the held picture, drawn by the kernel, drawn without the
/// held picture (from nothing), held longer than the asked grid by the
/// governor.
#[cfg(feature = "psi-stats")]
pub static RATE_FRAMES: [std::sync::atomic::AtomicU32; 4] = [const { std::sync::atomic::AtomicU32::new(0) }; 4];

/// psi-probe's switch (feature `psi-stats`): a Formula's frames as before
/// console-a's Phase A (no compare; every pixel marked, a look's redrawn
/// rows whole), for A/B rows and play lines in one build. Not in a release.
#[cfg(feature = "psi-stats")]
pub static OLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// [`OLD`]'s state (false without `psi-stats`).
#[inline(always)]
fn old() -> bool {
    #[cfg(feature = "psi-stats")]
    return OLD.load(Ordering::Relaxed);
    #[cfg(not(feature = "psi-stats"))]
    false
}

/// A frame whose changes marked this many pixels for the blit (half the
/// panel) did not pay for its compare: consolea-b on a test board, per frame
/// (render + blit), Endless road −0.65 ms at ~570 px pushed, Starfield,
/// Glitter, Rotozoom floor −0.47..−0.51 ms at 290-400, Big clock +0.18 ms
/// at ~2,550 (the compare ~0.18 ms and the marking ~0.1, against ~0.23 µs
/// of blit a pixel not pushed). After [`FULL_RUN`] such frames in a row a
/// Formula's frames are not compared (all marked), but one in [`RECHECK`]
/// is (then compared again as soon as one marks less).
const FULL_PX: u32 = 2048;
const FULL_RUN: u8 = 4;
const RECHECK: u8 = 32;

/// What pushing the panel costs, in tenths of a µs (the firmware's blit:
/// `hub75_draw_span` ~0.45 µs a pixel and ~1 µs a call, rows y and y + 16
/// whole through `hub75_draw_row_pair` 74 µs: CLAUDE.md "Frame time: the
/// blit"): how [`mark_changed`] chooses between a row's spans, the whole
/// row and the whole pair. Only the cost moves with these; any choice
/// marks every pixel that changed.
const SPAN_PX: u32 = 5;
const SPAN_CALL: u32 = 10;
const PAIR: u32 = 740;

/// A row's push as spans of its changed groups (bit g: pixels 4g..4g + 4).
fn spans_cost(m: u32) -> u32 {
    let runs = (m & !(m << 1)).count_ones();
    SPAN_PX * 4 * m.count_ones() + SPAN_CALL * runs
}

/// Mark in `dirty` what `changed` says changed (`psi::render_rows_changed`:
/// a bit for each 4-pixel group of a row), as the blit pushes it cheapest:
/// a row's runs as spans, or the whole row when its spans would cost more,
/// or both rows of a pair whole when one pair write costs less than their
/// spans. Never less than what changed (a few pixels more, by the groups).
/// Returns the pixels it marked.
pub fn mark_changed(dirty: &mut crate::canvas::DirtyMap, changed: &[u32; psi::H]) -> u32 {
    const HALF: usize = psi::H / 2;
    let mut marked = 0;
    for p in 0..HALF {
        marked += mark_pair(dirty, p, changed[p], changed[p + HALF]);
    }
    marked
}

/// [`mark_changed`] for row pair `p` alone (rows `p` and `p + 16`, their
/// changed groups `a` and `b`): what racing the beam marks as each pair is
/// drawn. Returns the pixels it marked.
#[cfg_attr(all(target_os = "espidf", feature = "sram1-iram"), link_section = ".iram1.s1")]
pub fn mark_pair(dirty: &mut crate::canvas::DirtyMap, p: usize, a: u32, b: u32) -> u32 {
    const HALF: usize = psi::H / 2;
    let whole = spans_cost(u32::MAX);
    let w = crate::canvas::WIDTH as u32;
    let row = |dirty: &mut crate::canvas::DirtyMap, y: usize, m: u32| -> u32 {
        if m == 0 {
            return 0;
        }
        if spans_cost(m) >= whole {
            dirty.mark_span(y, 0, crate::canvas::WIDTH);
            return w;
        }
        let px = 4 * m.count_ones();
        let mut m = m;
        while m != 0 {
            let g0 = m.trailing_zeros();
            let run = (!(m >> g0)).trailing_zeros();
            dirty.mark_span(y, 4 * g0 as usize, 4 * (g0 + run) as usize);
            m &= u32::MAX.checked_shl(g0 + run).unwrap_or(0);
        }
        px
    };
    if a != 0 && b != 0 && PAIR <= spans_cost(a).min(whole) + spans_cost(b).min(whole) {
        dirty.mark_span(p, 0, crate::canvas::WIDTH);
        dirty.mark_span(p + HALF, 0, crate::canvas::WIDTH);
        2 * w
    } else {
        row(dirty, p, a) + row(dirty, p + HALF, b)
    }
}

/// A formula estimated to take longer than this a frame (µs on the ESP32)
/// is not drawn over GIFs: it runs at the GIF's own frame rate whenever the
/// GIF changes (a governor can only slow its own animation), next to the
/// GIF's decoding (p90 ~4 ms) and the blit. 8 ms until any-look
/// (2026-10-26), on an estimate that read full-panel looks ~1.5× low; with
/// the estimate calibrated on the device, 12 ms: on a test board, looks of
/// 8.6 and 12.3 ms measured left the GIFs at 0 steady misses (the animated
/// one at 25 fps: over 8 ms its governor holds 40 ms frames).
pub const LOOK_MAX_EST_US: u32 = psi::LOOK_MAX_US;

/// Why a formula that never lets the picture under it show through (it
/// reads none of `ur ug ub ul`, and its bottom layer covers every pixel) is
/// not drawn over GIFs: the GIF would be hidden. Any Formula is a look once
/// the web app composes it with a blend or an amount under 100 %
/// (`psi::look`); this is the one choice left that hides the GIF.
pub const COVERS: &str =
    "this Formula covers the GIF entirely: choose a lower amount or another blend";

/// How soon a Formula look that would not build is tried again: a
/// moment later when it was only short of memory ([`psi::NO_MEMORY`]: the
/// heap comes back, as after a burst of uploads), never for a real error
/// (it doesn't compile, its file is missing or isn't a compiled Formula),
/// which stays until the look or its file changes.
pub fn look_retry(e: &psi::Error) -> Option<Duration> {
    (e.msg == psi::NO_MEMORY).then_some(LOOK_RETRY)
}

/// [`look_retry`]'s wait.
pub const LOOK_RETRY: Duration = Duration::from_secs(1);

/// A formula drawn over whatever the player shows — a GIF, a transition, a
/// live item: the Look setting's formula, or the editor's "over the GIF".
/// Its `ur ug ub ul` read the picture under it and its layers blend onto
/// it; the panel then shows [`FormulaLook::apply`]'s picture instead (the
/// tint and the glow still come after it, in `Panel::blit`).
///
/// **Memory.** The program (2.8 KB) is held from [`FormulaLook::from_path`];
/// the VM's working memory and the picture it makes (8 + 12 KB) only
/// between [`take`](FormulaLook::take) and [`release`](FormulaLook::release),
/// both fallible, so the firmware can hold them only while the heap has
/// room (as the glow's copy). Not held, [`apply`](FormulaLook::apply)
/// passes the picture through.
///
/// **Time.** A formula that reads `t` has frames of its own (on its grid,
/// `fps=`, held longer by its [`Governor`]); one that reads the clock but
/// not `t`, one a second; between them, only the rows the picture under it
/// changed are drawn again, with the same `t`. One that reads neither is
/// drawn only where the picture changes.
pub struct FormulaLook {
    prog: ProgBox,
    /// Boxed, so a `FormulaLook` (two of them in the firmware's main loop)
    /// is small on the main task's stack: the picture's dirty map alone is
    /// 516 B.
    held: Option<Box<LookBuffers>>,
    inputs: psi::Inputs,
    delay_ms: u8,
    gov: Governor,
    /// When its loop started (its first frame of its own).
    t0: Option<Duration>,
    /// When its next frame of its own is due (animated formulas).
    next: Option<Duration>,
    /// Draw every row at the next `apply` (just taken, or a new formula).
    fresh: bool,
    /// When the heap last refused the look's canvas (its `apply` time):
    /// asked again a second later, not every frame.
    ink_tried: Option<Duration>,
    /// Frames of its own still left out of the governor: the first after a
    /// take, a new formula or a new file (a cold flash cache, the file's
    /// open in the same tick).
    cold: u8,
    /// The JIT's kernel for its pixel stage (the ESP32 with `psi-jit`).
    #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
    jit: jit::Jit,
}

struct LookBuffers {
    scratch: Box<psi::Scratch<LANES>>,
    out: Canvas,
    /// The draw stage's canvas, while the look draws.
    ink: Option<Box<psi::Ink>>,
}

/// The formula drawn over the GIFs: its mean render (µs), its frame time
/// (ms, 0: it has none of its own) and the heap it holds (B).
static LOOK_STATS: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];

/// [`FormulaLook`]'s numbers for `/api/status`: (mean render µs, its frame
/// time ms or 0, heap held B).
pub fn look_stats() -> (u32, u32, u32) {
    (
        LOOK_STATS[0].load(Ordering::Relaxed),
        LOOK_STATS[1].load(Ordering::Relaxed),
        LOOK_STATS[2].load(Ordering::Relaxed),
    )
}

impl FormulaLook {
    /// The heap [`take`](Self::take) asks for: the working memory and the
    /// picture.
    pub const BUFFER_BYTES: usize = std::mem::size_of::<psi::Scratch<LANES>>()
        + PIXELS * 3
        + std::mem::size_of::<LookBuffers>();

    /// What [`take`](Self::take) asks for this formula: [`BUFFER_BYTES`]
    /// and, when it draws, its canvas (4 KB).
    ///
    /// [`BUFFER_BYTES`]: Self::BUFFER_BYTES
    pub fn buffer_bytes(&self) -> usize {
        Self::BUFFER_BYTES
            + if self.prog.draws() {
                std::mem::size_of::<psi::Ink>()
            } else {
                0
            }
    }

    /// The look for a formula's `@formula?src=…` path (sliders and `fps=`
    /// as for the effect), or why not: it does not compile, or it is too
    /// heavy to draw over GIFs ([`LOOK_MAX_EST_US`]).
    pub fn from_path(path: &str) -> Result<FormulaLook, psi::Error> {
        Self::open(&resolve(Self::query_of(path))?)
    }

    /// The look for a compiled Formula (read from its `.psb` file) with
    /// `opts`.
    pub fn from_bytes(psb: &[u8], opts: &str) -> Result<FormulaLook, psi::Error> {
        Self::open(&with_bytes(psb, opts))
    }

    fn open(q: &Query) -> Result<FormulaLook, psi::Error> {
        let (prog, est) = Self::compile(q)?;
        let mut look = FormulaLook {
            prog,
            held: None,
            inputs: psi::Inputs::default(),
            delay_ms: Self::delay(q),
            gov: Governor::default(),
            t0: None,
            next: None,
            fresh: true,
            ink_tried: None,
            cold: Governor::SKIP,
            #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
            jit: jit::Jit::default(),
        };
        look.inputs.seed = next_seed();
        look.set(q, est, Retune::Play);
        Ok(look)
    }

    fn query_of(path: &str) -> &str {
        path.strip_prefix(crate::synth::FORMULA).unwrap_or(path)
    }

    fn compile(q: &Query) -> Result<(ProgBox, u32), psi::Error> {
        // Its look program: its own `look screen 40` line composed in by
        // the compiler (else the program as it plays).
        let prog = load_boxed(q.bytes(), psi::psb::Which::Look)?;
        clock_check(&prog)?;
        if !prog.over() {
            return Err(psi::Error {
                start: 0,
                end: 0,
                msg: COVERS.to_string(),
            });
        }
        let est = prog.esp32_us();
        if est > LOOK_MAX_EST_US {
            return Err(psi::Error {
                start: 0,
                end: 0,
                msg: format!(
                    "this formula is too heavy to draw over GIFs (about {} ms a frame; the limit is {} ms) — simplify it",
                    est.div_ceil(1000),
                    LOOK_MAX_EST_US / 1000
                ),
            });
        }
        Ok((prog, est))
    }

    /// A look's frame time asked: its path's `fps=`, else 20 ms. A
    /// formula's own `fps N` is not followed over a GIF: the look is drawn
    /// at the GIF's frames, inside the GIF's budget ([`LOOK_MAX_EST_US`]).
    fn delay(q: &Query) -> u8 {
        q.delay_ms.unwrap_or(20)
    }

    fn set(&mut self, q: &Query, est: u32, how: Retune) {
        self.delay_ms = Self::delay(q);
        [self.inputs.a, self.inputs.b, self.inputs.c] = self.prog.meta().values(q.sliders);
        let est = if jit_built() { est.min(self.prog.esp32_us_jit()) } else { est };
        let g = Governor::new(est, self.ms());
        if g.level > self.gov.level || self.t0.is_none() || how == Retune::Play {
            self.gov = g;
        }
        self.gov.edited();
        self.cold = Governor::SKIP;
        self.fresh = true;
        self.next = None;
    }

    /// A new file is on screen: its first frames (its open in the same
    /// tick, a cold cache) do not steer the governor.
    pub fn new_file(&mut self) {
        self.cold = Governor::SKIP;
    }

    /// Its mean render time, µs (the governor's).
    pub fn avg_us(&self) -> u32 {
        self.gov.avg_us()
    }

    /// The picture the panel shows while it is held and drawn: its own;
    /// `None` when the panel shows the picture under it as it is.
    pub fn shown(&self) -> Option<&Canvas> {
        match &self.held {
            Some(b) if !self.fresh => Some(&b.out),
            _ => None,
        }
    }

    /// Let the buffers go without asking for a repaint: the panel keeps
    /// showing the look's last picture (paused: nothing is pushed until
    /// it plays again, and the next take draws every row).
    pub fn let_go(&mut self) {
        self.held = None;
        LOOK_STATS[2].store(0, Ordering::Relaxed);
    }

    /// Another formula in place (a live edit from the editor,
    /// [`Retune::Edit`], or another Formula sent over the same GIF,
    /// [`Retune::Play`]): the clock and the buffers stay; the governor's
    /// level goes on for an edit (slower where the new estimate says) and
    /// starts from the new estimate for a play. On an error nothing changes.
    pub fn retune(&mut self, path: &str, how: Retune) -> Result<(), psi::Error> {
        let q = resolve(Self::query_of(path))?;
        let (prog, est) = Self::compile(&q)?;
        self.prog = prog;
        // Another program: its kernel (if any) is made once the edits pause.
        #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
        {
            self.jit = jit::Jit::settling(1);
        }
        // Held: a look that starts drawing takes its canvas now (if the heap
        // hasn't it, `apply` asks again a second later and `ink` reads 0
        // till then); one that stops gives it back.
        self.ink_tried = None;
        if let Some(b) = self.held.as_mut() {
            b.ink = canvas_for(&self.prog, &mut b.ink).unwrap_or(None);
            let ink = b
                .ink
                .as_ref()
                .map_or(0, |_| std::mem::size_of::<psi::Ink>());
            LOOK_STATS[2].store((Self::BUFFER_BYTES + ink) as u32, Ordering::Relaxed);
        }
        self.set(&q, est, how);
        Ok(())
    }

    pub fn program(&self) -> &psi::Prog {
        &self.prog
    }

    /// Whether it has frames of its own (it reads `t` or the clock).
    pub fn is_animated(&self) -> bool {
        // (`t`, the clock, or a built-in input: those move a second at most.)
        self.prog.inputs() & (24 | 128) != 0
    }

    /// Its own frames' grid (ms): its frame time if it reads `t`, a second
    /// if only the clock.
    fn own_grid(&self) -> u32 {
        if self.prog.inputs() & 8 != 0 {
            self.ms()
        } else {
            1000
        }
    }

    fn ms(&self) -> u32 {
        u32::from(self.delay_ms.max(crate::fx::floor()))
    }

    /// Take the working memory and the picture (8 + 12 KB), if the heap
    /// has them; true if held. The next [`apply`](Self::apply) draws every
    /// row.
    pub fn take(&mut self) -> bool {
        if self.held.is_some() {
            return true;
        }
        let mut px: Vec<Rgb> = Vec::new();
        if px.try_reserve_exact(PIXELS).is_err() {
            return false;
        }
        px.resize(PIXELS, Rgb::BLACK);
        let Ok(ink) = canvas_for(&self.prog, &mut None) else {
            return false;
        };
        let layout = std::alloc::Layout::new::<psi::Scratch<LANES>>();
        // SAFETY: a non-zero-sized layout; `Scratch` is integers all the way
        // down, so the zeroed memory is a valid value (and `reset` fills in
        // its columns); the box frees it with the same layout.
        let scratch = unsafe {
            let p = std::alloc::alloc_zeroed(layout).cast::<psi::Scratch<LANES>>();
            if p.is_null() {
                return false;
            }
            Box::from_raw(p)
        };
        // The box itself, fallibly too (a full heap must not abort).
        let layout = std::alloc::Layout::new::<LookBuffers>();
        // SAFETY: a non-zero-sized layout; written before the box is made,
        // and the box frees it with the same layout.
        let mut b = unsafe {
            let p = std::alloc::alloc(layout).cast::<LookBuffers>();
            if p.is_null() {
                return false;
            }
            p.write(LookBuffers {
                scratch,
                out: Canvas::from_parts(px, crate::canvas::DirtyMap::default()),
                ink,
            });
            Box::from_raw(p)
        };
        b.scratch.reset();
        self.held = Some(b);
        self.fresh = true;
        self.cold = Governor::SKIP;
        LOOK_STATS[2].store(self.buffer_bytes() as u32, Ordering::Relaxed);
        true
    }

    /// Let the buffers go. `display` (the picture under the look, what the
    /// panel shows from now on) is marked to be pushed in full, since the
    /// panel still shows the look's picture.
    pub fn release(&mut self, display: &mut Canvas) {
        if self.held.take().is_some() {
            display.dirty_mut().mark_all();
        }
        LOOK_STATS[2].store(0, Ordering::Relaxed);
    }

    pub fn is_held(&self) -> bool {
        self.held.is_some()
    }

    /// The panel showed something else meanwhile (a status card, a test
    /// pattern): every row is drawn again at the next `apply`.
    pub fn redraw(&mut self) {
        self.fresh = true;
    }

    /// When it must be drawn again whatever the picture under it does: its
    /// next frame of its own, or at once after a take or a new formula;
    /// `None` while not held, or when it only follows the picture.
    pub fn due(&self) -> Option<Duration> {
        self.held.as_ref()?;
        if self.fresh {
            return Some(Duration::ZERO);
        }
        if self.is_animated() {
            self.next.or(Some(Duration::ZERO))
        } else {
            None
        }
    }

    /// When it must be drawn *between* the player's frames: [`due`](Self::due),
    /// unless a transition runs, and then never. A transition's steps come
    /// at least every [`STEP`](crate::compositor::STEP) and each goes
    /// through [`apply`](Self::apply), which draws the look's own frame
    /// when it is due (every row, its `t` on its grid): the look keeps
    /// moving over the transition at the steps' rate. Drawn between the
    /// steps as well, a dissolve (it changes pixels on every row) had the
    /// look draw all 32 rows twice within a few ms — its own frame, then
    /// the step's — and the second ran past the file's frame.
    pub fn due_between(&self, transitioning: bool) -> Option<Duration> {
        if transitioning {
            return None;
        }
        self.due()
    }

    /// The picture to show at `now` over `under` (what the player shows;
    /// its dirty map says what changed since the last call, and is cleared
    /// here): the rows `under` changed drawn again — every row on a frame of
    /// the look's own — into the look's picture, those rows marked in its
    /// dirty map. Not held: `under` itself. `extra_us`: what the frame's
    /// decoding took (µs), which the governor counts with the look's own
    /// time.
    pub fn apply<'a>(
        &'a mut self,
        now: Duration,
        under: &'a mut Canvas,
        extra_us: u32,
    ) -> &'a mut Canvas {
        let animated = self.is_animated();
        let grid = self.own_grid();
        // Only a formula that moves with `t` is governed.
        let governed = self.prog.inputs() & 8 != 0;
        let Some(b) = self.held.as_mut() else {
            return under;
        };
        // No canvas yet (refused at a retune or a moment ago): asked again
        // once a second; when it comes, every row is drawn (the draw stage
        // runs only on a full render).
        if self.prog.draws()
            && b.ink.is_none()
            && self
                .ink_tried
                .is_none_or(|t| now >= t + Duration::from_micros(u64::from(CANVAS_RETRY_US)))
        {
            b.ink = psi::Ink::try_boxed_for(&self.prog);
            if b.ink.is_some() {
                self.ink_tried = None;
                self.fresh = true;
            } else {
                self.ink_tried = Some(now);
            }
        }
        let own = self.fresh || (animated && self.next.is_none_or(|n| now >= n));
        let rows = if own {
            u32::MAX
        } else {
            under.dirty().row_mask()
        };
        if rows == 0 {
            return &mut b.out;
        }
        if own {
            let t0 = *self.t0.get_or_insert(now);
            let el = now.saturating_sub(t0);
            let el_ms = (el.as_secs() as u32)
                .wrapping_mul(1000)
                .wrapping_add(el.subsec_millis());
            let n = (self.prog.loop_ms() / grid).max(1);
            self.inputs.t = psi::Inputs::at_ms(el_ms / grid % n * grid);
            // (One that never repeats: the time since it was taken on.)
            self.inputs.tw =
                psi::Inputs::wide_ms(el.as_millis() as u64 / u64::from(grid) * u64::from(grid));
            if let Some(w) = reads_clock(&self.prog)
                .then(crate::synth::wall_now)
                .flatten()
            {
                self.inputs.hour = w.hour;
                self.inputs.minute = w.minute;
                self.inputs.second = w.second;
            }
            if reads_world(&self.prog) {
                fill_world(&mut self.inputs.world);
            }
            // The next frame of its own: on its grid, held as the governor
            // says, never in the past.
            let held = u64::from(if governed {
                self.gov.delay_ms(grid)
            } else {
                grid
            });
            let k = el.as_millis() as u64 / held + 1;
            self.next = Some(t0 + Duration::from_millis(k * held));
        }
        let clock = clock();
        let t_start = clock.map_or(0, |c| c());
        let (bytes, dirty) = b.out.bytes_and_dirty_mut();
        let mut changed = [0u32; psi::H];
        // The kernel draws it when it is made, checked and (after the
        // trial, kernel and VM in turn, timed a row) the faster.
        #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
        let trial = self.jit.trialing();
        #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
        let drawn = match self.jit.kernel_for(1, &self.prog) {
            // (False when the VM drew it: the inputs out of range.)
            Some(k) => k.render(
                &self.prog,
                &mut b.scratch,
                b.ink.as_deref_mut(),
                None,
                &self.inputs,
                &tables(),
                Some(under.as_bytes()),
                rows,
                bytes,
                Some(&mut changed),
            ),
            None => false,
        };
        #[cfg(not(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf")))]
        let drawn = false;
        if !drawn {
            psi::render_rows_changed::<LANES>(
                &self.prog,
                &mut b.scratch,
                b.ink.as_deref_mut(),
                None,
                &self.inputs,
                &tables(),
                Some(under.as_bytes()),
                rows,
                bytes,
                &mut changed,
            );
        }
        #[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
        {
            if trial {
                if let Some(c) = clock {
                    let us = c().wrapping_sub(t_start);
                    self.jit.timed(1, drawn, us / rows.count_ones().max(1));
                }
            } else if !drawn {
                self.jit.check(
                    1,
                    &self.prog,
                    &mut b.scratch,
                    b.ink.as_deref_mut(),
                    &self.inputs,
                    Some(under.as_bytes()),
                    bytes,
                );
            }
        }
        if old() {
            let mut r = rows;
            while r != 0 {
                let y = r.trailing_zeros() as usize;
                r &= r - 1;
                dirty.mark_span(y, 0, crate::canvas::WIDTH);
            }
        } else if self.fresh {
            // (Its picture is not what the panel shows: all of it.)
            dirty.mark_all();
        } else {
            // Its picture is what the panel shows but for its map: what the
            // rows drawn changed (a still look over a GIF: the GIF's
            // changed pixels, not their whole rows).
            mark_changed(dirty, &changed);
        }
        under.dirty_mut().clear();
        self.fresh = false;
        // Timed when it drew every row: a frame of its own steers the
        // governor (with the decoding next to it); any other only the mean.
        if let (Some(c), true) = (clock, rows == u32::MAX) {
            let us = c().wrapping_sub(t_start).saturating_add(extra_us);
            if governed && own && self.cold > 0 {
                self.cold -= 1;
            } else if governed && own {
                if self.gov.observe(us, grid) {
                    log::info!(
                        "formula look: {} us a frame -> {} ms frames (asked {grid})",
                        self.gov.avg_us(),
                        self.gov.delay_ms(grid)
                    );
                }
            } else {
                self.gov.note(us);
            }
            LOOK_STATS[0].store(self.gov.avg_us(), Ordering::Relaxed);
            let ms = match (animated, governed) {
                (true, true) => self.gov.delay_ms(grid),
                (true, false) => grid,
                _ => 0,
            };
            LOOK_STATS[1].store(ms, Ordering::Relaxed);
        }
        &mut b.out
    }
}

impl Drop for FormulaLook {
    fn drop(&mut self) {
        // (The look's JIT status is its own: none once it goes.)
        set_jit_status(1, false, "no look", 0);
    }
}

impl std::fmt::Debug for FormulaLook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "FormulaLook({:?}, held {})",
            self.prog,
            self.held.is_some()
        )
    }
}

/// The kill switch (`/api/config` `formula_jit`): off, every formula plays
/// on the VM (a kernel held is let go at its next frame).
static JIT_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Turn the JIT on or off (the firmware, from its settings).
pub fn set_jit(on: bool) {
    JIT_ON.store(on, Ordering::Relaxed);
}

/// Whether the JIT is allowed (the setting).
pub fn jit_allowed() -> bool {
    JIT_ON.load(Ordering::Relaxed)
}

/// What `/api/status` says of the formula on screen and the JIT: (a kernel
/// draws it, why not if not, the kernel's code bytes); `[1]` the look's.
static JIT_STATUS: [std::sync::Mutex<(bool, &'static str, u32)>; 2] = [
    std::sync::Mutex::new((false, "not built in", 0)),
    std::sync::Mutex::new((false, "not built in", 0)),
];

/// The playing formula's JIT status: (on, why not, code bytes).
pub fn jit_status() -> (bool, &'static str, u32) {
    JIT_STATUS[0].lock().map_or((false, "", 0), |s| *s)
}

/// The formula look's JIT status, as [`jit_status`].
pub fn look_jit_status() -> (bool, &'static str, u32) {
    JIT_STATUS[1].lock().map_or((false, "", 0), |s| *s)
}

#[allow(dead_code)]
fn set_jit_status(which: usize, on: bool, why: &'static str, bytes: u32) {
    if let Ok(mut s) = JIT_STATUS[which & 1].lock() {
        *s = (on, why, bytes);
    }
}

#[cfg(all(feature = "psi-jit", target_arch = "xtensa", target_os = "espidf"))]
mod jit {
    use super::{psi, set_jit_status, tables, PsiFx};

    /// A formula's kernel: made at its first frame (never at open: the
    /// HTTP server's open check must not take executable heap), checked
    /// against the VM's frame, timed against it, then drawing if faster.
    /// All-zero is "not made yet". `which`: whose status it reports (0 the
    /// effect, 1 the look).
    #[derive(Default)]
    pub struct Jit {
        kernel: Option<Box<psi::jit::Kernel>>,
        checked: bool,
        /// Frames timed after the check (kernel and VM in turn), and each
        /// one's best render (µs).
        trial: u8,
        best: [u32; 2],
        /// Why the VM draws this formula (`None`: not decided yet).
        why: Option<&'static str>,
        /// When the heap last had no room for the kernel's build (the
        /// clock's µs | 1; 0: not refused).
        gate_tried: u32,
    }

    /// Why, with the switch off.
    const OFF: &str = "off (setting)";
    /// Frames timed, kernel and VM in turn, before the faster is kept.
    const TRIAL: u8 = 8;
    /// Why, when the VM drew the formula faster in the trial.
    const SLOWER: &str = "the VM is faster for this formula";
    /// Why, for a `rate` formula as a look.
    const LOOK_RATE: &str = "a formula with `rate` as a look draws on the VM";
    /// Why, after a self-check found a difference.
    const DIFFERS: &str = "the kernel's bytes differed from the VM's (self-check)";

    impl Jit {
        /// After a live edit: no kernel until the edits have paused for
        /// half a second (the short-heap gate's 1 s wait, started 0.5 s
        /// early), so a burst of edits makes and checks one kernel, not
        /// one per edit; the VM draws meanwhile.
        pub(super) fn settling(which: usize) -> Self {
            set_jit_status(which, false, "being checked", 0);
            let now = super::clock().map_or(0, |c| c());
            Jit {
                gate_tried: now.wrapping_sub(500_000) | 1,
                ..Jit::default()
            }
        }

        /// The kernel's block address, 0 for none (`BUFFERS`).
        pub(super) fn kernel_addr(&self) -> usize {
            self.kernel.as_ref().map_or(0, |k| k.addr())
        }

        /// The kernel, if it draws this frame (a trial's frames kernel and
        /// VM in turn). Not made yet: made now, and the VM draws this frame
        /// (the check reads its bytes).
        pub(super) fn kernel_for(&mut self, which: usize, prog: &psi::Prog) -> Option<&mut psi::jit::Kernel> {
            if !super::jit_allowed() {
                if self.why != Some(OFF) {
                    *self = Jit { why: Some(OFF), ..Jit::default() };
                    set_jit_status(which, false, OFF, 0);
                }
                return None;
            }
            if self.why == Some(OFF) {
                *self = Jit::default();
            }
            // (A `rate` formula as a look draws its slow layers every frame,
            // on the VM: no held picture to draw the fast part over.)
            if which == 1 && prog.rate().is_some() && self.why.is_none() {
                self.why = Some(LOOK_RATE);
                set_jit_status(which, false, LOOK_RATE, 0);
            }
            if self.kernel.is_none() && self.why.is_none() {
                // (A short heap: on the VM for now, asked again a second
                // later.)
                let now = super::clock().map_or(0, |c| c());
                if self.gate_tried != 0 && now.wrapping_sub(self.gate_tried) < 1_000_000 {
                    return None;
                }
                if !super::jit_build_allowed(super::jit_build_bytes(prog)) {
                    self.gate_tried = now | 1;
                    return None;
                }
                self.gate_tried = 0;
                match psi::jit::emit(prog) {
                    Ok(k) => {
                        log::info!("formula: kernel of {} B at {:#x}, to be checked", k.code_bytes(), k.addr());
                        set_jit_status(which, false, "being checked", k.code_bytes() as u32);
                        self.kernel = Some(Box::new(k));
                        self.checked = false;
                    }
                    // A short or fragmented heap mid-build (the lowering
                    // takes its tables fallibly): on the VM for now, asked
                    // again a second later, as when the gate refuses.
                    Err(psi::jit::NO_MEMORY) => {
                        log::info!("formula: on the VM for now ({})", psi::jit::NO_MEMORY);
                        set_jit_status(which, false, psi::jit::NO_MEMORY, 0);
                        self.gate_tried = now | 1;
                    }
                    Err(why) => {
                        log::info!("formula: on the VM ({why})");
                        self.why = Some(why);
                        set_jit_status(which, false, why, 0);
                    }
                }
                return None;
            }
            let trial = self.trial < TRIAL;
            match &mut self.kernel {
                // (The trial's odd frames are the VM's.)
                Some(_) if self.checked && trial && self.trial % 2 == 1 => None,
                Some(k) if self.checked => Some(&mut **k),
                _ => None,
            }
        }

        /// Whether a frame now is one of the trial's (kernel against VM).
        pub(super) fn trialing(&self) -> bool {
            super::jit_allowed() && self.kernel.is_some() && self.checked && self.trial < TRIAL
        }

        /// A trial frame took `us` (the kernel's or the VM's; a look's for
        /// a row, its frames drawing the rows its picture changed): after
        /// [`TRIAL`] of them the faster draws from then on (a formula never
        /// gets slower with the JIT on).
        pub(super) fn timed(&mut self, which: usize, kernel: bool, us: u32) {
            let Some(k) = self.kernel.as_ref() else { return };
            let b = &mut self.best[usize::from(!kernel)];
            *b = (*b).min(us);
            self.trial += 1;
            if self.trial < TRIAL {
                return;
            }
            let [kb, vb] = self.best;
            if kb < vb {
                log::info!("formula: kernel {kb} us a frame against the VM's {vb}: the kernel draws");
                set_jit_status(which, true, "", k.code_bytes() as u32);
            } else {
                log::info!("formula: kernel {kb} us a frame against the VM's {vb}: the VM draws");
                self.kernel = None;
                self.why = Some(SLOWER);
                set_jit_status(which, false, SLOWER, 0);
            }
        }

        /// The program's listing (`Program::listing_line`) after a
        /// mismatch, for the bug report: a few lines to a log record (the
        /// firmware's log ring keeps a record whole up to 1,536 B, and
        /// holds 4 KB), with a pause after each for the console to drain
        /// it, so none is dropped. No listing of the whole on the heap: one
        /// record's lines at a time, taken fallibly (none: no listing).
        /// A mismatch is rare and the VM has the frames meanwhile.
        #[optimize(size)]
        #[inline(never)]
        fn log_listing(prog: &psi::Prog) {
            const RECORD: usize = 1024;
            let mut buf = String::new();
            if buf.try_reserve(RECORD + 128).is_err() {
                log::error!("formula: no memory for the listing");
                return;
            }
            let n = prog.len();
            for k in 0..n {
                let line = prog.listing_line(k);
                if buf.try_reserve(line.len()).is_err() {
                    log::error!("formula: no memory for the rest of the listing");
                    return;
                }
                buf.push_str(&line);
                if buf.len() >= RECORD || k + 1 == n {
                    log::error!("{}", buf.trim_end());
                    buf.clear();
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }

        /// After a VM frame (`frame_bytes`, with `ink` and `under` as it had
        /// them; a look's rows not drawn are what it would draw now: the
        /// same inputs over the same rows of the picture): a kernel not yet
        /// checked is checked against it.
        #[allow(clippy::too_many_arguments)]
        pub(super) fn check(
            &mut self,
            which: usize,
            prog: &psi::Prog,
            scratch: &mut psi::Scratch<{ super::LANES }>,
            ink: Option<&mut psi::Ink>,
            inputs: &psi::Inputs,
            under: Option<&[u8]>,
            frame_bytes: &[u8],
        ) {
            let Some(k) = self.kernel.as_mut().filter(|_| !self.checked) else {
                return;
            };
            match k.check(prog, scratch, ink, inputs, &tables(), under, frame_bytes) {
                Ok(true) => {
                    self.checked = true;
                    self.trial = 0;
                    self.best = [u32::MAX; 2];
                    log::info!("formula: kernel of {} B checked against the VM: the same bytes; timing it", k.code_bytes());
                    set_jit_status(which, false, "being timed against the VM", k.code_bytes() as u32);
                }
                Ok(false) => {} // a checked frame: next time
                Err((row, x)) => {
                    log::error!(
                        "formula: the kernel's row {row} differs from the VM's at x = {x}: playing on the VM; its {} steps:",
                        prog.steps()
                    );
                    Self::log_listing(prog);
                    self.kernel = None;
                    self.why = Some(DIFFERS);
                    set_jit_status(which, false, DIFFERS, 0);
                }
            }
        }
    }

    impl PsiFx {
        /// Draw the frame with the kernel if it draws this one (`true`),
        /// what changed into `changed` when given; else leave it to the VM.
        pub(super) fn jit_render(
            &mut self,
            bytes: &mut [u8],
            changed: Option<&mut [u32; psi::H]>,
            beam: Option<psi::PairSink<'_>>,
        ) -> bool {
            // (A `rate` formula's kernel draws its fast part over the held
            // slow layers: without them, the VM.)
            let rate = self.prog.rate().is_some();
            if rate && self.held.is_none() {
                return false;
            }
            let Some(k) = self.jit.kernel_for(0, &self.prog) else {
                return false;
            };
            // (Its slow rows are already brought up to date: `render`.)
            let under = self.held.as_deref().filter(|_| rate).map(|h| h.picture());
            // (False when the inputs were out of range and the VM drew it:
            // a trial frame counted as the VM's.)
            if let Some(sink) = beam {
                // (Racing the beam: never a `rate` formula's frame.)
                return k.render_beam(
                    &self.prog,
                    &mut self.scratch,
                    self.ink.as_deref_mut(),
                    Some(&mut self.still),
                    &self.inputs,
                    &tables(),
                    None,
                    bytes,
                    changed,
                    sink,
                );
            }
            k.render(
                &self.prog,
                &mut self.scratch,
                self.ink.as_deref_mut(),
                Some(&mut self.still),
                &self.inputs,
                &tables(),
                under,
                u32::MAX,
                bytes,
                changed,
            )
        }

        pub(super) fn jit_check(&mut self, bytes: &[u8]) {
            let rate = self.prog.rate().is_some();
            if rate && self.held.is_none() {
                return;
            }
            let under = self.held.as_deref().filter(|_| rate).map(|h| h.picture());
            self.jit.check(0, &self.prog, &mut self.scratch, self.ink.as_deref_mut(), &self.inputs, under, bytes);
        }
    }
}

mod alloc_box {
    use super::PsiFx;

    /// The one allocation: a `Box<PsiFx>` made zeroed (every field is
    /// integers, so zero is a valid value) and filled in place, so ~10
    /// KB never passes through the stack. `Clone` the same way.
    pub struct PsiBox(Box<PsiFx>);

    impl PsiBox {
        pub fn new() -> PsiBox {
            Self::try_new().unwrap_or_else(|| {
                std::alloc::handle_alloc_error(std::alloc::Layout::new::<PsiFx>())
            })
        }

        /// [`PsiBox::new`], or `None` when the heap has no block for it
        /// (an open refused, not the abort of an infallible `Box`).
        pub fn try_new() -> Option<PsiBox> {
            // SAFETY: `PsiFx` is integers and arrays of integers all the
            // way down (`Program`, `Scratch`, `Inputs`, `u8`) but for its
            // canvas, an `Option<Box<_>>`, whose all-zero value is `None`:
            // all-zero is a valid value of it.
            let mut b = PsiBox(super::hinted(super::ARENA_FX, || unsafe { super::try_zeroed::<PsiFx>() })?);
            b.0.scratch.reset();
            b.0.delay_ms = 20;
            b.0.gov = super::Governor::default();
            Some(b)
        }
    }

    impl Default for PsiBox {
        fn default() -> PsiBox {
            PsiBox::new()
        }
    }

    impl std::ops::Deref for PsiBox {
        type Target = PsiFx;
        fn deref(&self) -> &PsiFx {
            &self.0
        }
    }

    impl std::ops::DerefMut for PsiBox {
        fn deref_mut(&mut self) -> &mut PsiFx {
            &mut self.0
        }
    }

    impl Clone for PsiBox {
        fn clone(&self) -> PsiBox {
            let mut b = PsiBox::new();
            // (As `new`: the heap's refusal is an abort.)
            b.0.prog = self.0.prog.try_clone().unwrap_or_else(|| {
                std::alloc::handle_alloc_error(opendmd_psi::Prog::layout(self.0.prog.store_len()))
            });
            // (A clone that can't have its canvas draws nothing: `ink` 0.)
            b.0.ink = super::canvas_for(&b.0.prog, &mut None).unwrap_or(None);
            b.0.clock = self.0.clock;
            b.0.delay_ms = self.0.delay_ms;
            b.0.inputs = self.0.inputs;
            b.0.gov = self.0.gov;
            b.0.played_ms = self.0.played_ms;
            b.0.played_us = self.0.played_us;
            b
        }
    }

    impl PartialEq for PsiBox {
        fn eq(&self, o: &PsiBox) -> bool {
            self.0.prog == o.0.prog
                && self.0.delay_ms == o.0.delay_ms
                && self.0.inputs == o.0.inputs
        }
    }

    impl Eq for PsiBox {}

    impl std::fmt::Debug for PsiBox {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Psi({:?}, {} ms)", self.0.prog, self.0.delay_ms)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Fx;

    /// Where a formula path's query starts (after `@formula?`).
    const Q: usize = crate::synth::FORMULA.len() + 1;

    fn path(src: &str) -> String {
        super::path(src, "")
    }

    #[test]
    fn with_no_clock_a_formula_that_reads_the_time_does_not_open() {
        let clock = psi::build("gray(minute/60)").unwrap();
        let plain = psi::build("gray(x)").unwrap();
        // The clock known (or coming): both open.
        set_no_clock(false);
        assert!(PsiFx::from_bytes(&clock, "").is_ok());
        // No clock this boot: the one that reads the time is refused with
        // the sentence the playlist's skip logs; the other still opens.
        set_no_clock(true);
        let e = PsiFx::from_bytes(&clock, "").expect_err("refused");
        assert_eq!(e.msg, NEEDS_TIME);
        assert!(PsiFx::from_bytes(&plain, "").is_ok());
        // As a path too (what a playlist's `@formula…` line opens), and in
        // place (`retune_from`): the playing formula stays.
        let mut fx = PsiFx::from_bytes(&plain, "").unwrap();
        assert_eq!(fx.retune_from(&path("gray(second/60)")[Q..], Retune::Edit).err().map(|e| e.msg), Some(NEEDS_TIME.to_string()));
        assert!(fx.retune_from(&path("gray(y)")[Q..], Retune::Edit).is_ok());
        assert!(Fx::parse(&path("gray(hour/24)")).is_none());
        // A look of it likewise.
        let look = psi::build("look screen 40\ngray(minute/60)").unwrap();
        assert_eq!(FormulaLook::from_bytes(&look, "").err().map(|e| e.msg), Some(NEEDS_TIME.to_string()));
        set_no_clock(false);
        assert!(FormulaLook::from_bytes(&look, "").is_ok());
    }

    #[test]
    fn core_lends_the_same_tables_psi_has() {
        let t = tables();
        assert_eq!(t.sin, &psi::SIN);
        for i in 0..5 {
            assert_eq!(t.palettes[i], &psi::PALETTES[i], "palette {i}");
        }
        // The very tables `text` draws with (Psi's own copy is the same
        // file, `include!`d).
        assert!(std::ptr::eq(t.small, &crate::text::FONT_4X6));
        assert!(std::ptr::eq(t.normal, &crate::text::FONT_5X7));
        assert_eq!(t.small, &psi::FONT_4X6);
        assert_eq!(t.normal, &psi::FONT_5X7);
        assert_eq!(t.bold, &psi::FONT_BOLD);
    }

    /// A formula that never repeats plays in passes of a second (the
    /// engine looks at the dwell after each), its `t` going on across them
    /// as the frame times it returned add up, the VM's frame at that `t`
    /// and the open's `seed`; a live edit keeps the time.
    #[test]
    fn a_formula_that_never_repeats_goes_on_across_passes() {
        let src = "loop forever\nu = t*0.15 + seed*100\ngray(noise(u + x*4, y*2, 0) * frac(t*3))";
        let mut fx = Fx::parse(&path(src)).unwrap();
        let Fx::Psi(b) = &mut fx;
        b.set_seed(12_345);
        assert_eq!(fx.loop_frames(), PASS_MS / 20);
        assert_eq!(fx.loop_time(), Duration::from_millis(u64::from(PASS_MS)));
        let prog = psi::compile(src).unwrap();
        let (mut c, mut want) = (Canvas::new(), vec![0u8; psi::W * psi::H * 3]);
        let mut s = psi::Scratch::<128>::new();
        let mut played = 0u64;
        // Three and a half passes, as SynthSource plays them: each pass's
        // time from 0 (its rewind), the formula's own going on.
        for _pass in 0..3 {
            let mut el = Duration::ZERO;
            while el < fx.loop_time() {
                let d = fx.frame(el, &mut c);
                let inputs = psi::Inputs {
                    tw: psi::Inputs::wide_ms(played),
                    seed: 12_345,
                    ..Default::default()
                };
                psi::render(&prog, &mut s, &inputs, &psi::OWN, &mut want);
                assert!(c.as_bytes() == want.as_slice(), "at {played} ms");
                el += d;
                played += d.as_millis() as u64;
            }
        }
        assert_eq!(played, 3000);
        // A live edit: the time goes on.
        let Fx::Psi(b) = &mut fx;
        b.retune_from(&path("loop forever\ngray(frac(t/10))")[crate::synth::FORMULA.len() + 1..], Retune::Edit)
            .unwrap();
        fx.frame(Duration::ZERO, &mut c);
        assert_eq!(
            c.as_bytes()[0],
            psi::fixed::chan(psi::fixed::w_frac(psi::fixed::w_div(
                psi::Inputs::wide_ms(3000),
                10 << 16
            )))
        );
        // A new open is a new seed.
        let mut seeds = std::collections::HashSet::new();
        for _ in 0..8 {
            let Some(Fx::Psi(b)) = Fx::parse(&path(src)) else {
                panic!()
            };
            seeds.insert(b.inputs.seed);
            assert!((0..psi::Q16_ONE).contains(&b.inputs.seed));
        }
        assert!(seeds.len() >= 7, "{seeds:?}");
    }

    /// `mark_changed` marks every pixel of every changed group, whatever
    /// it chooses (spans, a whole row, a whole pair), and nothing in a row
    /// or pair that did not change.
    #[test]
    fn marking_what_changed_covers_it() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for k in 0..4000 {
            let mut ch = [0u32; psi::H];
            for (y, m) in ch.iter_mut().enumerate() {
                let r = next();
                *m = match (r % 7, k % 3) {
                    (0, _) => 0,
                    (1, _) => u32::MAX,
                    (2, _) => (r >> 8) as u32 & (r >> 40) as u32,
                    (3, _) => 1 << ((r >> 8) % 32),
                    (_, 0) => (r >> 8) as u32,
                    _ => ((r >> 8) as u32).rotate_left(y as u32) & 0x00ff_ff00,
                };
            }
            let mut d = crate::canvas::DirtyMap::default();
            mark_changed(&mut d, &ch);
            for y in 0..psi::H {
                let pair = ch[y % 16] | ch[y % 16 + 16];
                for x in 0..psi::W {
                    if ch[y] & (1 << (x / 4)) != 0 {
                        assert!(d.is_dirty(x, y), "case {k}: ({x}, {y}) changed, not marked");
                    }
                }
                if ch[y] == 0 && pair == 0 {
                    assert!(d.row_spans(y).next().is_none(), "case {k}: row {y} marked");
                }
            }
        }
        // A whole pair through one write; a few pixels as one span.
        let mut ch = [0u32; psi::H];
        ch[2] = u32::MAX;
        ch[18] = u32::MAX >> 4;
        ch[5] = 0b110;
        let mut d = crate::canvas::DirtyMap::default();
        mark_changed(&mut d, &ch);
        assert_eq!(d.row_spans(18).collect::<Vec<_>>(), [(0, 128)]);
        assert_eq!(d.row_spans(5).collect::<Vec<_>>(), [(4, 12)]);
        assert_eq!(d.count(), 256 + 8);
    }

    /// Racing the beam: over one late frame in a window of 1,024, every 2nd
    /// refresh for 4,096 frames, then every refresh again — for a formula
    /// near its budget (here a mean of 1,000 µs against 1,200).
    #[test]
    fn late_raced_frames_hold_every_second_refresh_for_a_while() {
        const NEAR: u32 = 1_200;
        let mut g = Governor::new(1_000, 10);
        assert_eq!(g.beam_frames(1), 1);
        // The first frames after an open are cold: late or not, not counted.
        for _ in 0..Governor::LATE_GRACE {
            assert!(!g.note_late(true, NEAR));
        }
        assert_eq!(g.beam_frames(1), 1);
        // One late frame a window: fine.
        for k in 0..3 * Governor::LATE_WINDOW {
            assert!(!g.note_late(k % Governor::LATE_WINDOW == 7, NEAR));
        }
        assert_eq!(g.beam_frames(1), 1);
        assert!(!g.note_late(true, NEAR));
        assert!(g.note_late(true, NEAR), "two in a window");
        assert_eq!(g.beam_frames(1), 2);
        for _ in 0..Governor::LATE_HOLD {
            assert_eq!(g.beam_frames(1), 2);
            g.note_late(false, NEAR);
        }
        assert_eq!(g.beam_frames(1), 1, "tried again");
    }

    /// A formula well under its budget keeps every refresh through late
    /// frames that come from outside it (a stall: a flash write on CPU0,
    /// the loop's work before the tick), however they bunch; one near its
    /// budget is still held by two in a window. A test board's numbers:
    /// Starfield ~3,300 µs of a 6,468 µs budget; a formula at 5,500.
    #[test]
    fn only_a_formula_near_its_budget_is_held_for_late_frames() {
        const BUDGET: u32 = 6_468;
        let mut light = Governor::new(3_300, 10);
        for k in 0..20 * Governor::LATE_WINDOW as u32 {
            // Two late in a row every 300 frames: 0.7 %, seven times the
            // test board's rate under its measurement's HTTP.
            assert!(!light.note_late(k % 300 < 2, BUDGET), "frame {k}");
        }
        assert_eq!(light.beam_frames(1), 1, "light: every refresh");
        let mut near = Governor::new(5_500, 20);
        for _ in 0..Governor::LATE_GRACE {
            near.note_late(false, BUDGET);
        }
        assert!(!near.note_late(true, BUDGET));
        assert!(near.note_late(true, BUDGET), "near its budget: held");
        assert_eq!(near.beam_frames(1), 2);
        // The edge: three quarters of the budget.
        assert_eq!(Governor::LATE_NEAR_PCT, 75);
        let mut at = Governor::new(BUDGET * 3 / 4, 10);
        for _ in 0..Governor::LATE_GRACE + 2 {
            at.note_late(true, BUDGET);
        }
        assert_eq!(at.beam_frames(1), 1, "at three quarters: not yet");
    }

    /// A Formula played over one whose raced frames were held to every 2nd
    /// refresh (late frames) starts on its own grid: `/api/play` of a
    /// Formula over a Formula is a retune, and the hold was the old one's.
    /// A heavier one still starts at its own (slower) level.
    #[test]
    fn a_formula_played_over_a_held_one_starts_on_its_own_grid() {
        const ROAD: &str = include_str!("../../opendmd-web/formulas/endless-road.psi");
        const SCAN: &str = include_str!("../../opendmd-web/formulas/scanlines.psi");
        let mut fx = PsiFx::from_query(&super::path(ROAD, "")[Q..]).unwrap();
        // (Its first frames are the grace's: cold, not counted. A budget
        // it is near: late frames count.)
        const NEAR: u32 = 5_000;
        for _ in 0..Governor::LATE_GRACE {
            assert!(!fx.gov.note_late(true, NEAR));
        }
        assert_eq!(fx.gov.beam_frames(1), 1, "late in its grace: not held");
        assert!(!fx.gov.note_late(true, NEAR));
        assert!(fx.gov.note_late(true, NEAR));
        assert_eq!(fx.gov.beam_frames(1), 2, "the road held");
        fx.retune_from(&super::path(SCAN, "")[Q..], Retune::Play).unwrap();
        assert_eq!((fx.ms(), fx.gov.delay_ms(fx.ms())), (20, 20));
        assert_eq!(fx.gov.beam_frames(2), 2, "every 2nd refresh: its 50 fps, not 25");
        // One late frame after the retune is not two in a window.
        assert!(!fx.gov.note_late(true, NEAR));
        assert_eq!(fx.gov.beam_frames(2), 2);
        // Heavier: its own start (about 9 ms, two noises: 40 ms frames).
        let heavy = "loop 4\ngray(noise(x*8, y*4, t) + noise(x*3, y*9, t*2))";
        fx.retune_from(&super::path(heavy, "")[Q..], Retune::Play).unwrap();
        assert_eq!(fx.gov.delay_ms(fx.ms()), 40);
    }

    /// The bug (1.1.0-beta.2, on a test panel): a light `smooth` Formula
    /// (~1-2 ms a frame) played with `/api/play` over a heavy one on screen
    /// (`smooth`, 9 noises, ~35.5 ms: level 3) kept level 3, every 8th
    /// refresh, then sped up one level per [`Governor::CALM`] frames:
    /// seconds at a fraction of its rate. Played over another Formula it
    /// starts from its own estimate, as after a GIF, both ways.
    #[test]
    fn a_formula_played_over_another_starts_from_its_own_estimate() {
        let heavy = Governor::new(35_500, 10);
        let light = Governor::new(1_500, 10);
        assert_eq!((heavy.delay_ms(10), heavy.beam_frames(1)), (80, 8));
        assert_eq!((light.delay_ms(10), light.beam_frames(1)), (10, 1));
        // Heavy, then light played over it: every refresh at once.
        let mut g = heavy;
        g.retune(light, Retune::Play);
        assert_eq!((g.delay_ms(10), g.beam_frames(1)), (10, 1));
        assert_eq!(g, light, "its own start entire");
        // Light, then heavy: slow from its first frame.
        let mut g = light;
        g.retune(heavy, Retune::Play);
        assert_eq!((g.delay_ms(10), g.beam_frames(1)), (80, 8));
        // A step between: 40 ms frames on screen, a 6 ms one played over
        // it takes its own 20, not the old level's 40.
        let mut g = Governor::new(9_000, 10);
        assert_eq!(g.delay_ms(10), 40);
        g.retune(Governor::new(6_000, 10), Retune::Play);
        assert_eq!(g.delay_ms(10), 20);
    }

    /// A live edit is the same Formula's frames: the level goes on (slower
    /// at once where the new estimate says so) and speeds up a level per
    /// [`Governor::CALM`] calm frames, as before.
    #[test]
    fn a_live_edit_keeps_the_governors_level() {
        let heavy = Governor::new(35_500, 10);
        let light = Governor::new(1_500, 10);
        let mut g = heavy;
        g.retune(light, Retune::Edit);
        assert_eq!(
            (g.delay_ms(10), g.beam_frames(1)),
            (80, 8),
            "the level goes on"
        );
        assert_eq!(g.avg_us(), 1_500, "the mean is the new estimate's");
        // Its cold frames left out, then one level per CALM calm frames.
        for _ in 0..Governor::SKIP {
            assert!(!g.observe(1_500, 10));
        }
        for level in (0..Governor::MAX_LEVEL).rev() {
            for k in 1..=Governor::CALM {
                let changed = g.observe(1_500, 10);
                assert_eq!(changed, k == Governor::CALM, "level {level}, frame {k}");
            }
            assert_eq!(g.delay_ms(10), 10 << level);
        }
        // Edited heavier: slower at once.
        let mut g = light;
        g.retune(heavy, Retune::Edit);
        assert_eq!(g.delay_ms(10), 80);
    }

    /// The same through [`PsiFx::retune_from`]: a heavy `smooth` Formula
    /// (9 moving noises) on screen, a light `smooth` one played over it
    /// races every refresh at once; the other way round, slow at once; a
    /// live edit of the heavy one to the light one keeps the heavy one's
    /// level.
    #[test]
    fn a_light_formula_played_over_a_heavy_one_races_every_refresh() {
        let noises = (1..=9)
            .map(|k| format!("noise(x*{k}, y*{}, t*{k})", 10 - k))
            .collect::<Vec<_>>()
            .join(" + ");
        let heavy = format!("smooth\nloop 4\ngray(({noises}) / 9)");
        let light = "smooth\nloop 1\ngray(abs(x - phase) < 0.02)";
        let open = |src: &str| PsiFx::from_query(&super::path(src, "")[Q..]).unwrap();
        // (Its frame time asked, the governor's, and the refreshes a raced
        // picture is held for on a test panel's 8 ms refresh: `refreshes`.)
        const CYCLE: u32 = 8_000;
        let level = |fx: &PsiFx| {
            let unit = crate::scan::beam_frames(fx.ms() * 1000, CYCLE);
            (
                fx.ms(),
                fx.governor().delay_ms(fx.ms()),
                fx.governor().beam_frames(unit),
            )
        };
        let est = open(&heavy).governor().avg_us();
        assert!(est > Governor::budget_us(80), "heavy: {est} us");
        // (Past 32 ms a `smooth` one asks every 2nd refresh: 20 ms frames,
        // held to 8 of them, every 16th refresh, as on the panel.)
        assert_eq!(level(&open(&heavy)), (20, 160, 16));
        assert_eq!(level(&open(light)), (10, 10, 1));
        // Played over: its own start.
        let mut fx = open(&heavy);
        fx.retune_from(&super::path(light, "")[Q..], Retune::Play)
            .unwrap();
        assert_eq!(level(&fx), (10, 10, 1), "every refresh, as after a GIF");
        let mut fx = open(light);
        fx.retune_from(&super::path(&heavy, "")[Q..], Retune::Play)
            .unwrap();
        assert_eq!(level(&fx), (20, 160, 16), "slow from its first frame");
        // A live edit of the heavy one to the light one: the level goes on.
        let mut fx = open(&heavy);
        fx.retune_from(&super::path(light, "")[Q..], Retune::Edit)
            .unwrap();
        assert_eq!(
            level(&fx),
            (10, 80, 8),
            "the level goes on: every 8th refresh"
        );
    }

    thread_local! {
        static PANEL: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(vec![0; psi::W * psi::H * 3]);
        static TOLD: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
        static SLOW: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        static STARTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    fn slow_clock() -> u32 {
        SLOW.with(|n| {
            n.set(n.get().wrapping_add(100));
            n.get()
        })
    }

    fn beam_begin() -> Option<u32> {
        Some(7_980)
    }

    fn beam_start(_est: u32, _frames: u32) {
        TOLD.with(|t| assert!(t.borrow().is_empty(), "the start after a pair"));
        STARTS.with(|n| n.set(n.get() + 1));
    }

    /// The panel's side: the marked pixels of rows p and p + 16 copied.
    fn beam_pair(p: usize, bytes: &[u8], dirty: &crate::canvas::DirtyMap) {
        TOLD.with(|t| t.borrow_mut().push(p));
        PANEL.with(|pn| {
            let mut pn = pn.borrow_mut();
            for y in [p, p + psi::H / 2] {
                for (x0, x1) in dirty.row_spans(y) {
                    let r = (y * psi::W + x0) * 3..(y * psi::W + x1) * 3;
                    pn[r.clone()].copy_from_slice(&bytes[r]);
                }
            }
        });
    }

    fn beam_end() -> (i32, u32, bool) {
        (0, 0, false)
    }

    /// Racing the beam: from the second frame each pair is written as it
    /// is drawn, in the panel's order, and what was written makes the
    /// panel the canvas; nothing is left for the blit; the frames are the
    /// ones drawn at once (on the grid's times), and the delay is the
    /// refresh.
    #[test]
    fn a_raced_formula_writes_each_pair_as_it_is_drawn() {
        const ROAD: &str = include_str!("../../opendmd-web/formulas/endless-road.psi");
        const STARS: &str = include_str!("../../opendmd-web/formulas/starfield.psi");
        const GATE: &str = include_str!("../../opendmd-web/formulas/time-gate.psi");
        set_clock(Some(slow_clock));
        set_beam(Some(BeamHooks { begin: beam_begin, start: beam_start, pair: beam_pair, end: beam_end }));
        for (name, src) in psi::sketches::ALL.iter().copied().chain([
            ("Endless road", ROAD),
            ("Starfield", STARS),
            ("Time gate", GATE),
        ]) {
            let mut raced = Fx::parse(&path(src)).unwrap_or_else(|| panic!("{name}"));
            let mut plain = Fx::parse(&path(src)).unwrap();
            let (Fx::Psi(a), Fx::Psi(b)) = (&mut raced, &mut plain);
            a.set_seed(12_345);
            b.set_seed(12_345);
            // (One that never repeats has its own time, `tw`: the frame
            // times it returned, 7.98 ms raced; its bytes are the psi
            // tests'.)
            let forever = a.program().forever();
            let (mut c, mut want) = (Canvas::new(), Canvas::new());
            for k in 0..30u32 {
                let t = plain.delay() * k;
                arm_beam(false);
                plain.frame(t, &mut want);
                want.dirty_mut().clear();
                arm_beam(true);
                TOLD.with(|t| t.borrow_mut().clear());
                let d = raced.frame(t, &mut c);
                assert!(forever || c.as_bytes() == want.as_bytes(), "{name} frame {k}: the bytes");
                if k == 0 {
                    // Not raced: the blit's.
                    assert!(TOLD.with(|t| t.borrow().is_empty()));
                    PANEL.with(|p| p.borrow_mut().copy_from_slice(c.as_bytes()));
                    c.dirty_mut().clear();
                    continue;
                }
                assert_eq!(TOLD.with(|t| t.borrow().clone()), (0..16).collect::<Vec<_>>(), "{name} frame {k}");
                assert_eq!(STARTS.with(|n| n.replace(0)), 1, "{name} frame {k}: started once");
                assert!(c.dirty().is_clean(), "{name} frame {k}: left for the blit");
                assert!(PANEL.with(|p| p.borrow().as_slice() == c.as_bytes()), "{name} frame {k}: the panel");
                assert_eq!(d, Duration::from_micros(7_980 << raced_level(&raced)), "{name} frame {k}");
            }
        }
        a_rate_formula_is_not_raced_and_keeps_its_grid();
        starfields_grid_does_not_depend_on_what_played_before();
        raced_loops_wrap_at_their_length_through_the_engine();
        arm_beam(false);
        set_beam(None);
        set_clock(None);
    }

    fn raced_level(fx: &Fx) -> u32 {
        let _ = fx;
        u32::from(stats().1 > 12)
    }

    thread_local! {
        static COLD: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    /// The panel's end of a raced frame whose first frames after an open
    /// come in late (cold caches): `COLD` of them.
    fn beam_end_cold() -> (i32, u32, bool) {
        let late = COLD.with(|c| {
            let n = c.get();
            c.set(n.saturating_sub(1));
            n > 0
        });
        (0, 0, late)
    }

    /// Starfield (100 fps) plays every refresh whether it opens fresh or is
    /// played over Glitter or Old photo (a retune), even when its first
    /// raced frames run late from cold caches. (They counted: two of them
    /// held it to every 2nd refresh for a minute — a test board after a reboot
    /// and over Glitter, not over Old photo.)
    /// (Called from `a_raced_formula_writes_each_pair_as_it_is_drawn`: the
    /// hooks are the process's.)
    fn starfields_grid_does_not_depend_on_what_played_before() {
        const STARS: &str = include_str!("../../opendmd-web/formulas/starfield.psi");
        const GLITTER: &str = include_str!("../../opendmd-web/formulas/glitter.psi");
        const PHOTO: &str = include_str!("../../opendmd-web/formulas/old-photo.psi");
        set_beam(Some(BeamHooks { begin: beam_begin, start: beam_start, pair: beam_pair, end: beam_end_cold }));
        let run = |fx: &mut Fx, from: u32| -> Vec<Duration> {
            let mut c = Canvas::new();
            arm_beam(true);
            (from..from + 60)
                .map(|k| {
                    TOLD.with(|t| t.borrow_mut().clear());
                    STARTS.with(|n| n.set(0));
                    fx.frame(Duration::from_millis(u64::from(k) * 8), &mut c)
                })
                .collect()
        };
        let mut grids = Vec::new();
        for before in [None, Some(GLITTER), Some(PHOTO)] {
            let mut fx = Fx::parse(&path(before.unwrap_or(STARS))).unwrap();
            let mut from = 0;
            if before.is_some() {
                run(&mut fx, 0);
                from = 60;
                let Fx::Psi(b) = &mut fx;
                b.retune_from(&super::path(STARS, "")[Q..], Retune::Play).unwrap();
            }
            COLD.with(|c| c.set(3));
            run(&mut fx, from);
            // Then two late frames well after its grace, a frame apart
            // (a stall from outside: Starfield is far under its budget).
            COLD.with(|c| c.set(2));
            let d = run(&mut fx, from + 60);
            grids.push(*d.last().unwrap());
        }
        assert_eq!(grids, [Duration::from_micros(7_980); 3], "fresh, over Glitter, over Old photo");
        COLD.with(|c| c.set(0));
        set_beam(Some(BeamHooks { begin: beam_begin, start: beam_start, pair: beam_pair, end: beam_end }));
    }

    /// A `rate` formula with the beam's hooks armed is not raced: its
    /// frames go up whole through the blit on its own grid (Storm front's
    /// 20 ms), as without the hooks. (Raced, beam-on held it to 16 or 32
    /// ms frames on the refresh grid and its starts slipped: a test board's
    /// 213 misses a minute.)
    /// (Called from `a_raced_formula_writes_each_pair_as_it_is_drawn`, with
    /// its clock and hooks: they are the process's, so one test.)
    fn a_rate_formula_is_not_raced_and_keeps_its_grid() {
        const STORM: &str = include_str!("../../opendmd-web/formulas/storm-front.psi");
        let mut raced = Fx::parse(&path(STORM)).unwrap();
        let mut plain = Fx::parse(&path(STORM)).unwrap();
        let (Fx::Psi(a), Fx::Psi(b)) = (&mut raced, &mut plain);
        assert!(a.program().rate().is_some());
        a.set_seed(7);
        b.set_seed(7);
        let (mut c, mut want) = (Canvas::new(), Canvas::new());
        for k in 0..30u32 {
            let t = plain.delay() * k;
            arm_beam(false);
            let dw = plain.frame(t, &mut want);
            arm_beam(true);
            TOLD.with(|t| t.borrow_mut().clear());
            let d = raced.frame(t, &mut c);
            assert!(TOLD.with(|t| t.borrow().is_empty()), "frame {k}: raced");
            assert_eq!(STARTS.with(|n| n.replace(0)), 0, "frame {k}: started");
            assert_eq!(c.as_bytes(), want.as_bytes(), "frame {k}: the bytes");
            assert!(k == 0 || !c.dirty().is_clean(), "frame {k}: nothing left for the blit");
            assert_eq!((d, dw), (Duration::from_millis(20), Duration::from_millis(20)), "frame {k}: its grid");
            c.dirty_mut().clear();
            want.dirty_mut().clear();
        }
    }

    /// A Formula played as an effect marks every pixel its frame changed
    /// (what the canvas held before is what the panel shows, but for the
    /// map), and a frame that changes little marks little.
    #[test]
    fn a_formula_marks_what_its_frame_changed() {
        const STARS: &str = include_str!("../../opendmd-web/formulas/starfield.psi");
        const ROAD: &str = include_str!("../../opendmd-web/formulas/endless-road.psi");
        const CLOCK: &str = include_str!("../../opendmd-web/formulas/big-clock.psi");
        const GLITTER: &str = include_str!("../../opendmd-web/formulas/glitter.psi");
        let mut few = 0;
        for (name, src) in psi::sketches::ALL.iter().copied().chain([
            ("Starfield", STARS),
            ("Endless road", ROAD),
            ("Big clock", CLOCK),
            ("Glitter", GLITTER),
        ]) {
            let mut fx = Fx::parse(&path(src)).unwrap_or_else(|| panic!("{name}"));
            let mut c = Canvas::new();
            let delay = fx.delay();
            let mut marked = 0;
            for k in 0..24u32 {
                let before = c.as_bytes().to_vec();
                c.dirty_mut().clear();
                fx.frame(delay * k, &mut c);
                let now = c.as_bytes();
                for y in 0..psi::H {
                    for x in 0..psi::W {
                        let o = (y * psi::W + x) * 3;
                        if before[o..o + 3] != now[o..o + 3] {
                            assert!(
                                c.dirty().is_dirty(x, y),
                                "{name} frame {k}: ({x}, {y}) not marked"
                            );
                        }
                    }
                }
                marked += c.dirty().count();
            }
            if marked < 24 * 4096 / 2 {
                few += 1;
            }
            eprintln!("{name}: {} px marked a frame", marked / 24);
        }
        assert!(few >= 3, "only {few} formulas marked under half the panel");
    }

    /// A `rate` formula plays with its slow layers held: every frame the
    /// bytes drawn from nothing, what changed marked, frames skipped too.
    #[test]
    fn a_rate_formula_plays_its_slow_layers_held() {
        let src = "loop 2\nlayer rate 4 { hsv(x + t/3, 1, frac(y + t)) }\nlayer add { gray(px == floor(t*60) % 128) }";
        let mut fx = PsiFx::from_query(&path(src)[Q..]).unwrap();
        let mut c = Canvas::new();
        let mut ms = 0u64;
        for k in 0..60u64 {
            ms += 20 * (1 + u64::from(k % 13 == 5) * 3);
            let before = c.as_bytes().to_vec();
            c.dirty_mut().clear();
            fx.frame(Duration::from_millis(ms), &mut c);
            assert!(fx.held.is_some(), "frame {k}: no held picture");
            let mut want = vec![0u8; psi::W * psi::H * 3];
            psi::render_rows::<LANES>(&fx.prog, &mut psi::Scratch::new(), &fx.inputs, &tables(), None, u32::MAX, &mut want);
            assert_eq!(c.as_bytes(), &want[..], "frame {k} at {ms} ms");
            for y in 0..psi::H {
                for x in 0..psi::W {
                    let o = (y * psi::W + x) * 3;
                    if before[o..o + 3] != want[o..o + 3] {
                        assert!(c.dirty().is_dirty(x, y), "frame {k}: ({x}, {y}) not marked");
                    }
                }
            }
        }
    }

    /// A Formula that changes every pixel stops being compared after
    /// `FULL_RUN` frames (marks all, checked again every `RECHECK`th), and
    /// one that changes little is always compared.
    #[test]
    fn a_formula_that_changes_everything_is_not_compared() {
        let mut fx = Fx::parse(&path("loop 1\nrgb(frac(t*7 + x), frac(y - t*3), 0.5)")).unwrap();
        let mut c = Canvas::new();
        let mut counts = Vec::new();
        for k in 0..80u32 {
            c.dirty_mut().clear();
            fx.frame(Duration::from_millis(u64::from(k) * 20), &mut c);
            counts.push(c.dirty().count());
        }
        assert!(counts.iter().all(|&n| n == 4096), "{counts:?}");
        let Fx::Psi(p) = &fx;
        assert!(p.full >= FULL_RUN && p.uncompared < RECHECK);
        // A still one: every frame compared, nothing marked after the first.
        let mut fx = Fx::parse(&path("gray(x)")).unwrap();
        fx.frame(Duration::ZERO, &mut c);
        for k in 1..40u32 {
            c.dirty_mut().clear();
            fx.frame(Duration::from_millis(u64::from(k) * 20), &mut c);
            assert!(c.dirty().is_clean(), "frame {k}");
        }
    }

    #[test]
    fn a_formula_plays_as_an_effect() {
        for (name, src) in psi::sketches::ALL {
            let mut fx = Fx::parse(&path(src)).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(fx.name(), "psi");
            assert_eq!(fx.title(), "Formula");
            let prog = psi::compile(src).unwrap();
            assert_eq!(fx.loop_frames(), prog.loop_ms() / 20);
            let mut c = Canvas::new();
            // Frame 7 at 50 fps: t = 140 ms; no wall clock in tests.
            fx.frame(Duration::from_millis(140), &mut c);
            let mut want = vec![0u8; psi::W * psi::H * 3];
            let mut s = psi::Scratch::<128>::new();
            let inputs = psi::Inputs {
                t: psi::Inputs::at_ms(140),
                ..Default::default()
            };
            psi::render(&prog, &mut s, &inputs, &psi::OWN, &mut want);
            assert!(c.as_bytes() == want.as_slice(), "{name}");
        }
    }

    #[test]
    fn options_and_errors() {
        let p = super::path("rgb(a, b, c)", "a=0.5&b=1&c=.25&fps=100");
        let Some(Fx::Psi(fx)) = Fx::parse(&p) else {
            panic!("{p}")
        };
        assert_eq!(fx.inputs.a, 1 << 15);
        assert_eq!(fx.inputs.b, 1 << 16);
        assert_eq!(fx.inputs.c, 1 << 14);
        assert_eq!(fx.delay(), Duration::from_millis(10));
        assert!(Fx::parse(&path("gray(tme)")).is_none());
        assert!(Fx::parse("@formula").is_none());
        assert!(Fx::parse("@fx:psi?src=gray(x)").is_none());
        assert!(Fx::parse(&path("loop 2\ngray(x)")).is_some());
        assert_eq!(
            PsiFx::from_query("src=gray(tme)").unwrap_err().msg,
            psi::psb::NOT_PSB
        );
        // A clone is equal and renders the same.
        let a = Fx::parse(&path(psi::sketches::PLASMA)).unwrap();
        assert_eq!(a.delay(), Duration::from_millis(20), "50 fps by default");
        assert_eq!(a.clone(), a);
    }

    #[test]
    fn the_formulas_fps_line_sets_its_grid_and_the_path_wins() {
        let delay = |src: &str, opts: &str| {
            let Some(Fx::Psi(fx)) = Fx::parse(&super::path(src, opts)) else {
                panic!("{src}")
            };
            (fx.delay(), fx.loop_time())
        };
        const F: &str = "fps 100\nloop 2\nhsv(x + t/2, 1, 1)";
        let ms = Duration::from_millis;
        assert_eq!(delay(F, ""), (ms(10), ms(2000)), "`fps 100`: a 10 ms grid");
        assert_eq!(delay(F, "a=0.5"), (ms(10), ms(2000)));
        assert_eq!(delay(F, "fps=50").0, ms(20), "the path's `fps=` wins");
        assert_eq!(delay(F, "fps=25").0, ms(40));
        assert_eq!(delay("fps 25\ngray(x)", "").0, ms(40));
        assert_eq!(delay("fps 50\ngray(x)", "fps=100").0, ms(10));
        // A frame on the 10 ms grid is the VM's frame at that time.
        let Some(Fx::Psi(mut fx)) = Fx::parse(&super::path(F, "")) else {
            panic!()
        };
        assert_eq!(fx.loop_frames(), 200);
        let mut c = Canvas::new();
        fx.frame(Duration::from_millis(130), &mut c);
        let prog = psi::compile(F).unwrap();
        let mut want = vec![0u8; psi::W * psi::H * 3];
        let inputs = psi::Inputs {
            t: psi::Inputs::at_ms(130),
            ..Default::default()
        };
        psi::render(
            &prog,
            &mut psi::Scratch::<128>::new(),
            &inputs,
            &psi::OWN,
            &mut want,
        );
        assert!(c.as_bytes() == want.as_slice());
        // Too heavy for the governor every refresh (over 32 ms) but not
        // every 2nd: opened at every 2nd (20 ms), as with `fps=50`, not
        // refused; the same `fps=100`. Past 40 ms: refused at any rate.
        let mid = (0..9)
            .map(|k| format!("noise(x*{k}, y, t)"))
            .collect::<Vec<_>>()
            .join(" + ");
        let mid = format!("smooth\ngray({mid})");
        for opts in ["", "fps=100", "fps=50"] {
            let fx = PsiFx::from_query(&super::path(&mid, opts)[Q..]).unwrap();
            assert_eq!(fx.ms(), 20, "{opts:?}");
        }
        // The same under the glow (its 20 ms floor): it opens at 20 by its
        // own rate, so it keeps it when the glow goes off.
        crate::fx::set_frame_floor(20);
        let fx = PsiFx::from_query(&super::path(&mid, "")[Q..]).unwrap();
        assert_eq!(fx.delay_ms, 20, "under the glow");
        crate::fx::set_frame_floor(0);
        assert_eq!(fx.ms(), 20, "the glow off");
        let big = (0..12)
            .map(|k| format!("noise(x*{k}, y, t)"))
            .collect::<Vec<_>>()
            .join(" + ");
        assert!(PsiFx::from_query(&super::path(&format!("smooth\ngray({big})"), "")[Q..])
            .unwrap_err()
            .msg
            .ends_with("the limit is 40 ms) — simplify it"));
        // Over its 5 ms budget, `fps 100` starts from the estimate: two
        // moving noises (~9.1 ms) at 40 ms frames before its first; a
        // tunnel of noise from its estimate with the still cache (~6.8 ms,
        // 15.9 without) at 20, and at 40 when the gate would refuse the
        // cache. (The ring clock was this one until psi-speed: 4.x ms now.)
        let heavy = "fps 100\nloop 4\ngray(noise(x*8, y*4, t) + noise(x*3, y*9, t*2))";
        let fx = PsiFx::from_query(&super::path(heavy, "")[Q..]).unwrap();
        assert_eq!((fx.ms(), fx.gov.delay_ms(fx.ms())), (10, 40));
        let ring = "fps 100\nloop 4\nd = 1/len(px - 63.5, (15.5 - py)*2)\nr = atan2(15.5 - py, px - 63.5)\ngray(noise(d*8 + t, r*4, t) + sin(d*3 - t)*0.2)";
        let fx = PsiFx::from_query(&super::path(ring, "")[Q..]).unwrap();
        assert_eq!((fx.ms(), fx.gov.delay_ms(fx.ms())), (10, 20));
        set_still_gate(Some(|_| false));
        let fx = PsiFx::from_query(&super::path(ring, "")[Q..]).unwrap();
        assert_eq!((fx.ms(), fx.gov.delay_ms(fx.ms())), (10, 40));
        set_still_gate(None);
        // Over a GIF, a look follows its path's `fps=` (else 50), never the
        // formula's line: it is drawn at the GIF's frames.
        let look = "fps 100\nloop 4\nlayer screen { gray(x + t/4, 0.4) }";
        assert_eq!(
            FormulaLook::from_path(&super::path(look, "")).unwrap().ms(),
            20
        );
        assert_eq!(
            FormulaLook::from_path(&super::path(look, "fps=100"))
                .unwrap()
                .ms(),
            10
        );
    }

    #[test]
    fn a_sent_formula_plays_whatever_its_length() {
        // Endless road V2 (22 KB of text): its compiled form is past any
        // path, so it is held in memory and named `@formula:~sent`.
        const V2: &str = include_str!("../../opendmd-web/experiments/road-v2/road-v2.psi");
        let psb = psi::build(V2).unwrap();
        assert!(path_of(&psb, "").len() > crate::synth::MAX_FORMULA_PATH);
        crate::synth::set_sent(Some(psb.clone()));
        let rest = &crate::synth::SENT[crate::synth::FORMULA.len()..];
        let q = resolve(&format!("{rest}?a=0.5&fps=25")).unwrap();
        assert_eq!(q.bytes(), &psb[..]);
        assert_eq!((q.sliders[0], q.delay_ms), (Some(1 << 15), Some(40)));
        let mut fx = PsiFx::from_query(rest).unwrap();
        // A live edit: in place, from the bytes held now.
        crate::synth::set_sent(Some(psi::build("gray(px / 128)").unwrap()));
        fx.retune_from(rest, Retune::Edit).unwrap();
        assert!(crate::synth::title(crate::synth::SENT).is_some());
        assert!(
            crate::synth::is_sent("@formula:~sent?a=1")
                && !crate::synth::is_sent("@formula:~sentx")
        );
        // Let go once the panel moves on: a 404 that says so.
        crate::synth::set_sent(None);
        let e = io_error(PsiFx::from_query(rest).err().unwrap());
        assert_eq!(
            (e.kind(), e.to_string()),
            (std::io::ErrorKind::NotFound, SENT_GONE.to_string())
        );
        assert_eq!(
            crate::synth::title(crate::synth::SENT).as_deref(),
            Some("Formula")
        );
    }

    #[test]
    fn every_sketch_fits_a_path() {
        for (name, src) in psi::sketches::ALL {
            let p = path(src);
            assert!(p.len() <= crate::synth::MAX_PATH, "{name}: {} B", p.len());
        }
        // The compiled Formula comes back as it went.
        let src = "k = ' ' + '+' + '%' + '#' + '&' + '?' + ';' + '=' -- c\ngray(glyph(k, px % 5, 31 - py))";
        let q = query(path(src).strip_prefix("@formula?").unwrap());
        assert_eq!(&*q.psb, &psi::build(src).unwrap()[..]);
    }

    #[test]
    fn the_governor_holds_a_formula_to_the_budget() {
        // From the estimate: 11.4 ms (the ring clock) at 50 fps is held to
        // 40 ms frames before it has drawn anything; light ones are not.
        assert_eq!(Governor::new(11_400, 20).delay_ms(20), 40);
        assert_eq!(Governor::new(7_000, 20).delay_ms(20), 20);
        assert_eq!(
            Governor::new(6_000, 10).delay_ms(10),
            20,
            "over 5 ms: no 100 fps"
        );
        assert_eq!(Governor::new(MAX_EST_US, 20).delay_ms(20), 160);
        assert_eq!(
            Governor::new(u32::MAX / 2, 20).delay_ms(20),
            160,
            "8 frames at most"
        );
        // At 100 fps the slowest step is 80 ms: 32 ms is the most it can
        // hold, so a heavier formula asking for 100 fps does not open.
        let g = Governor::new(40_000, 10);
        assert_eq!(g.delay_ms(10), 80);
        assert!(40_000 > Governor::budget_us(g.delay_ms(10)));
        assert_eq!(Governor::slowest_budget_us(10), 32_000);
        assert_eq!(Governor::slowest_budget_us(20), 64_000);
        // A steady 12 ms a frame from a light estimate: slowed within a few
        // frames, then stays put.
        let mut g = Governor::new(5_000, 20);
        let mut changes = 0;
        for _ in 0..500 {
            changes += g.observe(12_000, 20) as u32;
        }
        assert_eq!((g.delay_ms(20), changes), (40, 1));
        assert!(g.avg_us().abs_diff(12_000) < 100);
        // Lighter again: back to 50 fps only after CALM calm frames.
        for k in 0..Governor::CALM * 2 {
            if g.observe(3_000, 20) {
                assert!(k >= Governor::CALM, "{k}");
                break;
            }
        }
        assert_eq!(g.delay_ms(20), 20);
        // One frame longer than its whole slot: slowed at once.
        assert!(g.observe(21_000, 20));
        assert_eq!(g.delay_ms(20), 40);
        // Hovering around the budget does not flap: never faster while the
        // mean is over 31/32 of the faster step's budget.
        let mut g = Governor::new(9_000, 20);
        let mut changes = 0;
        for k in 0..1000u32 {
            changes += g.observe(if k % 2 == 0 { 6_500 } else { 9_500 }, 20) as u32;
        }
        assert_eq!((g.delay_ms(20), changes), (40, 0));
        // The frames just after an open or a live edit run cold and are
        // left out — unless one takes its whole slot.
        let mut g = Governor::new(7_000, 20);
        for _ in 0..Governor::SKIP {
            assert!(!g.observe(15_000, 20));
        }
        assert_eq!((g.avg_us(), g.delay_ms(20)), (7_000, 20));
        g.edited();
        assert!(g.observe(25_000, 20), "a whole slot counts even then");
        // A test board's Matrix: 7.7 ms, pushed over 8 ms through a transition,
        // back at 50 fps once it settles.
        let mut g = Governor::new(7_700, 20);
        for _ in 0..40 {
            g.observe(8_400, 20);
        }
        assert_eq!(g.delay_ms(20), 40);
        for _ in 0..Governor::CALM + 40 {
            g.observe(7_730, 20);
        }
        assert_eq!(g.delay_ms(20), 20);
    }

    thread_local! {
        static NOW: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }
    /// A clock that moves 7.5 ms per read: read before and after a render,
    /// every render "takes" 7.5 ms.
    fn fake_clock() -> u32 {
        NOW.with(|n| {
            n.set(n.get().wrapping_add(7_500));
            n.get()
        })
    }

    #[test]
    fn a_governed_formula_holds_frames_of_its_own_sequence() {
        // Each render takes 7.5 ms on the fake clock against a 5 ms budget
        // at 100 fps: the governor moves to 20 ms frames, and every frame
        // drawn is exactly the VM's frame at its time — the frames the
        // browser draws, some held.
        // The same with the formula's own `fps 100` line.
        for (f, opts) in [
            ("loop 4\nhsv(x + t / 4, 1, y)", "fps=100"),
            ("fps 100\nloop 4\nhsv(x + t / 4, 1, y)", ""),
        ] {
            governed(f, opts);
        }
    }

    fn governed(f: &str, opts: &str) {
        set_clock(Some(fake_clock));
        let p = super::path(f, opts);
        let mut src = crate::synth::SynthSource::new(crate::synth::open(&p).unwrap()).unwrap();
        let prog = psi::compile(f).unwrap();
        assert!(prog.esp32_us() < 5_000);
        let mut s = psi::Scratch::<128>::new();
        let mut want = vec![0u8; psi::W * psi::H * 3];
        let (mut c, mut t, mut delays) = (Canvas::new(), 0u32, Vec::new());
        while let Some(d) = src.next_frame(&mut c).unwrap() {
            let inputs = psi::Inputs {
                t: psi::Inputs::at_ms(t),
                ..Default::default()
            };
            psi::render(&prog, &mut s, &inputs, &psi::OWN, &mut want);
            assert!(c.as_bytes() == want.as_slice(), "t = {t} ms");
            let d = d.as_millis() as u32;
            delays.push(d);
            t += d;
        }
        set_clock(None);
        assert!(delays.iter().all(|&d| d == 10 || d == 20), "{delays:?}");
        assert!(delays[..3].contains(&10), "starts as asked: {delays:?}");
        assert!(delays[delays.len() / 2..].iter().all(|&d| d == 20));
        assert_eq!(t, 4000, "one loop, all of it");
        let st = stats();
        assert_eq!((st.1, st.2), (20, 10));
        assert!(st.0.abs_diff(7_500) < 200, "{st:?}");
        assert_eq!(st.3, 7_500, "the compile, on the fake clock");
    }

    #[test]
    fn a_live_edit_keeps_the_clock_and_refuses_what_does_not_compile() {
        use std::io::ErrorKind;
        let a = super::path("rgb(a, x, t / 4)", "a=0.25");
        let b = super::path("rgb(b, y, t / 4)", "b=1");
        let mut src = crate::synth::SynthSource::new(crate::synth::open(&a).unwrap()).unwrap();
        let mut c = Canvas::new();
        for _ in 0..10 {
            src.next_frame(&mut c).unwrap();
        }
        src.retune(&b, Retune::Edit).unwrap().unwrap();
        src.next_frame(&mut c).unwrap();
        // Frame 11 of the loop (t = 200 ms) of the new formula.
        let prog = psi::compile("rgb(b, y, t / 4)").unwrap();
        let mut want = vec![0u8; psi::W * psi::H * 3];
        let inputs = psi::Inputs {
            t: psi::Inputs::at_ms(200),
            b: 1 << 16,
            ..Default::default()
        };
        psi::render(
            &prog,
            &mut psi::Scratch::<128>::new(),
            &inputs,
            &psi::OWN,
            &mut want,
        );
        assert!(c.as_bytes() == want.as_slice());
        let e = src
            .retune(&super::path("gray(tme)", ""), Retune::Edit)
            .unwrap()
            .unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidInput);
        assert_eq!(e.to_string(), psi::psb::NOT_PSB);
        assert!(src.retune("@power-on", Retune::Edit).is_none());
        assert!(src.retune("@formulas", Retune::Edit).is_none());
        let mut clock =
            crate::synth::SynthSource::new(crate::synth::open("@power-on").unwrap()).unwrap();
        assert!(
            clock.retune(&b, Retune::Edit).is_none(),
            "a formula only replaces a formula"
        );
    }

    /// `n` waves a pixel, each moving every frame: the JIT draws them ~4.5×
    /// faster than the VM, so the two estimates can fall either side of a
    /// limit (built and checked with `psi check`).
    fn waves(n: usize) -> String {
        let terms: Vec<String> = (1..=n)
            .map(|k| format!("step(0.5, frac(x*{} + y*{} + t*{}))*{}", k + 2, (k * 7) % 13 + 1, k % 5 + 1, k % 3 + 1))
            .collect();
        format!("loop 4\ng = {}\npal(rainbow, frac(g/40))", terms.join(" + "))
    }

    /// The panel refuses by the VM's estimate, never the JIT's or the still
    /// cache's (the panel may draw any Formula on the VM: the JIT off, its
    /// kernel not made yet or refused for memory, a failed self-check, the
    /// cache refused or let go); `psi::Program::panel_refusal` (what the
    /// editor, `psi check` and the gallery warn by) is the same rule, so a
    /// Formula the editor calls light or heavy but the panel refuses says
    /// so there. The cases: JIT and VM, and still cache and VM, either side
    /// of a limit.
    /// A look short of memory is tried again a second later; one that
    /// doesn't compile, or isn't a compiled Formula, is final.
    #[test]
    fn a_look_short_of_memory_is_tried_again() {
        let look = super::path("look\nlayer multiply { gray(x) }", "");
        refuse_loads(1);
        let e = FormulaLook::from_path(&look).expect_err("refused for the heap");
        assert_eq!((e.msg.as_str(), look_retry(&e)), (psi::NO_MEMORY, Some(LOOK_RETRY)));
        assert!(FormulaLook::from_path(&look).is_ok(), "the heap back: it builds");
        let e = FormulaLook::from_bytes(b"not a compiled Formula", "").expect_err("not a psb");
        assert_eq!(look_retry(&e), None, "{}", e.msg);
        let e = FormulaLook::from_path(&super::path("gray(x)", "")).expect_err("covers the GIF");
        assert_eq!(look_retry(&e), None, "{}", e.msg);
    }

    /// Phase 2 B2 through the engine's loop (`SynthSource`'s frames and
    /// its rewind): raced, `loop 4.85` draws `t` in its last 10 ms
    /// (4,840-4,850, past the 20 ms grid's 242 frames) and wraps at 4,850,
    /// the time past it carried into the next loop; an on-grid `loop 4`
    /// carries its remainder too (it restarted up to a frame late). A grid
    /// frame (not raced: the first, a JIT trial, a `rate` one) in that last
    /// partial frame draws `t` = 0, once.
    /// (Called from `a_raced_formula_writes_each_pair_as_it_is_drawn`: the
    /// hooks are the process's.)
    fn raced_loops_wrap_at_their_length_through_the_engine() {
        arm_beam(true);
        for (lp, src) in [(4_850u32, "loop 4.85\ngray(frac(x + t/4.85))"), (4_000, "loop 4\ngray(frac(x + t/4))")] {
            let p = path(src);
            let mut s = crate::synth::SynthSource::new(crate::synth::open(&p).unwrap()).unwrap();
            let mut c = Canvas::new();
            let (mut ts, mut wraps, mut clock_us) = (Vec::new(), 0, 0u64);
            while wraps < 2 {
                TOLD.with(|t| t.borrow_mut().clear());
                STARTS.with(|n| n.set(0));
                match s.next_frame(&mut c).unwrap() {
                    Some(d) => {
                        let Some(Fx::Psi(fx)) = s.fx_mut() else { unreachable!() };
                        ts.push((fx.last_t(), clock_us));
                        clock_us += d.as_micros() as u64;
                    }
                    None => {
                        wraps += 1;
                        s.rewind();
                    }
                }
                c.dirty_mut().clear();
            }
            // Every raced frame's `t` is the clock's time mod the loop (to
            // the ms), across both wraps: nothing dropped at the rewind.
            let raced: Vec<_> = ts.iter().skip(1).filter(|((_, r), _)| *r).collect();
            assert!(raced.len() >= 450, "{src}: {} raced", raced.len());
            for &&((t, _), us) in &raced {
                let want = (us / 1000) as u32 % lp;
                assert!(t.abs_diff(want) <= 1, "{src}: t {t} ms at {us} us, want {want}");
            }
            if lp == 4_850 {
                assert!(raced.iter().any(|((t, _), _)| (4_840..4_850).contains(t)), "{src}: no t in its last 10 ms");
            }
            // The wrap: the first `t` after it is under a frame (2 refreshes).
            let after: Vec<_> = raced.windows(2).filter(|w| w[1].0 .0 < w[0].0 .0).map(|w| w[1].0 .0).collect();
            assert!(!after.is_empty() && after.iter().all(|&t| t < 16), "{src}: after the wraps {after:?}");
        }
        // A grid frame in the last partial grid frame: t = 0 (once).
        let Fx::Psi(mut fx) = Fx::parse(&path("loop 4.85\ngray(frac(x + t/4.85))")).unwrap();
        arm_beam(false);
        let mut c = Canvas::new();
        fx.frame(Duration::from_millis(4_845), &mut c);
        assert_eq!(fx.last_t(), (0, false));
    }

    /// Phase 2 B2: raced, `t` wraps at exactly the loop's length (not at
    /// the whole frames of its grid under it); on the grid, at its frames.
    #[test]
    fn raced_t_wraps_at_the_loop() {
        // `loop 4.85` at 20 ms: 242 frames, 4,840 ms of grid.
        assert_eq!(loop_ms_at(4_845, true, 4_850, 242, 20), 4_845);
        assert_eq!(loop_ms_at(4_851, true, 4_850, 242, 20), 1);
        assert_eq!(loop_ms_at(4_845, false, 4_850, 242, 20), 0);
        assert_eq!(loop_ms_at(4_839, false, 4_850, 242, 20), 4_820);
        // A loop forever one: its passes (its `t` is not read).
        assert_eq!(loop_ms_at(1_005, true, 0, 50, 20), 5);
    }

    #[test]
    fn the_panel_and_psi_refuse_by_the_same_figure() {
        for grid in [10, 20, 40] {
            assert_eq!(MAX_EST_US.min(Governor::slowest_budget_us(grid)), psi::tier_limit_us(grid), "{grid} ms");
            assert_eq!(tier_limit(grid as u8), psi::tier_limit_us(grid), "{grid} ms");
            for est in [1_000, 31_000, 33_000, 39_000, 41_000] {
                assert_eq!(u32::from(opens_at(grid as u8, est)), psi::opens_ms(grid, est), "{grid} ms, {est} us");
            }
        }
        assert_eq!(LOOK_MAX_EST_US, psi::LOOK_MAX_US);
        let open =
            |p: &str| crate::synth::SynthSource::new(crate::synth::open(p).unwrap()).map(|_| ());
        // ~11 ms a frame with the JIT, ~52 without.
        let jit_side = waves(48);
        // ~1 ms with the still cache and the JIT, ~64 without either.
        let still_side = format!(
            "loop 4\ng = {}\npal(rainbow, frac(g - t/4))",
            (1..=16).map(|k| format!("noise(x*{}, y*{}, 0)/{}", k * 5 + 3, (k * 11) % 37 + 3, 1 << (k % 4))).collect::<Vec<_>>().join(" + ")
                + " + atan2(y - 0.5, x - 0.5) + atan2(y - 0.2, x - 0.8) + atan2(y - 0.9, x - 0.1) + sqrt(len(x - 0.3, y - 0.6))"
        );
        for (what, src) in [("jit", &jit_side), ("still", &still_side)] {
            let prog = psi::compile(src).unwrap();
            let (tier, vm) = (prog.tier_us(false), prog.esp32_us());
            assert!(tier < MAX_EST_US / 2 && vm > MAX_EST_US, "{what}: tier {tier}, VM {vm}");
            assert_eq!(prog.panel_refusal(false), Some((vm, MAX_EST_US)), "{what}");
            let e = open(&super::path(src, "")).unwrap_err();
            assert!(e.to_string().starts_with("this formula is too heavy for the panel"), "{what}: {e}");
        }
        assert!(psi::compile(&jit_side).unwrap().jit_takes());
        assert!(psi::compile(&still_side).unwrap().still_lanes() > 0);
        // A look: ~5 ms with the JIT, ~18 without.
        let look = format!("look\nlayer multiply {{\n{}\n}}", waves(14).replace("loop 4\n", "").replace("pal(rainbow, frac(g/40))", "gray(frac(g/40))"));
        let prog = psi::compile(&look).unwrap();
        let (tier, vm) = (prog.tier_us(true), prog.esp32_us());
        assert!(tier < LOOK_MAX_EST_US && vm > LOOK_MAX_EST_US, "look: tier {tier}, VM {vm}");
        assert_eq!(prog.panel_refusal(true), Some((vm, LOOK_MAX_EST_US)));
        let e = FormulaLook::from_path(&super::path(&look, "")).unwrap_err();
        assert!(e.msg.starts_with("this formula is too heavy to draw over GIFs"), "{}", e.msg);
        // Under the limits, each opens (a lighter count of the same waves).
        let fine = waves(30);
        assert_eq!(psi::compile(&fine).unwrap().panel_refusal(false), None);
        open(&super::path(&fine, "")).unwrap();
    }

    #[test]
    fn opening_reports_why_and_holds_the_limits() {
        use std::io::ErrorKind;
        let open =
            |p: &str| crate::synth::SynthSource::new(crate::synth::open(p).unwrap()).map(|_| ());
        let e = open(&super::path("gray(tme)", "")).unwrap_err();
        assert_eq!(
            (e.kind(), e.to_string().as_str()),
            (
                ErrorKind::InvalidInput,
                psi::psb::NOT_PSB
            )
        );
        // Too heavy: ~48 ms a frame estimated.
        let heavy = (0..11)
            .map(|k| format!("noise(x*{k}, y, t)"))
            .collect::<Vec<_>>()
            .join(" + ");
        let heavy = format!("gray({heavy})");
        assert!(psi::compile(&heavy).unwrap().esp32_us() > MAX_EST_US);
        let e = open(&super::path(&heavy, "")).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidInput);
        assert!(
            e.to_string()
                .starts_with("this formula is too heavy for the panel (about 4"),
            "{e}"
        );
        // 32-40 ms: fine every 2nd refresh, too heavy for the governor
        // every refresh: `fps=100` opens it every 2nd, not refused.
        let mid = (0..9)
            .map(|k| format!("noise(x*{k}, y, t)"))
            .collect::<Vec<_>>()
            .join(" + ");
        let mid = format!("gray({mid})");
        let est = psi::compile(&mid).unwrap().esp32_us();
        assert!((32_001..=MAX_EST_US).contains(&est), "{est}");
        open(&super::path(&mid, "")).unwrap();
        open(&super::path(&mid, "fps=100")).unwrap();
        // An unknown live item, or a retired effect: InvalidData (a 415).
        for p in ["@fx:nonsense", "@fx:tunnel", "@fx:psi?src=gray(x)"] {
            assert_eq!(open(p).unwrap_err().kind(), ErrorKind::InvalidData);
        }
        // A formula may be longer than other virtual paths, up to its limit.
        let terms: Vec<String> = (0..40).map(|k| format!("sin(x*{k}.125 + y)")).collect();
        let src = format!("loop 8\ngray(frac({}))", terms.join(" + "));
        let p = super::path(&src, "");
        assert!(
            p.len() > crate::synth::MAX_PATH && p.len() <= crate::synth::MAX_FORMULA_PATH,
            "{}",
            p.len()
        );
        open(&p).unwrap();
        let text = format!("@card:{}", "x".repeat(600));
        assert_eq!(open(&text).unwrap_err().kind(), ErrorKind::InvalidData);
    }

    /// The golden frames' formulas sent as the panel's path (compacted,
    /// encoded, sliders after) compile to exactly the program their source
    /// does: the frames the device draws are the golden frames.
    #[test]
    fn golden_formulas_through_the_path_are_the_same_programs() {
        for &(name, src, _, _) in psi::golden::FRAMES {
            let p = super::path(src, "a=0.5&b=0.25&c=0.75");
            let Some(Fx::Psi(fx)) = Fx::parse(&p) else {
                panic!("{name}: {p}")
            };
            assert!(*fx.program() == *psi::compile(src).unwrap(), "{name}");
            let g = psi::golden::inputs(0);
            assert_eq!(
                (fx.inputs.a, fx.inputs.b, fx.inputs.c),
                (g.a, g.b, g.c),
                "{name}"
            );
        }
    }

    #[test]
    fn plays_through_the_synth_source() {
        let p = path(psi::sketches::FIRE);
        assert!(p.len() < crate::synth::MAX_PATH);
        assert_eq!(crate::synth::title(&p).as_deref(), Some("Formula"));
        assert_eq!(
            crate::synth::title("@formula?src=gray(nope)").as_deref(),
            Some("Formula")
        );
        let mut src = crate::synth::SynthSource::new(crate::synth::open(&p).unwrap()).unwrap();
        let mut c = Canvas::new();
        let mut frames = 0;
        while src.next_frame(&mut c).unwrap().is_some() {
            frames += 1;
        }
        assert_eq!(frames, 4000 / 20);
    }

    // --- The formula look ------------------------------------------------------

    fn picture() -> Canvas {
        let mut c = Canvas::new();
        psi::golden::picture(c.as_mut_bytes());
        c.dirty_mut().mark_all();
        c
    }

    /// The VM's frame of `src` at `ms` over `under`.
    fn over(src: &str, ms: u32, under: &Canvas) -> Vec<u8> {
        over_prog(&psi::compile(src).unwrap(), ms, under)
    }

    /// `src` with its look line set to `blend` at `k` %.
    fn as_look(src: &str, blend: &str, k: u8) -> String {
        psi::look::set(src, blend, k).unwrap()
    }

    fn over_prog(prog: &psi::Prog, ms: u32, under: &Canvas) -> Vec<u8> {
        let mut want = vec![0u8; psi::W * psi::H * 3];
        let inputs = psi::Inputs {
            t: psi::Inputs::at_ms(ms),
            ..Default::default()
        };
        psi::render_rows_ink(
            prog,
            &mut psi::Scratch::<128>::new(),
            Some(&mut psi::Ink::boxed()),
            &inputs,
            &psi::OWN,
            Some(under.as_bytes()),
            u32::MAX,
            &mut want,
        );
        want
    }

    #[test]
    fn a_look_draws_over_the_picture_and_follows_what_changes() {
        const F: &str = "rgb(1 - ur, ug*0.5, ub)\n";
        let mut look = FormulaLook::from_path(&super::path(F, "")).unwrap();
        assert!(!look.is_animated());
        let mut under = picture();
        // Not held: the picture itself.
        let p = &under as *const Canvas;
        assert!(std::ptr::eq(look.apply(Duration::ZERO, &mut under, 0), p));
        assert_eq!(look.due(), None);
        assert!(look.take());
        assert_eq!(look.due(), Some(Duration::ZERO), "drawn at once");
        let want = over(F, 0, &under);
        let out = look.apply(Duration::ZERO, &mut under, 0);
        assert!(out.as_bytes() == want.as_slice());
        assert_eq!(out.dirty().count(), 4096);
        out.dirty_mut().clear();
        assert!(under.dirty().is_clean(), "the look took what changed");
        assert_eq!(look.due(), None, "it only follows the picture");
        // Two rows change: only they are drawn again and marked.
        under.set(3, 5, Rgb::new(10, 200, 30));
        under.set(100, 20, Rgb::new(255, 255, 255));
        let want = over(F, 0, &under);
        let out = look.apply(Duration::from_millis(40), &mut under, 0);
        assert_eq!(out.dirty().row_mask(), (1 << 5) | (1 << 20));
        assert!(out.as_bytes() == want.as_slice());
        out.dirty_mut().clear();
        // Nothing changed: nothing drawn, nothing to push.
        assert!(look
            .apply(Duration::from_millis(60), &mut under, 0)
            .dirty()
            .is_clean());
        // Let go: the picture under it goes up in full.
        look.release(&mut under);
        assert!(!look.is_held());
        assert_eq!(under.dirty().count(), 4096);
    }

    #[test]
    fn an_animated_look_has_frames_of_its_own_on_its_grid() {
        const F: &str = "loop 2\nlayer add { gray(sin(t/2)*0.5 + 0.5, 0.3) }\n";
        let mut look = FormulaLook::from_path(&super::path(F, "fps=25")).unwrap();
        assert!(look.is_animated() && look.program().over());
        assert!(look.take());
        let mut under = picture();
        let ms = Duration::from_millis;
        look.apply(ms(1000), &mut under, 0).dirty_mut().clear();
        assert_eq!(look.due(), Some(ms(1040)), "40 ms frames from its start");
        // Before it: nothing changed, nothing drawn.
        assert!(look.apply(ms(1030), &mut under, 0).dirty().is_clean());
        // The picture changes between its frames: those rows, the same `t`.
        under.set(0, 0, Rgb::new(1, 2, 3));
        let want = over(F, 0, &under);
        let out = look.apply(ms(1035), &mut under, 0);
        assert_eq!(out.dirty().row_mask(), 1);
        assert!(out.as_bytes() == want.as_slice());
        out.dirty_mut().clear();
        // Its frame, late: every row, at the grid frame it is in (2 x 40 ms).
        let want = over(F, 80, &under);
        let out = look.apply(ms(1093), &mut under, 0);
        assert_eq!(out.dirty().count(), 4096);
        assert!(out.as_bytes() == want.as_slice());
        assert_eq!(look.due(), Some(ms(1120)));
        // Its loop: 2 s, then t starts again.
        look.apply(ms(3000), &mut under, 0);
        assert!(look.held.as_ref().unwrap().out.as_bytes() == over(F, 0, &under).as_slice());
        // A live edit: the buffers and the clock stay, drawn at once.
        look.retune(
            &super::path("loop 2\nlayer multiply { gray(1 - t/4) }", "fps=25"),
            Retune::Edit,
        )
        .unwrap();
        assert!(look.is_held());
        assert_eq!(look.due(), Some(Duration::ZERO));
        let want = over("loop 2\nlayer multiply { gray(1 - t/4) }", 0, &under);
        let out = look.apply(ms(3010), &mut under, 0);
        assert!(out.as_bytes() == want.as_slice());
        // A bad edit changes nothing.
        assert!(look.retune(&super::path("gray(tme)", ""), Retune::Edit).is_err());
        assert!(look.is_animated());
    }

    /// A slot the path leaves unset plays at the formula's declared
    /// default (formula-apps F1); a value on the path wins, even outside
    /// the range (the formula holds it to its range itself).
    #[test]
    fn an_unset_control_plays_at_its_default() {
        let src = "control zoom 1..4 = 2.5\ncontrol mode = calm, wild\ngray(zoom/4 + mode*0.1 + c)";
        let Some(Fx::Psi(fx)) = Fx::parse(&path(src)) else {
            panic!()
        };
        assert_eq!((fx.inputs.a, fx.inputs.b, fx.inputs.c), (5 << 15, 0, 0));
        let Some(Fx::Psi(fx)) = Fx::parse(&super::path(src, "a=9&b=1&c=0.5")) else {
            panic!()
        };
        assert_eq!(
            (fx.inputs.a, fx.inputs.b, fx.inputs.c),
            (9 << 16, 1 << 16, 1 << 15)
        );
        // Below zero is a value; empty, junk and out of range are unset.
        let Some(Fx::Psi(fx)) = Fx::parse(&super::path(src, "a=-0.5&b=&c=40000")) else {
            panic!()
        };
        assert_eq!((fx.inputs.a, fx.inputs.b, fx.inputs.c), (-(1 << 15), 0, 0));
        let Some(Fx::Psi(fx)) = Fx::parse(&super::path(src, "a=x&a=32768")) else {
            panic!()
        };
        assert_eq!(fx.inputs.a, 5 << 15);
        // A live edit takes the new formula's defaults.
        let Some(Fx::Psi(mut fx)) = Fx::parse(&path("gray(a)")) else {
            panic!()
        };
        assert_eq!(fx.inputs.a, 0);
        fx.retune_from(&super::path(src, "")[Q..], Retune::Edit).unwrap();
        assert_eq!(fx.inputs.a, 5 << 15);
        // And a look.
        let look = FormulaLook::from_path(&super::path(
            "control k 0..1 = 0.75\nlayer multiply { gray(k) }",
            "",
        ))
        .unwrap();
        assert_eq!(look.inputs.a, 3 << 14);
    }

    /// A formula's own `look screen 40` line: the player draws the
    /// composition it asks for, the program `look::compose` gives.
    #[test]
    fn a_formulas_look_line_is_its_look() {
        let body = "control k 0..1 = 0.5\nhsv(x, 1, k)";
        let mine =
            FormulaLook::from_path(&super::path(&format!("look screen 40\n{body}"), "")).unwrap();
        let theirs = psi::look::compose(body, "screen", 40).unwrap();
        assert!(*mine.prog == *theirs);
        assert_eq!(mine.inputs.a, 1 << 15);
        // `look` alone is the formula itself: one that covers the GIF is
        // still no look.
        assert_eq!(
            FormulaLook::from_path(&super::path(&format!("look\n{body}"), ""))
                .unwrap_err()
                .msg,
            COVERS
        );
        // Played on its own the line is ignored.
        let Some(Fx::Psi(fx)) = Fx::parse(&path(&format!("look screen 40\n{body}"))) else {
            panic!()
        };
        assert!(!fx.prog.over());
    }

    #[test]
    fn a_look_is_held_to_what_fits_over_a_gif() {
        // One that hides the GIF is no look.
        let e = FormulaLook::from_path(&super::path("hsv(x, 1, 1)", "")).unwrap_err();
        assert_eq!(e.msg, COVERS);
        assert!(FormulaLook::from_path(&super::path(
            "layer { hsv(x, 1, 1) }\nlayer add { gray(0.1) }",
            ""
        ))
        .is_err());
        // Composed with a blend or a lower amount (Settings' choices), it is.
        let under = picture();
        for (b, k) in [("screen", 40), ("over", 30), ("multiply", 100), ("max", 50)] {
            let src = as_look("hsv(x, 1, 1)", b, k);
            let mut look = FormulaLook::from_path(&super::path(&src, "")).unwrap();
            assert!(look.take());
            let mut c = picture();
            let out = look.apply(Duration::ZERO, &mut c, 0);
            assert!(
                out.as_bytes() == over_prog(&psi::look::compose("hsv(x, 1, 1)", b, k).unwrap(), 0, &under).as_slice(),
                "{b} {k}"
            );
            assert!(out.as_bytes() != under.as_bytes(), "{b} {k}: drawn");
        }
        let e = FormulaLook::from_path(&super::path("layer multiply { gray(noise(x*8, y, t) + noise(x, y*8, t) + len(x, y) + len(y, x) + atan2(x, y) + sqrt(x) + noise(y, x, t)) }", "")).unwrap_err();
        assert!(
            e.msg
                .starts_with("this formula is too heavy to draw over GIFs (about "),
            "{}",
            e.msg
        );
        assert!(
            e.msg.ends_with("the limit is 12 ms) — simplify it"),
            "{}",
            e.msg
        );
        assert_eq!(
            FormulaLook::from_path("@formula?src=gray(tme)")
                .unwrap_err()
                .msg,
            psi::psb::NOT_PSB
        );
        assert_eq!(
            FormulaLook::BUFFER_BYTES,
            // (The scratch: 8.5 KB, its scalar slots 256 since psi-p0, and
            // racing the beam's three words.)
            8704 + 3 * std::mem::size_of::<usize>() + 12288 + std::mem::size_of::<LookBuffers>()
        );
    }

    #[test]
    fn a_look_over_an_effect_is_the_vm_over_its_frames() {
        // A live effect underneath (as a GIF would be): every frame the
        // panel would show is the VM's frame over the effect's.
        const F: &str =
            "layer multiply { gray(py % 2 ? 1 : 0.5) }\nlayer screen { rgb(0, 0, 0.1) }";
        let mut look = FormulaLook::from_path(&super::path(F, "")).unwrap();
        assert!(look.take());
        let mut src =
            crate::synth::SynthSource::new(crate::synth::open(&super::path(STARS, "")).unwrap())
                .unwrap();
        let mut c = Canvas::new();
        let mut t = Duration::ZERO;
        for _ in 0..30 {
            let d = src.next_frame(&mut c).unwrap().unwrap();
            let snap = c.clone();
            let out = look.apply(t, &mut c, 0);
            assert!(out.as_bytes() == over(F, 0, &snap).as_slice());
            out.dirty_mut().clear();
            t += d;
        }
    }

    #[test]
    fn a_clock_look_redraws_once_a_second_and_a_heavy_one_is_slowed() {
        let ms = Duration::from_millis;
        let mut under = picture();
        let mut look = FormulaLook::from_path(&super::path(
            "m = minute / 60\nlayer { gray(1, step(px, m*128) * 0.3) }",
            "",
        ))
        .unwrap();
        assert!(look.is_animated() && look.take());
        look.apply(ms(500), &mut under, 0);
        assert_eq!(
            look.due(),
            Some(ms(1500)),
            "the clock alone: a frame a second"
        );
        // 7.5 ms a render (the fake clock) at 100 fps (5 ms): 20 ms frames.
        set_clock(Some(fake_clock));
        let mut look = FormulaLook::from_path(&super::path(
            "loop 4\nlayer add { gray(t/4, 0.2) }",
            "fps=100",
        ))
        .unwrap();
        assert!(look.take());
        let mut t = ms(0);
        for _ in 0..20 {
            look.apply(t, &mut under, 0);
            t = look.due().unwrap();
        }
        set_clock(None);
        assert_eq!(look.gov.delay_ms(10), 20, "100 fps asked, 20 ms frames");
    }

    #[test]
    fn the_glow_fades_towards_the_looks_picture() {
        use crate::canvas::DirtyMap;
        use crate::look::{Glow, GlowLevel};
        // Pixel (0, 0) after a look blit, a darker GIF frame on its row (so
        // the row fades) and one glow step towards `to_look`'s target.
        let run = |to_look: bool| {
            let mut look =
                FormulaLook::from_path(&super::path("layer add { gray(0.25) }", "")).unwrap();
            assert!(look.take());
            let mut under = Canvas::new();
            under.set(0, 0, Rgb::new(40, 40, 40));
            under.set(1, 0, Rgb::new(200, 200, 200));
            under.dirty_mut().mark_all();
            let mut glow = Glow::new(GlowLevel::Long);
            let mut push = DirtyMap::default();
            let out = look.apply(Duration::ZERO, &mut under, 0);
            assert!(glow.frame(out, &mut push));
            out.dirty_mut().clear();
            let lit = look.shown().unwrap().get(0, 0);
            under.set(1, 0, Rgb::new(0, 0, 0));
            let out = look.apply(Duration::from_millis(20), &mut under, 0);
            glow.frame(out, &mut push);
            out.dirty_mut().clear();
            match to_look {
                true => glow.step(look.shown().unwrap(), &mut push),
                false => glow.step(&under, &mut push),
            }
            (lit, glow.shown().unwrap()[0])
        };
        let (lit, kept) = run(true);
        assert_eq!(lit, Rgb::new(104, 104, 104), "the look brightened it");
        assert_eq!(kept, lit, "towards the look's picture it stays");
        let (_, faded) = run(false);
        assert!(
            faded.r < lit.r,
            "towards the GIF alone it fades: the step ran"
        );
    }

    /// Over a GIF too: another Formula sent over the same GIF starts from
    /// its own estimate, a live edit keeps the look's level.
    #[test]
    fn a_look_sent_over_a_look_starts_from_its_own_estimate() {
        let heavy = "loop 4\nlayer add { gray(noise(x*8, y*4, t) + noise(x*3, y*9, t*2), 0.2) }";
        let light = "loop 4\nlayer add { gray(t/4, 0.2) }";
        let open = |src: &str| FormulaLook::from_path(&super::path(src, "")).unwrap();
        assert_eq!(open(heavy).gov.delay_ms(20), 40, "heavy: from its estimate");
        assert_eq!(open(light).gov.delay_ms(20), 20);
        for (how, want) in [(Retune::Play, 20), (Retune::Edit, 40)] {
            let mut look = open(heavy);
            // (Drawing: a look not yet drawn takes any start.)
            look.t0 = Some(Duration::ZERO);
            look.retune(&super::path(light, ""), how).unwrap();
            assert_eq!(look.gov.delay_ms(20), want, "{how:?}");
        }
    }

    #[test]
    fn a_new_files_open_does_not_steer_the_look() {
        let ms = Duration::from_millis;
        set_clock(Some(fake_clock));
        let mut look =
            FormulaLook::from_path(&super::path("loop 4\nlayer add { gray(t/4, 0.2) }", ""))
                .unwrap();
        assert!(look.take());
        let mut under = picture();
        let mut t = ms(0);
        for _ in 0..10 {
            look.apply(t, &mut under, 0);
            t = look.due().unwrap();
        }
        assert_eq!(look.gov.delay_ms(20), 20, "7.5 ms a frame fits 50 fps");
        // A new file: its open (300 ms in the same tick) is left out.
        look.new_file();
        look.apply(t, &mut under, 300_000);
        t = look.due().unwrap();
        look.apply(t, &mut under, 300_000);
        assert_eq!(look.gov.delay_ms(20), 20, "the level is unchanged");
        // Later, a frame that really takes 300 ms slows it.
        t = look.due().unwrap();
        look.apply(t, &mut under, 300_000);
        set_clock(None);
        assert!(look.gov.delay_ms(20) > 20);
    }

    #[test]
    fn over_a_transition_only_its_steps_draw_the_look() {
        // The firmware's loop, a tick a millisecond through a dissolve: a
        // step draws the look (every row the dissolve changed, its own
        // frame when that is due); between steps nothing does. Before
        // `due_between`, the look's own frames were drawn between the
        // steps as well, and the step after one drew every row again.
        use crate::compositor::{Compositor, Kind, Step, Transition, STEP};
        const F: &str = "loop 4\nlayer add { gray(t/4, 0.2) }";
        let ms = Duration::from_millis;
        let mut look = FormulaLook::from_path(&super::path(F, "fps=50")).unwrap();
        assert!(look.take());
        let mut canvas = picture();
        let first = look.apply(ms(990), &mut canvas, 0);
        first.dirty_mut().clear();
        // The look's picture as last drawn (what the panel shows).
        let mut shown = first.as_bytes().to_vec();
        assert_eq!(
            look.due_between(false),
            Some(ms(1010)),
            "its own frames between the file's"
        );
        assert_eq!(look.due_between(true), None);
        let mut c = Compositor::new(Transition::new(Kind::Dissolve, ms(400)));
        c.begin(&canvas, 0);
        // The new file's first frame.
        for p in canvas.as_mut_slice() {
            *p = Rgb::new(255 - p.r, p.g / 2, 255 - p.b);
        }
        canvas.dirty_mut().mark_all();
        c.frame_decoded();
        let (mut steps, mut own, mut between) = (0, 0, 0);
        let mut last_step = None;
        let mut now = ms(1000);
        while c.is_active() {
            if c.step(now, &mut canvas) == Step::Composited {
                if let Some(l) = last_step {
                    assert!(now - l <= STEP, "a step at least every {STEP:?}");
                }
                last_step = Some(now);
                steps += 1;
                let due = look.due();
                let under = c.output_mut().unwrap();
                let snap = under.clone();
                let out = look.apply(now, under, 0);
                if due.is_some_and(|d| d <= now) {
                    // Its own frame, at the grid frame the step is in.
                    own += 1;
                    let t = (now - ms(990)).as_millis() as u32 / 20 * 20;
                    assert!(out.as_bytes() == over(F, t, &snap).as_slice(), "t {t}");
                }
                // Whatever it drew, what changed is marked.
                for (k, (a, b)) in shown.chunks(3).zip(out.as_bytes().chunks(3)).enumerate() {
                    if a != b {
                        assert!(
                            out.dirty().is_dirty(k % 128, k / 128),
                            "pixel {k} not marked"
                        );
                    }
                }
                shown.copy_from_slice(out.as_bytes());
                out.dirty_mut().clear();
            } else if c.is_active() && look.due_between(true).is_some_and(|d| d <= now) {
                between += 1;
            }
            now += ms(1);
        }
        assert_eq!(between, 0, "nothing drawn between the steps");
        assert!(steps >= 19, "{steps} steps");
        assert_eq!(
            own,
            steps - 1,
            "every step but the first (t = 0) is a frame of its own"
        );
        // After it, its own frames between the file's again.
        assert!(look.due_between(c.is_active()).is_some());
    }

    /// The VM's frame of a formula that draws, alone (the effect's case).
    fn drawn(src: &str, ms: u32) -> Vec<u8> {
        let prog = psi::compile(src).unwrap();
        let mut want = vec![0u8; psi::W * psi::H * 3];
        let inputs = psi::Inputs {
            t: psi::Inputs::at_ms(ms),
            ..Default::default()
        };
        psi::render_rows_ink(
            &prog,
            &mut psi::Scratch::<128>::new(),
            Some(&mut psi::Ink::boxed()),
            &inputs,
            &psi::OWN,
            None,
            u32::MAX,
            &mut want,
        );
        want
    }

    const STARS: &str = "loop 2\ndraw { for k < 64 { z = frac(hash(k, 1) - t/2)\nline(64 + (hash(k, 2) - 0.5)*200*z, 16 + (hash(k, 3) - 0.5)*60*z, 64 + (hash(k, 2) - 0.5)*220*z, 16 + (hash(k, 3) - 0.5)*66*z, 0.2, z) } }\npal(ice, ink)";

    #[test]
    fn a_formula_that_draws_plays_as_an_effect_and_retunes() {
        let mut fx = Fx::parse(&path(STARS)).unwrap();
        let mut c = Canvas::new();
        fx.frame(Duration::from_millis(140), &mut c);
        assert_eq!(c.as_bytes(), drawn(STARS, 140).as_slice());
        assert!(c.as_bytes().iter().any(|&b| b > 0), "it draws");
        let Fx::Psi(p) = &mut fx;
        assert!(p.ink.is_some());
        // To one that doesn't draw: the canvas goes; back: it comes again.
        let q = |src: &str| super::path(src, "")[Q..].to_string();
        p.retune_from(&q("gray(x)"), Retune::Edit).unwrap();
        assert!(p.ink.is_none());
        let src = "draw { dot(10, 10) }\ngray(ink)";
        p.retune_from(&q(src), Retune::Edit).unwrap();
        assert!(p.ink.is_some());
        fx.frame(Duration::from_millis(0), &mut c);
        assert_eq!(c.as_bytes(), drawn(src, 0).as_slice());
        // A clone draws the same.
        let mut d = fx.clone();
        let mut c2 = Canvas::new();
        d.frame(Duration::from_millis(0), &mut c2);
        assert_eq!(c.as_bytes(), c2.as_bytes());
    }

    #[test]
    fn a_look_that_draws_takes_its_canvas() {
        let under = picture();
        let src = as_look(STARS, "screen", 60);
        let mut look = FormulaLook::from_path(&super::path(&src, "")).unwrap();
        assert_eq!(look.buffer_bytes(), FormulaLook::BUFFER_BYTES + core::mem::size_of::<psi::Ink>());
        assert!(look.take());
        let mut c = picture();
        let out = look.apply(Duration::ZERO, &mut c, 0);
        assert_eq!(out.as_bytes(), over_prog(&psi::look::compose(STARS, "screen", 60).unwrap(), 0, &under).as_slice());
        // Retuned to a look that doesn't draw: the canvas goes.
        let plain = as_look("gray(x)", "screen", 40);
        look.retune(&super::path(&plain, ""), Retune::Edit).unwrap();
        assert_eq!(look.buffer_bytes(), FormulaLook::BUFFER_BYTES);
        assert!(look.held.as_ref().unwrap().ink.is_none());
        look.retune(&super::path(&src, ""), Retune::Edit).unwrap();
        assert!(look.held.as_ref().unwrap().ink.is_some());
    }

    /// The built-in effects' rasteriser, from before they were Formulas
    /// (`fx::line`, 2026-10-27): Bresenham, the brightness in 16.16 and
    /// eight steps, clipped, writing only pixels that differ.
    fn reference_line(canvas: &mut Canvas, a: (i32, i32, i32), b: (i32, i32, i32), c: Rgb) {
        use crate::canvas::{HEIGHT, WIDTH};
        let (x1, y1) = (b.0, b.1);
        let (dx, dy) = ((x1 - a.0).abs(), -(y1 - a.1).abs());
        let (sx, sy) = (if a.0 < x1 { 1 } else { -1 }, if a.1 < y1 { 1 } else { -1 });
        let steps = dx.max(-dy).max(1);
        let (mut x, mut y, mut err) = (a.0, a.1, dx + dy);
        let mut shades = [Rgb::BLACK; 8];
        for (i, s) in shades.iter_mut().enumerate() {
            *s = crate::fx::scale(c, 32 * i as u32 + 32);
        }
        let mut k = a.2.clamp(0, 255) << 16;
        let dk = ((b.2.clamp(0, 255) << 16) - k) / steps;
        for _ in 0..1024 {
            if (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
                let (ux, uy) = (x as usize, y as usize);
                let col = shades[(k >> 21) as usize & 7];
                if canvas.get(ux, uy) != col {
                    canvas.set(ux, uy, col);
                }
            }
            if x == x1 && y == y1 {
                break;
            }
            k += dk;
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Psi's `line` is the built-in effects' rasteriser: the same pixels
    /// for any two ends (both count rows down from the top), off the
    /// panel too.
    #[test]
    fn psi_lines_are_the_effects_lines() {
        let mut r = 0x9e37_79b9u32;
        for _ in 0..400 {
            let mut n = |lo: i32, hi: i32| {
                r ^= r << 13;
                r ^= r >> 17;
                r ^= r << 5;
                lo + (r % (hi - lo + 1) as u32) as i32
            };
            let (x0, y0, x1, y1) = (n(-40, 170), n(-20, 50), n(-40, 170), n(-20, 50));
            let mut c = Canvas::new();
            reference_line(
                &mut c,
                (x0, y0, 255),
                (x1, y1, 255),
                Rgb::new(255, 255, 255),
            );
            let src = format!("draw {{ line({x0}, {y0}, {x1}, {y1}) }}\ngray(ink)");
            let want = drawn(&src, 0);
            for (k, (a, b)) in c.as_bytes().chunks(3).zip(want.chunks(3)).enumerate() {
                assert_eq!(a[0] > 0, b[0] > 0, "{x0},{y0} -> {x1},{y1}: pixel {k}");
            }
        }
    }

    /// A formula that draws but lost its canvas (a clone made when the heap
    /// hadn't 4 KB) asks for it again in `frame` and then draws as it
    /// should; with a clock, once a second, not every frame.
    /// The still cache through the effect: taken on the first frame, the
    /// same frames as without it, let go when the gate says, asked again
    /// once a second when refused (every frame without a clock).
    #[test]
    fn a_formulas_still_cache_changes_no_frame_and_gives_way() {
        let src =
            "loop 2\nd = 1/len(px - 63.5, (15.5 - py)*2)\ngray(frac(d*8 + t/2) * step(0.02, d))";
        let mut fx = PsiFx::from_query(&path(src)[Q..]).unwrap();
        assert_eq!(fx.prog.still_lanes(), 1);
        assert!(!fx.still.is_held(), "nothing taken at open");
        let mut c = Canvas::new();
        let want = |ms: u32| {
            let mut out = vec![0u8; PIXELS * 3];
            let p = psi::compile_plain(src).unwrap();
            let inputs = psi::Inputs {
                t: psi::Inputs::at_ms(ms),
                ..Default::default()
            };
            psi::render(
                &p,
                &mut psi::Scratch::<128>::new(),
                &inputs,
                &psi::OWN,
                &mut out,
            );
            out
        };
        for (k, ms) in [0u32, 20, 40, 1000, 1980].into_iter().enumerate() {
            fx.frame(Duration::from_millis(ms.into()), &mut c);
            assert!(fx.still.is_held() && still_held() == 16384, "frame {k}");
            assert!(c.as_bytes() == want(ms).as_slice(), "frame {k} at {ms} ms");
        }
        // The gate says no: given back at the next frame, drawn the same.
        set_still_gate(Some(|_| false));
        fx.frame(Duration::from_millis(60), &mut c);
        assert!(!fx.still.is_held() && still_held() == 0);
        assert!(c.as_bytes() == want(60).as_slice());
        // Yes again: taken (no clock here, so at once).
        set_still_gate(Some(|_| true));
        fx.frame(Duration::from_millis(80), &mut c);
        assert!(fx.still.is_held());
        assert!(c.as_bytes() == want(80).as_slice());
        // A live edit to another formula with one value keeps the lanes and
        // draws the new one (the cache checks whose values it holds).
        let other = "loop 2\nd = 1/len(px - 60, (15.5 - py)*3)\ngray(frac(d*8 - t/2))";
        fx.retune_from(&path(other)[Q..], Retune::Edit).unwrap();
        assert!(fx.still.is_held());
        fx.frame(Duration::from_millis(100), &mut c);
        let mut out = vec![0u8; PIXELS * 3];
        let inputs = psi::Inputs {
            t: psi::Inputs::at_ms(100),
            ..Default::default()
        };
        psi::render(
            &psi::compile_plain(other).unwrap(),
            &mut psi::Scratch::<128>::new(),
            &inputs,
            &psi::OWN,
            &mut out,
        );
        assert!(c.as_bytes() == out.as_slice());
        // A live edit of a table's entries only (the same steps): the
        // lanes stay, the values are kept again, never drawn stale.
        let ta = "loop 4\ntable T = 0.1, 0.5, 0.9, 0.3\nk = T[floor(len(px - 63.5, (15.5 - py)*2)/8)]\ngray(frac(k + t/4))";
        let tb = "loop 4\ntable T = 0.9, 0.2, 0.1, 0.6\nk = T[floor(len(px - 63.5, (15.5 - py)*2)/8)]\ngray(frac(k + t/4))";
        fx.retune_from(&path(ta)[Q..], Retune::Edit).unwrap();
        fx.frame(Duration::from_millis(120), &mut c);
        fx.frame(Duration::from_millis(140), &mut c);
        assert!(fx.still.is_held());
        fx.retune_from(&path(tb)[Q..], Retune::Edit).unwrap();
        assert!(fx.still.is_held());
        fx.frame(Duration::from_millis(160), &mut c);
        let inputs = psi::Inputs {
            t: psi::Inputs::at_ms(160),
            ..Default::default()
        };
        psi::render(
            &psi::compile_plain(tb).unwrap(),
            &mut psi::Scratch::<128>::new(),
            &inputs,
            &psi::OWN,
            &mut out,
        );
        assert!(
            c.as_bytes() == out.as_slice(),
            "a table-only edit drew stale values"
        );
        // One without values lets them go.
        fx.retune_from(&path("gray(x)")[Q..], Retune::Edit).unwrap();
        assert!(!fx.still.is_held());
        set_still_gate(None);
        drop(fx);
        assert_eq!(still_held(), 0);
    }

    #[test]
    fn a_formula_without_its_canvas_asks_again() {
        let mut fx = Fx::parse(&path(STARS)).unwrap();
        let Fx::Psi(p) = &mut fx;
        p.ink = None;
        let mut c = Canvas::new();
        fx.frame(Duration::from_millis(140), &mut c);
        assert_eq!(c.as_bytes(), drawn(STARS, 140).as_slice());
        // With a clock: refused just now (a fake: a time 0.5 s after it),
        // not asked again until a second has passed.
        fn half() -> u32 {
            1_500_000
        }
        set_clock(Some(half));
        let Fx::Psi(p) = &mut fx;
        p.ink = None;
        p.ink_tried = 1_000_001;
        fx.frame(Duration::from_millis(140), &mut c);
        let Fx::Psi(p) = &mut fx;
        assert!(p.ink.is_none(), "asked again within the second");
        p.ink_tried = 400_001;
        fx.frame(Duration::from_millis(140), &mut c);
        set_clock(None);
        let Fx::Psi(p) = &fx;
        assert!(p.ink.is_some(), "asked again after it");
    }

    /// A look's partial redraws (the rows its GIF changed) don't run the
    /// draw stage: its time is the rows', not the lines'.
    #[test]
    fn a_looks_partial_redraw_does_not_draw_again() {
        let src = as_look(STARS, "screen", 60);
        let mut look = FormulaLook::from_path(&super::path(&src, "")).unwrap();
        assert!(look.take());
        let mut c = picture();
        look.apply(Duration::ZERO, &mut c, 0);
        let before = look.held.as_ref().unwrap().ink.as_ref().unwrap().0;
        // Poison the canvas: a partial redraw shows the poison (not drawn
        // again), the next frame of its own draws it fresh.
        look.held.as_mut().unwrap().ink.as_mut().unwrap().0.fill(0);
        let mut c = picture();
        c.dirty_mut().clear();
        c.dirty_mut().mark_span(5, 0, crate::canvas::WIDTH);
        let _ = look.apply(Duration::from_millis(1), &mut c, 0);
        assert!(look
            .held
            .as_ref()
            .unwrap()
            .ink
            .as_ref()
            .unwrap()
            .0
            .iter()
            .all(|&b| b == 0));
        look.redraw();
        let mut c = picture();
        let _ = look.apply(Duration::from_millis(2), &mut c, 0);
        assert_eq!(look.held.as_ref().unwrap().ink.as_ref().unwrap().0, before);
    }

}
