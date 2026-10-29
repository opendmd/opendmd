//! The lowering: a [`Program`]'s pixel stage, after its column and still
//! runs, as one Xtensa function a row
//! (`extern "C" fn(ctx: *const u32, out: *mut u8)`): a loop over the 128
//! pixels with every Psi register in an address register, every scalar,
//! lane address, table and helper read from `ctx` (words the driver fills
//! for each row, [`Word`]), and the colour's three bytes stored at `out`.
//!
//! Each step is a template derived from the `fixed` function the VM calls
//! for it, the same operations in the same order (so the same bits); the
//! heavy steps (`len`, `atan2`, `div`, `noise`, …) call that function
//! through a shim (`callx8`, [`hid`]). Anything else — a step without a
//! template, layers, `ur ug ub ul`, `ink`, more registers than the map
//! holds — is a [`Refusal`]: the whole formula plays on the VM.
//!
//! **Registers.** a0/a1 the windowed ABI's (return, stack); a2 `ctx`, a3
//! the pixel (0..127), a4 the output; a5..a12 the homes of the Psi
//! registers the stage uses (one each, ≤ 8) and, in what is left, values
//! hoisted out of the loop (the most used scalars and lane addresses);
//! a13..a15 a step's temporaries. A `callx8` keeps a2..a7 only: the homes
//! in a8..a12 that are live across one are saved to the frame and loaded
//! back, and a kernel with calls hoists into a5..a7 only. SAR (`src`'s
//! shift, 16 for `mul_w`) is set again after a call.

use super::enc::*;
use crate::fuse::{F_PICKPAL, F_SELPAL, F_SELPAL_K, PICK_MAX};
use crate::program::*;
use alloc::vec::Vec;

/// "No slot" in a [`Word`] (a scale of 1).
pub const NONE: u8 = 0xff;

/// What a word of `ctx` holds for a row: the driver works each out from
/// the VM's state after the row's steps (`super::fill`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    /// A lane's address at the row's first pixel: `L_X`, `L_I`, or a
    /// register's (`L_REG0 + k`: its lane, or its still cache's alias).
    Lane(u8),
    /// Scalar slot `k`'s value.
    Scalar(u8),
    /// Palette `k`'s address (a built-in, or a formula's own unpacked).
    Pal(u8),
    /// The sine table's address.
    Sin,
    /// A helper's address ([`hid`]).
    Helper(u8),
    /// The scalar slots' address.
    Sc,
    /// The program's tables' address.
    Tabs,
    /// The address of the font a glyph step of this code reads.
    Glyphs(u8),
    /// `pal(P, sc[v]) * k` for the row, packed `r | g << 8 | b << 16`
    /// (`k` a slot, shifted right by `shift`; [`NONE`]: 1).
    Colour { pal: u8, v: u8, k: u8, shift: u8 },
    /// The address of palette `pal`'s entry for `sc[v]`.
    Entry { pal: u8, v: u8 },
    /// A constant (a colour memo's start).
    Init(i32),
    /// Choice `i` of a fused `pal(P, pick(m, …)) * k`'s colours for the
    /// row (`b`, `n` the pick's), packed; `i` = [`PICK_MAX`] the colour of
    /// a pick outside its choices.
    Pick { pal: u8, b: u8, n: u8, i: u8, k: u8, shift: u8 },
}

/// The helpers a kernel calls (`super::helper_addr`, `super::eval_helper`).
pub mod hid {
    pub const DIV: u8 = 0;
    pub const MOD: u8 = 1;
    pub const ATAN2: u8 = 2;
    pub const HASH: u8 = 3;
    pub const LEN: u8 = 4;
    pub const SQRT: u8 = 5;
    pub const RECIP: u8 = 6;
    pub const SMOOTH: u8 = 7;
    pub const NOISE: u8 = 8;
    pub const NOISE_O: u8 = 9;
    pub const MULR: u8 = 10;
    pub const MIX: u8 = 11;
    pub const BIT: u8 = 12;
    pub const TAB: u8 = 13;
    pub const PICK: u8 = 14;
    pub const SPR: u8 = 15;
    pub const GLYPH: u8 = 16;
    pub const HSV: u8 = 17;
    /// `scale_rgb(entry, s)`, packed.
    pub const SCALE: u8 = 18;
    pub const COUNT: u8 = 19;
}

/// Why a formula plays on the VM: shown in `/api/status` (`formula.jit`).
pub type Refusal = &'static str;

/// A lowered pixel stage.
pub struct Lowered {
    pub code: Vec<u8>,
    pub words: Vec<Word>,
    /// The first step the kernel draws (after the column and still runs,
    /// which the driver runs as lanes).
    pub main_start: usize,
    /// It calls helpers.
    pub calls: bool,
}

// The register map.
const CTX: Reg = 2;
const PIX: Reg = 3;
const OUT: Reg = 4;
const POOL: [Reg; 8] = [5, 6, 7, 8, 9, 10, 11, 12];
const TEMPS: [Reg; 3] = [13, 14, 15];
/// A step wanted more temporaries than it had (the lowering tries again
/// with more).
const TEMPS_OUT: Refusal = "the kernel ran out of temporaries";
/// The helper's address goes here for `callx8` (and comes back as the
/// return address: it is the callee's a0).
const CALLEE: Reg = 8;
/// Frame slots: a8..a12's homes across a call, then one spare.
const SLOT_SPARE: i32 = 20;
const FRAME: u32 = 32 + 24;

/// One unit of the kernel: a step, or a fused colour the kernel draws in
/// one piece (as the VM's fused runs, `crate::fuse`).
#[derive(Clone, Copy, Debug)]
enum Item {
    Step(Op),
    /// `pal(P, select(c, A, B)) [* k]`: c a lane, A and B scalars.
    SelPal { c: u8, a: u8, b: u8, col: Op },
    /// `pal(P, pick(m, …)) [* k]`: m a lane, k a scalar or none.
    PickPal { m: u8, pk: Op, col: Op },
}

#[inline]
fn is_lane(v: u8) -> bool {
    v & LANE != 0
}

