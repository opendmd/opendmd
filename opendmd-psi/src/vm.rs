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

use crate::draw::Ink;
use crate::fixed::{self, ONE};
use crate::program::*;
use crate::still::Still;
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
/// (worked out once, by [`Scratch::new`] or [`Scratch::reset`]): 8,192 B
/// at 128 lanes, 4,864 B at 64. Plain integers, so a zeroed allocation plus `reset` is a
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
    /// The value `k` ahead of the cursor.
    fn at(self, k: usize) -> i32;
    /// Move the cursor `n` ahead (a lane's pointer; a scalar stays).
    fn step(&mut self, n: usize);
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

    #[inline(always)]
    fn step(&mut self, n: usize) {
        // SAFETY: stays within (or one past) the lane.
        self.0 = unsafe { self.0.add(n) };
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

    #[inline(always)]
    fn step(&mut self, _: usize) {}
}

#[derive(Clone, Copy)]
enum Arg {
    Lane(*const i32),
    Sc(i32),
}

// The loops. `run*` are one element a turn, for the steps whose work
// dwarfs the loop's (`len`, `noise`, ...); `fast*` four a turn, for the
// cheap ones, where the ESP32 would otherwise spend as long on the loop
// (counter, branch, pointers: it has no post-increment loads) as on the
// add. Each is its own function, not inlined into the dispatch: with the
// whole VM in one function the ESP32's compiler ran out of registers and
// reloaded constants from flash and spilled to the stack every element.

#[inline(never)]
fn fast1<const N: usize, A: Src>(d: &mut [i32; N], mut a: A, f: impl Fn(i32) -> i32) {
    for o in d.as_chunks_mut::<4>().0 {
        o[0] = f(a.at(0));
        o[1] = f(a.at(1));
        o[2] = f(a.at(2));
        o[3] = f(a.at(3));
        a.step(4);
    }
}

#[inline(never)]
fn fast2<const N: usize, A: Src, B: Src>(
    d: &mut [i32; N],
    mut a: A,
    mut b: B,
    f: impl Fn(i32, i32) -> i32,
) {
    for o in d.as_chunks_mut::<4>().0 {
        o[0] = f(a.at(0), b.at(0));
        o[1] = f(a.at(1), b.at(1));
        o[2] = f(a.at(2), b.at(2));
        o[3] = f(a.at(3), b.at(3));
        a.step(4);
        b.step(4);
    }
}

#[inline(never)]
fn fast3<const N: usize, A: Src, B: Src, C: Src>(
    d: &mut [i32; N],
    mut a: A,
    mut b: B,
    mut c: C,
    f: impl Fn(i32, i32, i32) -> i32,
) {
    for o in d.as_chunks_mut::<4>().0 {
        o[0] = f(a.at(0), b.at(0), c.at(0));
        o[1] = f(a.at(1), b.at(1), c.at(1));
        o[2] = f(a.at(2), b.at(2), c.at(2));
        o[3] = f(a.at(3), b.at(3), c.at(3));
        a.step(4);
        b.step(4);
        c.step(4);
    }
}

#[inline(never)]
fn run1<const N: usize, A: Src>(d: &mut [i32; N], mut a: A, f: impl Fn(i32) -> i32) {
    for o in d.iter_mut() {
        *o = f(a.at(0));
        a.step(1);
    }
}

#[inline(never)]
fn run2<const N: usize, A: Src, B: Src>(
    d: &mut [i32; N],
    mut a: A,
    mut b: B,
    f: impl Fn(i32, i32) -> i32,
) {
    for o in d.iter_mut() {
        *o = f(a.at(0), b.at(0));
        a.step(1);
        b.step(1);
    }
}

#[inline(never)]
fn run3<const N: usize, A: Src, B: Src, C: Src>(
    d: &mut [i32; N],
    mut a: A,
    mut b: B,
    mut c: C,
    f: impl Fn(i32, i32, i32) -> i32,
) {
    for o in d.iter_mut() {
        *o = f(a.at(0), b.at(0), c.at(0));
        a.step(1);
        b.step(1);
        c.step(1);
    }
}

#[inline(never)]
fn out3<A: Src, B: Src, C: Src>(
    px: &mut [[u8; 3]],
    mut a: A,
    mut b: B,
    mut c: C,
    f: impl Fn(i32, i32, i32) -> [u8; 3],
) {
    for o in px.iter_mut() {
        *o = f(a.at(0), b.at(0), c.at(0));
        a.step(1);
        b.step(1);
        c.step(1);
    }
}

