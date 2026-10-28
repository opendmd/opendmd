//! Fused pixel steps: short runs of steps that the VM works out in one
//! loop over the lane instead of one loop each (the per-element load,
//! store and loop of each step are most of a cheap step's cost on the
//! ESP32).
//!
//! [`fuse`] runs after `range::narrow` on a laid-out program and puts a
//! marker (`code::FUSE d, a`: kind `d`, the `a` steps after it) before each
//! run it recognises. **The steps stay**: the VM runs the run's kernel in
//! their place only on a frame that runs the narrowed steps (and never in
//! wasm); a frame that runs the checked steps, or a kernel that does not
//! take its operands' kinds, runs the steps as ever. Each kernel is the
//! composition of the same `fixed` functions the steps call, in the same
//! order, so its value is the steps' value bit for bit; what [`fuse`]
//! checks is that skipping the steps' other writes changes nothing: every
//! value a run works out on the way is read by nothing after it (it is
//! written again first), the run reads nothing it wrote except along its
//! own pattern, and a kernel's destination is none of its sources.
//!
//! Only in the steps after the column run and the still cache's run (the
//! markers' counts are left as they are), and only where the step limit
//! leaves room for the markers; they are not counted as steps
//! ([`Program::steps`]).

use crate::program::*;
use alloc::vec::Vec;

/// `mask(add_w(floor(x), floor(y)), m)`: a checker's `(floor(u) +
/// floor(v)) % 2^k`, x and y lanes, m a value for the lane.
pub(crate) const F_CHECKER: u8 = 1;
/// `add_w(mul_w(a, b), c)` (or `mul_n`), a a lane, b a value for the
/// lane, c either (a value for the lane with `mul_n`, or when the result
/// goes into a's register).
pub(crate) const F_MADD: u8 = 2;
/// The bottom layer's `pal(P, select(c, A, B))` (`* k`, k a lane or a
/// value for the lane), A and B values for the lane.
pub(crate) const F_SELPAL: u8 = 3;
/// The bottom layer's `pal(P, pick(m, …))` (`* k`, k a value for the
/// lane): the choices' colours worked out once a lane, then looked up.
pub(crate) const F_PICKPAL: u8 = 4;
/// The bottom layer's colour of the draw stage's canvas alone: `ink`, up
/// to [`INK_STEPS`] steps of it and values for the lane, and a colour of
/// those (`pal(P, ink)`, `#40c0ff * ink`, `gray(ink)`): worked out for
/// each byte of the canvas a lane meets, kept while the next pixel's byte
/// is the same (mostly 0: nothing drawn).
pub(crate) const F_INK: u8 = 5;

/// An `F_INK` run's cost, ns a pixel (a guess until psi-probe measures
/// it): a byte read, a compare and the colour's three bytes written, the
/// colour worked out again only where the byte changes.
pub(crate) const INK_NS: u32 = 40;

/// Steps an `F_INK` run may hold between its `ink` and its colour.
pub(crate) const INK_STEPS: usize = 6;

/// A pick fused with its colour has at most this many choices.
pub(crate) const PICK_MAX: usize = 16;

/// A kind's name, for listings.
pub(crate) fn name(kind: u8) -> &'static str {
    match kind {
        F_CHECKER => "checker",
        F_MADD => "mul_add",
        F_SELPAL => "pal_select",
        F_PICKPAL => "pal_pick",
        F_INK => "colour_of_ink",
        _ => "?",
    }
}

#[inline]
fn is_lane(v: u8) -> bool {
    v & LANE != 0
}

#[inline]
fn is_reg(v: u8) -> bool {
    v & LANE != 0 && v & !LANE >= L_REG0
}

/// The register a pixel step writes, if it writes one.
#[inline]
fn writes(op: &Op) -> Option<u8> {
    (code::writes_scalar(op.code) && is_reg(op.d)).then_some(op.d)
}

/// The bottom layer's palette colour reading `r` as its entry (and, scaled,
/// a scale other than `r`; `scalar_k`: a scale that is a value for the
/// lane).
fn pal_of(op: &Op, r: u8, scalar_k: bool) -> bool {
    if op.code != code::OUT_PAL || op.a != r {
        return false;
    }
    if op.d & PAL_SCALED == 0 {
        return true;
    }
    op.c != r && !(scalar_k && is_lane(op.c))
}

