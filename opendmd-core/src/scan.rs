//! Refresh-synchronised blits: writing a frame into the panel's one DMA
//! buffer without the scan showing half of it.
//!
//! The panel is single-buffered (`config::DOUBLE_BUFFER` in the firmware):
//! the I2S DMA reads the buffer the blit writes, row pair after row pair
//! (rows `y` and `y + 16` share every DMA word), top to bottom, every ~8 ms.
//! A blit is much faster than a scan (a full frame ~1.2 ms), so it overtakes
//! the scan at most once: if it does, the pairs on either side of the
//! crossing first show the new frame a refresh apart — a seam for one
//! refresh, which drifts up or down the panel with the beat between the
//! GIF's frame rate and the refresh (a scrolling marquee shows it as a wave).
//!
//! Each row pair `k` has a slot in every refresh `n`, `[n*C + k*R, n*C +
//! (k+1)*R)` of scan time (`R` a pair's share, `C = 16 R` a refresh). A
//! frame is clean when every dirty pair is written after its slot in some
//! refresh `T - 1` and before its slot in refresh `T`, the same `T` for all:
//! then refresh `T` is the first to show any of it, and shows all of it.
//! [`Pacer`] picks `T` for a blit and says, before each pair, how long to
//! wait for its slot in `T - 1` to pass; [`Judge`] checks a blit afterwards
//! from the scan positions read around each pair's write.
//!
//! Positions are microseconds of scan time into the refresh (`0..C`), read
//! together with a microsecond clock; [`Clock`] turns them into one running
//! scan time, the wraps counted from the clock (the scan runs in real time),
//! so a wait or a sleep longer than half a refresh cannot lose one.

/// Row pairs in a refresh (rows `y` and `y + HEIGHT / 2` go out together).
pub const PAIRS: usize = crate::canvas::HEIGHT / 2;

/// A split frame's bottom pass sleeps until this long (µs) before its last
/// pair may be written — the pairs before it are free by then, and their
/// deadlines are most of a refresh away — then paces the rest.
pub const SPLIT_LEAD_US: u32 = 3_000;

/// One scan reading: the position in the refresh (µs of scan) and the
/// clock (µs, wrapping) when it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    pub pos: u32,
    pub at: u32,
}

/// Scan readings unwrapped onto one running scan time, counted from the
/// start of the refresh the first reading was in.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    row_us: u32,
    first: Reading,
}

impl Clock {
    pub fn new(row_us: u32, first: Reading) -> Self {
        Clock { row_us, first }
    }

    pub fn cycle(&self) -> u32 {
        self.row_us * PAIRS as u32
    }

    /// Running scan time of `r`: its position plus the whole refreshes the
    /// clock says have passed since the first reading.
    pub fn unwrap(&self, r: Reading) -> u32 {
        let c = self.cycle();
        let guess = self.first.pos + r.at.wrapping_sub(self.first.at);
        // The refresh whose `r.pos` is nearest the guess.
        let n = (guess + c / 2).saturating_sub(r.pos) / c;
        n * c + r.pos
    }

    /// The clock (µs, wrapping) at running scan time `u`.
    pub fn at(&self, u: u32) -> u32 {
        self.first.at.wrapping_add(u.wrapping_sub(self.first.pos))
    }

    /// Where pair `k`'s slot in refresh `n` starts.
    fn slot(&self, n: u32, k: u32) -> u32 {
        n * self.cycle() + k * self.row_us
    }
}

/// Where a write of pair `k` between running scan times `a` and `b` lands:
/// `Some(n)` if the scan stays at least `margin` from the pair's slot
/// meanwhile and refresh `n` is the first to show the write complete;
/// `None` if the scan is in the pair, or within `margin` of it.
fn place(clock: &Clock, k: u32, a: u32, b: u32, margin: u32) -> Option<u32> {
    let c = clock.cycle();
    // The first slot starting `margin` after the write ends.
    let n = (b + margin).saturating_sub(k * clock.row_us).div_ceil(c);
    // The slot before it ended `margin` before the write began.
    (n == 0 || clock.slot(n - 1, k) + clock.row_us + margin <= a).then_some(n)
}

