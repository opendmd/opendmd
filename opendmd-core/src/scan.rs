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

/// Kept from a frame's deadline for what follows a synchronised blit's
/// last wait: the last pair's write (up to 450 µs tinted) and the loop's
/// own work after the blit (a log line, the status), µs.
pub const POST_US: u32 = 1_000;

/// A stretch stops this far (µs) short of half the frame's delay (with
/// `POST_US` that keeps the next tick over 1 ms clear of it): an effect tick later than half a frame re-times its schedule
/// (`Engine::advance`).
const STRETCH_GUARD_US: u32 = 500;

/// How long a synchronised frame blit may wait for the scan, from when it
/// starts.
///
/// [`fit_us`](Self::fit_us) keeps the next frame on time. A frame of a
/// fast *live effect* (an `@fx:` file, whose schedule the engine keeps
/// through a tick up to half a frame late, `Engine::advance`; its delay
/// under two refreshes: the 10 ms effects on a ~8 ms refresh) may also
/// *stretch* past it: a paced blit can need up to a refresh of waiting,
/// which a 10 ms frame's own render leaves no room for, and written at
/// once instead it tears. Stretched, the blit lands on the next refresh
/// boundary and the next frame starts that much late (at most a
/// refresh, and short of half its delay, so the effect keeps its
/// schedule);
/// the next frame, starting just after a boundary, then has most of a
/// refresh to land on the one after, so the lateness does not carry on.
/// Every frame is shown, each from a refresh boundary: what a 100 fps
/// source on a 125 Hz panel can get. Such lateness is the sync's choice,
/// not a deadline miss (the firmware counts it as `scan` `stretched` /
/// `late`). GIFs never stretch, at any refresh: the engine times a GIF's
/// next frame from a late tick, so a stretch would slow it down. The
/// caller decides by passing `delay_us` only for an effect's frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    /// Wait at all (`false`: written at once, nothing counted).
    pub sync: bool,
    /// Until the next frame is due, µs.
    pub left_us: u32,
    /// The effect's frame delay when this blit is one of its frames on its
    /// schedule (`None`: a GIF, a transition step, a file's first frame, a
    /// glow step — no stretch).
    pub delay_us: Option<u32>,
}

impl Budget {
    /// No waiting at all: written at once.
    pub const NONE: Budget = Budget {
        sync: false,
        left_us: 0,
        delay_us: None,
    };

    /// A blit whose next frame is due in `left_us`.
    pub fn frame(left_us: u32, delay_us: Option<u32>) -> Budget {
        Budget {
            sync: true,
            left_us,
            delay_us,
        }
    }

    /// Waiting this long still leaves the next frame on time.
    pub fn fit_us(&self) -> u32 {
        self.left_us.saturating_sub(POST_US)
    }

    /// How far past the next frame's time the waits may run, for a
    /// refresh of `cycle_us`: 0 unless this is an effect's frame shorter
    /// than two refreshes.
    pub fn stretch_us(&self, cycle_us: u32) -> u32 {
        match self.delay_us {
            Some(d) if d < 2 * cycle_us => cycle_us.min((d / 2).saturating_sub(STRETCH_GUARD_US)),
            _ => 0,
        }
    }

