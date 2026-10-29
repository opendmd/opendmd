//! The row stage as a table (console-lessons Phase A, 2.3 and 2.4).
//!
//! The row section (`ops[draw_end..row_end]`, scalar steps once a row:
//! anything that depends on `py`/`y` but not on the pixel) used to run
//! between each row's lanes, 32 times a frame, through the scalar
//! evaluator. Here it runs once a frame, before the first row's pixels and
//! after the draw stage (where the slots stand as the row steps have always
//! found them), as lanes of the 32 rows: each row step once over a lane
//! whose element `j` is row `j` from the top, through the draw lanes' own
//! machinery (`vm::lane_draw`, psi-speed2/3: the frame loop's IRAM loops).
//! The values the pixel steps read are kept in a table, one 32-entry
//! column each, and each row copies its own into their slots before its
//! lanes: the same slot values the row steps would have left, so the same
//! bytes.
//!
//! Where it lives: the lane registers (`Scratch::lanes`), which are free
//! before the rows. A lane of `N` holds `N / 32` columns; the table takes
//! the top registers the pixel steps don't write, and the row steps work
//! in the ones below, by liveness. A program the table doesn't fit (too
//! many values for the registers, a row step that reads a value of the
//! row before, a `pick` among row values) runs a row at a time as before:
//! `table` says `None` and changes nothing.
//!
//! The row noise (`noise` and the wide `noise` of `loop forever`) runs
//! row by row with a noise-cell memo: the last cell's eight corner hashes
//! kept while the next row's lattice cell is the same one (Endless road's
//! 19 road rows read within a fraction of a cell of one another), exactly
//! the value `fixed::noise_off` gives.
//!
//! Not in wasm (the web app's previews run the rows one by one, the same
//! bytes, without the lane dispatch in the download).

#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
use crate::program::*;

/// psi-probe's switches (feature `stats`): the row table off (every row
/// one by one, as before), the noise-cell memo off (the hashes every row),
/// for A/B rows in one build. Not in a release build.
#[cfg(feature = "stats")]
pub mod probe {
    use core::sync::atomic::AtomicBool;
    pub static TABLE_OFF: AtomicBool = AtomicBool::new(false);
    pub static MEMO_OFF: AtomicBool = AtomicBool::new(false);
}

/// Row values the table holds at most (those the pixel steps read).
pub(crate) const COLS: usize = 16;

/// The table `table` made: column `c` holds slot `slot[c]`'s value for
/// each row, from element `at[c]` of the lane registers taken as one array
/// (`at[c] + row` is row `row`'s).
pub(crate) struct RowTab {
    n: u8,
    slot: [u8; COLS],
    at: [u16; COLS],
}

impl RowTab {
    /// Column `c`'s place: register `REGS - c / (N / H)`, at
    /// `(c % (N / H)) * H`.
    #[cfg(not(target_arch = "wasm32"))]
    fn place<const N: usize>(c: usize) -> u16 {
        let per = N / H;
        ((REGS - c / per) * N + (c % per) * H) as u16
    }