#[inline(never)]
fn out2<A: Src, B: Src>(px: &mut [[u8; 3]], mut a: A, mut b: B, f: impl Fn(i32, i32) -> [u8; 3]) {
    for o in px.iter_mut() {
        *o = f(a.at(0), b.at(0));
        a.step(1);
        b.step(1);
    }
}

#[inline(never)]
fn out1<A: Src>(px: &mut [[u8; 3]], mut a: A, f: impl Fn(i32) -> [u8; 3]) {
    for o in px.iter_mut() {
        *o = f(a.at(0));
        a.step(1);
    }
}

macro_rules! un {
    ($run:ident, $d:expr, $a:expr, $f:expr) => {
        match $a {
            Arg::Lane(p) => $run($d, L(p), $f),
            Arg::Sc(v) => $d.fill(($f)(v)),
        }
    };
}

macro_rules! bin {
    ($run:ident, $d:expr, $a:expr, $b:expr, $f:expr) => {
        match ($a, $b) {
            (Arg::Lane(p), Arg::Lane(q)) => $run($d, L(p), L(q), $f),
            (Arg::Lane(p), Arg::Sc(v)) => $run($d, L(p), S(v), $f),
            (Arg::Sc(u), Arg::Lane(q)) => $run($d, S(u), L(q), $f),
            (Arg::Sc(u), Arg::Sc(v)) => $d.fill(($f)(u, v)),
        }
    };
}