/// The Psi register (0-based) an operand names, if it is one.
#[inline]
fn reg_of(v: u8) -> Option<usize> {
    let l = v & !LANE;
    (is_lane(v) && l >= L_REG0).then(|| (l - L_REG0) as usize)
}

fn reads(it: &Item) -> ([u8; 4], usize) {
    match *it {
        Item::Step(op) => code::operands(&op),
        Item::SelPal { c, col, .. } => {
            if col.d & PAL_SCALED != 0 {
                ([c, col.c, 0, 0], 2)
            } else {
                ([c, 0, 0, 0], 1)
            }
        }
        Item::PickPal { m, .. } => ([m, 0, 0, 0], 1),
    }
}

fn writes(it: &Item) -> Option<usize> {
    match *it {
        Item::Step(op) if !code::is_colour(op.code) => reg_of(op.d),
        _ => None,
    }
}

/// Where the kernel starts: after the column run and the still cache's run
/// (with the values laid out before the still marker): the driver runs
/// those as lanes, the VM's way.
pub fn main_start(prog: &Program) -> usize {
    let (re, pe) = (prog.row_end as usize, prog.len());
    if let Some(k) = prog.still_marker() {
        return (k + 1 + prog.ops[k].a as usize).min(pe);
    }
    let col = prog.column_run();
    if col.end > col.start {
        col.end
    } else {
        re
    }
}

/// Lower `prog`'s pixel stage, or say why the VM must draw it.
pub fn lower(prog: &Program) -> Result<Lowered, Refusal> {
    if prog.uses & (UNDER | BACKDROP) != 0 {
        return Err("it reads the picture under it");
    }
    if prog.uses & INKED != 0 || prog.draws() {
        return Err("it draws (ink)");
    }
    if prog.layers() != 1 {
        return Err("it has layers");
    }
    let (pe, start) = (prog.len(), main_start(prog));
    if start >= pe {
        return Err("no pixel steps");
    }
    let items = items(prog, start, pe)?;
    // The fewest temporaries the steps get by with (a kernel without calls
    // hoists into the rest).
    let mut e = Emitter::new(prog, &items, 1)?;
    for t in 1..=3 {
        e = Emitter::new(prog, &items, t)?;
        match e.emit() {
            Ok(()) => break,
            Err(TEMPS_OUT) if t < 3 => continue,
            Err(w) => return Err(w),
        }
    }
    if e.c.bad {
        return Err("an operand out of the kernel's range");
    }
    if e.words.len() > 255 {
        return Err("too many values for the kernel");
    }
    Ok(Lowered {
        code: e.c.bytes,
        words: e.words,
        main_start: start,
        calls: e.calls,
    })
}

/// Is `code` a step the kernel has (a template or a helper)?
fn supported(c: u8) -> Result<(), Refusal> {
    use code::*;
    match c {
        ADD | SUB | MUL | ADD_W | SUB_W | MUL_W | MUL_N | MASK | LT | LE | GT | GE | EQ | NE
        | AND | OR | MIN | MAX | STEP | NEG | NOT | ABS | FLOOR | FRAC | SIN | COS | TRI | SAW
        | SQR | SHR | SELECT | CLAMP => Ok(()),
        DIV | MOD | ATAN2 | HASH | LEN | BIT | SQRT | RECIP | PICK | TAB | SPR | MIX | SMOOTH
        | NOISE | MULR => Ok(()),
        DIGIT..=DIGIT_B | NOISE_O..=NOISE_O_LAST => Ok(()),
        OUT_RGB | OUT_HSV | OUT_PAL | OUT_GRAY => Ok(()),
        U_R..=U_L => Err("it reads the picture under it"),
        INK => Err("it draws (ink)"),
        c if c >= OUT_LAYER => Err("it has layers"),
        STILL | COLUMN => Err("an unexpected layout"),
        _ => Err("a step the kernel has no template for"),
    }
}

/// The kernel's units, from `ops[start..pe]`.
fn items(prog: &Program, start: usize, pe: usize) -> Result<Vec<Item>, Refusal> {
    let ops = &prog.ops[..pe];
    let mut out = Vec::new();
    let mut k = start;
    while k < pe {
        let op = ops[k];
        if op.code == code::FUSE {
            let n = op.a as usize;
            let run = &ops[(k + 1).min(pe)..(k + 1 + n).min(pe)];
            if let Some(it) = fused(op.d, run) {
                out.push(it);
                k += 1 + n;
            } else {
                // The steps one at a time: the same values.
                k += 1;
            }
            continue;
        }
        supported(op.code)?;
        if code::is_colour(op.code) && k + 1 != pe {
            return Err("it has layers");
        }
        out.push(Item::Step(op));
        k += 1;
    }
    match out.last() {
        Some(Item::Step(op)) if code::is_colour(op.code) => {}
        Some(Item::SelPal { .. } | Item::PickPal { .. }) => {}
        _ => return Err("an unexpected layout"),
    }
    Ok(out)
}

/// A fused colour the kernel draws in one piece, when its operands are of
/// the kinds the VM's kernel takes too (else `None`: the steps).
fn fused(kind: u8, run: &[Op]) -> Option<Item> {
    match (kind, run) {
        (F_SELPAL | F_SELPAL_K, [sel, col])
            if sel.code == code::SELECT
                && col.code == code::OUT_PAL
                && is_lane(sel.a)
                && !is_lane(sel.b)
                && !is_lane(sel.c)
                && col.a == sel.d =>
        {
            Some(Item::SelPal { c: sel.a, a: sel.b, b: sel.c, col: *col })
        }
        (F_PICKPAL, [pk, col])
            if pk.code == code::PICK
                && col.code == code::OUT_PAL
                && is_lane(pk.a)
                && col.a == pk.d
                && (col.d & PAL_SCALED == 0 || !is_lane(col.c)) =>
        {
            Some(Item::PickPal { m: pk.a, pk: *pk, col: *col })
        }
        _ => None,
    }
}

/// An operand argument of a helper call.
#[derive(Clone, Copy)]
enum Arg {
    Val(u8),
    Word(Word),
    Imm(i32),
    /// The address of a `ctx` word.
    AddrOf(Word),
    Out,
}

