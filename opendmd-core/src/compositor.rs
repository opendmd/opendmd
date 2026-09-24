//! Transitions between files: the compositor that sits between the decoder's
//! canvas and whatever shows the picture (the panel, the simulator window,
//! the browser preview).
//!
//! **The decoder's canvas is never drawn on.** It is the file's accumulating
//! state (`Keep`/`Previous` disposal and ODM `KEEP` runs read it back), so a
//! transition cannot blend into it. Instead, while one runs, the compositor
//! owns two more 128x32 buffers — a **snapshot** of the picture that was on
//! display when the new file started, and an **output** canvas that it
//! composites the snapshot and the new file's current frame into — and the
//! caller shows the output instead of the decoder's canvas
//! ([`Engine::display`](crate::engine::Engine::display)). Both are allocated
//! when a transition starts and freed when it ends: 24 KB for ~400 ms, and
//! nothing in the steady state. The allocation is fallible; with no room the
//! file simply cuts in, as it would with [`Kind::None`].
//!
//! **Layers.** The output is built by [`compose`], a pure function of the
//! transition `(kind, duration, t)` and its two inputs. It is the first (and
//! for now the only) layer over the decoded picture; overlays that come later
//! (a title card, a clock) are further passes over the same output canvas in
//! the same step, each marking what it changes, so they reuse this plumbing:
//! the output buffer, its dirty map, and the "show the output, not the
//! decoder's canvas" switch.
//!
//! **Dirty marks.** Every step writes only the output pixels whose value
//! changes and marks exactly those, so the firmware's dirty-region blit
//! pushes a dissolve's few hundred new pixels per step rather than the whole
//! screen. When the transition ends the decoder's canvas is shown again, and
//! its dirty map is replaced by exactly the pixels where it differs from the
//! last output (plus anything the output still had unpushed), so the handover
//! costs nothing visible and nothing extra.

use crate::canvas::{Canvas, DirtyMap, Rgb, HEIGHT, PIXELS, WIDTH};
use std::fmt;
use std::time::Duration;

/// How a transition moves from the old picture to the new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Kind {
    /// Cut straight to the new file.
    None,
    /// Pixels flip from old to new in a fixed pseudo-random order (a
    /// bijection on the 4,096 pixel indices computed per pixel — no table).
    #[default]
    Dissolve,
    /// A vertical edge sweeps right to left, the new picture behind it, with
    /// a 2-pixel soft band.
    WipeLeft,
    /// The edge sweeps left to right.
    WipeRight,
    /// A horizontal edge sweeps top to bottom.
    WipeDown,
    /// The edge sweeps bottom to top.
    WipeUp,
    /// Eight 4-row horizontal bands, each opening top to bottom at once.
    Blinds,
    /// The new picture slides in from the right and pushes the old one out
    /// to the left.
    Push,
    /// Dip to black over the first half, up to the new picture over the
    /// second.
    Fade,
}

impl Kind {
    /// Every kind, in the order a UI lists them.
    pub const ALL: [Kind; 9] = [
        Kind::Dissolve,
        Kind::WipeLeft,
        Kind::WipeRight,
        Kind::WipeDown,
        Kind::WipeUp,
        Kind::Blinds,
        Kind::Push,
        Kind::Fade,
        Kind::None,
    ];

    /// The name used in NVS, `/api/config`, the simulator's `--transition`
    /// and the web app: lowercase, words joined by `_`.
    pub const fn name(self) -> &'static str {
        match self {
            Kind::None => "none",
            Kind::Dissolve => "dissolve",
            Kind::WipeLeft => "wipe_left",
            Kind::WipeRight => "wipe_right",
            Kind::WipeDown => "wipe_down",
            Kind::WipeUp => "wipe_up",
            Kind::Blinds => "blinds",
            Kind::Push => "push",
            Kind::Fade => "fade",
        }
    }

    /// [`name`](Self::name) back to a kind; case-insensitive, and `-` is
    /// accepted for `_`. `wipe` alone is [`Kind::WipeLeft`].
    pub fn from_name(s: &str) -> Option<Kind> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("wipe") {
            return Some(Kind::WipeLeft);
        }
        Kind::ALL.into_iter().find(|k| {
            let n = k.name();
            n.len() == s.len()
                && n.bytes()
                    .zip(s.bytes())
                    .all(|(a, b)| a == b.to_ascii_lowercase() || (a == b'_' && b == b'-'))
        })
    }
}

/// A transition: what it does and how long it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Transition {
    pub kind: Kind,
    pub duration: Duration,
}

/// The duration used when none is given: 400 ms.
pub const DEFAULT_DURATION: Duration = Duration::from_millis(400);
/// Durations are clamped to this range by [`Transition::new`] and
/// [`Transition::parse`]: shorter than one step is a cut, and a transition
/// holds 24 KB for as long as it runs.
pub const MIN_DURATION: Duration = Duration::from_millis(20);
pub const MAX_DURATION: Duration = Duration::from_millis(2000);
/// While a transition runs the picture is recomposited at least this often:
/// at every frame of the new file (whose own timing carries on underneath)
/// and, between them, a step this long after the last.
pub const STEP: Duration = Duration::from_millis(20);
/// The most a transition's clock moves between two steps: a stall longer
/// than this (a slow file open) delays the rest of it instead of skipping
/// ahead.
pub const MAX_ADVANCE: Duration = Duration::from_millis(40);
/// Width in pixels of a wipe's soft edge.
pub const WIPE_BAND: u64 = 2;
/// Rows per band of [`Kind::Blinds`] (eight bands on the 32-row panel).
pub const BLIND_ROWS: usize = HEIGHT / 8;

