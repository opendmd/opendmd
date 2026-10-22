//! The row-vectorised VM.
//!
//! A frame runs the per-frame steps once (scalars), then for each of the 32
//! rows the per-row steps (scalars: anything that depends on `y` but not
//! `x`), then the per-pixel steps over a *lane* of `N` pixels at once (`N`
//! = 128, one row, or 64, half of one): each step is one tight loop over
//! the lane, so the interpreter's dispatch (a `match` per step) is paid
//! once per `N` pixels, and an operand that is the same across the lane (a
//! constant, a per-frame or per-row value) is a register in that loop, not
//! a broadcast array. The colour step writes the lane's RGB888 bytes
//! straight into the frame.
//!
//! Every loop is monomorphised over which operands are lanes and which are
//! scalars ([`Src`]); the arithmetic is `fixed`'s, the same functions the
//! constant folder and the scalar steps use, so the output is bit-identical
//! wherever it runs. Nothing here allocates; the working memory is the
//! caller's [`Scratch`].

use crate::fixed::{self, ONE};
use crate::program::*;
use crate::tables::Tables;

/// What a frame depends on besides the program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    /// Seconds into the loop, Q16.16 (see [`Inputs::at_ms`]).
    pub t: i32,
    /// The wall clock: 0..23, 0..59, 0..59 (whole numbers).
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// The sliders `a b c`, Q16.16 (0..1 by convention).
    pub a: i32,
    pub b: i32,
    pub c: i32,
}

impl Inputs {
    /// `t` for `ms` into the loop: exactly `ms · 65536 / 1000`, rounded
    /// down.
    pub fn at_ms(ms: u32) -> i32 {
        ((ms as u64 * 65536) / 1000).min(i32::MAX as u64) as i32
    }
}

/// A Q16.16 from a decimal fraction `num / den` (sliders, tests).
pub fn q16(num: i32, den: i32) -> i32 {
    fixed::div(num << 16, den << 16)
}

/// The VM's working memory for a lane width `N` (64 or 128): the lane
/// registers, the `i` lane, the scalar slots, and the `x`/`px` columns
/// (worked out once, by [`Scratch::new`] or [`Scratch::reset`]). 128: 8.7
/// KB; 64: 5.6 KB. Plain integers, so a zeroed allocation plus `reset` is a
/// valid one.
pub struct Scratch<const N: usize> {
    /// `[0]` is the `i` lane, then the [`REGS`] registers.
    lanes: [[i32; N]; REGS + 1],
    sc: [i32; MAX_SCALARS],
    xs: [i32; W],
    pxs: [i32; W],
}

impl<const N: usize> Scratch<N> {
    const _OK: () = assert!(N > 0 && W.is_multiple_of(N));

    pub fn new() -> Self {
        let mut s = Scratch {
            lanes: [[0; N]; REGS + 1],
            sc: [0; MAX_SCALARS],
            xs: [0; W],
            pxs: [0; W],
        };
        s.reset();
        s
    }

    /// Fill in the constant columns (for a zeroed allocation).
    pub fn reset(&mut self) {
        #[allow(clippy::let_unit_value)]
        let () = Self::_OK;
        for k in 0..W {
            self.pxs[k] = (k as i32) << 16;
            self.xs[k] = fixed::div(self.pxs[k], ((W - 1) as i32) << 16);
        }
    }
}

impl<const N: usize> Default for Scratch<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// An operand inside a lane loop.
trait Src: Copy {
    fn at(self, k: usize) -> i32;
}

/// A lane: `N` values.
#[derive(Clone, Copy)]
struct L(*const i32);

impl Src for L {
    #[inline(always)]
    fn at(self, k: usize) -> i32 {
        // SAFETY: `L` is only made from a lane of `N` values that outlives
        // the step, and `k < N`.
        unsafe { *self.0.add(k) }
    }
}

/// A scalar, the same in every pixel of the lane.
#[derive(Clone, Copy)]
struct S(i32);

impl Src for S {
    #[inline(always)]
    fn at(self, _: usize) -> i32 {
        self.0
    }
}