/// A value a Psi register holds in the kernel: written by an item (or, for
/// a register read before any item writes it, its lane from before the
/// kernel: `livein`), alive until its last read; an address register for
/// that span.
#[derive(Clone, Copy, Debug)]
struct Val {
    start: usize,
    end: usize,
    livein: bool,
    /// Alive across a helper call (then it wants a5..a7, which a call
    /// keeps).
    crosses: bool,
    gpr: Reg,
}

struct Emitter<'a> {
    items: &'a [Item],
    c: Code,
    words: Vec<Word>,
    vals: Vec<Val>,
    /// The value each Psi register holds now (during emission).
    cur: [Option<usize>; REGS],
    /// Each register's value from before the kernel, if it is read so.
    livein: [Option<usize>; REGS],
    /// The value each item writes.
    wv: Vec<Option<usize>>,
    /// Whether a value is in its register this pixel (a live-in one is
    /// loaded at its first read; one saved across a call may be dropped).
    valid: Vec<bool>,
    /// Words held in registers for the whole row.
    hoist: Vec<(Word, Reg)>,
    temps: Vec<Reg>,
    free: Vec<Reg>,
    sar16: bool,
    calls: bool,
}

/// The values of `items` (each with its interval) and which value each
/// register reads first, and each item writes.
fn values(prog: &Program, items: &[Item]) -> (Vec<Val>, [Option<usize>; REGS], Vec<Option<usize>>) {
    let mut vals: Vec<Val> = Vec::new();
    let mut cur: [Option<usize>; REGS] = [None; REGS];
    let mut livein: [Option<usize>; REGS] = [None; REGS];
    let mut wv = alloc::vec![None; items.len()];
    for (k, it) in items.iter().enumerate() {
        let (v, n) = reads(it);
        for &x in &v[..n] {
            let Some(r) = reg_of(x) else { continue };
            match cur[r] {
                Some(id) => vals[id].end = k,
                None => {
                    vals.push(Val { start: k, end: k, livein: true, crosses: false, gpr: 0 });
                    let id = vals.len() - 1;
                    cur[r] = Some(id);
                    livein[r] = Some(id);
                }
            }
        }
        if let Some(w) = writes(it) {
            vals.push(Val { start: k, end: k, livein: false, crosses: false, gpr: 0 });
            cur[w] = Some(vals.len() - 1);
            wv[k] = cur[w];
        }
    }
    for (k, it) in items.iter().enumerate() {
        if needs_call(prog, it) {
            for v in vals.iter_mut() {
                // (Its own result, and operands read for the last time, need not survive.)
                if v.start <= k && v.end > k && !(v.start == k && !v.livein) {
                    v.crosses = true;
                }
            }
        }
    }
    (vals, livein, wv)
}

/// The most values alive at once.
fn pressure(vals: &[Val], n: usize) -> usize {
    (0..n).map(|k| vals.iter().filter(|v| v.start <= k && k <= v.end).count()).max().unwrap_or(0)
}

/// Give each value a register from `pool` (a linear scan over the items:
/// a value's register is free again after the item that reads it last),
/// those alive across a call preferring a5..a7 and the rest the others.
fn allocate(vals: &mut [Val], n: usize, pool: &[Reg]) -> Result<(), Refusal> {
    let mut free: Vec<Reg> = pool.to_vec();
    for k in 0..n {
        for v in vals.iter_mut().filter(|v| v.start == k) {
            let pick = if v.crosses {
                free.iter().position(|&r| r <= 7).or(if free.is_empty() { None } else { Some(0) })
            } else {
                free.iter().rposition(|&r| r >= 8).or(if free.is_empty() { None } else { Some(free.len() - 1) })
            };
            let j = pick.ok_or("more values at once than the kernel's registers")?;
            v.gpr = free.remove(j);
        }
        for v in vals.iter().filter(|v| v.end == k) {
            free.push(v.gpr);
        }
        free.sort_unstable();
    }
    Ok(())
}

impl<'a> Emitter<'a> {
    /// The emitter with `t` temporaries (a kernel without calls: values,
    /// hoists and temporaries share a5..a15; with calls: values in a5..a12,
    /// hoists in what a5..a7 the values leave, temporaries a13..a15).
    fn new(prog: &'a Program, items: &'a [Item], t: usize) -> Result<Emitter<'a>, Refusal> {
        let calls = items.iter().any(|it| needs_call(prog, it));
        let (mut vals, livein, wv) = values(prog, items);
        let p = pressure(&vals, items.len());
        let all: [Reg; 11] = [5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let (temps, spare): (Vec<Reg>, Vec<Reg>) = if calls {
            if p > POOL.len() {
                return Err("more values at once than the kernel's registers");
            }
            allocate(&mut vals, items.len(), &POOL)?;
            let spare = [5, 6, 7].into_iter().filter(|&r| vals.iter().all(|v| v.gpr != r)).collect();
            (TEMPS.to_vec(), spare)
        } else {
            if p + t > all.len() {
                return Err("more values at once than the kernel's registers");
            }
            allocate(&mut vals, items.len(), &all[..p])?;
            (all[all.len() - t..].to_vec(), all[p..all.len() - t].to_vec())
        };
        let mut e = Emitter {
            items,
            c: Code::new(),
            words: Vec::new(),
            valid: alloc::vec![false; vals.len()],
            vals,
            cur: livein,
            livein,
            wv,
            hoist: Vec::new(),
            temps,
            free: Vec::new(),
            sar16: false,
            calls,
        };
        e.choose_hoists(&spare);
        Ok(e)
    }