impl Transition {
    /// No transition: files cut in.
    pub const NONE: Transition = Transition {
        kind: Kind::None,
        duration: DEFAULT_DURATION,
    };
    /// The shipped default, Dissolve over 400 ms.
    pub const DEFAULT: Transition = Transition {
        kind: Kind::Dissolve,
        duration: DEFAULT_DURATION,
    };

    /// `duration` clamped to [`MIN_DURATION`]..=[`MAX_DURATION`].
    pub fn new(kind: Kind, duration: Duration) -> Transition {
        Transition {
            kind,
            duration: duration.clamp(MIN_DURATION, MAX_DURATION),
        }
    }

    /// `kind[:ms]`, e.g. `dissolve`, `wipe_left:250`, `none`; what NVS
    /// stores and the simulator's `--transition` takes. The duration defaults
    /// to 400 ms and is clamped like [`new`](Self::new).
    pub fn parse(s: &str) -> Option<Transition> {
        let (kind, ms) = match s.split_once(':') {
            Some((k, ms)) => (k, Some(ms.trim().parse::<u64>().ok()?)),
            None => (s, None),
        };
        let kind = Kind::from_name(kind)?;
        Some(Transition::new(
            kind,
            ms.map_or(DEFAULT_DURATION, Duration::from_millis),
        ))
    }

    pub fn is_none(&self) -> bool {
        self.kind == Kind::None
    }
}

impl Default for Transition {
    fn default() -> Self {
        Transition::DEFAULT
    }
}

/// `kind:ms`, which [`Transition::parse`] reads back.
impl fmt::Display for Transition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind.name(), self.duration.as_millis())
    }
}

/// Where pixel `i` (`y * WIDTH + x`) falls in the dissolve order: a
/// bijection on `0..4096` made of steps that are each invertible on 12 bits
/// (multiplying by an odd constant, adding, `x ^= x >> k`), so every pixel
/// has a distinct rank and none is skipped. Pixel `i` has flipped to the new
/// picture once `rank(i) < 4096 * t / duration`. [`dissolve_pixel`] is the
/// inverse, which is how a step finds the pixels it flips without a table.
pub fn dissolve_rank(i: usize) -> u32 {
    let mut x = i as u32 & MASK12;
    x = x.wrapping_mul(M1).wrapping_add(A1) & MASK12;
    x ^= x >> 7;
    x = x.wrapping_mul(M2) & MASK12;
    x ^= x >> 5;
    x = x.wrapping_mul(M3).wrapping_add(A3) & MASK12;
    x ^= x >> 6;
    x
}

/// The pixel of dissolve rank `r`: [`dissolve_rank`] undone step by step.
pub fn dissolve_pixel(r: u32) -> usize {
    let mut x = r & MASK12;
    x ^= x >> 6;
    x = x.wrapping_sub(A3).wrapping_mul(inverse12(M3)) & MASK12;
    x ^= (x >> 5) ^ (x >> 10);
    x = x.wrapping_mul(inverse12(M2)) & MASK12;
    x ^= x >> 7;
    x = x.wrapping_sub(A1).wrapping_mul(inverse12(M1)) & MASK12;
    x as usize
}

const MASK12: u32 = (PIXELS as u32) - 1;
const _: () = assert!(PIXELS == 4096);
const M1: u32 = 0x9E5;
const A1: u32 = 0x3A7;
const M2: u32 = 0x6CB;
const M3: u32 = 0xB3D;
const A3: u32 = 0x1F1;

/// The inverse of odd `a` modulo 4096 (Newton's iteration doubles the
/// correct low bits each round: 3, 6, 12).
const fn inverse12(a: u32) -> u32 {
    let mut x = a; // correct to 3 bits for any odd a
    let mut k = 0;
    while k < 3 {
        x = x.wrapping_mul(2u32.wrapping_sub(a.wrapping_mul(x)));
        k += 1;
    }
    x & MASK12
}

/// `a + (b - a) * w / 256`, per channel; `w` in `0..=256`, exact at both
/// ends.
fn mix(a: Rgb, b: Rgb, w: u32) -> Rgb {
    let ch = |a: u8, b: u8| (i32::from(a) + (i32::from(b) - i32::from(a)) * w as i32 / 256) as u8;
    Rgb::new(ch(a.r, b.r), ch(a.g, b.g), ch(a.b, b.b))
}

/// `c * f / 256`, per channel; `f` in `0..=256`.
fn scale(c: Rgb, f: u32) -> Rgb {
    let ch = |v: u8| (u32::from(v) * f / 256) as u8;
    Rgb::new(ch(c.r), ch(c.g), ch(c.b))
}

/// One instant of a transition, with everything a pixel's value depends on
/// (besides the pixel and the two pictures) worked out once: no division
/// per pixel.
#[derive(Clone, Copy)]
struct At {
    kind: Kind,
    /// Dissolve: ranks below this show the new picture.
    n: u32,
    /// Wipes and blinds: the soft edge's front, in 1/256 px along the sweep.
    front: u32,
    /// Push: columns the pictures have moved.
    off: usize,
    /// Fade: the level (0..=256), and which picture it applies to.
    level: u32,
    fading_in: bool,
}