#[derive(Clone, Copy)]
enum Arg {
    Lane(*const i32),
    Sc(i32),
}

#[inline(always)]
fn run1<const N: usize, A: Src>(d: &mut [i32; N], a: A, f: impl Fn(i32) -> i32) {
    for (k, o) in d.iter_mut().enumerate() {
        *o = f(a.at(k));
    }
}

#[inline(always)]
fn run2<const N: usize, A: Src, B: Src>(d: &mut [i32; N], a: A, b: B, f: impl Fn(i32, i32) -> i32) {
    for (k, o) in d.iter_mut().enumerate() {
        *o = f(a.at(k), b.at(k));
    }
}

#[inline(always)]
fn run3<const N: usize, A: Src, B: Src, C: Src>(
    d: &mut [i32; N],
    a: A,
    b: B,
    c: C,
    f: impl Fn(i32, i32, i32) -> i32,
) {
    for (k, o) in d.iter_mut().enumerate() {
        *o = f(a.at(k), b.at(k), c.at(k));
    }
}

#[inline(always)]
fn out3<A: Src, B: Src, C: Src>(
    px: &mut [[u8; 3]],
    a: A,
    b: B,
    c: C,
    f: impl Fn(i32, i32, i32) -> [u8; 3],
) {
    for (k, o) in px.iter_mut().enumerate() {
        *o = f(a.at(k), b.at(k), c.at(k));
    }
}

#[inline(always)]
fn out1<A: Src>(px: &mut [[u8; 3]], a: A, f: impl Fn(i32) -> [u8; 3]) {
    for (k, o) in px.iter_mut().enumerate() {
        *o = f(a.at(k));
    }
}

macro_rules! un {
    ($d:expr, $a:expr, $f:expr) => {
        match $a {
            Arg::Lane(p) => run1($d, L(p), $f),
            Arg::Sc(v) => $d.fill(($f)(v)),
        }
    };
}

macro_rules! bin {
    ($d:expr, $a:expr, $b:expr, $f:expr) => {
        match ($a, $b) {
            (Arg::Lane(p), Arg::Lane(q)) => run2($d, L(p), L(q), $f),
            (Arg::Lane(p), Arg::Sc(v)) => run2($d, L(p), S(v), $f),
            (Arg::Sc(u), Arg::Lane(q)) => run2($d, S(u), L(q), $f),
            (Arg::Sc(u), Arg::Sc(v)) => $d.fill(($f)(u, v)),
        }
    };
}

macro_rules! tern {
    ($d:expr, $a:expr, $b:expr, $c:expr, $f:expr) => {
        match ($a, $b, $c) {
            (Arg::Lane(p), Arg::Lane(q), Arg::Lane(r)) => run3($d, L(p), L(q), L(r), $f),
            (Arg::Lane(p), Arg::Lane(q), Arg::Sc(w)) => run3($d, L(p), L(q), S(w), $f),
            (Arg::Lane(p), Arg::Sc(v), Arg::Lane(r)) => run3($d, L(p), S(v), L(r), $f),
            (Arg::Lane(p), Arg::Sc(v), Arg::Sc(w)) => run3($d, L(p), S(v), S(w), $f),
            (Arg::Sc(u), Arg::Lane(q), Arg::Lane(r)) => run3($d, S(u), L(q), L(r), $f),
            (Arg::Sc(u), Arg::Lane(q), Arg::Sc(w)) => run3($d, S(u), L(q), S(w), $f),
            (Arg::Sc(u), Arg::Sc(v), Arg::Lane(r)) => run3($d, S(u), S(v), L(r), $f),
            (Arg::Sc(u), Arg::Sc(v), Arg::Sc(w)) => $d.fill(($f)(u, v, w)),
        }
    };
}