    /// Hoist the most used loop-invariant words into `spare`.
    fn choose_hoists(&mut self, spare: &[Reg]) {
        let mut counts: Vec<(Word, u32)> = Vec::new();
        let mut note = |w: Word| match counts.iter_mut().find(|(x, _)| *x == w) {
            Some((_, n)) => *n += 1,
            None => counts.push((w, 1)),
        };
        for it in self.items {
            if let Item::Step(op) = it {
                let (v, n) = reads(it);
                for &x in &v[..n] {
                    match x & !LANE {
                        _ if !is_lane(x) => note(Word::Scalar(x)),
                        L_X | L_I => note(Word::Lane(x & !LANE)),
                        _ => {}
                    }
                }
                match op.code {
                    code::SIN | code::COS => {
                        note(Word::Sin);
                        note(Word::Sin);
                    }
                    code::OUT_PAL if op.d & PAL_SCALED == 0 => {
                        note(Word::Pal(op.d & code::PAL_MASK));
                        note(Word::Pal(op.d & code::PAL_MASK));
                    }
                    _ => {}
                }
            }
        }
        // Live-in registers' lanes: read once a pixel each.
        for (r, id) in self.livein.iter().enumerate() {
            if id.is_some() {
                note(Word::Lane(L_REG0 + r as u8));
            }
        }
        counts.sort_by_key(|&(_, n)| core::cmp::Reverse(n));
        for (&(w, _), &r) in counts.iter().zip(spare.iter()) {
            self.hoist.push((w, r));
        }
    }

    /// The `ctx` offset of `w` (its word added the first time).
    fn word(&mut self, w: Word) -> i32 {
        let k = match self.words.iter().position(|&x| x == w) {
            Some(k) => k,
            None => {
                self.words.push(w);
                self.words.len() - 1
            }
        };
        (k * 4) as i32
    }

    /// `n` words in a row, each new: the offset of the first.
    fn words_run(&mut self, ws: &[Word]) -> i32 {
        let k = self.words.len();
        self.words.extend_from_slice(ws);
        (k * 4) as i32
    }

    fn hoisted(&self, w: Word) -> Option<Reg> {
        self.hoist.iter().find(|(x, _)| *x == w).map(|&(_, r)| r)
    }

    fn tmp(&mut self) -> Result<Reg, Refusal> {
        self.free.pop().ok_or(TEMPS_OUT)
    }

    fn release(&mut self, r: Reg) {
        if self.temps.contains(&r) && !self.free.contains(&r) {
            self.free.push(r);
        }
    }

    fn reset_temps(&mut self) {
        self.free = self.temps.iter().rev().copied().collect();
    }

    /// `w` (a word) into `r`.
    fn load_word(&mut self, r: Reg, w: Word) {
        match self.hoisted(w) {
            Some(h) => self.c.mov(r, h),
            None => {
                let off = self.word(w);
                self.c.l32i(r, CTX, off);
            }
        }
    }

    /// A register holding `w` (its hoist, or `r` loaded).
    fn word_in(&mut self, r: Reg, w: Word) -> Reg {
        match self.hoisted(w) {
            Some(h) => h,
            None => {
                let off = self.word(w);
                self.c.l32i(r, CTX, off);
                r
            }
        }
    }

    /// The pixel's value of lane `l` (`L_X`, `L_I`, a register's) into `r`.
    fn lane_into(&mut self, r: Reg, l: u8) {
        let w = Word::Lane(l);
        let p = self.word_in(r, w);
        self.c.addx4(r, PIX, p);
        self.c.l32i(r, r, 0);
    }

    /// A register holding operand `v` for this pixel.
    fn val(&mut self, v: u8) -> Result<Reg, Refusal> {
        if !is_lane(v) {
            let w = Word::Scalar(v);
            if let Some(h) = self.hoisted(w) {
                return Ok(h);
            }
            let t = self.tmp()?;
            let off = self.word(w);
            self.c.l32i(t, CTX, off);
            return Ok(t);
        }
        match v & !LANE {
            L_PX => {
                // px = the pixel << 16 (exactly the VM's column: N = W, base 0).
                let t = self.tmp()?;
                self.c.slli(t, PIX, 16);
                Ok(t)
            }
            L_X | L_I => {
                let t = self.tmp()?;
                self.lane_into(t, v & !LANE);
                Ok(t)
            }
            l => {
                let id = self.cur[(l - L_REG0) as usize].ok_or("a register read before it is written")?;
                let h = self.vals[id].gpr;
                if !self.valid[id] {
                    // A value from before the kernel (the column or still
                    // run, or the still cache): its lane, at this pixel.
                    self.lane_into(h, l);
                    self.valid[id] = true;
                }
                Ok(h)
            }
        }
    }

    /// Item `k`'s result: its value and register.
    fn dst(&self, k: usize) -> Result<(usize, Reg), Refusal> {
        let id = self.wv[k].ok_or("a step without a register")?;
        Ok((id, self.vals[id].gpr))
    }

    /// Item `k`'s result `id` is in its register now.
    fn wrote(&mut self, k: usize, id: usize) {
        self.valid[id] = true;
        if let Item::Step(op) = self.items[k] {
            if let Some(r) = reg_of(op.d) {
                self.cur[r] = Some(id);
            }
        }
    }

    fn sar(&mut self) {
        if !self.sar16 {
            self.c.ssai(16);
            self.sar16 = true;
        }
    }

    /// `d = (1 << 16)`.
    fn one(&mut self, d: Reg) {
        self.c.movi(d, 1);
        self.c.slli(d, d, 16);
    }

    /// `d = i32::MAX`.
    fn max_i32(&mut self, d: Reg) {
        self.c.movi(d, -1);
        self.c.srli(d, d, 1);
    }

    fn emit(&mut self) -> Result<(), Refusal> {
        let uses_src = self.items.iter().any(|it| matches!(it, Item::Step(op) if matches!(op.code, code::MUL_W | code::MUL)));
        self.c.entry(if self.calls { FRAME } else { 32 });
        self.c.mov(OUT, PIX);
        for (w, r) in self.hoist.clone() {
            let off = self.word(w);
            self.c.l32i(r, CTX, off);
        }
        if uses_src {
            self.c.ssai(16);
        }
        self.c.movi(PIX, 0);
        let top = self.c.here();
        // At the loop's top SAR is 16 unless a call in the body changed it.
        self.sar16 = uses_src && !self.calls;
        self.cur = self.livein;
        for v in self.valid.iter_mut() {
            *v = false;
        }
        for k in 0..self.items.len() {
            self.reset_temps();
            let it = self.items[k];
            match it {
                Item::Step(op) if code::is_colour(op.code) => self.colour(k, op)?,
                Item::Step(op) => self.step(k, op)?,
                Item::SelPal { c, a, b, col } => self.sel_pal(k, c, a, b, col)?,
                Item::PickPal { m, pk, col } => self.pick_pal(m, pk, col)?,
            }
        }
        self.c.addi_n(PIX, PIX, 1);
        self.c.addi_n(OUT, OUT, 3);
        let off = top as i32 - self.c.here() as i32 - 4;
        if (-128..128).contains(&off) {
            self.c.bi(Imm::Nei, PIX, 128, top);
        } else {
            let f = self.c.bi_fwd(Imm::Eqi, PIX, 128);
            self.c.j(top);
            self.c.bind(f);
        }
        self.c.retw_n();
        Ok(())
    }