    /// Row `row`'s values into their slots (once a row, in the IRAM frame
    /// loop: no checks to carry, `table` made the places).
    #[inline(always)]
    pub(crate) fn load<const N: usize>(&self, s: &mut crate::vm::Scratch<N>, row: usize) {
        let lanes = s.lanes.as_ptr().cast::<i32>();
        for c in 0..(self.n as usize).min(COLS) {
            // SAFETY: `table` made `at[c] + H - 1` an element of the lane
            // registers ((REGS + 1) * N of them) and `slot[c]` a slot, and
            // row < H.
            unsafe {
                let v = *lanes.add(*self.at.get_unchecked(c) as usize + row);
                *s.sc.get_unchecked_mut(*self.slot.get_unchecked(c) as usize) = v;
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use lanes::table;

/// How many row values `prog`'s row table keeps (the values its pixel steps
/// read), `None` when its rows run one by one (no row stage, or one the
/// table doesn't take): psi-check, the probe and the tests. Works the
/// table out once in `scratch` (it holds the per-frame values `prog.init`
/// gives, so a row step reading a frame value reads its initial one).
#[doc(hidden)]
#[cfg(not(target_arch = "wasm32"))]
pub fn row_table<const N: usize>(
    prog: &Program,
    scratch: &mut crate::vm::Scratch<N>,
    tables: &crate::tables::Tables,
) -> Option<usize> {
    let ns = (prog.n_scalars as usize).min(MAX_SCALARS);
    scratch.sc[..ns].copy_from_slice(&prog.init[..ns]);
    table(prog, scratch, tables).map(|t| t.n as usize)
}

#[cfg(not(target_arch = "wasm32"))]
mod lanes {
    use super::{RowTab, COLS};
    use crate::fixed;
    use crate::program::*;
    use crate::tables::Tables;
    use crate::vm::{lane_draw, scalar_value, spr, Scratch};

    /// Row steps the table takes at most (more: a row at a time).
    pub(super) const MAX_STEPS: usize = 128;

    /// A register operand (`lanes[r]`, r ≥ 1), as the draw lanes write it.
    #[inline(always)]
    fn lane(r: u8) -> u8 {
        LANE | (L_I + r)
    }

    #[inline(always)]
    fn m(v: u8) -> usize {
        (v & 0x7f) as usize
    }

    /// The register a pixel step writes (as `vm::reg_index`).
    #[inline(always)]
    fn reg_of(v: u8) -> usize {
        ((v & !LANE).saturating_sub(L_I) as usize).clamp(1, REGS)
    }

    /// The noise context `c - NOISE_O`'s first offset slot.
    #[inline(always)]
    fn ctx_slot(c: u8) -> usize {
        noise_slot(c.wrapping_sub(code::NOISE_O) as usize % NOISE_CTX, 0)
    }

    /// Run `prog`'s row section for all rows into the table (the slots as
    /// the per-frame and draw steps left them), or `None` without touching
    /// anything when the program is not one the table takes.
    #[inline(never)]
    pub(crate) fn table<const N: usize>(
        prog: &Program,
        s: &mut Scratch<N>,
        t: &Tables,
    ) -> Option<RowTab> {
        let (de, re, pe) = (
            prog.draw_end as usize,
            prog.row_end as usize,
            (prog.pixel_end as usize).min(MAX_OPS),
        );
        if N < H || de >= re || re > pe {
            return None;
        }
        let rows = &prog.ops[de..re];
        let len = rows.len();
        if len > MAX_STEPS {
            return None;
        }
        // The slots the row steps write.
        let mut written = 0u128;
        for op in rows {
            // (A pick reads its choices through a table of slots, and the
            // VM's scalar steps never run one; a wide step is once a frame.)
            if !code::writes_scalar(op.code) || code::is_wide(op.code) || op.code == code::PICK {
                return None;
            }
            written |= 1 << m(op.d);
        }
        // The slots the pixel steps read (every field of theirs that is not
        // a lane, a few too many is only a column more; a pick's choices; a
        // wide noise's offsets), and the highest register they write.
        let mut read = 0u128;
        let mut top = 0usize;
        for op in &prog.ops[re..pe] {
            for v in [op.a, op.b, op.c, op.d] {
                if v & LANE == 0 {
                    read |= 1 << m(v);
                }
            }
            match op.code {
                code::PICK => {
                    for j in 0..op.c as usize {
                        let k = (op.b as usize + j) & (MAX_SCALARS - 1);
                        read |= 1 << k;
                        read |= 1 << (prog.init[k] as usize & (MAX_SCALARS - 1));
                    }
                }
                code::NOISE_O..=code::NOISE_O_LAST => read |= 7 << ctx_slot(op.code),
                _ => {}
            }
            if code::writes_scalar(op.code) && op.d & LANE != 0 {
                top = top.max(reg_of(op.d));
            }
            if op.code == code::STILL {
                top = top.max(reg_of(op.b)).max(reg_of(op.c));
            }
        }
        let need = read & written;
        let n = need.count_ones() as usize;
        let per = N / H;
        if n > COLS {
            return None;
        }
        // The table's registers at the top, the row steps' below.
        let rmax = REGS - n.div_ceil(per);
        if top > rmax {
            return None;
        }

        // Pass 1: each step's value is a version (its index + 1; py and y
        // two more); where each is last read; each read is of this row's
        // value or of one the row steps never write.
        let (pyv, yv) = (len + 1, len + 2);
        // (One slot map for the three passes, in turn: each slot's version,
        // then again as the registers are given out, then each slot's
        // register: the stack is the main task's, under the frame loop.)
        let mut ver = [0u8; MAX_SCALARS];
        ver[S_PY as usize] = pyv as u8;
        ver[S_Y as usize] = yv as u8;
        let mut last = [0u8; MAX_STEPS + 3];
        for (i, op) in rows.iter().enumerate() {
            let ar = code::arity(op.code);
            for &v in &[op.a, op.b, op.c][..ar] {
                if v & LANE != 0 {
                    return None;
                }
                let k = m(v);
                if ver[k] == 0 && written & (1 << k) != 0 {
                    // A value of the row before (never from the compiler).
                    return None;
                }
                if ver[k] != 0 {
                    last[ver[k] as usize] = (i + 1) as u8;
                }
            }
            if (code::NOISE_O..=code::NOISE_O_LAST).contains(&op.code)
                && written & (7 << ctx_slot(op.code)) != 0
            {
                return None;
            }
            ver[m(op.d)] = (i + 1) as u8;
        }
        // The columns: the last value of each slot the pixel steps read.
        let mut tab = RowTab {
            n: 0,
            slot: [0; COLS],
            at: [0; COLS],
        };
        let mut col = [u8::MAX; MAX_STEPS + 1];
        for k in 0..MAX_SCALARS {
            if need & (1 << k) != 0 {
                col[ver[k] as usize] = tab.n;
                tab.slot[tab.n as usize] = k as u8;
                tab.at[tab.n as usize] = RowTab::place::<N>(tab.n as usize);
                tab.n += 1;
            }
        }

        // Pass 2: registers 1..=rmax by liveness (a value's register is
        // free once its last reader has run; the destination is never one
        // of the step's operands).
        let mut free: u16 = ((1u32 << (rmax + 1)) - 2) as u16;
        let mut reg = [0u8; MAX_STEPS + 3];
        for v in [pyv, yv] {
            if last[v] != 0 {
                reg[v] = alloc(&mut free)?;
            }
        }
        let at = &mut ver;
        at.fill(0);
        at[S_PY as usize] = pyv as u8;
        at[S_Y as usize] = yv as u8;
        for (i, op) in rows.iter().enumerate() {
            let r = alloc(&mut free)?;
            reg[i + 1] = r;
            for &v in &[op.a, op.b, op.c][..code::arity(op.code)] {
                let w = at[m(v)] as usize;
                if w != 0 && last[w] as usize == i + 1 {
                    free |= 1 << reg[w];
                }
            }
            if last[i + 1] == 0 {
                // Read by no later row step (copied to its column, if it
                // has one, as soon as it is worked out).
                free |= 1 << r;
            }
            at[m(op.d)] = (i + 1) as u8;
        }

        // Run: py and y down the rows, then each step over the rows.
        let lane_at = at;
        lane_at.fill(0);
        let (rpy, ry) = (reg[pyv], reg[yv]);
        for j in 0..H {
            let py = ((H - 1 - j) as i32) << 16;
            if rpy != 0 {
                s.lanes[rpy as usize][j] = py;
            }
            if ry != 0 {
                s.lanes[ry as usize][j] = fixed::div(py, ((H - 1) as i32) << 16);
            }
        }
        lane_at[S_PY as usize] = rpy;
        lane_at[S_Y as usize] = ry;
        for (i, op) in rows.iter().enumerate() {
            let ar = code::arity(op.code);
            let d = reg[i + 1];
            let mut o = *op;
            let mut any = false;
            for f in [&mut o.a, &mut o.b, &mut o.c].into_iter().take(ar) {
                let r = lane_at[m(*f)];
                if r != 0 {
                    *f = lane(r);
                    any = true;
                }
            }
            if !any {
                // Of values the same every row: once.
                let v = scalar_value(&s.sc, op, t, &prog.tables);
                s.lanes[d as usize][..H].fill(v);
            } else if op.code == code::NOISE
                || (code::NOISE_O..=code::NOISE_O_LAST).contains(&op.code)
            {
                noise_rows(s, &o, d);
            } else if lane_loop(op.code) {
                o.d = lane(d);
                lane_draw(s, &o, t, &prog.tables);
            } else {
                each_row(s, &o, d, t, &prog.tables);
            }
            let c = col[i + 1];
            if c != u8::MAX {
                let (c, d) = (c as usize, d as usize);
                let (lo, hi) = s.lanes.split_at_mut(REGS - c / per);
                let at = (c % per) * H;
                debug_assert_eq!(RowTab::place::<N>(c) as usize, (REGS - c / per) * N + at);
                hi[0][at..at + H].copy_from_slice(&lo[d][..H]);
            }
            lane_at[m(op.d)] = d;
        }
        Some(tab)
    }

    /// Operand `v` of a rewritten row step at row `j`: its register's
    /// element, or its slot.
    #[inline(always)]
    fn opnd<const N: usize>(s: &Scratch<N>, v: u8, j: usize) -> i32 {
        if v & LANE != 0 {
            s.lanes[reg_of(v)][j]
        } else {
            s.sc[m(v)]
        }
    }

    /// A row step without a lane loop, row by row with the scalar steps'
    /// own functions (`vm::scalar_value`'s, the operands read already).
    #[inline(never)]
    fn each_row<const N: usize>(
        s: &mut Scratch<N>,
        op: &Op,
        d: u8,
        t: &Tables,
        tabs: &[i32; MAX_TABLE],
    ) {
        let ar = code::arity(op.code);
        for j in 0..H {
            let x = if ar > 0 { opnd(s, op.a, j) } else { 0 };
            let y = if ar > 1 { opnd(s, op.b, j) } else { 0 };
            let z = if ar > 2 { opnd(s, op.c, j) } else { 0 };
            let v = match op.code {
                code::TAB => tab(tabs, op.b, op.c, x),
                code::SPR => spr(tabs, op.c, x, y),
                c => eval(c, x, y, z, Some(t)),
            };
            s.lanes[d as usize][j] = v;
        }
    }

    /// `noise` (or a wide noise, its context's offsets) row by row, the
    /// last cell's corners kept while the rows stay in it: exactly
    /// `fixed::noise_off` (and `noise` is `noise_off` with no offsets).
    #[inline(never)]
    fn noise_rows<const N: usize>(s: &mut Scratch<N>, op: &Op, d: u8) {
        let off = if op.code == code::NOISE {
            [0; 3]
        } else {
            let k = ctx_slot(op.code);
            [s.sc[k] as u32, s.sc[k + 1] as u32, s.sc[k + 2] as u32]
        };
        let mut key = [0u32; 3];
        let mut corners = [0i32; 8];
        let mut have = false;
        for j in 0..H {
            let (x, y, z) = (opnd(s, op.a, j), opnd(s, op.b, j), opnd(s, op.c, j));
            let k = fixed::noise_key(x, y, z, off);
            #[cfg(feature = "stats")]
            if super::probe::MEMO_OFF.load(core::sync::atomic::Ordering::Relaxed) {
                have = false;
            }
            if !have || k != key {
                corners = fixed::noise_corners(k);
                key = k;
                have = true;
            }
            s.lanes[d as usize][j] = fixed::noise_blend(&corners, x, y, z);
        }
    }

    /// The lowest free register of `free` (bit r: register r), taken.
    fn alloc(free: &mut u16) -> Option<u8> {
        if *free == 0 {
            return None;
        }
        let r = free.trailing_zeros() as u8;
        *free &= *free - 1;
        Some(r)
    }
}