macro_rules! tern {
    ($run:ident, $d:expr, $a:expr, $b:expr, $c:expr, $f:expr) => {
        match ($a, $b, $c) {
            (Arg::Lane(p), Arg::Lane(q), Arg::Lane(r)) => $run($d, L(p), L(q), L(r), $f),
            (Arg::Lane(p), Arg::Lane(q), Arg::Sc(w)) => $run($d, L(p), L(q), S(w), $f),
            (Arg::Lane(p), Arg::Sc(v), Arg::Lane(r)) => $run($d, L(p), S(v), L(r), $f),
            (Arg::Lane(p), Arg::Sc(v), Arg::Sc(w)) => $run($d, L(p), S(v), S(w), $f),
            (Arg::Sc(u), Arg::Lane(q), Arg::Lane(r)) => $run($d, S(u), L(q), L(r), $f),
            (Arg::Sc(u), Arg::Lane(q), Arg::Sc(w)) => $run($d, S(u), L(q), S(w), $f),
            (Arg::Sc(u), Arg::Sc(v), Arg::Lane(r)) => $run($d, S(u), S(v), L(r), $f),
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
/// the top), with lanes of `N` pixels, with nothing under it (`ur ug ub ul`
/// read 0; a layer that lets something show through shows black). Writes
/// every byte.
pub fn render_with<const N: usize>(
    prog: &Program,
    scratch: &mut Scratch<N>,
    inputs: &Inputs,
    tables: &Tables,
    out: &mut [u8],
) {
    render_rows::<N>(prog, scratch, inputs, tables, None, u32::MAX, out)
}

/// Render the rows of `prog`'s frame that `rows` has a bit for (bit `k`:
/// row `k` from the top) into `out` (128 × 32 RGB888, row-major from the
/// top), over `under` (the picture under the formula, the same layout: what
/// `ur ug ub ul` read and what shows through its layers; `None`: black).
/// The rest of `out` is left as it was. A row's bytes are the same whichever
/// other rows are drawn with it. A program that draws
/// ([`Program::draws`]) draws nothing here (`ink` reads 0): give it a
/// canvas with [`render_rows_ink`].
pub fn render_rows<const N: usize>(
    prog: &Program,
    scratch: &mut Scratch<N>,
    inputs: &Inputs,
    tables: &Tables,
    under: Option<&[u8]>,
    rows: u32,
    out: &mut [u8],
) {
    render_rows_ink::<N>(prog, scratch, None, inputs, tables, under, rows, out)
}

/// [`render_rows`] with the draw stage's canvas: a program that draws
/// ([`Program::draws`]) draws its frame's lines into `ink` first (cleared)
/// on a **full call** (`rows == u32::MAX`), and its `ink` reads them back.
/// A call for some rows only does not draw: it reads the canvas the last
/// full call left — the caller's job to make that the same program and
/// inputs (a look redraws only the rows its picture changed, with the same
/// `t`; anything new for it is drawn with every row first). `None`, or a
/// program that does not draw: `ink` reads 0.
#[allow(clippy::too_many_arguments)]
pub fn render_rows_ink<const N: usize>(
    prog: &Program,
    scratch: &mut Scratch<N>,
    ink: Option<&mut Ink>,
    inputs: &Inputs,
    tables: &Tables,
    under: Option<&[u8]>,
    rows: u32,
    out: &mut [u8],
) {
    render_rows_still::<N>(prog, scratch, ink, None, inputs, tables, under, rows, out)
}

/// What the still cache does this frame.
enum Cache<'a> {
    Off,
    /// Skip the cached run (at the marker `at`), copy the kept lanes in.
    Use(&'a Still),
    /// Run everything and keep the cached values after the run.
    Keep(&'a mut Still, usize),
}

/// [`render_rows_ink`] with the still cache ([`Still`]): a program with
/// cached values ([`Program::still_lanes`]) given a `still` that holds
/// lanes for them keeps them on a full call (`rows == u32::MAX`) and, once
/// kept, skips the steps that work them out on every call after. The bytes
/// are the same with or without it, kept or not; `None`, too few lanes, or
/// a program without cached values: every step runs.
#[allow(clippy::too_many_arguments)]
pub fn render_rows_still<const N: usize>(
    prog: &Program,
    scratch: &mut Scratch<N>,
    ink: Option<&mut Ink>,
    still: Option<&mut Still>,
    inputs: &Inputs,
    tables: &Tables,
    under: Option<&[u8]>,
    rows: u32,
    out: &mut [u8],
) {
    assert_eq!(out.len(), W * H * 3);
    if let Some(u) = under {
        assert_eq!(u.len(), W * H * 3);
    }
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
    let (fe, de, re, pe) = (
        prog.frame_end as usize,
        prog.draw_end as usize,
        prog.row_end as usize,
        prog.pixel_end as usize,
    );
    for op in &prog.ops[..fe] {
        scalar(sc, op, tables, &prog.tables);
    }
    let ink: Option<&Ink> = match ink {
        Some(k) if prog.draws() => {
            if rows == u32::MAX {
                crate::draw::run(prog, sc, tables, k);
            }
            Some(k)
        }
        _ => None,
    };
    let mut cache = match (still, prog.still_marker()) {
        (Some(st), Some(at)) if st.lanes_for(prog) => {
            if st.holds(prog) {
                Cache::Use(st)
            } else if rows == u32::MAX {
                st.kept = 0;
                Cache::Keep(st, at)
            } else {
                Cache::Off
            }
        }
        _ => Cache::Off,
    };
    let uses_i = prog.uses & USES_I != 0;
    let backdrop = prog.uses & BACKDROP != 0;
    for row in 0..H {
        if rows & (1 << row) == 0 {
            continue;
        }
        // y and py count from the bottom (y = 0 is the bottom row).
        let py = ((H - 1 - row) as i32) << 16;
        scratch.sc[S_PY as usize] = py;
        scratch.sc[S_Y as usize] = fixed::div(py, ((H - 1) as i32) << 16);
        for op in &prog.ops[de..re] {
            scalar(&mut scratch.sc, op, tables, &prog.tables);
        }
        let span = row * W * 3..(row + 1) * W * 3;
        let row_bytes = &mut out[span.clone()];
        let under_row = under.map(|u| &u[span.clone()]);
        for base in (0..W).step_by(N) {
            if uses_i {
                for (k, v) in scratch.lanes[0].iter_mut().enumerate() {
                    *v = ((row * W + base + k) as i32) << 16;
                }
            }
            let lane = base * 3..(base + N) * 3;
            let ul = under_row.map(|u| &u[lane.clone()]);
            let il = ink.map(|k| &k.0[row * W + base..row * W + base + N]);
            let px = row_bytes[lane].as_chunks_mut::<3>().0;
            if backdrop {
                match ul {
                    Some(u) => px.as_flattened_mut().copy_from_slice(u),
                    None => px.fill([0; 3]),
                }
            }
            let mut k = re;
            let mut keep_at = usize::MAX;
            while k < pe {
                let op = &prog.ops[k];
                if k == keep_at {
                    if let Cache::Keep(st, at) = &mut cache {
                        let m = &prog.ops[*at];
                        keep(scratch, st, [m.b, m.c], m.d, row * W + base);
                    }
                }
                if op.code == code::STILL {
                    match &cache {
                        Cache::Use(st) => {
                            fetch(scratch, st, [op.b, op.c], op.d, row * W + base);
                            k += 1 + op.a as usize;
                        }
                        Cache::Keep(..) => {
                            keep_at = k + 1 + op.a as usize;
                            k += 1;
                        }
                        Cache::Off => k += 1,
                    }
                    continue;
                }
                if code::is_colour(op.code) {
                    colour_step(scratch, op, base, tables, px);
                } else {
                    lane_op(scratch, op, base, tables, &prog.tables, ul, il);
                }
                k += 1;
            }
        }
    }
    if let Cache::Keep(st, _) = cache {
        st.kept = crate::still::fingerprint(prog);
    }
}

/// The lane register a register operand names (as `lane_op`'s
/// destination).
#[inline(always)]
fn reg_index(v: u8) -> usize {
    ((v & !LANE).saturating_sub(L_I) as usize).clamp(1, REGS)
}

/// Copy the cached registers' lanes (`n` of `regs`) into the kept lanes at
/// pixel `at`.
#[inline(never)]
fn keep<const N: usize>(s: &Scratch<N>, st: &mut Still, regs: [u8; 2], n: u8, at: usize) {
    for (j, &r) in regs.iter().enumerate().take(n as usize) {
        if let Some(l) = st.lanes[j].as_deref_mut() {
            l.0[at..at + N].copy_from_slice(&s.lanes[reg_index(r)]);
        }
    }
}

/// Copy the kept lanes at pixel `at` into the cached registers.
#[inline(never)]
fn fetch<const N: usize>(s: &mut Scratch<N>, st: &Still, regs: [u8; 2], n: u8, at: usize) {
    for (j, &r) in regs.iter().enumerate().take(n as usize) {
        if let Some(l) = st.lanes[j].as_deref() {
            s.lanes[reg_index(r)].copy_from_slice(&l.0[at..at + N]);
        }
    }
}

#[inline(always)]
pub(crate) fn scalar(sc: &mut [i32; MAX_SCALARS], op: &Op, t: &Tables, tabs: &[i32; MAX_TABLE]) {
    // Operands of scalar steps are always scalar slots (< 128).
    let g = |v: u8| sc[(v & 0x7f) as usize];
    let v = if op.code == code::TAB {
        tab(tabs, op.b, op.c, g(op.a))
    } else {
        eval(op.code, g(op.a), g(op.b), g(op.c), Some(t))
    };
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

fn lane_op<const N: usize>(
    s: &mut Scratch<N>,
    op: &Op,
    base: usize,
    t: &Tables,
    tabs: &[i32; MAX_TABLE],
    under: Option<&[u8]>,
    ink: Option<&[u8]>,
) {
    use code::*;
    let (a, b, c) = (s.arg(op.a, base), s.arg(op.b, base), s.arg(op.c, base));
    // The destination is a register the compiler never also reads in the
    // same step (`compile` allocates it that way), so the `&mut` lane does
    // not alias the source pointers.
    let di = reg_index(op.d);
    debug_assert!(op.d & LANE != 0 && op.d & !LANE >= L_REG0);
    let d: &mut [i32; N] = &mut s.lanes[di];
    match op.code {
        ADD => bin!(fast2, d, a, b, fixed::add),
        SUB => bin!(fast2, d, a, b, fixed::sub),
        MUL => bin!(fast2, d, a, b, fixed::mul),
        DIV => bin!(run2, d, a, b, fixed::div),
        MOD => bin!(run2, d, a, b, fixed::modulo),
        MASK => bin!(fast2, d, a, b, fixed::mask),
        LT => bin!(fast2, d, a, b, fixed::lt),
        LE => bin!(fast2, d, a, b, fixed::le),
        GT => bin!(fast2, d, a, b, fixed::gt),
        GE => bin!(fast2, d, a, b, fixed::ge),
        EQ => bin!(fast2, d, a, b, fixed::eq),
        NE => bin!(fast2, d, a, b, fixed::ne),
        AND => bin!(fast2, d, a, b, fixed::and),
        OR => bin!(fast2, d, a, b, fixed::or),
        MIN => bin!(fast2, d, a, b, fixed::min),
        MAX => bin!(fast2, d, a, b, fixed::max),
        STEP => bin!(fast2, d, a, b, fixed::step),
        ATAN2 => bin!(run2, d, a, b, fixed::atan2),
        HASH => bin!(run2, d, a, b, fixed::hash),
        LEN => bin!(run2, d, a, b, fixed::len),
        BIT => bin!(fast2, d, a, b, fixed::bit),
        NEG => un!(fast1, d, a, fixed::neg),
        NOT => un!(fast1, d, a, fixed::not),
        ABS => un!(fast1, d, a, fixed::abs),
        FLOOR => un!(fast1, d, a, fixed::floor),
        FRAC => un!(fast1, d, a, fixed::frac),
        SIN => un!(fast1, d, a, |v| fixed::sin(t, v)),
        COS => un!(fast1, d, a, |v| fixed::cos(t, v)),
        TRI => un!(fast1, d, a, fixed::tri),
        SAW => un!(fast1, d, a, fixed::saw),
        SQR => un!(fast1, d, a, fixed::sqr),
        SQRT => un!(run1, d, a, fixed::sqrt),
        RECIP => un!(run1, d, a, fixed::recip),
        // (k is always a lane: a pick with a scalar k is compares and
        // selects.)
        PICK => {
            if let Arg::Lane(p) = a {
                let sc = &s.sc;
                run1(d, L(p), |k| pick(sc, op.b, op.c, k))
            }
        }
        // (k is always a lane here: a table read of a scalar is a scalar
        // step.)
        TAB => {
            if let Arg::Lane(p) = a {
                run1(d, L(p), |k| tab(tabs, op.b, op.c, k))
            }
        }
        SELECT => tern!(fast3, d, a, b, c, fixed::select),
        CLAMP => tern!(fast3, d, a, b, c, fixed::clamp),
        MIX => tern!(fast3, d, a, b, c, fixed::mix),
        SMOOTH => tern!(run3, d, a, b, c, fixed::smooth),
        // A whole `z` (`noise(x, y, 0)`): the plane's four corners only —
        // the other four are weighed 0 there, so the bytes are the same.
        NOISE => match c {
            Arg::Sc(z) if z & 0xffff == 0 => bin!(run2, d, a, b, |x, y| fixed::noise_flat(x, y, z)),
            _ => tern!(run3, d, a, b, c, fixed::noise),
        },
        // One loop for the four, its operands read with a stride (0: a
        // value for the whole lane): the lookup dwarfs the loop, and seven
        // monomorphised copies would be ~1 KB more of wasm.
        DIGIT..=DIGIT_B => glyph_lane(d, [a, b, c], &glyphs(t, op.code)),
        U_R..=U_L => load_under(d, under, op.code - U_R),
        INK => load_ink(d, ink),
        _ => {}
    }
}

/// A font's pixel for each element of the lane.
#[inline(never)]
fn glyph_lane<const N: usize>(d: &mut [i32; N], args: [Arg; 3], g: &crate::tables::Glyphs) {
    let mut one = [0i32; 3];
    let mut p = [core::ptr::null::<i32>(); 3];
    let mut stride = [0usize; 3];
    for k in 0..3 {
        match args[k] {
            Arg::Lane(q) => (p[k], stride[k]) = (q, 1),
            Arg::Sc(v) => one[k] = v,
        }
    }
    for k in 0..3 {
        if stride[k] == 0 {
            p[k] = &one[k];
        }
    }
    for o in d.iter_mut() {
        // SAFETY: each pointer is a lane of `N` values read once per
        // element (stride 1), or one value (stride 0), alive for the call.
        let (c, x, y) = unsafe { (*p[0], *p[1], *p[2]) };
        *o = fixed::glyph(g, c, x, y);
        for k in 0..3 {
            p[k] = p[k].wrapping_add(stride[k]);
        }
    }
}

/// A channel of what is under the lane (0 red, 1 green, 2 blue, 3 the
/// luminance) as 0..1; 0 with nothing under.
#[inline(never)]
fn load_under<const N: usize>(d: &mut [i32; N], under: Option<&[u8]>, ch: u8) {
    let Some(u) = under else {
        d.fill(0);
        return;
    };
    let px = u.as_chunks::<3>().0;
    match ch {
        0..=2 => {
            let ch = ch as usize;
            for (o, p) in d.iter_mut().zip(px) {
                *o = fixed::byte(p[ch]);
            }
        }
        _ => {
            for (o, p) in d.iter_mut().zip(px) {
                *o = fixed::byte(fixed::luma(p[0], p[1], p[2]));
            }
        }
    }
}

/// The draw stage's canvas under the lane, 0..1; 0 without one.
#[inline(never)]
fn load_ink<const N: usize>(d: &mut [i32; N], ink: Option<&[u8]>) {
    match ink {
        Some(k) => {
            for (o, &b) in d.iter_mut().zip(k) {
                *o = fixed::byte(b);
            }
        }
        None => d.fill(0),
    }
}

/// A colour step: the bottom layer's colour written over the lane, or a
/// layer's colour (into a row on the stack) blended onto it.
fn colour_step<const N: usize>(
    s: &Scratch<N>,
    op: &Op,
    base: usize,
    t: &Tables,
    px: &mut [[u8; 3]],
) {
    let (kind, blend) = code::colour_parts(op.code);
    let Some(mode) = blend else {
        colour(s, op, base, t, px);
        return;
    };
    let (v, n) = code::operands(op);
    let plain = Op {
        code: kind,
        d: if kind == code::OUT_PAL { op.d } else { 0 },
        ..*op
    };
    let mut row = [[0u8; 3]; W];
    let src = &mut row[..px.len()];
    colour(s, &plain, base, t, src);
    // A layer that covers everything (the usual: no coverage given) is one
    // loop per blend, so the blend is not a branch per channel (an indirect
    // jump on the ESP32); one with a coverage, a lane or a value read with
    // a stride of 0, shares one loop.
    let one;
    let (k, stride) = match s.arg(v[n - 1], base) {
        Arg::Sc(v) if v >= ONE => {
            match mode {
                fixed::BLEND_ADD => blend_all::<{ fixed::BLEND_ADD }>(px, src),
                fixed::BLEND_MULTIPLY => blend_all::<{ fixed::BLEND_MULTIPLY }>(px, src),
                fixed::BLEND_SCREEN => blend_all::<{ fixed::BLEND_SCREEN }>(px, src),
                fixed::BLEND_MAX => blend_all::<{ fixed::BLEND_MAX }>(px, src),
                fixed::BLEND_MIN => blend_all::<{ fixed::BLEND_MIN }>(px, src),
                _ => px.copy_from_slice(src),
            }
            return;
        }
        Arg::Lane(p) => (p, 1),
        Arg::Sc(v) => {
            one = v;
            (&one as *const i32, 0)
        }
    };
    blend_cover(px, src, k, stride, mode);
}

#[inline(never)]
fn blend_all<const M: u8>(px: &mut [[u8; 3]], src: &[[u8; 3]]) {
    for (d, s) in px.iter_mut().zip(src) {
        for c in 0..3 {
            d[c] = fixed::blend(M, d[c], s[c]);
        }
    }
}

/// `px` (what is below) with `src` blended on by `mode`, covered by the
/// values at `k`, `stride` apart (0: one value for the lane).
#[inline(never)]
fn blend_cover(px: &mut [[u8; 3]], src: &[[u8; 3]], mut k: *const i32, stride: usize, mode: u8) {
    for (d, s) in px.iter_mut().zip(src) {
        // SAFETY: `k` is a lane of at least `px.len()` values (stride 1) or
        // one value (stride 0), alive for the call.
        let a = unsafe { *k };
        for c in 0..3 {
            d[c] = fixed::cover(d[c], fixed::blend(mode, d[c], s[c]), a);
        }
        k = k.wrapping_add(stride);
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
        OUT_PAL if op.d & PAL_SCALED != 0 => {
            let p = t.palettes[((op.d & !PAL_SCALED) as usize).min(4)];
            let f = |k, s| fixed::pal_lit(p, k, s);
            match (a, c) {
                (Arg::Lane(q), Arg::Lane(r)) => out2(px, L(q), L(r), f),
                (Arg::Lane(q), Arg::Sc(w)) => out2(px, L(q), S(w), f),
                (Arg::Sc(v), Arg::Lane(r)) => out2(px, S(v), L(r), f),
                (Arg::Sc(v), Arg::Sc(w)) => px.fill(f(v, w)),
            }
        }
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