    // --- Steps -----------------------------------------------------------

    fn step(&mut self, k: usize, op: Op) -> Result<(), Refusal> {
        use code::*;
        match op.code {
            DIV => return self.call(k, hid::DIV, &[Arg::Val(op.a), Arg::Val(op.b)], Some(op.d)),
            MOD => return self.call(k, hid::MOD, &[Arg::Val(op.a), Arg::Val(op.b)], Some(op.d)),
            ATAN2 => return self.call(k, hid::ATAN2, &[Arg::Val(op.a), Arg::Val(op.b)], Some(op.d)),
            HASH => return self.call(k, hid::HASH, &[Arg::Val(op.a), Arg::Val(op.b)], Some(op.d)),
            LEN => return self.call(k, hid::LEN, &[Arg::Val(op.a), Arg::Val(op.b)], Some(op.d)),
            BIT => return self.call(k, hid::BIT, &[Arg::Val(op.a), Arg::Val(op.b)], Some(op.d)),
            SQRT => return self.call(k, hid::SQRT, &[Arg::Val(op.a)], Some(op.d)),
            RECIP => return self.call(k, hid::RECIP, &[Arg::Val(op.a)], Some(op.d)),
            SMOOTH => {
                return self.call(k, hid::SMOOTH, &[Arg::Val(op.a), Arg::Val(op.b), Arg::Val(op.c)], Some(op.d))
            }
            MIX => return self.call(k, hid::MIX, &[Arg::Val(op.a), Arg::Val(op.b), Arg::Val(op.c)], Some(op.d)),
            MULR => return self.call(k, hid::MULR, &[Arg::Val(op.a), Arg::Val(op.b), Arg::Val(op.c)], Some(op.d)),
            NOISE => return self.call(k, hid::NOISE, &[Arg::Val(op.a), Arg::Val(op.b), Arg::Val(op.c)], Some(op.d)),
            NOISE_O..=NOISE_O_LAST => {
                let args = [
                    Arg::Val(op.a),
                    Arg::Val(op.b),
                    Arg::Val(op.c),
                    Arg::Word(Word::Sc),
                    Arg::Imm(i32::from(op.code)),
                ];
                return self.call(k, hid::NOISE_O, &args, Some(op.d));
            }
            TAB => {
                let args = [
                    Arg::Val(op.a),
                    Arg::Word(Word::Tabs),
                    Arg::Imm(i32::from(op.b) | i32::from(op.c) << 8),
                ];
                return self.call(k, hid::TAB, &args, Some(op.d));
            }
            PICK => {
                let args = [
                    Arg::Val(op.a),
                    Arg::Word(Word::Sc),
                    Arg::Imm(i32::from(op.b) | i32::from(op.c) << 8),
                ];
                return self.call(k, hid::PICK, &args, Some(op.d));
            }
            SPR => {
                let args = [
                    Arg::Val(op.a),
                    Arg::Val(op.b),
                    Arg::Word(Word::Tabs),
                    Arg::Imm(i32::from(op.c)),
                ];
                return self.call(k, hid::SPR, &args, Some(op.d));
            }
            DIGIT..=DIGIT_B => {
                let args = [
                    Arg::Val(op.a),
                    Arg::Val(op.b),
                    Arg::Val(op.c),
                    Arg::Word(Word::Glyphs(op.code)),
                ];
                return self.call(k, hid::GLYPH, &args, Some(op.d));
            }
            _ => {}
        }
        let (id, d) = self.dst(k)?;
        let n = code::arity(op.code);
        let a = self.val(op.a)?;
        let b = if n >= 2 { self.val(op.b)? } else { 0 };
        let cc = if n >= 3 { self.val(op.c)? } else { 0 };
        if [a, b, cc][..n].contains(&d) {
            return Err("a step reads its own register");
        }
        let c = &mut self.c;
        match op.code {
            ADD_W => c.add_n(d, a, b),
            SUB_W => c.sub(d, a, b),
            MASK => c.and(d, a, b),
            MIN => c.min(d, a, b),
            MAX => c.max(d, a, b),
            MUL_N => {
                c.mull(d, a, b);
                c.srai(d, d, 16);
            }
            MUL_W => {
                let t = self.tmp()?;
                self.c.mull(d, a, b);
                self.c.mulsh(t, a, b);
                self.sar();
                self.c.src(d, t, d);
            }
            ADD | SUB => {
                // As `fixed::add`/`sub`: the wrapped result, unless it
                // overflowed: then the end of `a`'s sign.
                let t = self.tmp()?;
                let c = &mut self.c;
                if op.code == ADD {
                    c.add(d, a, b);
                    c.xor(t, a, b);
                    let f1 = c.bz_fwd(Zero::Ltz, t);
                    c.xor(t, a, d);
                    let f2 = c.bz_fwd(Zero::Gez, t);
                    c.srai(d, a, 31);
                    self.max_i32(t);
                    self.c.xor(d, d, t);
                    self.c.bind(f1);
                    self.c.bind(f2);
                } else {
                    c.sub(d, a, b);
                    c.xor(t, a, b);
                    let f1 = c.bz_fwd(Zero::Gez, t);
                    c.xor(t, a, d);
                    let f2 = c.bz_fwd(Zero::Gez, t);
                    c.srai(d, a, 31);
                    self.max_i32(t);
                    self.c.xor(d, d, t);
                    self.c.bind(f1);
                    self.c.bind(f2);
                }
            }
            MUL => {
                // As `fixed::mul`: the high word says whether the product
                // fits (hi in −32,768..32,767): then (p >> 16), else the end
                // of hi's sign.
                let t = self.tmp()?;
                self.c.mull(d, a, b);
                self.c.mulsh(t, a, b);
                self.release(a);
                self.release(b);
                let u = self.tmp()?;
                self.sar();
                let c = &mut self.c;
                c.srai(u, t, 15);
                c.addi(u, u, 1);
                let fit = c.bui_fwd(false, u, 2);
                c.srai(d, t, 31);
                self.max_i32(u);
                self.c.xor(d, d, u);
                let done = self.c.j_fwd();
                self.c.bind(fit);
                self.c.src(d, t, d);
                self.c.bind(done);
            }
            LT | LE | GT | GE | EQ | NE | STEP => {
                // 1 where the compare holds (a branch over the 0), then << 16.
                let (cond, x, y) = match op.code {
                    LT => (Cond::Lt, a, b),
                    LE => (Cond::Ge, b, a),
                    GT => (Cond::Lt, b, a),
                    GE => (Cond::Ge, a, b),
                    EQ => (Cond::Eq, a, b),
                    NE => (Cond::Ne, a, b),
                    _ => (Cond::Ge, b, a), // step(e, x): x >= e
                };
                c.movi(d, 1);
                let f = c.b_fwd(cond, x, y);
                c.movi(d, 0);
                c.bind(f);
                c.slli(d, d, 16);
            }
            AND => {
                c.movi(d, 0);
                let f1 = c.bz_fwd(Zero::Eqz, a);
                let f2 = c.bz_fwd(Zero::Eqz, b);
                c.movi(d, 1);
                c.bind(f1);
                c.bind(f2);
                c.slli(d, d, 16);
            }
            OR => {
                c.movi(d, 1);
                let f1 = c.bz_fwd(Zero::Nez, a);
                let f2 = c.bz_fwd(Zero::Nez, b);
                c.movi(d, 0);
                c.bind(f1);
                c.bind(f2);
                c.slli(d, d, 16);
            }
            NOT => {
                c.movi(d, 1);
                let f = c.bz_fwd(Zero::Eqz, a);
                c.movi(d, 0);
                c.bind(f);
                c.slli(d, d, 16);
            }
            NEG => {
                // `saturating_neg`: only i32::MIN overflows (and only then
                // are a and -a both negative).
                let t = self.tmp()?;
                let c = &mut self.c;
                c.neg(d, a);
                c.and(t, d, a);
                let f = c.bz_fwd(Zero::Gez, t);
                self.max_i32(d);
                self.c.bind(f);
            }
            ABS => {
                c.abs(d, a);
                let f = c.bz_fwd(Zero::Gez, d);
                self.max_i32(d);
                self.c.bind(f);
            }
            FLOOR => {
                c.srai(d, a, 16);
                c.slli(d, d, 16);
            }
            FRAC => c.extui(d, a, 0, 16),
            SHR => c.srai(d, a, u32::from(op.c & 31)),
            SIN => self.sin(d, a)?,
            COS => {
                // cos(a) = sin(a + 1/4), wrapping (only the turn's fraction is read).
                let t = self.tmp()?;
                self.c.addmi(t, a, 0x4000);
                self.release(a);
                self.sin(d, t)?;
            }
            TRI => {
                let t = self.tmp()?;
                let c = &mut self.c;
                c.extui(d, a, 0, 16);
                c.slli(d, d, 2);
                // The quarter of the turn: p < 0x4000 is 0, < 0xc000 is 1 or 2.
                c.extui(t, a, 14, 2);
                let q0 = c.bz_fwd(Zero::Eqz, t);
                let q3 = c.bi_fwd(Imm::Eqi, t, 3);
                c.movi(t, 2);
                c.slli(t, t, 16);
                c.sub(d, t, d);
                let done = c.j_fwd();
                c.bind(q3);
                c.movi(t, 1);
                c.slli(t, t, 18);
                c.sub(d, d, t);
                c.bind(q0);
                c.bind(done);
            }
            SAW => {
                c.extui(d, a, 0, 16);
                c.slli(d, d, 1);
                c.addmi(d, d, -32768);
                c.addmi(d, d, -32768);
            }
            SQR => {
                c.movi(d, 1);
                c.slli(d, d, 16);
                let f = c.bbci_fwd(a, 15);
                c.neg(d, d);
                c.bind(f);
            }
            SELECT => {
                c.mov(d, cc);
                c.movnez(d, b, a);
            }
            CLAMP => {
                c.max(d, a, b);
                c.min(d, d, cc);
            }
            _ => return Err("a step the kernel has no template for"),
        }
        self.wrote(k, id);
        Ok(())
    }

