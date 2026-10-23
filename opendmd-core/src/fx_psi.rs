//! `@fx:psi?src=<formula>`: a Psi formula ([`opendmd_psi`]) as a live
//! effect. Behind the `psi` feature (spike; nothing user-facing yet).
//!
//! `@fx:psi?src=<url-encoded source>[&a=0.5&b=..&c=..][&fps=50|100|25]`:
//! the formula compiled when the path is opened (an error makes the path
//! fail to open, as an unknown effect does); `a b c` are its sliders
//! (decimal, default 0); 50 fps by default. A loop is the formula's `loop
//! N` seconds; `t` is the frame's time into it, so the loop is seamless
//! when everything the formula moves completes whole cycles in it.
//!
//! **Memory.** The program (1,808 B) and the VM's working memory
//! ([`opendmd_psi::Scratch`], 8,192 B at 128 lanes) are one allocation made
//! when the path is opened, zeroed and filled in place (never built on the
//! main task's stack); no heap per frame, and [`Fx`](crate::fx::Fx) itself
//! holds only the pointer. The sine and the palettes are lent from
//! [`crate::fx`] (`SIN`, `PALETTES`), so the device carries one copy.
//!
//! **Cost.** A formula whose estimated render time
//! ([`psi::Program::esp32_us`]) is over [`MAX_EST_US`] does not open. One
//! that opens is held to the effect budget by its [`Governor`] once a
//! [`set_clock`] is installed (the firmware): it shows every 2nd, 4th or
//! 8th frame of its own sequence, never a different picture, so a frame on
//! the panel is still the browser's frame for the same `t`.

use crate::canvas::{Canvas, Rgb};
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
    delay_ms: u8,
    inputs: psi::Inputs,
    gov: Governor,
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
static STATS: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];

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

/// The last formula drawn: (mean render µs, frame time ms, asked ms) — for
/// `/api/status`. Zeros before any.
pub fn stats() -> (u32, u32, u32) {
    let d = STATS[1].load(Ordering::Relaxed);
    (STATS[0].load(Ordering::Relaxed), d >> 16, d & 0xffff)
}

/// The tables Psi reads, lent from `fx`.
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

/// An `@fx:psi` query: the source, the frame time asked, the sliders.
struct Query {
    src: String,
    delay_ms: u8,
    sliders: [i32; 3],
}