    /// The longest the waits may run, stretch included (0: written at
    /// once). A frame that starts already late (`left_us` 0, after a
    /// stretched one) still has its stretch.
    pub fn limit_us(&self, cycle_us: u32) -> u32 {
        if !self.sync {
            return 0;
        }
        (self.left_us.saturating_add(self.stretch_us(cycle_us))).saturating_sub(POST_US)
    }
}

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

    /// The main loop's frame path, simulated: a source with frames every
    /// `delay` µs, each rendered in `render` µs (from `lo..hi`, a fixed
    /// pseudo-random sequence), woken up to 1 ms late, blitted full frame
    /// (74 µs a pair) with the waits honoured exactly, and the engine's
    /// schedule: an effect's (`fx`) kept unless a tick is half a frame
    /// late, a GIF's timed from a late tick.
    #[derive(Default, Debug)]
    struct LoopStats {
        blits: u32,
        torn: u32,
        gave_up: u32,
        stretched: u32,
        /// Frames late only because the blit stretched (not misses).
        late: u32,
        misses: u32,
        /// Needed wait vs what the budget allowed, for the old policy's
        /// gave-up analysis: (needed, allowed) in µs.
        short: Vec<(u32, u32)>,
        /// Effect schedule re-timed (a tick over half a frame late).
        retimed: u32,
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Policy {
        /// beta.11: next frame - now - 3 ms, never past it.
        Slack3,
        /// [`Budget`]: next frame - now - `POST_US`, stretched for fast
        /// sources.
        Stretch,
    }

    fn run_loop(
        policy: Policy,
        row: u32,
        fx: bool,
        delay: u32,
        lo: u32,
        hi: u32,
        frames: u32,
    ) -> LoopStats {
        let cycle = row * PAIRS as u32;
        let mut seed = 0x2545_F491u32;
        let mut rnd = |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % n.max(1)
        };
        let p0 = 1234;
        let read = |t: u32| Reading {
            pos: (p0 + t) % cycle,
            at: t,
        };
        let mut st = LoopStats::default();
        let mut due = 10_000u32;
        let mut free = 0u32; // when the loop may start the next tick
        for _ in 0..frames {
            let start = free.max(due + rnd(1000));
            let keep = fx && start.saturating_sub(due) < delay / 2;
            st.retimed += u32::from(fx && !keep);
            let next = if keep { due + delay } else { start + delay };
            let mut t = start + lo + rnd(hi - lo);
            let left = next.saturating_sub(t);
            let (fit, limit) = match policy {
                Policy::Slack3 => (left.saturating_sub(3_000), left.saturating_sub(3_000)),
                Policy::Stretch => {
                    let b = Budget::frame(left, fx.then_some(delay));
                    (b.fit_us(), b.limit_us(cycle))
                }
            };
            // What the pacer would need, unbounded.
            let need = Pacer::plan(row, read(t), 0xFFFF, 300, 30, u32::MAX)
                .map(|(_, took)| took)
                .unwrap();
            let plan = if limit > 0 {
                Pacer::plan(row, read(t), 0xFFFF, 300, 30, limit)
            } else {
                None
            };
            let blit_start = t;
            let mut judge = Judge::new(row);
            let mut stretched = false;
            match plan {
                Some((pacer, took)) => {
                    stretched = took > fit;
                    for k in 0..PAIRS {
                        t += pacer.wait(k, read(t));
                        let b = read(t);
                        t += 74;
                        judge.pair(k, b, read(t));
                    }
                }
                None => {
                    st.gave_up += 1;
                    st.short.push((need, limit));
                    for k in 0..PAIRS {
                        let b = read(t);
                        t += 74;
                        judge.pair(k, b, read(t));
                    }
                }
            }
            let waited = t - blit_start - 16 * 74;
            t += 200; // the loop's own work after the blit
            st.blits += 1;
            st.torn += u32::from(judge.straddled);
            st.stretched += u32::from(stretched);
            if t > next {
                if stretched && t - waited <= next {
                    // The blit slept through most of its wait: no
                    // CATCHUP_SLEEP for it.
                    st.late += 1;
                    free = t;
                } else {
                    st.misses += 1;
                    free = t + 1_000; // CATCHUP_SLEEP
                }
            } else {
                free = t;
            }
            due = next;
        }
        st
    }

    /// Prints the 10 ms frame analysis (`cargo test -p opendmd-core
    /// sync_at_100fps -- --nocapture`) and holds the new policy to it.
    #[test]
    fn sync_at_100fps_stretches_instead_of_tearing() {
        // The measured refresh: 499 µs a pair (7.98 ms).
        for (lo, hi) in [(500, 1500), (1000, 3500), (3000, 5000)] {
            let old = run_loop(Policy::Slack3, 499, true, 10_000, lo, hi, 2000);
            let new = run_loop(Policy::Stretch, 499, true, 10_000, lo, hi, 2000);
            let mean = |v: &[(u32, u32)], f: fn(&(u32, u32)) -> u32| {
                v.iter().map(f).sum::<u32>() / (v.len() as u32).max(1)
            };
            println!(
                "render {lo}-{hi} us, 10 ms frames: slack 3 ms: {} gave up, {} torn, {} misses \
                 (gave up: needed {} us on average, allowed {}); \
                 stretch: {} gave up, {} torn, {} stretched, {} late by the stretch, {} misses, {} re-timed",
                old.gave_up,
                old.torn,
                old.misses,
                mean(&old.short, |s| s.0),
                mean(&old.short, |s| s.1),
                new.gave_up,
                new.torn,
                new.stretched,
                new.late,
                new.misses,
                new.retimed,
            );
            assert!(old.gave_up > old.blits / 5, "the old policy gives up often");
            if hi <= 3500 {
                // The rig's effects (<= 3.5 ms p90): every frame on a
                // refresh boundary, none a deadline miss.
                assert_eq!((new.gave_up, new.torn, new.misses), (0, 0, 0), "{new:?}");
                assert_eq!(new.retimed, 0);
            } else {
                // Heavier renders: a frame after a stretched one starts
                // late but keeps its stretch, so it still lands.
                assert!(new.torn * 100 < old.torn, "{new:?} vs {old:?}");
                assert_eq!((new.misses, new.retimed), (0, 0), "{new:?}");
            }
        }
    }

    #[test]
    fn slow_sources_never_stretch() {
        // GIF frames (>= 20 ms) and a 50 fps Formula: no stretch, and the
        // smaller slack only makes more frames fit.
        for delay in [20_000, 40_000, 100_000] {
            assert_eq!(Budget::frame(delay, Some(delay)).stretch_us(7_984), 0);
            let old = run_loop(Policy::Slack3, 499, false, delay, 2_000, 12_000, 500);
            let new = run_loop(Policy::Stretch, 499, false, delay, 2_000, 12_000, 500);
            assert_eq!(new.stretched, 0);
            assert!(new.gave_up <= old.gave_up && new.torn <= old.torn);
            assert_eq!(new.misses, old.misses, "{delay}");
        }
        // A transition step or a glow step (no delay given): no stretch.
        assert_eq!(Budget::frame(5_000, None).limit_us(7_984), 4_000);
        // 10 ms frames: short of half the delay (the effect's re-timing),
        // at most a refresh; none once the frame is two refreshes long.
        assert_eq!(Budget::frame(5_000, Some(10_000)).stretch_us(7_984), 4_500);
        assert_eq!(Budget::frame(5_000, Some(10_000)).stretch_us(16_000), 4_500);
        assert_eq!(Budget::frame(5_000, Some(10_000)).stretch_us(5_500), 4_500);
        assert_eq!(Budget::frame(5_000, Some(10_000)).stretch_us(5_000), 0);
        assert_eq!(Budget::frame(5_000, Some(8_000)).stretch_us(4_100), 3_500);
        // Nothing left: written at once, unless an effect's frame (late
        // after a stretched one) still has its stretch.
        assert_eq!(Budget::frame(900, None).limit_us(7_984), 0);
        assert_eq!(Budget::frame(0, Some(10_000)).limit_us(7_984), 3_500);
        assert_eq!(Budget::NONE.limit_us(7_984), 0);
    }

    /// A slow scan (`min_refresh` low: a 16-33 ms refresh): a 20 ms GIF
    /// never stretches (it would slow down); a 10 ms effect, with a
    /// refresh longer than its frame, stretches until the lateness reaches
    /// the cap, then a frame is written at once (and may tear) and the
    /// next starts on time again — never worse than the 3 ms slack.
    #[test]
    fn slow_refresh_gifs_never_stretch_and_effects_degrade_gracefully() {
        for row in [1_000, 1_500, 2_062] {
            let old = run_loop(Policy::Slack3, row, false, 20_000, 2_000, 8_000, 1000);
            let new = run_loop(Policy::Stretch, row, false, 20_000, 2_000, 8_000, 1000);
            println!(
                "{} us refresh, 20 ms GIF: slack 3 ms {} gave up, {} torn, {} misses; \
                 now {} gave up, {} torn, {} misses",
                row * 16,
                old.gave_up,
                old.torn,
                old.misses,
                new.gave_up,
                new.torn,
                new.misses
            );
            assert_eq!((new.stretched, new.late), (0, 0));
            assert!(new.gave_up <= old.gave_up && new.torn <= old.torn);
            assert!(new.misses <= old.misses);
            let old = run_loop(Policy::Slack3, row, true, 10_000, 1_000, 3_500, 1000);
            let new = run_loop(Policy::Stretch, row, true, 10_000, 1_000, 3_500, 1000);
            println!(
                "{} us refresh, 10 ms effect: slack 3 ms {} gave up, {} torn, {} misses; \
                 now {} gave up, {} torn, {} stretched, {} late, {} misses, {} re-timed",
                row * 16,
                old.gave_up,
                old.torn,
                old.misses,
                new.gave_up,
                new.torn,
                new.stretched,
                new.late,
                new.misses,
                new.retimed
            );
            assert!(new.torn <= old.torn);
            assert!(new.misses <= old.misses + new.blits / 100);
            assert_eq!(new.retimed, 0);
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