    /// `d = sin(a)` as `fixed::sin`: the table at the turn's top byte,
    /// interpolated by the next.
    fn sin(&mut self, d: Reg, a: Reg) -> Result<(), Refusal> {
        let i = self.tmp()?;
        self.c.extui(d, a, 0, 8); // f
        self.c.extui(i, a, 8, 8); // i
        self.release(a);
        let s = self.tmp()?;
        let s = self.word_in(s, Word::Sin);
        let p = self.tmp()?;
        let c = &mut self.c;
        c.addx2(p, i, s);
        c.l16si(p, p, 0); // s0
        c.addi(i, i, 1);
        c.extui(i, i, 0, 8);
        c.addx2(i, i, s);
        c.l16si(i, i, 0); // s1
        c.sub(i, i, p);
        c.mull(i, i, d);
        c.srai(i, i, 8);
        c.add_n(d, p, i);
        c.slli(d, d, 2);
        Ok(())
    }

    // --- Helper calls --------------------------------------------------------

    fn call(&mut self, k: usize, h: u8, args: &[Arg], dst: Option<u8>) -> Result<(), Refusal> {
        if args.len() > 6 {
            return Err("a helper with too many arguments");
        }
        let dv = match dst {
            Some(_) => Some(self.dst(k)?.0),
            None => None,
        };
        // Save what a8..a12 hold that is still wanted (read here or later):
        // the values alive at this item, in their registers.
        let mut saved: Vec<usize> = Vec::new();
        for id in 0..self.vals.len() {
            let v = self.vals[id];
            if v.start > k || v.end < k || Some(id) == dv || v.gpr < 8 || !self.valid[id] {
                continue;
            }
            self.c.s32i(v.gpr, 1, slot(v.gpr));
            saved.push(id);
        }
        for (j, &a) in args.iter().enumerate() {
            let t = 10 + j as Reg;
            match a {
                Arg::Imm(v) => self.c.movi(t, v),
                Arg::Word(w) => self.load_word(t, w),
                Arg::AddrOf(w) => {
                    let off = self.word(w);
                    self.c.movi(t, off);
                    self.c.add(t, t, CTX);
                }
                Arg::Out => self.c.mov(t, OUT),
                Arg::Val(v) if !is_lane(v) => self.load_word(t, Word::Scalar(v)),
                Arg::Val(v) => match v & !LANE {
                    L_PX => self.c.slli(t, PIX, 16),
                    L_X | L_I => self.lane_into(t, v & !LANE),
                    l => {
                        let id = self.cur[(l - L_REG0) as usize].ok_or("a register read before it is written")?;
                        let g = self.vals[id].gpr;
                        if !self.valid[id] {
                            self.lane_into(t, l);
                        } else if g < 8 {
                            self.c.mov(t, g);
                        } else {
                            // (Saved above: a10.. may hold an earlier argument.)
                            self.c.l32i(t, 1, slot(g));
                        }
                    }
                },
            }
        }
        self.load_word(CALLEE, Word::Helper(h));
        self.c.callx8(CALLEE);
        self.sar16 = false;
        if let Some(id) = dv {
            let g = self.vals[id].gpr;
            self.c.mov(g, 10);
            self.wrote(k, id);
        }
        for id in saved {
            let v = self.vals[id];
            if v.end > k {
                self.c.l32i(v.gpr, 1, slot(v.gpr));
            } else {
                self.valid[id] = false;
            }
        }
        // Whatever else a8..a15 held is gone.
        for id in 0..self.vals.len() {
            let v = self.vals[id];
            if v.gpr >= 8 && Some(id) != dv && !(v.start <= k && v.end > k) {
                self.valid[id] = false;
            }
        }
        Ok(())
    }