fn query(query: &str) -> Query {
    let mut q = Query {
        src: String::new(),
        delay_ms: 20,
        sliders: [0; 3],
    };
    for kv in query.split('&') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        match k {
            "src" => q.src = crate::synth::url_decode(v.as_bytes()),
            "fps" => {
                q.delay_ms = match v {
                    "100" => 10,
                    "50" => 20,
                    "25" => 40,
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

/// Compile `q` into `prog` and hold it to [`MAX_EST_US`]; the estimate.
fn compile(q: &Query, prog: &mut psi::Program) -> Result<u32, psi::Error> {
    psi::compile_into(&q.src, prog)?;
    let est = prog.esp32_us();
    if est > MAX_EST_US {
        return Err(psi::Error {
            start: 0,
            end: 0,
            msg: format!(
                "this formula is too heavy for the panel (about {} ms a frame; the limit is {} ms) — simplify it",
                est.div_ceil(1000),
                MAX_EST_US / 1000
            ),
        });
    }
    Ok(est)
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
        let est = compile(&q, &mut fx.prog)?;
        fx.delay_ms = q.delay_ms;
        fx.set_sliders(q.sliders);
        fx.gov = Governor::new(est, fx.ms());
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
    /// estimate says. Only a program's worth (1.8 KB) is allocated, and
    /// only while compiling.
    pub fn retune_from(&mut self, q: &str) -> Result<(), psi::Error> {
        let q = query(q);
        // SAFETY: `Program` is integers all the way down: zero is valid.
        let mut prog = unsafe { Box::<psi::Program>::new_zeroed().assume_init() };
        prog.clear();
        let est = compile(&q, &mut prog)?;
        self.prog = *prog;
        self.delay_ms = q.delay_ms;
        self.set_sliders(q.sliders);
        let g = Governor::new(est, self.ms());
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
        let (bytes, dirty) = canvas.bytes_and_dirty_mut();
        psi::render_with::<LANES>(
            &self.prog,
            &mut self.scratch,
            &self.inputs,
            &tables(),
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

mod alloc_box {
    use super::PsiFx;

    /// The one allocation: a `Box<PsiFx>` made zeroed (every field is
    /// integers, so zero is a valid value) and filled in place, so ~10
    /// KB never passes through the stack. `Clone` the same way.
    pub struct PsiBox(Box<PsiFx>);

    impl PsiBox {
        pub fn new() -> PsiBox {
            // SAFETY: `PsiFx` is integers and arrays of integers all the
            // way down (`Program`, `Scratch`, `Inputs`, `u8`): all-zero is
            // a valid value of it.
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
        assert_eq!(a.clone(), a);
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
    }

    #[test]
    fn the_governor_holds_a_formula_to_the_budget() {
        // From the estimate: 11.4 ms (the ring clock) at 50 fps is held to
        // 40 ms frames before it has drawn anything; light ones are not.
        assert_eq!(Governor::new(11_400, 20).delay_ms(20), 40);
        assert_eq!(Governor::new(7_000, 20).delay_ms(20), 20);
        assert_eq!(Governor::new(6_000, 10).delay_ms(10), 20, "over 5 ms: no 100 fps");
        assert_eq!(Governor::new(MAX_EST_US, 20).delay_ms(20), 160);
        assert_eq!(Governor::new(u32::MAX / 2, 20).delay_ms(20), 160, "8 frames at most");
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
        set_clock(Some(fake_clock));
        const F: &str = "loop 4\nhsv(x + t / 4, 1, y)";
        let p = super::path(F, "fps=100");
        let mut src = crate::synth::SynthSource::new(crate::synth::open(&p).unwrap()).unwrap();
        let prog = psi::compile(F).unwrap();
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
        assert_eq!(stats(), (stats().0, 20, 10));
        assert!(stats().0.abs_diff(7_500) < 200, "{:?}", stats());
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
        psi::render(&prog, &mut psi::Scratch::<128>::new(), &inputs, &psi::OWN, &mut want);
        assert!(c.as_bytes() == want.as_slice());
        let e = src.retune(&super::path("gray(tme)", "")).unwrap().unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidInput);
        assert_eq!(e.to_string(), "unknown name `tme` — did you mean `t`?");
        assert!(src.retune("@clock").is_none());
        assert!(src.retune("@fx:psimple").is_none());
        let mut clock = crate::synth::SynthSource::new(crate::synth::open("@clock").unwrap()).unwrap();
        assert!(clock.retune(&b).is_none(), "a formula only replaces a formula");
    }

    #[test]
    fn opening_reports_why_and_holds_the_limits() {
        use std::io::ErrorKind;
        let open = |p: &str| crate::synth::SynthSource::new(crate::synth::open(p).unwrap()).map(|_| ());
        let e = open(&super::path("gray(tme)", "")).unwrap_err();
        assert_eq!((e.kind(), e.to_string().as_str()), (ErrorKind::InvalidInput, "unknown name `tme` — did you mean `t`?"));
        // Too heavy: ~48 ms a frame estimated.
        let heavy = (0..11).map(|k| format!("noise(x*{k}, y, t)")).collect::<Vec<_>>().join(" + ");
        let heavy = format!("gray({heavy})");
        assert!(psi::compile(&heavy).unwrap().esp32_us() > MAX_EST_US);
        let e = open(&super::path(&heavy, "")).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidInput);
        assert!(e.to_string().starts_with("this formula is too heavy for the panel (about 4"), "{e}");
        // An unknown effect stays what it was: InvalidData (a 415).
        assert_eq!(open("@fx:nonsense").unwrap_err().kind(), ErrorKind::InvalidData);
        // A formula may be longer than other virtual paths, up to its limit.
        let mut src = String::from("loop 8\n");
        for k in 0..60 {
            src.push_str(&format!("v{k} = x * {k}.125 + y\n"));
        }
        src.push_str("gray(frac(v59))");
        let p = super::path(&src, "");
        assert!(p.len() > crate::synth::MAX_PATH && p.len() <= crate::synth::MAX_FORMULA_PATH, "{}", p.len());
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
            assert_eq!((fx.inputs.a, fx.inputs.b, fx.inputs.c), (g.a, g.b, g.c), "{name}");
        }
    }

    #[test]
    fn plays_through_the_synth_source() {
        let p = path(psi::sketches::FIRE);
        assert!(p.len() < crate::synth::MAX_PATH);
        assert_eq!(crate::synth::title(&p).as_deref(), Some("Formula"));
        assert_eq!(crate::synth::title("@fx:psi?src=gray(nope)").as_deref(), Some("Formula"));
        let mut src = crate::synth::SynthSource::new(crate::synth::open(&p).unwrap()).unwrap();
        let mut c = Canvas::new();
        let mut frames = 0;
        while src.next_frame(&mut c).unwrap().is_some() {
            frames += 1;
        }
        assert_eq!(frames, 4000 / 20);
    }
}
