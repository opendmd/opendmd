//! Refresh-synchronised blits: when a frame can go into the panel's one DMA
//! buffer without the scan showing half of it.
//!
//! The panel is single-buffered (`config::DOUBLE_BUFFER` in the firmware):
//! the I2S DMA reads the buffer the blit writes, row pair after row pair
//! (rows `y` and `y + 16` share every DMA word), top to bottom, about every
//! 10 ms. A blit is much faster than a scan (a full frame ~1.2-1.8 ms), so
//! it overtakes the scan at most once: if it does, the pairs above the
//! crossing show the new frame one refresh later than the pairs below — a
//! seam for one refresh, which drifts up or down the panel when the GIF's
//! frame rate and the refresh beat (a scrolling marquee shows it as a wave).
//!
//! The blit writes the dirty row pairs in ascending order. Each pair `k`
//! is written at some time inside `[lo, hi]` after the blit starts (the
//! earliest it can start and the latest it can end, from cost bounds per
//! pixel). It is clean when no pair is written while the scan is inside
//! it and every pair is first shown, complete, by the same refresh.
//! [`clear_at`] checks that for a start at a scan position; [`wait_us`]
//! finds how long to wait for one; [`Judge`] checks a blit afterwards
//! from scan positions read around each pair's write (the probe's count).
//!
//! Positions are microseconds of scan time into the refresh (`0..PAIRS *
//! row_us`), where `row_us` is one row pair's share of a refresh.

/// Row pairs in a refresh (rows `y` and `y + HEIGHT / 2` go out together).
pub const PAIRS: usize = crate::canvas::HEIGHT / 2;

/// One dirty row pair in a blit's write order, with the window (µs from
/// the blit's start) in which its write happens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Write {
    pub pair: u8,
    /// The earliest the write can start.
    pub lo_us: u32,
    /// The latest it can end.
    pub hi_us: u32,
}

/// A blit's writes, in order: at most one per row pair.
#[derive(Clone, Copy, Debug, Default)]
pub struct Plan {
    writes: [Write; PAIRS],
    len: usize,
    /// The earliest the next write can start.
    lo_next: u32,
}

impl Plan {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append pair `pair`, costing between `min_us` and `max_us` to write.
    /// Pairs must come in ascending order (the blit's).
    pub fn push(&mut self, pair: usize, min_us: u32, max_us: u32) {
        debug_assert!(pair < PAIRS && self.len < PAIRS);
        debug_assert!(self.len == 0 || usize::from(self.writes[self.len - 1].pair) < pair);
        let hi_prev = self.span_us();
        self.writes[self.len] = Write {
            pair: pair as u8,
            lo_us: self.lo_next,
            hi_us: hi_prev + max_us,
        };
        self.lo_next += min_us;
        self.len += 1;
    }