    // --- Colours ---------------------------------------------------------------

    /// Store a packed colour (`r | g << 8 | b << 16`) at the pixel.
    fn store_packed(&mut self, x: Reg) -> Result<(), Refusal> {
        let y = self.tmp()?;
        let c = &mut self.c;
        c.s8i(x, OUT, 0);
        c.srli(y, x, 8);
        c.s8i(y, OUT, 1);
        c.extui(y, x, 16, 8);
        c.s8i(y, OUT, 2);
        Ok(())
    }

    /// `fixed::chan(v)` into a temporary.
    fn chan(&mut self, v: Reg) -> Result<Reg, Refusal> {
        let t = self.tmp()?;
        let u = self.tmp()?;
        let c = &mut self.c;
        c.movi(t, 0);
        c.max(t, v, t);
        self.release(v);
        self.one(u);
        let c = &mut self.c;
        c.min(t, t, u);
        c.slli(u, t, 8);
        c.sub(t, u, t); // · 255
        c.movi(u, 1);
        c.slli(u, u, 15);
        c.add_n(t, t, u); // + 0x8000
        c.extui(t, t, 16, 8);
        self.release(u);
        Ok(t)
    }

    /// `fixed::pal_index(k)` into a temporary.
    fn pal_index(&mut self, k: Reg) -> Result<Reg, Refusal> {
        let t = self.tmp()?;
        self.c.movi(t, 0);
        self.c.max(t, k, t);
        self.release(k);
        let u = self.tmp()?;
        let c = &mut self.c;
        c.srai(t, t, 11);
        c.movi(u, 31);
        c.minu(t, t, u);
        self.release(u);
        Ok(t)
    }

    fn colour(&mut self, k: usize, op: Op) -> Result<(), Refusal> {
        use code::*;
        match op.code {
            OUT_GRAY => {
                let v = self.val(op.a)?;
                let g = self.chan(v)?;
                for o in 0..3 {
                    self.c.s8i(g, OUT, o);
                }
            }
            OUT_RGB => {
                for (o, x) in [op.a, op.b, op.c].into_iter().enumerate() {
                    let v = self.val(x)?;
                    let g = self.chan(v)?;
                    self.c.s8i(g, OUT, o as i32);
                    self.release(g);
                }
            }
            OUT_HSV => {
                self.call(k, hid::HSV, &[Arg::Val(op.a), Arg::Val(op.b), Arg::Val(op.c), Arg::Out], None)?;
            }
            OUT_PAL if op.d & PAL_SCALED == 0 => {
                let v = self.val(op.a)?;
                let i = self.pal_index(v)?;
                let p = self.tmp()?;
                let p = self.word_in(p, Word::Pal(op.d & code::PAL_MASK));
                let e = self.tmp()?;
                let c = &mut self.c;
                c.addx2(e, i, i);
                c.add(e, e, p);
                for o in 0..3 {
                    c.l8ui(i, e, o);
                    c.s8i(i, OUT, o);
                }
            }
            OUT_PAL => self.pal_lit(op)?,
            _ => return Err("a colour the kernel has no template for"),
        }
        Ok(())
    }

