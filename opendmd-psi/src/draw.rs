//! The draw stage: `draw { … }` run once a frame, before the rows — its
//! loops and values (scalar steps), and the lines and dots it puts into the
//! canvas ([`Ink`]) that the pixel steps read back as `ink`.
//!
//! Integer arithmetic throughout (the scalar steps are `eval`'s, the
//! rasteriser is integer Bresenham as `opendmd-core`'s `fx::line`), so a
//! frame's canvas is the same bytes on the ESP32, the host and in wasm. A
//! frame is still a pure function of its inputs: the canvas is cleared
//! first. Bounded whatever the program: [`TURNS`] loop turns, [`PRIMITIVES`]
//! lines and dots, [`LINE_STEPS`] steps a line and [`FRAME_STEPS`] a frame,
//! counted the same everywhere, so a program that reaches one draws the same
//! partial picture on every target.

use crate::fixed::{self, ONE};
use crate::program::*;
use crate::tables::Tables;
use alloc::boxed::Box;

/// Loop turns a frame (each `for` body run counts one, nested ones too):
/// the compiler refuses more, and the VM stops at it anyway.
pub const TURNS: u32 = 2048;
/// Lines and dots a frame; past it the rest are skipped.
pub const PRIMITIVES: u32 = 2048;
/// Steps one line takes at most (as the built-in effects' rasteriser).
pub const LINE_STEPS: u32 = 1024;
/// Line steps a frame (the canvas four times over); past it the rest of
/// the frame's primitives are skipped.
pub const FRAME_STEPS: u32 = 16_384;

/// A line's ends are pixels in −512..639 across and −512..543 up (four
/// panels round it): enough for anything that crosses the panel.
const X_MIN: i32 = -512;
const X_MAX: i32 = W as i32 + 511;
const Y_MIN: i32 = -512;
const Y_MAX: i32 = H as i32 + 511;

/// The draw stage's canvas: 128 × 32 bytes, row-major from the top as the
/// frame is; 0 where nothing was drawn, 255 at full ink. Plain bytes, so a
/// zeroed allocation is a valid one. Only a program that
/// [`draws`](Program::draws) needs one.
#[repr(C)]
pub struct Ink(pub [u8; W * H]);

impl Ink {
    pub const fn new() -> Ink {
        Ink([0; W * H])
    }

    /// A canvas on the heap, made zeroed in place (4 KB never passes
    /// through the stack).
    pub fn boxed() -> Box<Ink> {
        Self::try_boxed().unwrap_or_else(|| alloc::alloc::handle_alloc_error(Self::LAYOUT))
    }

    /// [`boxed`](Self::boxed), or `None` when the heap hasn't 4 KB.
    pub fn try_boxed() -> Option<Box<Ink>> {
        // SAFETY: a non-zero-sized layout; `Ink` is bytes, so the zeroed
        // memory is a valid value; the box frees it with the same layout.
        unsafe {
            let p = alloc::alloc::alloc_zeroed(Self::LAYOUT).cast::<Ink>();
            (!p.is_null()).then(|| Box::from_raw(p))
        }
    }

    const LAYOUT: core::alloc::Layout = core::alloc::Layout::new::<Ink>();

    /// The byte at column `x`, row `y` from the top.
    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.0[(y * W + x) & (W * H - 1)]
    }
}

impl Default for Ink {
    fn default() -> Ink {
        Ink::new()
    }
}