/// The pattern at `ops[0..]`: its kind, length, the values it works out on
/// the way (to be dead after it) and its kernel's destination (a register,
/// or 0 for one that writes the pixels).
fn pattern(ops: &[Op]) -> Option<(u8, usize, [u8; 3], u8)> {
    use code::*;
    let op = |k: usize| ops.get(k);
    let a = op(0)?;
    match a.code {
        FLOOR => {
            let (b, c, d) = (op(1)?, op(2)?, op(3)?);
            let (x, r1, y, r2) = (a.a, a.d, b.a, b.d);
            let sum = c.code == ADD_W && ((c.a == r1 && c.b == r2) || (c.a == r2 && c.b == r1));
            let ok = b.code == FLOOR
                && is_lane(x)
                && is_lane(y)
                && y != r1
                && r1 != r2
                && sum
                && d.code == MASK
                && d.a == c.d
                && !is_lane(d.b)
                && d.d != x
                && d.d != y;
            ok.then_some((F_CHECKER, 4, [r1, r2, c.d], d.d))
        }
        MUL_W | MUL_N => {
            let b = op(1)?;
            let (l, s) = match (is_lane(a.a), is_lane(a.b)) {
                (true, false) => (a.a, a.b),
                (false, true) => (a.b, a.a),
                _ => return None,
            };
            let _ = s;
            let r = a.d;
            let c = if b.a == r { b.b } else { b.a };
            // (The destination may be the lane multiplied when the added
            // value is one for the lane: a kernel that reads and writes
            // its own lane.)
            let ok = b.code == ADD_W
                && (b.a == r || b.b == r)
                && c != r
                // (mul_n's kernel takes a value for the lane only.)
                && (a.code == MUL_W || !is_lane(c))
                && (b.d != l || !is_lane(c))
                && b.d != c;
            ok.then_some((F_MADD, 2, [r, r, r], b.d))
        }
        SELECT => {
            let b = op(1)?;
            let ok = is_lane(a.a) && !is_lane(a.b) && !is_lane(a.c) && pal_of(b, a.d, false);
            ok.then_some((F_SELPAL, 2, [a.d; 3], 0))
        }
        PICK => {
            let b = op(1)?;
            let ok = is_lane(a.a) && a.c as usize <= PICK_MAX && pal_of(b, a.d, true);
            ok.then_some((F_PICKPAL, 2, [a.d; 3], 0))
        }
        INK => {
            // Steps that read only the run's registers and values for the
            // lane, then the bottom layer's plain colour (no coverage) of
            // them. (Every register it writes must be dead after it:
            // `fuse` checks them all, `ink_regs`.)
            let mut regs = [a.d; INK_STEPS + 1];
            for (j, o) in ops.iter().enumerate().skip(1).take(INK_STEPS + 1) {
                let (v, n) = code::operands(o);
                if !v[..n]
                    .iter()
                    .all(|&v| !is_lane(v) || regs[..j].contains(&v))
                {
                    return None;
                }
                if code::is_colour(o.code) {
                    return (o.code < OUT_LAYER).then_some((F_INK, j + 1, [0; 3], 0));
                }
                let ok = matches!(o.code, 0..=LAST_BIN | 32..=LAST_UN | 64..=LAST_TERN | INK)
                    && !code::is_wide(o.code)
                    && is_reg(o.d)
                    && j < INK_STEPS + 1;
                if !ok {
                    return None;
                }
                regs[j] = o.d;
            }
            None
        }
        _ => None,
    }
}

/// The registers an `F_INK` run of `n` steps writes.
fn ink_regs(ops: &[Op], n: usize) -> impl Iterator<Item = u8> + '_ {
    ops[..n.saturating_sub(1)].iter().filter_map(writes)
}

/// Whether register `r` is read by nothing in `ops` before something
/// writes it (registers carry nothing from one lane of pixels to the next
/// but the column run's, which the compiler never writes again).
fn dead(ops: &[Op], r: u8) -> bool {
    for op in ops {
        let (v, n) = code::operands(op);
        if v[..n].contains(&r) {
            return false;
        }
        if writes(op) == Some(r) {
            return true;
        }
    }
    true
}

/// Put a marker before each run the VM may fuse (see the module).
#[inline(never)]
pub(crate) fn fuse(prog: &mut Program) {
    let (re, pe) = (prog.row_end as usize, prog.len().min(MAX_OPS));
    let mut start = re.max(prog.column_run().end);
    if let Some(k) = prog.still_marker() {
        start = start.max(k + 1 + prog.ops[k].a as usize);
    }
    let room = MAX_OPS - pe;
    let mut runs: Vec<(usize, u8, usize)> = Vec::new();
    let mut k = start;
    while k < pe && runs.len() < room {
        let ops = &prog.ops[k..pe];
        if matches!(ops[0].code, code::STILL | code::COLUMN | code::FUSE) {
            k += 1;
            continue;
        }
        let ok = |kind: u8, n: usize, mid: [u8; 3], dst: u8| {
            if kind == F_INK {
                // Every register the run writes is dead after it.
                ink_regs(ops, n).all(|r| dead(&ops[n..], r))
            } else {
                mid.iter().all(|&r| r == dst || dead(&ops[n..], r))
            }
        };
        match pattern(ops) {
            Some((kind, n, mid, dst)) if ok(kind, n, mid, dst) => {
                runs.push((k, kind, n));
                k += n;
            }
            _ => k += 1,
        }
    }
    if runs.is_empty() {
        return;
    }
    // Insert the markers, from the end.
    let mut end = pe + runs.len();
    let mut src = pe;
    for &(at, kind, n) in runs.iter().rev() {
        while src > at {
            src -= 1;
            end -= 1;
            prog.ops[end] = prog.ops[src];
        }
        end -= 1;
        prog.ops[end] = Op {
            code: code::FUSE,
            d: kind,
            a: n as u8,
            b: 0,
            c: 0,
        };
    }
    debug_assert_eq!(end, src);
    prog.pixel_end = (pe + runs.len()) as u16;
}

/// What a fused run of kind `kind` saves, ns a pixel, against its steps'
/// costs (guesses until psi-probe measures them): the steps it folds in,
/// each a loop's load, store and turn: a checker's two floors, add and mask
/// in one loop (40 + 40 + 25 + 50 against ~45), a multiply-add's add (25,
/// ~5 of it kept), a colour's choice (60), pick (100) or canvas load (70),
/// less a little for the lookup in the colour's loop.
pub(crate) fn saved(kind: u8) -> u32 {
    match kind {
        F_CHECKER => 110,
        F_MADD => 20,
        F_SELPAL => 50,
        F_PICKPAL => 85,
        // (An `F_INK` run is costed whole by the estimate: `INK_NS`.)
        F_INK => 0,
        _ => 0,
    }
}