    /// `pal(P, k) * s` with the last index and scale's colour kept in `ctx`
    /// (the VM's `pal_lit_lane`): a new one from `scale_rgb` (a call).
    fn pal_lit(&mut self, op: Op) -> Result<(), Refusal> {
        let shift = u32::from((op.d & !PAL_SCALED) >> PAL_SHIFT);
        let memo = self.words_run(&[Word::Init(-1), Word::Init(0), Word::Init(0)]);
        let pal = Word::Pal(op.d & code::PAL_MASK);
        let v = self.val(op.a)?;
        let i = self.pal_index(v)?;
        let sv = self.val(op.c)?;
        let s = self.tmp()?;
        self.c.srai(s, sv, shift);
        self.release(sv);
        let x = self.tmp()?;
        let c = &mut self.c;
        c.l32i(x, CTX, memo);
        let m1 = c.b_fwd(Cond::Ne, i, x);
        c.l32i(x, CTX, memo + 4);
        let m2 = c.b_fwd(Cond::Ne, s, x);
        c.l32i(x, CTX, memo + 8);
        let hit = c.j_fwd();
        c.bind(m1);
        c.bind(m2);
        c.s32i(i, CTX, memo);
        c.s32i(s, CTX, memo + 4);
        // (Nothing a8..a12 hold is wanted after the colour: it is the last.)
        c.addx2(10, i, i);
        c.mov(11, s);
        let p = self.word_in(12, pal);
        self.c.add(10, 10, p);
        self.load_word(CALLEE, Word::Helper(hid::SCALE));
        self.c.callx8(CALLEE);
        self.sar16 = false;
        self.c.s32i(10, CTX, memo + 8);
        self.c.mov(x, 10);
        self.c.bind(hit);
        self.release(i);
        self.release(s);
        self.store_packed(x)
    }

    /// `pal(P, select(c, A, B)) [* k]`.
    fn sel_pal(&mut self, _k: usize, cv: u8, a: u8, b: u8, col: Op) -> Result<(), Refusal> {
        let pal = col.d & code::PAL_MASK;
        let shift = (col.d & !PAL_SCALED) >> PAL_SHIFT;
        let scaled = col.d & PAL_SCALED != 0;
        if !scaled || !is_lane(col.c) {
            // Two colours a row (the VM's: two a lane).
            let k = if scaled { col.c } else { NONE };
            let ca = self.word(Word::Colour { pal, v: a, k, shift });
            let cb = self.word(Word::Colour { pal, v: b, k, shift });
            let c = self.val(cv)?;
            let x = self.tmp()?;
            let y = self.tmp()?;
            self.c.l32i(x, CTX, cb);
            self.c.l32i(y, CTX, ca);
            self.c.movnez(x, y, c);
            self.release(y);
            self.release(c);
            return self.store_packed(x);
        }
        // k a lane: each choice's last scale and colour kept (the VM's
        // `pal_select_lanes`). Slot w (0: c ≠ 0, A; 1: c = 0, B):
        // [last scale, colour, entry, -].
        let base = self.words_run(&[
            Word::Init(i32::MIN),
            Word::Init(0),
            Word::Entry { pal, v: a },
            Word::Init(0),
            Word::Init(i32::MIN),
            Word::Init(0),
            Word::Entry { pal, v: b },
            Word::Init(0),
        ]);
        let c = self.val(cv)?;
        let o = self.tmp()?;
        let z = self.tmp()?;
        self.c.movi(o, 0);
        self.c.movi(z, 16);
        self.c.moveqz(o, z, c);
        self.release(z);
        self.release(c);
        self.c.add(o, o, CTX);
        let kv = self.val(col.c)?;
        let s = self.tmp()?;
        self.c.srai(s, kv, u32::from(shift));
        self.release(kv);
        let x = self.tmp()?;
        let cc = &mut self.c;
        cc.l32i(x, o, base);
        let miss = cc.b_fwd(Cond::Ne, s, x);
        cc.l32i(x, o, base + 4);
        let hit = cc.j_fwd();
        cc.bind(miss);
        cc.s32i(s, o, base);
        cc.s32i(o, 1, SLOT_SPARE);
        cc.l32i(10, o, base + 8);
        cc.mov(11, s);
        self.load_word(CALLEE, Word::Helper(hid::SCALE));
        self.c.callx8(CALLEE);
        self.sar16 = false;
        self.c.l32i(o, 1, SLOT_SPARE);
        self.c.s32i(10, o, base + 4);
        self.c.mov(x, 10);
        self.c.bind(hit);
        self.release(o);
        self.release(s);
        self.store_packed(x)
    }

    /// `pal(P, pick(m, …)) [* k]`: the choices' colours a row.
    fn pick_pal(&mut self, m: u8, pk: Op, col: Op) -> Result<(), Refusal> {
        let pal = col.d & code::PAL_MASK;
        let shift = (col.d & !PAL_SCALED) >> PAL_SHIFT;
        let k = if col.d & PAL_SCALED != 0 { col.c } else { NONE };
        let n = (pk.c as usize).min(PICK_MAX);
        let ws: Vec<Word> = (0..=PICK_MAX as u8)
            .map(|i| Word::Pick { pal, b: pk.b, n: pk.c, i, k, shift })
            .collect();
        let tbl = self.words_run(&ws);
        let mv = self.val(m)?;
        let i = self.tmp()?;
        self.c.srai(i, mv, 16);
        self.release(mv);
        let u = self.tmp()?;
        self.c.movi(u, n as i32);
        let ok = self.c.b_fwd(Cond::Ltu, i, u);
        self.c.movi(i, PICK_MAX as i32);
        self.c.bind(ok);
        self.release(u);
        let x = self.tmp()?;
        self.c.addx4(x, i, CTX);
        self.c.l32i(x, x, tbl);
        self.release(i);
        self.store_packed(x)
    }
}

/// The frame slot of a8..a12's home `h`.
fn slot(h: Reg) -> i32 {
    (i32::from(h) - 8) * 4
}

/// Whether an item calls a helper.
fn needs_call(_prog: &Program, it: &Item) -> bool {
    use code::*;
    match *it {
        Item::Step(op) => matches!(
            op.code,
            DIV | MOD | ATAN2 | HASH | LEN | BIT | SQRT | RECIP | PICK | TAB | SPR | MIX | SMOOTH
                | NOISE | MULR | OUT_HSV
        ) || (DIGIT..=DIGIT_B).contains(&op.code)
            || (NOISE_O..=NOISE_O_LAST).contains(&op.code)
            || (op.code == OUT_PAL && op.d & PAL_SCALED != 0),
        Item::SelPal { col, .. } => col.d & PAL_SCALED != 0 && is_lane(col.c),
        Item::PickPal { .. } => false,
    }
}