    pub fn writes(&self) -> &[Write] {
        &self.writes[..self.len]
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The latest the whole blit can end, µs from its start.
    pub fn span_us(&self) -> u32 {
        self.writes().last().map_or(0, |w| w.hi_us)
    }
}

/// Where a write of pair `k` between unwrapped scan positions `a` and
/// `b` (µs, `a <= b`) lands: `Some(n)` if the scan stays at least `margin`
/// away from the pair meanwhile and refresh `n` (counted from position 0
/// of the first cycle) is the first to show it complete; `None` if the
/// scan is in the pair, or within `margin` of it, while it is written.
fn place(k: u32, a: u32, b: u32, row_us: u32, margin: u32) -> Option<u32> {
    let cycle = row_us * PAIRS as u32;
    let k_at = k * row_us;
    // The first time the scan enters the pair, `margin` after the write.
    let n = (b + margin).saturating_sub(k_at).div_ceil(cycle);
    // The time before that ended `margin` before the write began.
    (n == 0 || (n - 1) * cycle + k_at + row_us + margin <= a).then_some(n)
}

/// A blit of `plan` started at scan position `pos_us` (`0..PAIRS *
/// row_us`) shows no seam: no pair is written while the scan is within
/// `margin_us` of it, and every pair is first shown complete by the same
/// refresh.
#[inline(never)]
pub fn clear_at(pos_us: u32, plan: &Plan, row_us: u32, margin_us: u32) -> bool {
    let mut refresh = None;
    for w in plan.writes() {
        match place(
            u32::from(w.pair),
            pos_us + w.lo_us,
            pos_us + w.hi_us,
            row_us,
            margin_us,
        ) {
            Some(n) if refresh.is_none_or(|r| r == n) => refresh = Some(n),
            _ => return false,
        }
    }
    true
}

/// How long after scan position `pos_us` a blit of `plan` can start
/// without a seam (0: now), tried every `step_us`; `None` if no start
/// within one refresh is clear (a blit too long for any window).
pub fn wait_us(pos_us: u32, plan: &Plan, row_us: u32, margin_us: u32, step_us: u32) -> Option<u32> {
    let cycle = row_us * PAIRS as u32;
    let step = step_us.max(1);
    let mut d = 0;
    while d < cycle {
        if clear_at((pos_us + d) % cycle, plan, row_us, margin_us) {
            return Some(d);
        }
        d += step;
    }
    None
}

/// A blit checked afterwards: the scan position read before and after each
/// pair's write. Reads must come less than half a refresh apart (they do:
/// a pair takes ~0.1 ms of a 10 ms refresh) so the wraps can be counted.
#[derive(Clone, Copy, Debug)]
pub struct Judge {
    row_us: u32,
    last: u32,
    wraps: u32,
    refresh: Option<u32>,
    /// Two pairs were first shown complete in different refreshes (or one
    /// was written under the scan): a seam.
    pub straddled: bool,
    /// A pair was written while the scan was inside it (its bit planes
    /// mixed for one refresh).
    pub in_pair: bool,
}

impl Judge {
    pub fn new(row_us: u32) -> Self {
        Judge {
            row_us,
            last: 0,
            wraps: 0,
            refresh: None,
            straddled: false,
            in_pair: false,
        }
    }

    /// A raw scan position, unwrapped against the previous one.
    fn unwrap(&mut self, pos: u32) -> u32 {
        let cycle = self.row_us * PAIRS as u32;
        if pos + cycle / 2 < self.last {
            self.wraps += 1;
        }
        self.last = pos;
        self.wraps * cycle + pos
    }