/// Paces one blit's writes (dirty pairs in ascending order) so that all of
/// them are first shown by the same refresh `T`.
#[derive(Clone, Copy, Debug)]
pub struct Pacer {
    clock: Clock,
    target: u32,
    /// The longest one pair's write may take, µs.
    write_us: u32,
    margin: u32,
}

impl Pacer {
    /// Plan a blit of `pairs` (bit `k` = pair `k`) starting at reading
    /// `now`, each pair's write taking at most `write_us`: the target
    /// refresh, and how long until the blit's last wait ends (when the scan
    /// leaves the last dirty pair's slot before the target; 0 for the
    /// current refresh). `None` with no dirty pair, or when that is over
    /// `budget_us` (the caller writes at once instead).
    ///
    /// The target is the first refresh, from the current one, for which
    /// the paced writes all make their deadlines: the current one if every
    /// dirty pair is far enough ahead of the scan; usually the next (pairs
    /// the scan has passed at once, the others as it passes them); the one
    /// after when the scan is about to come round to the first pair.
    pub fn plan(
        row_us: u32,
        now: Reading,
        pairs: u32,
        write_us: u32,
        margin: u32,
        budget_us: u32,
    ) -> Option<(Pacer, u32)> {
        if pairs & 0xFFFF == 0 {
            return None;
        }
        let clock = Clock::new(row_us, now);
        let u = now.pos;
        // The paced blit for `target`, simulated: when it ends, if every
        // pair makes its slot in `target`.
        let ends = |target: u32| {
            let mut t = u;
            for k in (0..PAIRS as u32).filter(|k| pairs & (1 << k) != 0) {
                if target > 0 {
                    t = t.max(clock.slot(target - 1, k) + row_us + margin);
                }
                t += write_us;
                if t + margin > clock.slot(target, k) {
                    return None;
                }
            }
            Some(())
        };
        let target = (0..3).find(|&t| ends(t).is_some())?;
        // The waits end when the scan leaves the last pair's slot.
        let last = 31 - (pairs & 0xFFFF).leading_zeros();
        let took = match target {
            0 => 0,
            t => (clock.slot(t - 1, last) + row_us + margin).saturating_sub(u),
        };
        (took <= budget_us).then_some((
            Pacer {
                clock,
                target,
                write_us,
                margin,
            },
            took,
        ))
    }

    /// Before writing pair `k`, with the scan at `now`: how long to wait
    /// (µs; 0: write now). Never asks to wait past the pair's deadline: a
    /// pair that can no longer make it is written at once (and the blit
    /// will have a seam, which [`Judge`] counts).
    pub fn wait(&self, k: usize, now: Reading) -> u32 {
        let u = self.clock.unwrap(now);
        if self.target == 0 {
            return 0;
        }
        let k = k as u32;
        let free_at = self.clock.slot(self.target - 1, k) + self.clock.row_us + self.margin;
        free_at.saturating_sub(u)
    }

    /// The refresh this blit is for, counted from the one it started in.
    pub fn target(&self) -> u32 {
        self.target
    }

    /// The same blit's next refresh: for the bottom half of a frame whose
    /// top half this pacer placed (the panel lights rows `r` and `r + 16`
    /// together, so a frame shown whole changes the time each row is lit
    /// by a jump of most of a refresh at row 16; the bottom a refresh
    /// later makes it rise smoothly down all 32 rows).
    pub fn next(&self) -> Pacer {
        Pacer {
            target: self.target + 1,
            ..*self
        }
    }

    /// The clock (µs, wrapping) when pair `k` may be written: the scan
    /// leaving its slot in the refresh before the target.
    pub fn free_at(&self, k: usize) -> u32 {
        let u = match self.target {
            0 => self.clock.first.pos,
            t => self.clock.slot(t - 1, k as u32) + self.clock.row_us + self.margin,
        };
        self.clock.at(u)
    }

    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// The longest one pair's write may take.
    pub fn write_us(&self) -> u32 {
        self.write_us
    }
}