impl At {
    fn new(kind: Kind, duration: Duration, t: Duration) -> At {
        let d = duration.as_micros().max(1) as u64;
        let t = (t.as_micros() as u64).min(d);
        let (len, band) = sweep_of(kind);
        // The last position along the sweep is fully covered exactly at the
        // end: the front runs from 0 to (len - 1 + band) px.
        let front = (t * (len - 1 + band) as u64 * 256 / d) as u32;
        let (level, fading_in) = if 2 * t < d {
            ((256 - 2 * t * 256 / d) as u32, false)
        } else {
            (((2 * t - d) * 256 / d) as u32, true)
        };
        At {
            kind,
            n: (PIXELS as u64 * t / d) as u32,
            front,
            off: ((WIDTH as u64 * t + d / 2) / d) as usize,
            level,
            fading_in,
        }
    }

    /// The new picture's weight (0..=256) at position `s` along a sweep
    /// with a soft band of `band` px.
    fn weight(&self, s: usize, band: usize) -> u32 {
        (self
            .front
            .saturating_sub(s as u32 * 256)
            .min(band as u32 * 256))
            / band as u32
    }

    /// Where pixel `(x, y)` is along this kind's sweep.
    fn position(&self, x: usize, y: usize) -> usize {
        match self.kind {
            Kind::WipeLeft => WIDTH - 1 - x,
            Kind::WipeRight => x,
            Kind::WipeDown => y,
            Kind::WipeUp => HEIGHT - 1 - y,
            _ => y % BLIND_ROWS,
        }
    }

    fn value(&self, x: usize, y: usize, old: &[Rgb], new: &[Rgb]) -> Rgb {
        let i = y * WIDTH + x;
        match self.kind {
            Kind::None => new[i],
            Kind::Dissolve => {
                if dissolve_rank(i) < self.n {
                    new[i]
                } else {
                    old[i]
                }
            }
            Kind::WipeLeft | Kind::WipeRight | Kind::WipeDown | Kind::WipeUp | Kind::Blinds => {
                let band = sweep_of(self.kind).1;
                mix(old[i], new[i], self.weight(self.position(x, y), band))
            }
            Kind::Push => {
                if x + self.off < WIDTH {
                    old[i + self.off]
                } else {
                    new[i + self.off - WIDTH]
                }
            }
            Kind::Fade => scale(if self.fading_in { new[i] } else { old[i] }, self.level),
        }
    }
}

/// A sweeping kind's length and soft band (px); `(1, 1)` for the others.
fn sweep_of(kind: Kind) -> (usize, usize) {
    match kind {
        Kind::WipeLeft | Kind::WipeRight => (WIDTH, WIPE_BAND as usize),
        Kind::WipeDown | Kind::WipeUp => (HEIGHT, WIPE_BAND as usize),
        Kind::Blinds => (BLIND_ROWS, 1),
        _ => (1, 1),
    }
}

/// The output being updated: pixels and dirty map, and a count of changes.
struct Writer<'a> {
    pixels: &'a mut [Rgb],
    dirty: &'a mut DirtyMap,
    changed: u32,
}

impl Writer<'_> {
    /// Set `(x, y)` to its value at `at`; write and mark it only if that
    /// changes it.
    #[inline(always)]
    fn update(&mut self, at: &At, x: usize, y: usize, old: &[Rgb], new: &[Rgb]) {
        let v = at.value(x, y, old, new);
        let px = &mut self.pixels[y * WIDTH + x];
        if *px != v {
            *px = v;
            self.dirty.mark(x, y);
            self.changed += 1;
        }
    }
}

/// Composite `kind` over `duration` at `t` since it started: `old` is the
/// snapshot, `new` the new file's current frame, and `out` is updated in
/// place — only pixels whose value changes are written, and exactly those
/// are marked in its dirty map. Returns how many changed.
///
/// Pure: the result depends on nothing but the arguments (`out`'s previous
/// contents only decide which pixels count as changed). At `t >= duration`
/// the output is exactly `new`, for every kind; at `t = 0` it is exactly
/// `old` (a wipe's band starts off-screen, a push at offset 0, a fade at
/// full level). Every pixel is visited; [`compose_step`] is the same result
/// for the price of the pixels that can have changed.
pub fn compose(
    kind: Kind,
    duration: Duration,
    t: Duration,
    old: &[Rgb],
    new: &[Rgb],
    out: &mut Canvas,
) -> u32 {
    assert!(old.len() == PIXELS && new.len() == PIXELS);
    let at = At::new(kind, duration, t);
    let (pixels, dirty) = out.pixels_and_dirty_mut();
    let mut w = Writer {
        pixels,
        dirty,
        changed: 0,
    };
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            w.update(&at, x, y, old, new);
        }
    }
    w.changed
}

