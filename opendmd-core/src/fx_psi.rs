//! `@fx:psi?src=<formula>`: a Psi formula ([`opendmd_psi`]) as a live
//! effect. Behind the `psi` feature (spike; nothing user-facing yet).
//!
//! `@fx:psi?src=<url-encoded source>[&a=0.5&b=..&c=..][&fps=50|100|25]`:
//! the formula compiled when the path is opened (an error makes the path
//! fail to open, as an unknown effect does); `a b c` are its sliders
//! (decimal, default 0); the frame rate is `fps=`, else the formula's
//! `fps N` line, else 50 (a look: `fps=`, else 50). A loop is the formula's `loop
//! N` seconds; `t` is the frame's time into it, so the loop is seamless
//! when everything the formula moves completes whole cycles in it.
//!
//! **Memory.** The program (2,832 B) and the VM's working memory
//! ([`opendmd_psi::Scratch`], 8,192 B at 128 lanes) are one allocation made
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
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;

/// The lane width: one panel row.
pub const LANES: usize = psi::LANES;

/// The compiled formula, its working memory and its options.
pub struct PsiFx {
    prog: psi::Program,
    scratch: psi::Scratch<LANES>,
    /// The draw stage's canvas (4 KB), held only while the formula draws
    /// (`draw { }`); `None` is all-zero, so the zeroed box is valid.
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
    delay_ms: u8,
    inputs: psi::Inputs,
    gov: Governor,
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
pub const MAX_EST_US: u32 = 40_000;

/// Keeps a formula inside the effect budget (CLAUDE.md, "Live effects":
/// ≤ 5 ms a frame at 100 fps, else ≤ 40 % of the frame — 8 ms at 50 fps)
/// by holding each frame for 1, 2, 4 or 8 of its frame times. Fed each
/// frame's measured render time; starts from the compiler's estimate, so a
/// heavy formula is slowed before its first frame can be late. Slows down
/// at once (the running mean over budget, or one frame longer than its
/// own slot); speeds up after [`Governor::CALM`] frames under 31/32 of the
/// faster step's budget. (Renders run ~3% slower while a transition or
/// heavy traffic shares the flash cache: on the spare a 7.7 ms formula
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
}

impl Governor {
    pub const MAX_LEVEL: u8 = 3;
    pub const CALM: u16 = 100;
    pub const SKIP: u8 = 2;