    /// Pair `pair` was written between scan positions `before` and `after`.
    #[inline(never)]
    pub fn pair(&mut self, pair: usize, before: u32, after: u32) {
        let (a, b) = (self.unwrap(before), self.unwrap(after));
        match place(pair as u32, a, b, self.row_us, 0) {
            Some(n) if self.refresh.is_none_or(|r| r == n) => self.refresh = Some(n),
            Some(_) => self.straddled = true,
            None => {
                self.in_pair = true;
                self.straddled = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: u32 = 624;
    const CYCLE: u32 = ROW * PAIRS as u32;

    fn full(min: u32, max: u32) -> Plan {
        let mut p = Plan::new();
        for k in 0..PAIRS {
            p.push(k, min, max);
        }
        p
    }

    #[test]
    fn plan_windows_accumulate() {
        let mut p = Plan::new();
        p.push(2, 10, 30);
        p.push(5, 10, 30);
        p.push(9, 10, 30);
        let w = p.writes();
        assert_eq!((w[0].lo_us, w[0].hi_us), (0, 30));
        assert_eq!((w[1].lo_us, w[1].hi_us), (10, 60));
        assert_eq!((w[2].lo_us, w[2].hi_us), (20, 90));
        assert_eq!(p.span_us(), 90);
    }

    #[test]
    fn a_full_frame_is_clear_only_late_in_the_last_pair_and_just_before() {
        let plan = full(60, 120);
        // In the middle of the refresh: a seam.
        assert!(!clear_at(CYCLE / 2, &plan, ROW, 30));
        // Pair 0 is being scanned: no.
        assert!(!clear_at(10, &plan, ROW, 30));
        // Late in pair 15: pairs 0..14 go ahead of the next scan, 15
        // behind this one's.
        let clear: Vec<u32> = (0..CYCLE)
            .step_by(16)
            .filter(|&p| clear_at(p, &plan, ROW, 30))
            .collect();
        assert!(!clear.is_empty());
        let (first, last) = (clear[0], *clear.last().unwrap());
        assert!(first >= 14 * ROW, "first clear start {first}");
        assert!(last < CYCLE, "last clear start {last}");
        // One window, no gaps.
        assert_eq!(clear.len() as u32, (last - first) / 16 + 1);
    }

    #[test]
    fn a_band_is_clear_anywhere_away_from_it() {
        // Pairs 4..=7 (rows 4-7 and 20-23).
        let mut plan = Plan::new();
        for k in 4..8 {
            plan.push(k, 5, 120);
        }
        // Scan well ahead of the band: all four written before it gets there.
        assert!(clear_at(0, &plan, ROW, 30));
        // Scan well past it: shown next refresh.
        assert!(clear_at(10 * ROW, &plan, ROW, 30));
        // Scan in it: no.
        assert!(!clear_at(5 * ROW, &plan, ROW, 30));
        // Just before it, with the band's writes outrun by the scan: no.
        assert!(!clear_at(4 * ROW - 100, &plan, ROW, 30));
    }

    #[test]
    fn wait_finds_the_next_clear_start() {
        let plan = full(60, 120);
        let d = wait_us(CYCLE / 2, &plan, ROW, 30, 8).expect("a window");
        let at = (CYCLE / 2 + d) % CYCLE;
        assert!(clear_at(at, &plan, ROW, 30));
        assert!(!clear_at((at + CYCLE - 8) % CYCLE, &plan, ROW, 30));
        // Already clear: no wait.
        let mut band = Plan::new();
        band.push(8, 5, 50);
        assert_eq!(wait_us(0, &band, ROW, 30, 8), Some(0));
        // A blit longer than a refresh never fits.
        assert_eq!(wait_us(0, &full(700, 1400), ROW, 30, 8), None);
    }

    #[test]
    fn judge_sees_a_seam_and_a_clean_blit() {
        // The scan at pair 8 when a fast full blit starts: 0..8 behind
        // it (next refresh), 9..15 ahead (this one).
        let mut j = Judge::new(ROW);
        let mut t = 8 * ROW + 300;
        for k in 0..PAIRS {
            j.pair(k, t, t + 70);
            t += 74;
        }
        assert!(j.straddled);
        // Started late in pair 15: every pair first shown by the next.
        let mut j = Judge::new(ROW);
        let mut t = 15 * ROW + 100;
        for k in 0..PAIRS {
            let (a, b) = (t % CYCLE, (t + 70) % CYCLE);
            j.pair(k, a, b);
            t += 74;
        }
        assert!(!j.straddled && !j.in_pair, "{j:?}");
        // A pair written under the scan.
        let mut j = Judge::new(ROW);
        j.pair(3, 3 * ROW + 10, 3 * ROW + 80);
        assert!(j.in_pair && j.straddled);
    }

    #[test]
    fn judge_agrees_with_clear_at_for_exact_costs() {
        // With min == max cost the plan is exact: judging the blit it
        // describes must give the same answer at every start.
        let plan = full(74, 74);
        for start in (0..CYCLE).step_by(13) {
            let mut j = Judge::new(ROW);
            for w in plan.writes() {
                j.pair(
                    usize::from(w.pair),
                    (start + w.lo_us) % CYCLE,
                    (start + w.hi_us) % CYCLE,
                );
            }
            assert_eq!(
                !j.straddled,
                clear_at(start, &plan, ROW, 0),
                "start {start}"
            );
        }
    }
}