/// [`compose`] at `t`, given that `out` is already its result at `prev`
/// (earlier) and `new` has changed since only where `changed` marks: only
/// the pixels that can differ are visited — those `changed` marks, plus
/// those whose blend moved between `prev` and `t` (the ranks a dissolve
/// flipped, the columns or rows a wipe's or the blinds' edge crossed). Push
/// and Fade move every pixel, so they visit them all. The result, the marks
/// and the count are exactly [`compose`]'s.
#[allow(clippy::too_many_arguments)]
pub fn compose_step(
    kind: Kind,
    duration: Duration,
    prev: Duration,
    t: Duration,
    old: &[Rgb],
    new: &[Rgb],
    changed: &DirtyMap,
    out: &mut Canvas,
) -> u32 {
    if matches!(kind, Kind::Push | Kind::Fade | Kind::None) || prev > t {
        return compose(kind, duration, t, old, new, out);
    }
    assert!(old.len() == PIXELS && new.len() == PIXELS);
    let (a, at) = (At::new(kind, duration, prev), At::new(kind, duration, t));
    let (pixels, dirty) = out.pixels_and_dirty_mut();
    let mut w = Writer {
        pixels,
        dirty,
        changed: 0,
    };
    if kind == Kind::Dissolve {
        for r in a.n..at.n {
            let i = dissolve_pixel(r);
            w.update(&at, i % WIDTH, i / WIDTH, old, new);
        }
    } else {
        // Positions whose weight can differ between the two fronts: past
        // where the old front's band ended, up to where the new front is.
        let (len, band) = sweep_of(kind);
        let lo = (a.front / 256) as usize;
        let lo = lo.saturating_sub(band);
        let hi = (at.front.div_ceil(256) as usize).min(len - 1);
        for s in lo..=hi {
            match kind {
                Kind::WipeLeft | Kind::WipeRight => {
                    let x = if kind == Kind::WipeLeft {
                        WIDTH - 1 - s
                    } else {
                        s
                    };
                    for y in 0..HEIGHT {
                        w.update(&at, x, y, old, new);
                    }
                }
                _ => {
                    let rows: &mut dyn Iterator<Item = usize> = match kind {
                        Kind::WipeDown => &mut std::iter::once(s),
                        Kind::WipeUp => &mut std::iter::once(HEIGHT - 1 - s),
                        _ => &mut (s..HEIGHT).step_by(BLIND_ROWS),
                    };
                    for y in rows {
                        for x in 0..WIDTH {
                            w.update(&at, x, y, old, new);
                        }
                    }
                }
            }
        }
    }
    // And wherever the new picture itself changed.
    let mut rows = changed.row_mask();
    while rows != 0 {
        let y = rows.trailing_zeros() as usize;
        rows &= rows - 1;
        for (x0, x1) in changed.row_spans(y) {
            for x in x0..x1 {
                w.update(&at, x, y, old, new);
            }
        }
    }
    w.changed
}

/// A running transition: the snapshot, the output, and its clock.
struct Active {
    transition: Transition,
    snapshot: Vec<Rgb>,
    output: Canvas,
    /// Set by the first step, so a transition begun where there is no clock
    /// (a seek) starts when it is first shown.
    start: Option<Duration>,
    /// When the next step is due.
    step_at: Duration,
    /// A new frame of the file underneath since the last step.
    pending: bool,
    /// When (since `start`) the output was last composited; `None` until
    /// the first step, which composites every pixel. Later steps visit only
    /// what can have changed ([`compose_step`]).
    last: Option<Duration>,
}

/// The compositor an [`Engine`](crate::engine::Engine) owns: the configured
/// [`Transition`] and, while one runs, its buffers.
#[derive(Default)]
pub struct Compositor {
    transition: Transition,
    active: Option<Box<Active>>,
    gate: Option<Gate>,
}

/// Asked before a transition allocates its buffers, with the bytes about to
/// be taken: its own ([`BUFFER_BYTES`]) plus what the new file's decoder
/// still has to allocate for its first frame ([`GIF_FIRST_FRAME_BYTES`] for
/// a GIF — the transition starts *before* that frame is decoded, so a gate
/// that ignored it would let the LZW allocation fail: found on the rig, an
/// abort in `weezl::Decoder::new`). False and the file cuts in instead. The
/// firmware's keeps a heap reserve; a plain `fn` so the core stays free of
/// platform types.
pub type Gate = fn(usize) -> bool;

/// What a running transition allocates: the snapshot and the output.
pub const BUFFER_BYTES: usize = 2 * PIXELS * 3;
/// What a GIF's decoder allocates when its first frame is decoded: `weezl`'s
/// LZW state (28,856 B, in pieces up to 16 KB) and the 4 KB indexed frame
/// buffer, rounded up. See "GIF heap budget" in CLAUDE.md.
pub const GIF_FIRST_FRAME_BYTES: usize = 34 * 1024;

/// What [`Compositor::step`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing is running, or it is not time yet.
    Idle,
    /// The output was recomposited; show [`Compositor::output`].
    Composited,
    /// The transition ended: the decoder's canvas is on display again, its
    /// dirty map set to what the panel needs.
    Finished,
}

fn try_pixels(src: &[Rgb]) -> Option<Vec<Rgb>> {
    let mut v = Vec::new();
    v.try_reserve_exact(PIXELS).ok()?;
    v.extend_from_slice(src);
    Some(v)
}

impl Compositor {
    pub fn new(transition: Transition) -> Self {
        Compositor {
            transition,
            active: None,
            gate: None,
        }
    }

    /// See [`Gate`]; `None` (the default) always allows.
    pub fn set_gate(&mut self, gate: Option<Gate>) {
        self.gate = gate;
    }

    /// The gate, for anything else that allocates on the player's behalf
    /// (a decoder fallback).
    pub fn gate(&self) -> Option<Gate> {
        self.gate
    }

    pub fn transition(&self) -> Transition {
        self.transition
    }