macro_rules! outt {
    ($px:expr, $a:expr, $b:expr, $c:expr, $f:expr) => {
        match ($a, $b, $c) {
            (Arg::Lane(p), Arg::Lane(q), Arg::Lane(r)) => out3($px, L(p), L(q), L(r), $f),
            (Arg::Lane(p), Arg::Lane(q), Arg::Sc(w)) => out3($px, L(p), L(q), S(w), $f),
            (Arg::Lane(p), Arg::Sc(v), Arg::Lane(r)) => out3($px, L(p), S(v), L(r), $f),
            (Arg::Lane(p), Arg::Sc(v), Arg::Sc(w)) => out3($px, L(p), S(v), S(w), $f),
            (Arg::Sc(u), Arg::Lane(q), Arg::Lane(r)) => out3($px, S(u), L(q), L(r), $f),
            (Arg::Sc(u), Arg::Lane(q), Arg::Sc(w)) => out3($px, S(u), L(q), S(w), $f),
            (Arg::Sc(u), Arg::Sc(v), Arg::Lane(r)) => out3($px, S(u), S(v), L(r), $f),
            (Arg::Sc(u), Arg::Sc(v), Arg::Sc(w)) => $px.fill(($f)(u, v, w)),
        }
    };
}

/// Render one frame of `prog` into `out` (128 × 32 RGB888, row-major from
/// the top), with lanes of `N` pixels. Writes every byte.
pub fn render_with<const N: usize>(
    prog: &Program,
    scratch: &mut Scratch<N>,
    inputs: &Inputs,
    tables: &Tables,
    out: &mut [u8],
) {
    assert_eq!(out.len(), W * H * 3);
    let ns = prog.n_scalars as usize;
    scratch.sc[..ns].copy_from_slice(&prog.init[..ns]);
    let sc = &mut scratch.sc;
    sc[S_T as usize] = inputs.t;
    sc[S_HOUR as usize] = (inputs.hour as i32) << 16;
    sc[S_MINUTE as usize] = (inputs.minute as i32) << 16;
    sc[S_SECOND as usize] = (inputs.second as i32) << 16;
    sc[S_A as usize] = inputs.a;
    sc[S_B as usize] = inputs.b;
    sc[S_C as usize] = inputs.c;
    let sin = |v: i32| fixed::sin(tables, v);
    let (fe, re, pe) = (
        prog.frame_end as usize,
        prog.row_end as usize,
        prog.pixel_end as usize,
    );
    for op in &prog.ops[..fe] {
        scalar(sc, op, sin);
    }
    let uses_i = prog.uses & USES_I != 0;
    for row in 0..H {
        // y and py count from the bottom (y = 0 is the bottom row).
        let py = ((H - 1 - row) as i32) << 16;
        scratch.sc[S_PY as usize] = py;
        scratch.sc[S_Y as usize] = fixed::div(py, ((H - 1) as i32) << 16);
        for op in &prog.ops[fe..re] {
            scalar(&mut scratch.sc, op, sin);
        }
        let row_bytes = &mut out[row * W * 3..(row + 1) * W * 3];
        for base in (0..W).step_by(N) {
            if uses_i {
                for (k, v) in scratch.lanes[0].iter_mut().enumerate() {
                    *v = ((row * W + base + k) as i32) << 16;
                }
            }
            for op in &prog.ops[re..pe] {
                lane_op(scratch, op, base, tables);
            }
            let px = row_bytes[base * 3..(base + N) * 3].as_chunks_mut::<3>().0;
            colour(scratch, &prog.ops[pe], base, tables, px);
        }
    }
}

#[inline(always)]
fn scalar(sc: &mut [i32; MAX_SCALARS], op: &Op, sin: impl Fn(i32) -> i32) {
    // Operands of scalar steps are always scalar slots (< 128).
    let g = |v: u8| sc[(v & 0x7f) as usize];
    let v = eval(op.code, g(op.a), g(op.b), g(op.c), sin);
    sc[(op.d & 0x7f) as usize] = v;
}

impl<const N: usize> Scratch<N> {
    /// The operand `v` for the lane at column `base`.
    #[inline(always)]
    fn arg(&self, v: u8, base: usize) -> Arg {
        if v & LANE == 0 {
            return Arg::Sc(self.sc[(v & 0x7f) as usize]);
        }
        Arg::Lane(match v & !LANE {
            L_X => self.xs[base..base + N].as_ptr(),
            L_PX => self.pxs[base..base + N].as_ptr(),
            l => self.lanes[((l - L_I) as usize).min(REGS)].as_ptr(),
        })
    }
}