/// Run `prog`'s draw stage into `ink` (cleared first), the scalar slots as
/// the per-frame steps left them.
#[inline(never)]
pub(crate) fn run(prog: &Program, sc: &mut [i32; MAX_SCALARS], t: &Tables, ink: &mut Ink) {
    ink.0.fill(0);
    let start = prog.frame_end as usize;
    let end = (prog.draw_end as usize).min(MAX_OPS);
    let m = |v: u8| (v & 0x7f) as usize;
    let mut pen = Pen {
        prims: 0,
        steps: 0,
        from: (0, 0, 0),
    };
    let mut turns = 0u32;
    let mut pc = start;
    while pc < end {
        let op = prog.ops[pc];
        pc += 1;
        match op.code {
            code::FOR => {
                sc[m(op.d)] = 0;
                // Out of turns: past the loop (its END), not into it.
                if turns >= TURNS {
                    pc = (op.b as usize + 1).max(pc);
                }
            }
            code::END => {
                turns += 1;
                let k = fixed::add(sc[m(op.a)], ONE);
                sc[m(op.a)] = k;
                let back = op.c as usize;
                if k < sc[m(op.b)] && turns < TURNS && back > start && back < pc {
                    pc = back;
                }
            }
            code::LINE_FROM => pen.from = (sc[m(op.a)], sc[m(op.b)], sc[m(op.c)]),
            code::LINE_TO => {
                let to = (sc[m(op.a)], sc[m(op.b)], sc[m(op.c)]);
                pen.line(ink, to);
            }
            code::DOT => pen.dot(ink, sc[m(op.a)], sc[m(op.b)], sc[m(op.c)]),
            _ => super::vm::scalar(sc, &op, t, &prog.tables),
        }
    }
}

/// What a frame has drawn so far, and a line's start.
struct Pen {
    prims: u32,
    steps: u32,
    from: (i32, i32, i32),
}

/// A coordinate (Q16.16 pixels) to the nearest pixel (x + ½ floored: ties
/// go up), held within `lo..=hi`.
#[inline]
fn pixel(v: i32, lo: i32, hi: i32) -> i32 {
    (fixed::add(v, ONE / 2) >> 16).clamp(lo, hi)
}

impl Pen {
    /// Whether the frame may draw another primitive (counting it).
    fn may(&mut self) -> bool {
        if self.prims >= PRIMITIVES || self.steps >= FRAME_STEPS {
            return false;
        }
        self.prims += 1;
        true
    }

    fn dot(&mut self, ink: &mut Ink, x: i32, y: i32, v: i32) {
        if !self.may() {
            return;
        }
        let (x, y) = (pixel(x, X_MIN, X_MAX), pixel(y, Y_MIN, Y_MAX));
        plot(ink, x, y, fixed::chan(v));
    }

    /// The line from `self.from` to `to`: integer Bresenham as
    /// `opendmd_core::fx::line` (its two tests in its order), the ink from
    /// the start's byte to the end's in equal integer steps.
    fn line(&mut self, ink: &mut Ink, to: (i32, i32, i32)) {
        if !self.may() {
            return;
        }
        let from = self.from;
        let (mut x, mut y) = (pixel(from.0, X_MIN, X_MAX), pixel(from.1, Y_MIN, Y_MAX));
        let (x1, y1) = (pixel(to.0, X_MIN, X_MAX), pixel(to.1, Y_MIN, Y_MAX));
        // Wholly off one side of the panel: nothing of it would show (its
        // pixels lie between its ends), so it costs no steps.
        let (w, h) = (W as i32 - 1, H as i32 - 1);
        if (x < 0 && x1 < 0) || (x > w && x1 > w) || (y < 0 && y1 < 0) || (y > h && y1 > h) {
            return;
        }
        let (dx, dy) = ((x1 - x).abs(), -(y1 - y).abs());
        let (sx, sy) = (if x < x1 { 1 } else { -1 }, if y < y1 { 1 } else { -1 });
        let steps = dx.max(-dy).max(1);
        let mut err = dx + dy;
        let mut k = (fixed::chan(from.2) as i32) << 16;
        let dk = (((fixed::chan(to.2) as i32) << 16) - k) / steps;
        for _ in 0..LINE_STEPS {
            self.steps += 1;
            plot(ink, x, y, ((k + 0x8000) >> 16).clamp(0, 255) as u8);
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
}

/// Lighten the pixel at `x`, `y` (up from the bottom) to `v`, if it is on
/// the panel.
#[inline(always)]
fn plot(ink: &mut Ink, x: i32, y: i32, v: u8) {
    if (0..W as i32).contains(&x) && (0..H as i32).contains(&y) {
        let i = (H - 1 - y as usize) * W + x as usize;
        let p = &mut ink.0[i];
        *p = (*p).max(v);
    }
}