    /// Used by the next transition that starts; one already running carries
    /// on as it began.
    pub fn set_transition(&mut self, transition: Transition) {
        self.transition = transition;
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// The composited picture while a transition runs.
    pub fn output(&self) -> Option<&Canvas> {
        self.active.as_ref().map(|a| &a.output)
    }

    pub fn output_mut(&mut self) -> Option<&mut Canvas> {
        self.active.as_mut().map(|a| &mut a.output)
    }

    /// When the running transition wants its next step (`None`: none
    /// running). A transition that has not been shown yet is due at once.
    pub fn step_at(&self) -> Option<Duration> {
        self.active.as_ref().map(|a| {
            if a.start.is_none() {
                Duration::ZERO
            } else {
                a.step_at
            }
        })
    }

    /// A new file is about to decode its first frame into `canvas`, which
    /// still holds the old one: start a transition from what is on display
    /// — the running transition's output if there is one (a skip halfway
    /// through), else `canvas`. The output starts as that picture with its
    /// dirty map (anything not yet pushed stays owed). Does nothing for
    /// [`Kind::None`]. If the 24 KB is not there, logs and does nothing: the
    /// file cuts in.
    ///
    /// `pending` is what the new file's decoder will still allocate before
    /// its first frame is on the canvas (for the [`Gate`]).
    pub fn begin(&mut self, canvas: &Canvas, pending: usize) {
        if self.transition.is_none() {
            return;
        }
        if let Some(a) = self.active.as_mut() {
            // From the picture on display, which is the output.
            if a.start.is_some() {
                a.snapshot.copy_from_slice(a.output.as_slice());
            }
            a.transition = self.transition;
            a.start = None;
            a.pending = true;
            a.last = None;
            return;
        }
        if self
            .gate
            .is_some_and(|allow| !allow(BUFFER_BYTES + pending))
        {
            return;
        }
        let (Some(snapshot), Some(pixels)) =
            (try_pixels(canvas.as_slice()), try_pixels(canvas.as_slice()))
        else {
            log::warn!("transition skipped: no room for its 24 KB of buffers");
            return;
        };
        let output = Canvas::from_parts(pixels, canvas.dirty().clone());
        self.active = Some(Box::new(Active {
            transition: self.transition,
            snapshot,
            output,
            start: None,
            step_at: Duration::ZERO,
            pending: true,
            last: None,
        }));
    }

    /// The file underneath drew a new frame: recomposite at the next step.
    pub fn frame_decoded(&mut self) {
        if let Some(a) = self.active.as_mut() {
            a.pending = true;
        }
    }

    /// Step the running transition at `now` if it is due (its cadence is
    /// [`STEP`]) or the file underneath has drawn since the last step.
    /// `canvas` is the decoder's canvas, the new file's current frame.
    ///
    /// While a transition runs nothing blits the decoder's canvas, so its
    /// dirty map is this step's to use: it says which pixels of the new
    /// picture changed since the last step, and is cleared after each (when
    /// the transition ends, [`finish`](Self::finish) sets it to what the
    /// panel needs).
    pub fn step(&mut self, now: Duration, canvas: &mut Canvas) -> Step {
        let Some(a) = self.active.as_mut() else {
            return Step::Idle;
        };
        let mut start = *a.start.get_or_insert(now);
        if !a.pending && now < a.step_at {
            return Step::Idle;
        }
        a.pending = false;
        let mut t = now.saturating_sub(start);
        // A stall between steps (the first step's `now` was read before the
        // new file's open, which can take ~330 ms in a big FAT directory)
        // slows the transition down rather than skipping most of it.
        if let Some(prev) = a.last {
            if t > prev + MAX_ADVANCE {
                start += t - (prev + MAX_ADVANCE);
                a.start = Some(start);
                t = prev + MAX_ADVANCE;
            }
        }
        let d = a.transition.duration;
        if t >= d {
            self.finish(canvas);
            return Step::Finished;
        }
        let kind = a.transition.kind;
        match a.last {
            // At t = 0 every kind is the old picture, which the output
            // already is (it starts as the picture on display): the first
            // step — the one inside the new file's open — costs nothing.
            None if t.is_zero() => 0,
            None => compose(kind, d, t, &a.snapshot, canvas.as_slice(), &mut a.output),
            Some(prev) => compose_step(
                kind,
                d,
                prev,
                t,
                &a.snapshot,
                canvas.as_slice(),
                canvas.dirty(),
                &mut a.output,
            ),
        };
        a.last = Some(t);
        canvas.dirty_mut().clear();
        // The next step 20 ms after this one (a frame of the file
        // underneath is composited when it lands, which moves the next step
        // along), the last one exactly at the end.
        a.step_at = (now + STEP).min(start + d);
        Step::Composited
    }

    /// End the running transition now: the decoder's canvas goes back on
    /// display, and its dirty map becomes exactly what the panel needs to
    /// get from the last output to it — the pixels where they differ, plus
    /// whatever the output still had marked. The buffers are freed.
    pub fn finish(&mut self, canvas: &mut Canvas) {
        let Some(a) = self.active.take() else {
            return;
        };
        let mut owed: DirtyMap = a.output.dirty().clone();
        for (i, (shown, want)) in a
            .output
            .as_slice()
            .iter()
            .zip(canvas.as_slice())
            .enumerate()
        {
            if shown != want {
                owed.mark(i % WIDTH, i / WIDTH);
            }
        }
        *canvas.dirty_mut() = owed;
    }

    /// The engine was paused for `d`: the transition's clock waits too.
    pub fn shift(&mut self, d: Duration) {
        if let Some(a) = self.active.as_mut() {
            if let Some(s) = a.start.as_mut() {
                *s += d;
            }
            a.step_at += d;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(seed: u8) -> Vec<Rgb> {
        (0..PIXELS)
            .map(|i| {
                let v = (i as u32).wrapping_mul(2_654_435_761) >> 8;
                Rgb::new(
                    (v as u8).wrapping_add(seed),
                    ((v >> 8) as u8) ^ seed,
                    ((v >> 16) as u8) | 1,
                )
            })
            .collect()
    }

    fn canvas_of(px: &[Rgb]) -> Canvas {
        let mut c = Canvas::new();
        c.as_mut_slice().copy_from_slice(px);
        c.dirty_mut().clear();
        c
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    const KINDS: [Kind; 9] = Kind::ALL;

    #[test]
    fn dissolve_order_is_a_permutation_and_spread_out() {
        let mut seen = vec![false; PIXELS];
        for i in 0..PIXELS {
            let r = dissolve_rank(i) as usize;
            assert!(!seen[r], "rank {r} twice");
            seen[r] = true;
        }
        // A quarter of the way in, every 16x16 block has flipped roughly a
        // quarter of its 256 pixels: no sweep, no clumping.
        for by in 0..HEIGHT / 16 {
            for bx in 0..WIDTH / 16 {
                let n = (0..256)
                    .filter(|k| {
                        let (x, y) = (bx * 16 + k % 16, by * 16 + k / 16);
                        dissolve_rank(y * WIDTH + x) < 1024
                    })
                    .count();
                assert!((40..=90).contains(&n), "block {bx},{by}: {n}/256");
            }
        }
    }

    #[test]
    fn dissolve_pixel_undoes_dissolve_rank() {
        for i in 0..PIXELS {
            assert_eq!(dissolve_pixel(dissolve_rank(i)), i);
        }
    }

    /// A step that visits only what can have changed gives exactly the
    /// full composite, marks and count included, while the new picture
    /// animates underneath (a moving block, marked as a decoder marks).
    #[test]
    fn compose_step_is_compose() {
        let old = picture(7);
        for kind in KINDS {
            for d in [ms(250), ms(400), ms(700)] {
                let mut new = canvas_of(&picture(8));
                let mut full = canvas_of(&old);
                let mut step = canvas_of(&old);
                compose(kind, d, Duration::ZERO, &old, new.as_slice(), &mut full);
                compose(kind, d, Duration::ZERO, &old, new.as_slice(), &mut step);
                let mut prev = Duration::ZERO;
                let mut t = ms(0);
                let mut k = 0;
                while t < d + ms(40) {
                    t += ms(if k % 3 == 0 { 20 } else { 13 });
                    k += 1;
                    // Frame k of the new file: a 9x5 block moves right.
                    if k % 2 == 0 {
                        for y in 10..15 {
                            for x in 0..9 {
                                new.set((x + 3 * k) % WIDTH, y, Rgb::new(k as u8, 200, 9));
                            }
                        }
                    }
                    full.dirty_mut().clear();
                    step.dirty_mut().clear();
                    let a = compose(kind, d, t, &old, new.as_slice(), &mut full);
                    let b = compose_step(
                        kind,
                        d,
                        prev,
                        t,
                        &old,
                        new.as_slice(),
                        new.dirty(),
                        &mut step,
                    );
                    new.dirty_mut().clear();
                    assert_eq!(step, full, "{kind:?} {d:?} at {t:?}");
                    assert_eq!(b, a, "{kind:?} {d:?} at {t:?}: count");
                    for i in 0..PIXELS {
                        let (x, y) = (i % WIDTH, i / WIDTH);
                        assert_eq!(
                            step.dirty().is_dirty(x, y),
                            full.dirty().is_dirty(x, y),
                            "{kind:?} at {t:?}: mark {i}"
                        );
                    }
                    prev = t;
                }
            }
        }
    }

    #[test]
    fn every_kind_starts_at_old_and_ends_exactly_at_new() {
        let (old, new) = (picture(0), picture(99));
        for kind in KINDS {
            let d = ms(400);
            let mut out = canvas_of(&old);
            compose(kind, d, d, &old, &new, &mut out);
            assert_eq!(out.as_slice(), &new[..], "{kind:?} at duration");
            compose(kind, d, ms(5000), &old, &new, &mut out);
            assert_eq!(out.as_slice(), &new[..], "{kind:?} past duration");
            if kind != Kind::None {
                let mut out = canvas_of(&new);
                compose(kind, d, Duration::ZERO, &old, &new, &mut out);
                assert_eq!(out.as_slice(), &old[..], "{kind:?} at 0");
                // Halfway it is neither.
                compose(kind, d, ms(200), &old, &new, &mut out);
                assert_ne!(out.as_slice(), &old[..], "{kind:?} at 200");
                assert_ne!(out.as_slice(), &new[..], "{kind:?} at 200");
            }
        }
    }

    #[test]
    fn deterministic() {
        let (old, new) = (picture(3), picture(200));
        for kind in KINDS {
            for t in [0, 17, 133, 250, 399] {
                let mut a = canvas_of(&old);
                let mut b = canvas_of(&new);
                // Different starting contents, same result.
                compose(kind, ms(400), ms(t), &old, &new, &mut a);
                compose(kind, ms(400), ms(t), &old, &new, &mut b);
                assert_eq!(a, b, "{kind:?} at {t}");
            }
        }
    }

    /// Weight of the new picture in one channel, 0..=256, where old and new
    /// differ by 255 in it.
    fn new_weight(old: u8, out: u8) -> i32 {
        (i32::from(out) - i32::from(old)).abs() * 256 / 255
    }

    #[test]
    fn wipes_and_blinds_cover_monotonically() {
        // Old all black, new all white: the red channel is the coverage.
        let old = vec![Rgb::BLACK; PIXELS];
        let new = vec![Rgb::new(255, 255, 255); PIXELS];
        for kind in [
            Kind::WipeLeft,
            Kind::WipeRight,
            Kind::WipeDown,
            Kind::WipeUp,
            Kind::Blinds,
            Kind::Dissolve,
        ] {
            let mut prev = vec![0i32; PIXELS];
            let mut out = canvas_of(&old);
            let mut covered_before = 0;
            for t in (0..=400).step_by(20) {
                compose(kind, ms(400), ms(t), &old, &new, &mut out);
                let mut covered = 0;
                for (i, px) in out.as_slice().iter().enumerate() {
                    let w = new_weight(0, px.r);
                    assert!(w >= prev[i], "{kind:?}: pixel {i} went back at {t} ms");
                    prev[i] = w;
                    covered += w;
                }
                assert!(covered >= covered_before, "{kind:?} at {t}");
                if t > 0 && t < 400 {
                    assert!(covered > covered_before, "{kind:?} stalled at {t}");
                }
                covered_before = covered;
            }
            assert_eq!(covered_before, 256 * PIXELS as i32, "{kind:?} complete");
        }
        // The wipe's edge is where it should be: WipeLeft at 50% has the
        // right half new and the left half old, a soft band between.
        let mut out = canvas_of(&old);
        compose(Kind::WipeLeft, ms(400), ms(200), &old, &new, &mut out);
        assert_eq!(out.get(127, 5).r, 255);
        assert_eq!(out.get(0, 5).r, 0);
        let soft = (0..WIDTH)
            .filter(|&x| (1..255).contains(&out.get(x, 9).r))
            .count();
        assert!((1..=2).contains(&soft), "soft band {soft} px");
        let mut out = canvas_of(&old);
        compose(Kind::WipeDown, ms(400), ms(100), &old, &new, &mut out);
        assert_eq!(out.get(40, 0).r, 255);
        assert_eq!(out.get(40, 31).r, 0);
    }

    #[test]
    fn push_slides_and_fade_dips_to_black() {
        let (old, new) = (picture(1), picture(2));
        let mut out = canvas_of(&old);
        // 25%: 32 px in.
        compose(Kind::Push, ms(400), ms(100), &old, &new, &mut out);
        assert_eq!(out.get(0, 7), old[7 * WIDTH + 32]);
        assert_eq!(out.get(95, 7), old[7 * WIDTH + 127]);
        assert_eq!(out.get(96, 7), new[7 * WIDTH]);
        compose(Kind::Fade, ms(400), ms(200), &old, &new, &mut out);
        assert!(out.as_slice().iter().all(|p| *p == Rgb::BLACK));
        compose(Kind::Fade, ms(400), ms(100), &old, &new, &mut out);
        assert_eq!(out.get(3, 3), scale(old[3 * WIDTH + 3], 128));
        compose(Kind::Fade, ms(400), ms(300), &old, &new, &mut out);
        assert_eq!(out.get(3, 3), scale(new[3 * WIDTH + 3], 128));
    }

    /// Step through a 400 ms transition at 20 ms, clearing the dirty map
    /// after each like the panel does: every step marks exactly the pixels
    /// it changed. Prints the per-step counts (`cargo test -- --nocapture`).
    #[test]
    fn dirty_marks_are_exactly_the_changed_pixels() {
        let (old, new) = (picture(10), picture(20));
        for kind in KINDS {
            let mut out = canvas_of(&old);
            let mut counts = Vec::new();
            for t in (20..=400).step_by(20) {
                let before = out.clone();
                let changed = compose(kind, ms(400), ms(t), &old, &new, &mut out);
                let real = before
                    .as_slice()
                    .iter()
                    .zip(out.as_slice())
                    .filter(|(a, b)| a != b)
                    .count() as u32;
                assert_eq!(changed, real, "{kind:?} at {t}");
                assert_eq!(out.dirty().count(), real, "{kind:?} at {t}: marks");
                for i in 0..PIXELS {
                    let (x, y) = (i % WIDTH, i / WIDTH);
                    assert_eq!(
                        out.dirty().is_dirty(x, y),
                        before.as_slice()[i] != out.as_slice()[i],
                        "{kind:?} at {t}: pixel {i}"
                    );
                }
                out.dirty_mut().clear();
                counts.push(changed);
            }
            let total: u32 = counts.iter().sum();
            println!(
                "{:<10} px/step {:?} (mean {})",
                kind.name(),
                counts,
                total / counts.len() as u32
            );
            if kind == Kind::Dissolve {
                // 4,096 pixels over 20 steps: ~205 a step, never the screen.
                assert!(
                    counts.iter().all(|&c| (150..=260).contains(&c)),
                    "{counts:?}"
                );
            }
        }
    }

    #[test]
    fn names_parse_and_print() {
        for kind in KINDS {
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
            let t = Transition::new(kind, ms(250));
            assert_eq!(Transition::parse(&t.to_string()), Some(t));
        }
        assert_eq!(Kind::from_name("Wipe-Left"), Some(Kind::WipeLeft));
        assert_eq!(Kind::from_name("wipe"), Some(Kind::WipeLeft));
        assert_eq!(Kind::from_name("wipeleft"), None);
        assert_eq!(Transition::parse("dissolve"), Some(Transition::DEFAULT));
        assert_eq!(
            Transition::parse("fade:9999").unwrap().duration,
            MAX_DURATION
        );
        assert_eq!(Transition::parse("fade:1").unwrap().duration, MIN_DURATION);
        assert_eq!(Transition::parse("fade:x"), None);
        assert_eq!(Transition::parse("sparkle"), None);
        assert_eq!(Transition::DEFAULT.to_string(), "dissolve:400");
    }

    #[test]
    fn the_compositor_steps_on_its_cadence_and_hands_back_cleanly() {
        let (old, new) = (picture(5), picture(6));
        let mut canvas = canvas_of(&old);
        let mut c = Compositor::new(Transition::new(Kind::WipeRight, ms(100)));
        c.begin(&canvas, 0);
        assert!(c.is_active());
        assert_eq!(c.step_at(), Some(Duration::ZERO));
        // The new file draws its first frame, then the first step.
        canvas.as_mut_slice().copy_from_slice(&new);
        canvas.dirty_mut().mark_all();
        c.frame_decoded();
        assert_eq!(c.step(ms(1000), &mut canvas), Step::Composited);
        assert_eq!(
            c.output().unwrap().as_slice(),
            &old[..],
            "t = 0 is the old picture"
        );
        assert_eq!(c.step_at(), Some(ms(1020)));
        assert_eq!(c.step(ms(1010), &mut canvas), Step::Idle);
        assert_eq!(c.step(ms(1020), &mut canvas), Step::Composited);
        // Paused for 500 ms: the clock waits.
        c.shift(ms(500));
        assert_eq!(c.step_at(), Some(ms(1540)));
        // The last step is clamped to the end.
        for t in [1540, 1560, 1580] {
            assert_eq!(c.step(ms(t), &mut canvas), Step::Composited, "{t}");
        }
        assert_eq!(c.step_at(), Some(ms(1600)));
        let shown = c.output().unwrap().clone();
        canvas.dirty_mut().clear();
        assert_eq!(c.step(ms(1600), &mut canvas), Step::Finished);
        assert!(!c.is_active());
        // The canvas owes exactly the pixels where the last output differed
        // from it (plus the output's unpushed marks from the last step).
        for i in 0..PIXELS {
            let (x, y) = (i % WIDTH, i / WIDTH);
            let want = shown.as_slice()[i] != canvas.as_slice()[i] || shown.dirty().is_dirty(x, y);
            assert_eq!(canvas.dirty().is_dirty(x, y), want, "pixel {i}");
        }
        // A gate that says no: a cut.
        let mut c = Compositor::new(Transition::DEFAULT);
        c.set_gate(Some(|need| need < 1000));
        c.begin(&canvas, 0);
        assert!(!c.is_active());
        c.set_gate(Some(|need| need == BUFFER_BYTES + 7));
        c.begin(&canvas, 7);
        assert!(c.is_active());
        // None never starts.
        let mut c = Compositor::new(Transition::NONE);
        c.begin(&canvas, 0);
        assert!(!c.is_active());
    }

    #[test]
    fn a_stall_slows_the_transition_instead_of_skipping_it() {
        let (old, new) = (picture(1), picture(2));
        let mut canvas = canvas_of(&old);
        let mut c = Compositor::new(Transition::new(Kind::Dissolve, ms(400)));
        c.begin(&canvas, 0);
        canvas.as_mut_slice().copy_from_slice(&new);
        // The first step's clock was read before a 330 ms open.
        assert_eq!(c.step(ms(1000), &mut canvas), Step::Composited);
        assert_eq!(c.step(ms(1350), &mut canvas), Step::Composited);
        // 40 ms in, not 350: a tenth of the pixels have flipped.
        let flipped = c
            .output()
            .unwrap()
            .as_slice()
            .iter()
            .zip(&new)
            .filter(|(a, b)| a == b)
            .count();
        assert!((350..470).contains(&flipped), "{flipped}");
        assert_eq!(c.step_at(), Some(ms(1370)));
        let mut t = 1370;
        while c.step(ms(t), &mut canvas) != Step::Finished {
            t += 20;
        }
        assert_eq!(t, 1350 + 360);
    }

    #[test]
    fn a_new_file_mid_transition_starts_from_the_output() {
        let (a, b, c3) = (picture(1), picture(2), picture(3));
        let mut canvas = canvas_of(&a);
        let mut comp = Compositor::new(Transition::new(Kind::Dissolve, ms(400)));
        comp.begin(&canvas, 0);
        canvas.as_mut_slice().copy_from_slice(&b);
        comp.step(ms(0), &mut canvas);
        comp.step(ms(200), &mut canvas);
        let halfway = comp.output().unwrap().clone();
        // A skip: the third file starts from the half-dissolved picture.
        comp.begin(&canvas, 0);
        canvas.as_mut_slice().copy_from_slice(&c3);
        assert_eq!(comp.step(ms(210), &mut canvas), Step::Composited);
        assert_eq!(comp.output().unwrap(), &halfway);
        comp.step(ms(410), &mut canvas);
        let out = comp.output().unwrap();
        let from_half = (0..PIXELS)
            .filter(|&i| {
                out.as_slice()[i] == halfway.as_slice()[i] && halfway.as_slice()[i] != c3[i]
            })
            .count();
        assert!(from_half > 1500, "{from_half}");
    }
}