fn lane_op<const N: usize>(s: &mut Scratch<N>, op: &Op, base: usize, t: &Tables) {
    use code::*;
    let (a, b, c) = (s.arg(op.a, base), s.arg(op.b, base), s.arg(op.c, base));
    // The destination is a register the compiler never also reads in the
    // same step (`compile` allocates it that way), so the `&mut` lane does
    // not alias the source pointers.
    let di = ((op.d & !LANE).saturating_sub(L_I) as usize).clamp(1, REGS);
    debug_assert!(op.d & LANE != 0 && op.d & !LANE >= L_REG0);
    let d: &mut [i32; N] = &mut s.lanes[di];
    match op.code {
        ADD => bin!(d, a, b, fixed::add),
        SUB => bin!(d, a, b, fixed::sub),
        MUL => bin!(d, a, b, fixed::mul),
        DIV => bin!(d, a, b, fixed::div),
        MOD => bin!(d, a, b, fixed::modulo),
        MASK => bin!(d, a, b, fixed::mask),
        LT => bin!(d, a, b, fixed::lt),
        LE => bin!(d, a, b, fixed::le),
        GT => bin!(d, a, b, fixed::gt),
        GE => bin!(d, a, b, fixed::ge),
        EQ => bin!(d, a, b, fixed::eq),
        NE => bin!(d, a, b, fixed::ne),
        AND => bin!(d, a, b, fixed::and),
        OR => bin!(d, a, b, fixed::or),
        MIN => bin!(d, a, b, fixed::min),
        MAX => bin!(d, a, b, fixed::max),
        STEP => bin!(d, a, b, fixed::step),
        ATAN2 => bin!(d, a, b, fixed::atan2),
        HASH => bin!(d, a, b, fixed::hash),
        LEN => bin!(d, a, b, fixed::len),
        NEG => un!(d, a, fixed::neg),
        NOT => un!(d, a, fixed::not),
        ABS => un!(d, a, fixed::abs),
        FLOOR => un!(d, a, fixed::floor),
        FRAC => un!(d, a, fixed::frac),
        SIN => un!(d, a, |v| fixed::sin(t, v)),
        COS => un!(d, a, |v| fixed::cos(t, v)),
        TRI => un!(d, a, fixed::tri),
        SAW => un!(d, a, fixed::saw),
        SQR => un!(d, a, fixed::sqr),
        SQRT => un!(d, a, fixed::sqrt),
        RECIP => un!(d, a, fixed::recip),
        SELECT => tern!(d, a, b, c, fixed::select),
        CLAMP => tern!(d, a, b, c, fixed::clamp),
        MIX => tern!(d, a, b, c, fixed::mix),
        SMOOTH => tern!(d, a, b, c, fixed::smooth),
        NOISE => tern!(d, a, b, c, fixed::noise),
        _ => {}
    }
}

fn colour<const N: usize>(s: &Scratch<N>, op: &Op, base: usize, t: &Tables, px: &mut [[u8; 3]]) {
    use code::*;
    let (a, b, c) = (s.arg(op.a, base), s.arg(op.b, base), s.arg(op.c, base));
    match op.code {
        OUT_RGB => outt!(px, a, b, c, |r, g, b| [
            fixed::chan(r),
            fixed::chan(g),
            fixed::chan(b)
        ]),
        OUT_HSV => outt!(px, a, b, c, fixed::hsv),
        OUT_PAL => {
            let p = t.palettes[(op.d as usize).min(4)];
            match a {
                Arg::Lane(q) => out1(px, L(q), |k| fixed::pal(p, k)),
                Arg::Sc(v) => px.fill(fixed::pal(p, v)),
            }
        }
        _ => match a {
            Arg::Lane(q) => out1(px, L(q), |k| {
                let v = fixed::chan(k);
                [v, v, v]
            }),
            Arg::Sc(v) => {
                let v = fixed::chan(v);
                px.fill([v, v, v])
            }
        },
    }
}

/// `ONE` for callers that build inputs by hand.
pub const Q16_ONE: i32 = ONE;