/// A blit checked afterwards: the scan read before and after each pair's
/// write.
#[derive(Clone, Copy, Debug)]
pub struct Judge {
    clock: Option<Clock>,
    row_us: u32,
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
            clock: None,
            row_us,
            refresh: None,
            straddled: false,
            in_pair: false,
        }
    }

    /// A judge counting refreshes on `clock` (a pacer's), so two passes of
    /// one frame can be compared.
    pub fn with_clock(clock: Clock) -> Self {
        Judge {
            clock: Some(clock),
            row_us: clock.row_us,
            refresh: None,
            straddled: false,
            in_pair: false,
        }
    }

    /// The refresh that first showed every pair judged, counted on the
    /// clock (`None`: nothing judged yet).
    pub fn refresh(&self) -> Option<u32> {
        self.refresh
    }

    /// Pair `pair` was written between readings `before` and `after`.
    #[inline(never)]
    pub fn pair(&mut self, pair: usize, before: Reading, after: Reading) {
        let clock = *self.clock.get_or_insert(Clock::new(self.row_us, before));
        let (a, b) = (
            clock.unwrap(before),
            clock.unwrap(after).max(clock.unwrap(before)),
        );
        match place(&clock, pair as u32, a, b, 0) {
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

    const ROW: u32 = 500;
    const CYCLE: u32 = ROW * PAIRS as u32;

    /// A simulated scan from position `p0` at clock 0, and a blit driven
    /// by a pacer: each pair written in `cost` µs, waits honoured exactly.
    /// Returns whether the judge saw a seam, and the blit's duration.
    fn simulate(p0: u32, pairs: u32, cost: u32, write_us: u32, budget: u32) -> Option<(bool, u32)> {
        let read = |t: u32| Reading {
            pos: (p0 + t) % CYCLE,
            at: t,
        };
        let (pacer, _) = Pacer::plan(ROW, read(0), pairs, write_us, 30, budget)?;
        let mut judge = Judge::new(ROW);
        let mut t = 0;
        for k in (0..PAIRS).filter(|k| pairs & (1 << k) != 0) {
            t += pacer.wait(k, read(t));
            let before = read(t);
            t += cost;
            judge.pair(k, before, read(t));
        }
        Some((judge.straddled, t))
    }

    #[test]
    fn clock_unwraps_by_time_across_long_gaps() {
        let c = Clock::new(ROW, Reading { pos: 7000, at: 100 });
        assert_eq!(c.unwrap(Reading { pos: 7000, at: 100 }), 7000);
        // 12 ms (1.5 refreshes) later.
        assert_eq!(
            c.unwrap(Reading {
                pos: 3000,
                at: 100 + 12_000
            }),
            2 * CYCLE + 3000
        );
        // A little jitter between position and clock is absorbed.
        assert_eq!(
            c.unwrap(Reading {
                pos: 50,
                at: 100 + 1_100
            }),
            CYCLE + 50
        );
        // The clock wrapping.
        let c = Clock::new(
            ROW,
            Reading {
                pos: 10,
                at: u32::MAX - 5,
            },
        );
        assert_eq!(c.unwrap(Reading { pos: 20, at: 4 }), 20);
    }

    #[test]
    fn every_start_is_clean_for_full_and_partial_frames() {
        // Full frames (74 µs a pair), scroller-like partial rows (fast or
        // slow writes), a band, single pairs: from every scan position.
        let cases: [(u32, u32); 5] = [
            (0xFFFF, 74),
            (0xFFFF, 20),
            (0xFFFF, 140),
            (0x0F00, 60),
            (0x8001, 60),
        ];
        for (pairs, cost) in cases {
            for p0 in (0..CYCLE).step_by(37) {
                let (torn, took) =
                    simulate(p0, pairs, cost, 150, u32::MAX).expect("no budget limit");
                assert!(!torn, "pairs {pairs:#x} cost {cost} from {p0}: torn");
                assert!(took <= 3 * CYCLE, "took {took}");
            }
        }
    }

    #[test]
    fn unpaced_blits_tear_from_most_positions() {
        // The same judge on an unpaced full blit: torn unless it starts
        // late in the last pair.
        let mut torn = 0;
        for p0 in (0..CYCLE).step_by(10) {
            let mut judge = Judge::new(ROW);
            let read = |t: u32| Reading {
                pos: (p0 + t) % CYCLE,
                at: t,
            };
            for k in 0..PAIRS {
                let t = k as u32 * 74;
                judge.pair(k, read(t), read(t + 74));
            }
            torn += u32::from(judge.straddled);
        }
        let share = torn * 100 / (CYCLE / 10);
        assert!((80..100).contains(&share), "{share}% torn");
    }

    #[test]
    fn plan_races_ahead_when_it_can_and_respects_the_budget() {
        // Pairs 10-12 with the scan at pair 2: written now, for this refresh.
        let now = Reading {
            pos: 2 * ROW,
            at: 0,
        };
        let (p, wait) = Pacer::plan(ROW, now, 0x1C00, 150, 30, 0).expect("no wait needed");
        assert_eq!((p.target(), wait), (0, 0));
        // Pairs 0-3 with the scan at pair 2: next refresh, after pair 3.
        let (p, wait) = Pacer::plan(ROW, now, 0x000F, 150, 30, u32::MAX).unwrap();
        assert_eq!(p.target(), 1);
        assert_eq!(wait, 2 * ROW + 30);
        assert_eq!(p.wait(0, now), 0);
        assert_eq!(p.wait(3, now), 2 * ROW + 30);
        assert!(Pacer::plan(ROW, now, 0x000F, 150, 30, ROW).is_none());
        // A full frame with the scan at the very end: the refresh after next.
        let late = Reading {
            pos: CYCLE - 20,
            at: 0,
        };
        let (p, _) = Pacer::plan(ROW, late, 0xFFFF, 150, 30, u32::MAX).unwrap();
        assert_eq!(p.target(), 2);
    }

    #[test]
    fn the_bottom_pass_lands_exactly_one_refresh_after_the_top() {
        for p0 in (0..CYCLE).step_by(41) {
            let read = |t: u32| Reading {
                pos: (p0 + t) % CYCLE,
                at: t,
            };
            let (top, _) = Pacer::plan(ROW, read(0), 0xFFFF, 150, 30, u32::MAX).unwrap();
            let bottom = top.next();
            let (mut jt, mut jb) = (
                Judge::with_clock(top.clock()),
                Judge::with_clock(top.clock()),
            );
            let mut t = 0;
            for k in 0..PAIRS {
                t += top.wait(k, read(t));
                let b = read(t);
                t += 74;
                jt.pair(k, b, read(t));
            }
            // A sleep until shortly before the last pair frees, as the
            // firmware does, then the paced bottom writes.
            t = t.max(bottom.free_at(PAIRS - 1).saturating_sub(SPLIT_LEAD_US));
            for k in 0..PAIRS {
                t += bottom.wait(k, read(t));
                let b = read(t);
                t += 74;
                jb.pair(k, b, read(t));
            }
            assert!(!jt.straddled && !jb.straddled, "from {p0}");
            assert_eq!(jb.refresh(), jt.refresh().map(|r| r + 1), "from {p0}");
            assert_eq!(jt.refresh(), Some(top.target()));
        }
    }

    #[test]
    fn judge_sees_a_pair_written_under_the_scan() {
        let mut j = Judge::new(ROW);
        j.pair(
            3,
            Reading {
                pos: 3 * ROW + 10,
                at: 0,
            },
            Reading {
                pos: 3 * ROW + 80,
                at: 70,
            },
        );
        assert!(j.in_pair && j.straddled);
    }
}