    /// A live edit: the next frames run cold, leave them out.
    pub fn edited(&mut self) {
        self.skip = Self::SKIP;
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
static STATS: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];

/// How a formula is compiled: [`psi::compile_into`] in place, unless the
/// firmware installs another with [`set_compiler`].
pub type Compiler = fn(&str, &mut psi::Program) -> Result<psi::Info, psi::Error>;

static COMPILER: std::sync::OnceLock<Compiler> = std::sync::OnceLock::new();

/// Install how formulas are compiled (once). The firmware runs the compiler
/// on a stack of its own: it recurses up to [`psi::MAX_DEPTH`] levels
/// (~9 KB at the worst on the ESP32), more than the main task (16 KB, much
/// of it used where a path opens) or the HTTP server (6.5 KB, which checks
/// an attract effect) can spare.
pub fn set_compiler(f: Compiler) {
    let _ = COMPILER.set(f);
}

/// Compile `src` into `prog` the installed way.
pub fn compile_into(src: &str, prog: &mut psi::Program) -> Result<psi::Info, psi::Error> {
    match COMPILER.get() {
        Some(f) => f(src, prog),
        None => psi::compile_into(src, prog),
    }
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

/// The last formula drawn: (mean render µs, frame time ms, asked ms, the
/// last compile's µs) — for `/api/status`. Zeros before any.
pub fn stats() -> (u32, u32, u32, u32) {
    let d = STATS[1].load(Ordering::Relaxed);
    (
        STATS[0].load(Ordering::Relaxed),
        d >> 16,
        d & 0xffff,
        STATS[2].load(Ordering::Relaxed),
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

/// The `@fx:psi?src=…` path for a formula: [`psi::compact`]ed, then
/// URL-encoded lightly (a space is `+`; `% & + # ?` and anything outside
/// printable ASCII are `%XX`), with `opts` (`a=0.5&fps=100`) after it.
/// May be longer than a virtual path may be (512 B): check.
pub fn path(src: &str, opts: &str) -> String {
    let mut p = String::from("@fx:psi?src=");
    for b in psi::compact(src).bytes() {
        match b {
            b' ' => p.push('+'),
            b'%' | b'&' | b'+' | b'#' | b'?' => p.push_str(&format!("%{b:02X}")),
            0x21..=0x7e => p.push(b as char),
            _ => p.push_str(&format!("%{b:02X}")),
        }
    }
    if !opts.is_empty() {
        p.push('&');
        p.push_str(opts);
    }
    p
}

/// An `@fx:psi` query: the source, the frame time asked (`fps=`; `None`:
/// the formula's `fps N`, else 20 ms), the sliders.
struct Query {
    src: String,
    delay_ms: Option<u8>,
    sliders: [i32; 3],
}

fn query(query: &str) -> Query {
    let mut q = Query {
        src: String::new(),
        delay_ms: None,
        sliders: [0; 3],
    };
    for kv in query.split('&') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        match k {
            "src" => q.src = crate::synth::url_decode(v.as_bytes()),
            "fps" => {
                q.delay_ms = match v {
                    "100" => Some(10),
                    "50" => Some(20),
                    "25" => Some(40),
                    _ => q.delay_ms,
                }
            }
            "a" | "b" | "c" => {
                if let Some(n) = psi::number(v) {
                    q.sliders[usize::from(k.as_bytes()[0] - b'a')] = n;
                }
            }
            _ => {}
        }
    }
    q
}

/// Compile `q` into `prog` and hold it to what the [`Governor`] can keep in
/// budget at its frame rate: [`MAX_EST_US`], and at 100 fps (grid 10 ms,
/// slowest step 80 ms) 32 ms. The estimate and the frame time asked (ms:
/// the path's `fps=`, else the formula's `fps N`, else 20).
fn compile(q: &Query, prog: &mut psi::Program) -> Result<(u32, u8), psi::Error> {
    let t0 = clock().map(|c| (c, c()));
    compile_into(&q.src, prog)?;
    if let Some((c, t0)) = t0 {
        STATS[2].store(c().wrapping_sub(t0), Ordering::Relaxed);
    }
    let est = prog.esp32_us();
    let delay = q
        .delay_ms
        .or(prog.frame_ms().map(|ms| ms as u8))
        .unwrap_or(20);
    let limit = MAX_EST_US.min(Governor::slowest_budget_us(grid(delay)));
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
    Ok((est, delay))
}

/// What the governor starts from: the estimate with the still cache when
/// the formula has values for one and the gate would let it take it now
/// (it is taken on the first frame, which the governor leaves out), else
/// `est` (every step every frame). The refusal limit is always held to
/// `est`: the cache may be refused or let go at any time.
fn start_estimate(prog: &psi::Program, est: u32) -> u32 {
    if prog.still_lanes() > 0 && still_allowed(psi::Still::bytes_for(prog)) {
        prog.esp32_us_still()
    } else {
        est
    }
}

/// The frame grid for a frame time asked (ms), floored like [`PsiFx::ms`].
fn grid(delay_ms: u8) -> u32 {
    u32::from(delay_ms.max(crate::fx::floor()))
}

/// What a formula that draws gets when the heap hasn't its canvas (4 KB):
/// as the compiler's own thread does when it can't start.
pub const NO_CANVAS: &str = "the player is short of memory just now: try again in a moment";

/// A canvas for `prog` if it draws (taken from the heap only if it may
/// fail: `None` for one that doesn't draw, `Err` when the heap hasn't 4 KB).
fn canvas_for(
    prog: &psi::Program,
    held: Option<Box<psi::Ink>>,
) -> Result<Option<Box<psi::Ink>>, psi::Error> {
    if !prog.draws() {
        return Ok(None);
    }
    match held.or_else(psi::Ink::try_boxed) {
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
    std::io::Error::new(std::io::ErrorKind::InvalidInput, e.msg)
}

/// Why a formula's path does not open (`None` if it is not a formula, or
/// it does).
pub fn open_error(path: &str) -> Option<std::io::Error> {
    if !crate::synth::is_formula(path) {
        return None;
    }
    let q = path[crate::synth::FORMULA.len()..].trim_start_matches('?');
    PsiFx::from_query(q).err().map(io_error)
}

impl PsiFx {
    /// The effect for an `@fx:psi` query, or why not.
    pub fn from_query(q: &str) -> Result<PsiBox, psi::Error> {
        let q = query(q);
        let mut fx = PsiBox::new();
        let (est, delay) = compile(&q, &mut fx.prog)?;
        fx.ink = canvas_for(&fx.prog, None)?;
        fx.delay_ms = delay;
        fx.set_sliders(q.sliders);
        fx.gov = Governor::new(start_estimate(&fx.prog, est), fx.ms());
        Ok(fx)
    }

    fn set_sliders(&mut self, s: [i32; 3]) {
        self.inputs.a = s[0];
        self.inputs.b = s[1];
        self.inputs.c = s[2];
    }

    /// Take another query's formula, sliders and frame rate in place (a
    /// live edit on the panel): the clock, the working memory and the
    /// governor stay — a heavier formula starts at least as slow as its
    /// estimate says. Only a program's worth (2.8 KB) is allocated, and
    /// only while compiling.
    pub fn retune_from(&mut self, q: &str) -> Result<(), psi::Error> {
        let q = query(q);
        // SAFETY: `Program` is integers all the way down: zero is valid.
        let mut prog = unsafe { Box::<psi::Program>::new_zeroed().assume_init() };
        prog.clear();
        let (est, delay) = compile(&q, &mut prog)?;
        // A formula that starts drawing takes its canvas here, between
        // frames (as an open does); one that stops gives it back.
        self.ink = canvas_for(&prog, self.ink.take())?;
        // The still cache keeps its lanes for a formula with as many values
        // (the next full frame keeps the new formula's: it checks), else
        // lets them go and the next frame asks again.
        if self.still.held_bytes() != psi::Still::bytes_for(&prog) {
            self.still.release();
            set_still_held(0);
        }
        self.prog = *prog;
        self.delay_ms = delay;
        self.set_sliders(q.sliders);
        let g = Governor::new(start_estimate(&self.prog, est), self.ms());
        if g.level > self.gov.level {
            self.gov = g;
        }
        self.gov.edited();
        Ok(())
    }

    fn ms(&self) -> u32 {
        u32::from(self.delay_ms.max(crate::fx::floor()))
    }

    /// The nominal frame time (the governor may hold frames longer).
    pub fn delay(&self) -> Duration {
        Duration::from_millis(u64::from(self.ms()))
    }

    /// The loop as played: its frames on the nominal grid.
    pub fn loop_time(&self) -> Duration {
        Duration::from_millis(u64::from(self.loop_frames() * self.ms()))
    }

    pub fn program(&self) -> &psi::Program {
        &self.prog
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

impl Drop for PsiFx {
    fn drop(&mut self) {
        if self.still.is_held() {
            set_still_held(0);
        }
    }
}

impl Effect for PsiFx {
    fn name(&self) -> &'static str {
        "psi"
    }

    fn loop_frames(&self) -> u32 {
        (self.prog.loop_ms() / self.ms()).max(1)
    }

    fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration {
        let n = self.loop_frames();
        let grid = self.ms();
        let ms = (t.as_secs() as u32)
            .wrapping_mul(1000)
            .wrapping_add(t.subsec_millis());
        // The frame of the grid at or before `t`: held frames skip some, never
        // draw between them.
        let i = ms / grid % n;
        self.inputs.t = psi::Inputs::at_ms(i * grid);
        if let Some(now) = crate::synth::wall_now() {
            self.inputs.hour = now.hour;
            self.inputs.minute = now.minute;
            self.inputs.second = now.second;
        }
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
                self.ink = psi::Ink::try_boxed();
                self.ink_tried = if self.ink.is_some() { 0 } else { t0 | 1 };
            }
        }
        self.still_step(clock.is_some(), t0);
        let (bytes, dirty) = canvas.bytes_and_dirty_mut();
        psi::render_rows_still::<LANES>(
            &self.prog,
            &mut self.scratch,
            self.ink.as_deref_mut(),
            Some(&mut self.still),
            &self.inputs,
            &tables(),
            None,
            u32::MAX,
            bytes,
        );
        dirty.mark_all();
        let Some(c) = clock else {
            return self.delay();
        };
        if self.gov.observe(c().wrapping_sub(t0), grid) {
            log::info!(
                "formula: {} us a frame -> {} ms frames (asked {grid})",
                self.gov.avg_us(),
                self.gov.delay_ms(grid)
            );
        }
        let ms = self.gov.delay_ms(grid);
        STATS[0].store(self.gov.avg_us(), Ordering::Relaxed);
        STATS[1].store(ms << 16 | grid, Ordering::Relaxed);
        Duration::from_millis(u64::from(ms))
    }
}

/// A formula estimated to take longer than this a frame (µs on the ESP32)
/// is not drawn over GIFs: it runs at the GIF's own frame rate whenever the
/// GIF changes (a governor can only slow its own animation), next to the
/// GIF's decoding (p90 ~4 ms) and the blit. 8 ms until any-look
/// (2026-10-26), on an estimate that read full-panel looks ~1.5× low; with
/// the estimate calibrated on the device, 12 ms: on the spare, looks of
/// 8.6 and 12.3 ms measured left the GIFs at 0 steady misses (the animated
/// one at 25 fps: over 8 ms its governor holds 40 ms frames).
pub const LOOK_MAX_EST_US: u32 = 12_000;

/// Why a formula that never lets the picture under it show through (it
/// reads none of `ur ug ub ul`, and its bottom layer covers every pixel) is
/// not drawn over GIFs: the GIF would be hidden. Any Formula is a look once
/// the web app composes it with a blend or an amount under 100 %
/// (`psi::look`); this is the one choice left that hides the GIF.
pub const COVERS: &str =
    "this Formula covers the GIF entirely: choose a lower amount or another blend";

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
    prog: Box<psi::Program>,
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

    /// The look for a formula's `@fx:psi?src=…` path (sliders and `fps=`
    /// as for the effect), or why not: it does not compile, or it is too
    /// heavy to draw over GIFs ([`LOOK_MAX_EST_US`]).
    pub fn from_path(path: &str) -> Result<FormulaLook, psi::Error> {
        let q = query(Self::query_of(path));
        // SAFETY: `Program` is integers all the way down: zero is valid.
        let mut prog = unsafe { Box::<psi::Program>::new_zeroed().assume_init() };
        let est = Self::compile(&q, &mut prog)?;
        let mut look = FormulaLook {
            prog,
            held: None,
            inputs: psi::Inputs::default(),
            delay_ms: Self::delay(&q),
            gov: Governor::default(),
            t0: None,
            next: None,
            fresh: true,
            ink_tried: None,
            cold: Governor::SKIP,
        };
        look.set(&q, est);
        Ok(look)
    }

    fn query_of(path: &str) -> &str {
        path.strip_prefix(crate::synth::FORMULA)
            .unwrap_or(path)
            .trim_start_matches('?')
    }

    fn compile(q: &Query, prog: &mut psi::Program) -> Result<u32, psi::Error> {
        crate::fx_psi::compile_into(&q.src, prog)?;
        if !prog.over() {
            prog.clear();
            return Err(psi::Error {
                start: 0,
                end: 0,
                msg: COVERS.to_string(),
            });
        }
        let est = prog.esp32_us();
        if est > LOOK_MAX_EST_US {
            prog.clear();
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
        Ok(est)
    }

    /// A look's frame time asked: its path's `fps=`, else 20 ms. A
    /// formula's own `fps N` is not followed over a GIF: the look is drawn
    /// at the GIF's frames, inside the GIF's budget ([`LOOK_MAX_EST_US`]).
    fn delay(q: &Query) -> u8 {
        q.delay_ms.unwrap_or(20)
    }

    fn set(&mut self, q: &Query, est: u32) {
        self.delay_ms = Self::delay(q);
        self.inputs.a = q.sliders[0];
        self.inputs.b = q.sliders[1];
        self.inputs.c = q.sliders[2];
        let g = Governor::new(est, self.ms());
        if g.level > self.gov.level || self.t0.is_none() {
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

    /// Another formula in place (a live edit from the editor): the clock,
    /// the buffers and the governor stay. On an error nothing changes.
    pub fn retune(&mut self, path: &str) -> Result<(), psi::Error> {
        let q = query(Self::query_of(path));
        // SAFETY: as in `from_path`.
        let mut prog = unsafe { Box::<psi::Program>::new_zeroed().assume_init() };
        let est = Self::compile(&q, &mut prog)?;
        self.prog = prog;
        // Held: a look that starts drawing takes its canvas now (if the heap
        // hasn't it, `apply` asks again a second later and `ink` reads 0
        // till then); one that stops gives it back.
        self.ink_tried = None;
        if let Some(b) = self.held.as_mut() {
            b.ink = canvas_for(&self.prog, b.ink.take()).unwrap_or(None);
            let ink = b
                .ink
                .as_ref()
                .map_or(0, |_| std::mem::size_of::<psi::Ink>());
            LOOK_STATS[2].store((Self::BUFFER_BYTES + ink) as u32, Ordering::Relaxed);
        }
        self.set(&q, est);
        Ok(())
    }

    pub fn program(&self) -> &psi::Program {
        &self.prog
    }

    /// Whether it has frames of its own (it reads `t` or the clock).
    pub fn is_animated(&self) -> bool {
        self.prog.inputs() & 24 != 0
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
        let Ok(ink) = canvas_for(&self.prog, None) else {
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
            b.ink = psi::Ink::try_boxed();
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
            if let Some(w) = crate::synth::wall_now() {
                self.inputs.hour = w.hour;
                self.inputs.minute = w.minute;
                self.inputs.second = w.second;
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
        psi::render_rows_ink::<LANES>(
            &self.prog,
            &mut b.scratch,
            b.ink.as_deref_mut(),
            &self.inputs,
            &tables(),
            Some(under.as_bytes()),
            rows,
            bytes,
        );
        if rows == u32::MAX {
            dirty.mark_all();
        } else {
            let mut r = rows;
            while r != 0 {
                let y = r.trailing_zeros() as usize;
                r &= r - 1;
                dirty.mark_span(y, 0, crate::canvas::WIDTH);
            }
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

mod alloc_box {
    use super::PsiFx;

    /// The one allocation: a `Box<PsiFx>` made zeroed (every field is
    /// integers, so zero is a valid value) and filled in place, so ~10
    /// KB never passes through the stack. `Clone` the same way.
    pub struct PsiBox(Box<PsiFx>);

    impl PsiBox {
        pub fn new() -> PsiBox {
            // SAFETY: `PsiFx` is integers and arrays of integers all the
            // way down (`Program`, `Scratch`, `Inputs`, `u8`) but for its
            // canvas, an `Option<Box<_>>`, whose all-zero value is `None`:
            // all-zero is a valid value of it.
            let mut b = PsiBox(unsafe { Box::<PsiFx>::new_zeroed().assume_init() });
            b.0.prog.clear();
            b.0.scratch.reset();
            b.0.delay_ms = 20;
            b.0.gov = super::Governor::default();
            b
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
            b.0.prog = self.0.prog;
            // (A clone that can't have its canvas draws nothing: `ink` 0.)
            b.0.ink = super::canvas_for(&b.0.prog, None).unwrap_or(None);
            b.0.delay_ms = self.0.delay_ms;
            b.0.inputs = self.0.inputs;
            b.0.gov = self.0.gov;
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

    fn path(src: &str) -> String {
        super::path(src, "")
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
        assert!(Fx::parse("@fx:psi").is_none());
        assert!(Fx::parse(&path("loop 2\ngray(x)")).is_some());
        assert_eq!(
            PsiFx::from_query("src=gray(tme)").unwrap_err().msg,
            "unknown name `tme` — did you mean `t`?"
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
        // Too heavy for the governor at 100 fps (over 32 ms): refused, as
        // with `fps=100`, unless the path asks for 50.
        let mid = (0..9)
            .map(|k| format!("noise(x*{k}, y, t)"))
            .collect::<Vec<_>>()
            .join(" + ");
        let mid = format!("fps 100\ngray({mid})");
        assert!(PsiFx::from_query(&super::path(&mid, "")[8..])
            .unwrap_err()
            .msg
            .ends_with("the limit is 32 ms) — simplify it"));
        assert!(PsiFx::from_query(&super::path(&mid, "fps=50")[8..]).is_ok());
        // Over its 5 ms budget, `fps 100` starts from the estimate: two
        // moving noises (~9.1 ms) at 40 ms frames before its first; the
        // ring clock from its estimate with the still cache (~5.7 ms, 11.5
        // without) at 20, and at 40 when the gate would refuse the cache.
        let heavy = "fps 100\nloop 4\ngray(noise(x*8, y*4, t) + noise(x*3, y*9, t*2))";
        let fx = PsiFx::from_query(&super::path(heavy, "")[8..]).unwrap();
        assert_eq!((fx.ms(), fx.gov.delay_ms(fx.ms())), (10, 40));
        let ring = format!("fps 100\n{}", psi::sketches::RING_CLOCK);
        let fx = PsiFx::from_query(&super::path(&ring, "")[8..]).unwrap();
        assert_eq!((fx.ms(), fx.gov.delay_ms(fx.ms())), (10, 20));
        set_still_gate(Some(|_| false));
        let fx = PsiFx::from_query(&super::path(&ring, "")[8..]).unwrap();
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
    fn every_sketch_fits_a_path() {
        for (name, src) in psi::sketches::ALL {
            let p = path(src);
            assert!(p.len() <= crate::synth::MAX_PATH, "{name}: {} B", p.len());
        }
        assert_eq!(
            path("v = x % 2 + 1 & 3 # é"),
            "@fx:psi?src=v+=+x+%25+2+%2B+1+%26+3+%23+%C3%A9"
        );
        // Characters in quotes, the URL's own among them, come back as they
        // went.
        let src = "k = ' ' + '+' + '%' + '#' + '&' + '?' + ';' + '=' -- c\ngray(glyph(k, px % 5, 31 - py))";
        let q = query(path(src).strip_prefix("@fx:psi?").unwrap());
        assert_eq!(q.src, psi::compact(src));
        assert_eq!(psi::compile(&q.src).unwrap(), psi::compile(src).unwrap());
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
        // The spare's Matrix: 7.7 ms, pushed over 8 ms through a transition,
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
        src.retune(&b).unwrap().unwrap();
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
            .retune(&super::path("gray(tme)", ""))
            .unwrap()
            .unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidInput);
        assert_eq!(e.to_string(), "unknown name `tme` — did you mean `t`?");
        assert!(src.retune("@clock").is_none());
        assert!(src.retune("@fx:psimple").is_none());
        let mut clock =
            crate::synth::SynthSource::new(crate::synth::open("@clock").unwrap()).unwrap();
        assert!(
            clock.retune(&b).is_none(),
            "a formula only replaces a formula"
        );
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
                "unknown name `tme` — did you mean `t`?"
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
        // 32-40 ms: fine at 50 fps, too heavy for the governor at 100.
        let mid = (0..9)
            .map(|k| format!("noise(x*{k}, y, t)"))
            .collect::<Vec<_>>()
            .join(" + ");
        let mid = format!("gray({mid})");
        let est = psi::compile(&mid).unwrap().esp32_us();
        assert!((32_001..=MAX_EST_US).contains(&est), "{est}");
        open(&super::path(&mid, "")).unwrap();
        let e = open(&super::path(&mid, "fps=100")).unwrap_err();
        assert!(
            e.to_string().ends_with("the limit is 32 ms) — simplify it"),
            "{e}"
        );
        // An unknown effect stays what it was: InvalidData (a 415).
        assert_eq!(
            open("@fx:nonsense").unwrap_err().kind(),
            ErrorKind::InvalidData
        );
        // A formula may be longer than other virtual paths, up to its limit.
        let mut src = String::from("loop 8\n");
        for k in 0..60 {
            src.push_str(&format!("v{k} = x * {k}.125 + y\n"));
        }
        src.push_str("gray(frac(v59))");
        let p = super::path(&src, "");
        assert!(
            p.len() > crate::synth::MAX_PATH && p.len() <= crate::synth::MAX_FORMULA_PATH,
            "{}",
            p.len()
        );
        open(&p).unwrap();
        let text = format!("@text?msg={}", "x".repeat(600));
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
            assert!(*fx.program() == psi::compile(src).unwrap(), "{name}");
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
            crate::synth::title("@fx:psi?src=gray(nope)").as_deref(),
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
        look.retune(&super::path(
            "loop 2\nlayer multiply { gray(1 - t/4) }",
            "fps=25",
        ))
        .unwrap();
        assert!(look.is_held());
        assert_eq!(look.due(), Some(Duration::ZERO));
        let want = over("loop 2\nlayer multiply { gray(1 - t/4) }", 0, &under);
        let out = look.apply(ms(3010), &mut under, 0);
        assert!(out.as_bytes() == want.as_slice());
        // A bad edit changes nothing.
        assert!(look.retune(&super::path("gray(tme)", "")).is_err());
        assert!(look.is_animated());
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
            let src = psi::look::compose("hsv(x, 1, 1)", b, k).unwrap();
            let mut look = FormulaLook::from_path(&super::path(&src, "")).unwrap();
            assert!(look.take());
            let mut c = picture();
            let out = look.apply(Duration::ZERO, &mut c, 0);
            assert!(
                out.as_bytes() == over(&src, 0, &under).as_slice(),
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
            FormulaLook::from_path("@fx:psi?src=gray(tme)")
                .unwrap_err()
                .msg,
            "unknown name `tme` — did you mean `t`?"
        );
        assert_eq!(
            FormulaLook::BUFFER_BYTES,
            8192 + 12288 + std::mem::size_of::<LookBuffers>()
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
            crate::synth::SynthSource::new(crate::synth::open("@fx:starfield").unwrap()).unwrap();
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
            "frame { m = minute / 60 }\nlayer { gray(1, step(px, m*128) * 0.3) }",
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
        look.apply(ms(990), &mut canvas, 0).dirty_mut().clear();
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
                    assert_eq!(out.dirty().count(), 4096);
                    assert!(out.as_bytes() == over(F, t, &snap).as_slice(), "t {t}");
                }
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
        let Fx::Psi(p) = &mut fx else { panic!() };
        assert!(p.ink.is_some());
        // To one that doesn't draw: the canvas goes; back: it comes again.
        let q = |src: &str| super::path(src, "")["@fx:psi?".len()..].to_string();
        p.retune_from(&q("gray(x)")).unwrap();
        assert!(p.ink.is_none());
        let src = "draw { dot(10, 10) }\ngray(ink)";
        p.retune_from(&q(src)).unwrap();
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
        let src = psi::look::compose(STARS, "screen", 60).unwrap();
        let mut look = FormulaLook::from_path(&super::path(&src, "")).unwrap();
        assert_eq!(look.buffer_bytes(), FormulaLook::BUFFER_BYTES + 4096);
        assert!(look.take());
        let mut c = picture();
        let out = look.apply(Duration::ZERO, &mut c, 0);
        assert_eq!(out.as_bytes(), over(&src, 0, &under).as_slice());
        // Retuned to a look that doesn't draw: the canvas goes.
        let plain = psi::look::compose("gray(x)", "screen", 40).unwrap();
        look.retune(&super::path(&plain, "")).unwrap();
        assert_eq!(look.buffer_bytes(), FormulaLook::BUFFER_BYTES);
        assert!(look.held.as_ref().unwrap().ink.is_none());
        look.retune(&super::path(&src, "")).unwrap();
        assert!(look.held.as_ref().unwrap().ink.is_some());
    }

    /// Psi's `line` is the built-in effects' rasteriser: the same pixels
    /// for any two ends (Psi counts y up from the bottom, `fx::line` rows
    /// down from the top), off the panel too.
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
            crate::fx::line(
                &mut c,
                (x0, y0, 255),
                (x1, y1, 255),
                Rgb::new(255, 255, 255),
            );
            let src = format!(
                "draw {{ line({x0}, {}, {x1}, {}) }}\ngray(ink)",
                31 - y0,
                31 - y1
            );
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
        let mut fx = PsiFx::from_query(&path(src)[8..]).unwrap();
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
        fx.retune_from(&path(other)[8..]).unwrap();
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
        fx.retune_from(&path(ta)[8..]).unwrap();
        fx.frame(Duration::from_millis(120), &mut c);
        fx.frame(Duration::from_millis(140), &mut c);
        assert!(fx.still.is_held());
        fx.retune_from(&path(tb)[8..]).unwrap();
        assert!(fx.still.is_held());
        fx.frame(Duration::from_millis(160), &mut c);
        let inputs = psi::Inputs {
            t: psi::Inputs::at_ms(160),
            ..Default::default()
        };
        psi::render(&psi::compile_plain(tb).unwrap(), &mut psi::Scratch::<128>::new(), &inputs, &psi::OWN, &mut out);
        assert!(c.as_bytes() == out.as_slice(), "a table-only edit drew stale values");
        // One without values lets them go.
        fx.retune_from(&path("gray(x)")[8..]).unwrap();
        assert!(!fx.still.is_held());
        set_still_gate(None);
        drop(fx);
        assert_eq!(still_held(), 0);
    }

    #[test]
    fn a_formula_without_its_canvas_asks_again() {
        let mut fx = Fx::parse(&path(STARS)).unwrap();
        let Fx::Psi(p) = &mut fx else { panic!() };
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
        let Fx::Psi(p) = &mut fx else { panic!() };
        p.ink = None;
        p.ink_tried = 1_000_001;
        fx.frame(Duration::from_millis(140), &mut c);
        let Fx::Psi(p) = &mut fx else { panic!() };
        assert!(p.ink.is_none(), "asked again within the second");
        p.ink_tried = 400_001;
        fx.frame(Duration::from_millis(140), &mut c);
        set_clock(None);
        let Fx::Psi(p) = &fx else { panic!() };
        assert!(p.ink.is_some(), "asked again after it");
    }

    /// A look's partial redraws (the rows its GIF changed) don't run the
    /// draw stage: its time is the rows', not the lines'.
    #[test]
    fn a_looks_partial_redraw_does_not_draw_again() {
        let src = psi::look::compose(STARS, "screen", 60).unwrap();
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
