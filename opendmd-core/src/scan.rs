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

/// How long a synchronised frame blit may wait for the scan, from when it
/// starts: as long as that keeps the next frame on time
/// ([`fit_us`](Self::fit_us)). Past that it writes at once. (A Formula's
/// frames race the beam instead, `scan::Beam`; until console-c a fast
/// Formula's paced blit could also stretch past its next frame's time.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    /// Wait at all (`false`: written at once, nothing counted).
    pub sync: bool,
    /// Until the next frame is due, µs.
    pub left_us: u32,
}

impl Budget {
    /// No waiting at all: written at once.
    pub const NONE: Budget = Budget {
        sync: false,
        left_us: 0,
    };

    /// A blit whose next frame is due in `left_us`.
    pub fn frame(left_us: u32) -> Budget {
        Budget {
            sync: true,
            left_us,
        }
    }

    /// Waiting this long still leaves the next frame on time (0: written
    /// at once).
    pub fn fit_us(&self) -> u32 {
        if !self.sync {
            return 0;
        }
        self.left_us.saturating_sub(POST_US)
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

/// Racing the beam (console-lessons §2.6): a Formula's frame drawn in the
/// panel's pair order, each pair written as soon as it is drawn, so that
/// every pair lands after the scan has left its slot in refresh `T - 1`
/// and before its slot in `T` — no wait for a clear start, no second pass,
/// no tear.
///
/// Pair `k`'s window is `[(T - 1) C + (k + 1) R, T C + k R)`: one refresh
/// less a pair long, sliding a pair (`R`) down per pair. A render whose
/// first pair is drawn `F` µs after its start `s` and its last `E` µs after
/// fits every window (its pairs between at most a pair's scan apart) when
/// `s` is in `[T C - E, T C - F - w]` (`w` a pair's write), and
/// [`Beam::plan`] aims at the middle of
/// [`BEAM_LATE_ROOM_US`] inside its late edge. The player's next frame is timed to come back there
/// ([`beam_delay_us`]), so in steady play a frame neither waits nor is
/// late; a pair that would still come early waits for its window
/// ([`Beam::wait`]), and one past it is written anyway and counted
/// ([`Beam::late`]: it shows a refresh late, a seam for one refresh).
#[derive(Clone, Copy, Debug)]
pub struct Beam {
    clock: Clock,
    target: u32,
    margin: u32,
}

/// How far (µs) a raced frame's start is aimed from the late edge of its
/// window: more than a wake is ever late (~0.2 ms mean, ~1 ms worst on the
/// spare with the `esp_timer` wake).
pub const BEAM_LATE_ROOM_US: u32 = 1_500;

/// The start window's late side: three quarters of it, at most
/// [`BEAM_LATE_ROOM_US`] (the early side is waited out, which costs a
/// frame nothing but CPU it has to spare). A half each, beam-c's
/// Starfield (rows ~2 ms: a window ~1.7 ms wide, a draw stage before it
/// that varies) was late 0.17-0.26 % of frames on a test board; simulated
/// (`late_side_takes_three_quarters`), 3/4 takes its late starts from
/// 0.3 % to under 0.06 %. (`LATE_SHARE`: the tests' knob, eighths.)
fn late_share(room: i64) -> i64 {
    #[cfg(test)]
    {
        let k = tests::LATE_SHARE.with(|c| c.get());
        if k > 0 {
            return (room * k / 8).min(i64::from(BEAM_LATE_ROOM_US));
        }
    }
    (room * 3 / 4).min(i64::from(BEAM_LATE_ROOM_US))
}

/// What [`Beam::plan`] decided for a frame.
#[derive(Clone, Copy, Debug)]
pub struct BeamPlan {
    pub beam: Beam,
    /// How far (µs) the start was after (+) or before (−) the middle of
    /// its window: what the next frame's delay corrects.
    pub err: i32,
    /// Wait this long (µs) before drawing: the start was so early that
    /// pairs would come before their windows open (or, `retarget`, so late
    /// that the frame goes to the next refresh).
    pub wait_us: u32,
    /// How far (µs) the start was past its window's late edge (0: inside
    /// it): its first pairs would be written after the scan came back.
    pub late_by: u32,
    /// The frame was moved to the next refresh for that (only when the
    /// caller has the time: a frame of two refreshes or more).
    pub retarget: bool,
}

impl Beam {
    /// Plan a frame whose drawing starts at reading `now`, its first pair
    /// expected drawn `first_us` later and its last `last_us` later (the
    /// writes between included), each write at most `write_us`, keeping `margin` from every slot.
    ///
    /// `spare_refresh`: the frame has a refresh to spare (it is held for two
    /// or more): a start past the late edge moves to the next refresh and
    /// waits for it, rather than writing its first pairs late.
    pub fn plan(
        row_us: u32,
        now: Reading,
        first_us: u32,
        last_us: u32,
        write_us: u32,
        margin: u32,
        spare_refresh: bool,
    ) -> BeamPlan {
        let clock = Clock::new(row_us, now);
        let c = clock.cycle() as i64;
        let u = i64::from(now.pos);
        let e = i64::from(last_us.max(first_us));
        let w = i64::from(write_us) + 2 * i64::from(margin);
        // The window for the start, as leads on T's start: from `e` (the
        // last pair just after the scan leaves it) to `first + w` (the
        // first pair written just before the scan comes back to it).
        // Aimed `late_share` of it from its late edge: a start comes late
        // more than early, and an early one only waits.
        let late_lead = i64::from(first_us) + w;
        let room = (e - late_lead).max(0);
        let late_room = late_share(room);
        let lead = late_lead + late_room;
        let mut target = ((u + lead + c / 2) / c).max(1);
        let err = u + lead - target * c;
        let late_by = (err - late_room).max(0) as u32;
        // Early past its window: wait for it. Late past it with a refresh
        // to spare: the next refresh, waited for to the aim.
        let retarget = late_by > 0 && spare_refresh;
        let wait_us = if retarget {
            target += 1;
            (c - err).max(0) as u32
        } else {
            (-err - (room - late_room)).max(0) as u32
        };
        BeamPlan {
            beam: Beam {
                clock,
                target: target as u32,
                margin,
            },
            err: err as i32,
            wait_us,
            late_by,
            retarget,
        }
    }

    /// Before writing pair `k`, with the scan at `now`: how long to wait
    /// for its window (µs; 0: write now).
    pub fn wait(&self, k: usize, now: Reading) -> u32 {
        let free_at = self.clock.slot(self.target - 1, k as u32) + self.clock.row_us + self.margin;
        free_at.saturating_sub(self.clock.unwrap(now))
    }

    /// Pair `k`'s write ended at `after` past its window: refresh `T`
    /// showed the old pair (a seam for that refresh).
    pub fn late(&self, k: usize, after: Reading) -> bool {
        self.clock.unwrap(after) + self.margin > self.clock.slot(self.target, k as u32)
    }

    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// The refresh the frame is first shown by, counted on the clock.
    pub fn target(&self) -> u32 {
        self.target
    }
}

/// The delay (µs) to the next raced frame: `frames` refreshes of
/// `cycle_us`, less half the start's error ([`BeamPlan::err`]: a late
/// start brings the next one forward, an early one puts it back), so the
/// frames lock onto the scan whatever the clocks' drift and the wake's
/// lateness. The error counted is at most half a refresh either way.
pub fn beam_delay_us(frames: u32, cycle_us: u32, err: i32) -> u32 {
    let half = (cycle_us / 2) as i32;
    let fix = err.clamp(-half, half) / 2;
    (frames.max(1) * cycle_us).saturating_add_signed(-fix)
}

/// The frames a raced Formula's frame grid holds each picture for: the
/// most refreshes (1, 2, 4 …) of `cycle_us` that still fit the frame time
/// it asked for (`grid_us`; its `fps`): 100 fps → every refresh (125 Hz on
/// a test panel), 50 → every 2nd (62.6 Hz), 25 → every 4th (31.3 Hz).
pub fn beam_frames(grid_us: u32, cycle_us: u32) -> u32 {
    let mut n = 1;
    while cycle_us > 0 && 2 * n * cycle_us <= grid_us && n < 64 {
        n *= 2;
    }
    n
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

    /// A raced frame loop, simulated: a scan of `ROW` µs pairs from phase
    /// `p0`; each frame starts at its due time plus a wake 0-`jitter` µs
    /// late, plans, waits if told, draws its pairs at `render` µs each
    /// (`frame_us` before the first: the frame and draw stages; each pair's
    /// time varied ±`spread`), waits per pair as the beam says, and is
    /// timed by [`beam_delay_us`]. Returns (frames with a late pair, frames
    /// that waited, the frames' mean period).
    fn race(
        p0: u32,
        render: u32,
        frame_us: u32,
        spread: u32,
        jitter: u32,
        frames: u32,
    ) -> (u32, u32, f64) {
        let read = |t: u64| Reading {
            pos: ((u64::from(p0) + t) % u64::from(CYCLE)) as u32,
            at: t as u32,
        };
        let mut rng = 12345u64;
        let mut rnd = |n: u32| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            if n == 0 {
                0
            } else {
                (rng % u64::from(n)) as u32
            }
        };
        let est = frame_us + 16 * (render + 74);
        let first_us = frame_us + render;
        let (mut due, mut late, mut waited, mut end) = (0u64, 0, 0, 0u64);
        let first = due;
        for f in 0..frames {
            let mut t = (due + u64::from(rnd(jitter))).max(end);
            let plan = Beam::plan(ROW, read(t), first_us, est, 80, 30, false);
            if plan.wait_us > 0 {
                waited += u32::from(f > 4);
                t += u64::from(plan.wait_us);
            }
            t += u64::from(frame_us);
            let mut frame_late = false;
            for k in 0..PAIRS {
                t += u64::from(render.saturating_sub(spread) + rnd(2 * spread + 1));
                let w = plan.beam.wait(k, read(t));
                if w > 0 && f > 4 {
                    // (A pair early in steady play: counted as waiting.)
                    waited += 1;
                }
                t += u64::from(w);
                t += 74;
                frame_late |= plan.beam.late(k, read(t));
            }
            late += u32::from(frame_late && f > 4);
            end = t;
            due += u64::from(beam_delay_us(1, CYCLE, plan.err));
        }
        (late, waited, (due - first) as f64 / f64::from(frames))
    }

    /// The race as beam-c plans it: the start after the frame and draw
    /// stages (`draw` µs, ±`draw_jitter`), a wake 0-`jitter` µs late and,
    /// one frame in `spike_every`, `spike` µs more before the tick (a
    /// heartbeat, a slow open). (frames with a late pair, waits).
    fn race2(p0: u32, draw: u32, draw_jitter: u32, per: u32, jitter: u32, spike_every: u32, spike: u32) -> (u32, u32) {
        let read = |t: u64| Reading {
            pos: ((u64::from(p0) + t) % u64::from(CYCLE)) as u32,
            at: t as u32,
        };
        let mut rng = 987_654_321u64;
        let mut rnd = |n: u32| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            if n == 0 { 0 } else { (rng % u64::from(n)) as u32 }
        };
        let (first_us, last_us) = (per + 74, 16 * (per + 74));
        let (mut due, mut late, mut waited, mut end) = (0u64, 0, 0, 0u64);
        for f in 0..4_000u32 {
            let mut t = (due + u64::from(rnd(jitter))).max(end);
            if spike_every > 0 && rnd(spike_every) == 0 {
                t += u64::from(spike);
            }
            t += u64::from(draw.saturating_sub(draw_jitter) + rnd(2 * draw_jitter + 1));
            let plan = Beam::plan(ROW, read(t), first_us, last_us, 80, 30, false);
            if plan.wait_us > 0 {
                waited += u32::from(f > 20);
                t += u64::from(plan.wait_us);
            }
            let mut frame_late = false;
            for k in 0..PAIRS {
                t += u64::from(per);
                t += u64::from(plan.beam.wait(k, read(t)));
                t += 74;
                frame_late |= plan.beam.late(k, read(t));
            }
            late += u32::from(frame_late && f > 20);
            end = t;
            due += u64::from(beam_delay_us(1, CYCLE, plan.err));
        }
        (late, waited)
    }

    thread_local! {
        pub(super) static LATE_SHARE: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
    }

    /// Starfield-like (beam-c on a test board): a 1.5 ms draw stage ±0.6 ms,
    /// 16 pairs of 50 µs + their writes, a 2.5 ms spike one frame in 40.
    /// Three quarters of the window on the late side: a fifth of the late
    /// frames half of it had (waits instead, harmless); without the spikes
    /// nothing is late from any phase.
    /// Held for two refreshes, a start past its window's late edge moves to
    /// the next refresh and waits: no pair late. Held for one, it is
    /// written late (counted).
    #[test]
    fn a_late_start_with_a_refresh_to_spare_takes_the_next() {
        let (first, last, per) = (124, 16 * 124, 124);
        // The window's late edge is `first + 140` before a refresh's start:
        // start 100 µs before one.
        let now = Reading { pos: CYCLE - 100, at: 1_000_000 };
        for spare in [false, true] {
            let plan = Beam::plan(ROW, now, first, last, 80, 30, spare);
            assert!(plan.late_by > 0);
            assert_eq!(plan.retarget, spare);
            let mut t = now.at + plan.wait_us;
            let read = |t: u32| Reading { pos: (now.pos + t.wrapping_sub(now.at)) % CYCLE, at: t };
            let mut late = false;
            for k in 0..PAIRS {
                t += per - 74;
                t += plan.beam.wait(k, read(t));
                t += 74;
                late |= plan.beam.late(k, read(t));
            }
            assert_eq!(late, !spare, "spare {spare}");
        }
    }

    #[test]
    fn late_side_takes_three_quarters() {
        let run = |k: i64| {
            LATE_SHARE.with(|c| c.set(k));
            let mut tot = (0, 0);
            for p0 in (0..CYCLE).step_by(997) {
                let a = race2(p0, 1_500, 600, 50, 400, 40, 2_500);
                tot = (tot.0 + a.0, tot.1 + a.1);
                assert_eq!(race2(p0, 1_500, 300, 50, 400, 0, 0).0, 0, "share {k}/8 from phase {p0}");
            }
            LATE_SHARE.with(|c| c.set(0));
            tot
        };
        let (half, three) = (run(4), run(6));
        eprintln!("late frames / waits, 8 phases × 4,000 frames: half {half:?}, three quarters {three:?}");
        assert!(three.0 * 4 < half.0, "half {half:?}, three quarters {three:?}");
        assert!(three.0 < 32_000 / 1_000, "{three:?}: over 0.1 %");
    }

    #[test]
    fn racing_the_beam_locks_on_and_never_tears() {
        // Renders from a cycled plasma's ~0.9 ms to the 6.5 ms budget
        // (the road ~3.6-4.4 ms, Time gate ~2.8-5), from every phase.
        for render_ms in [0.9, 2.0, 3.6, 4.4, 5.0, 6.0] {
            let total = (render_ms * 1000.0) as u32;
            // (The frame and draw stages before the first pair: a sixth or
            // a third of it, the road's.)
            for (frame_us, p0) in [6, 3]
                .into_iter()
                .flat_map(|d| (0..CYCLE).step_by(397).map(move |p| (total / d, p)))
            {
                let per = (total - frame_us) / 16;
                let (late, waited, period) =
                    race(p0, per.saturating_sub(74), frame_us, per / 5, 400, 400);
                assert_eq!(
                    late, 0,
                    "{render_ms} ms from phase {p0}: frames with a late pair"
                );
                // (Early starts wait: harmless; three quarters of the
                // window are on the late side, so the quicker renders wait
                // now and then.)
                assert!(
                    waited <= 40,
                    "{render_ms} ms from phase {p0}: {waited} waits"
                );
                assert!(
                    (period - f64::from(CYCLE)).abs() < 30.0,
                    "{render_ms} ms: period {period}"
                );
            }
        }
    }

    #[test]
    fn a_raced_frame_too_slow_for_its_windows_is_late_not_lost() {
        // 9 ms of drawing in one refresh: pairs past their windows, counted.
        let (late, _, _) = race(0, 9_000 / 16, 0, 0, 0, 50);
        assert!(late > 0);
    }

    #[test]
    fn the_beam_grid_holds_frames_for_whole_refreshes() {
        assert_eq!(beam_frames(10_000, 7_980), 1);
        assert_eq!(beam_frames(20_000, 7_980), 2);
        assert_eq!(beam_frames(40_000, 7_980), 4);
        assert_eq!(beam_frames(8_000, 7_980), 1);
        assert_eq!(beam_frames(5_000, 7_980), 1);
        assert_eq!(beam_delay_us(1, 8_000, 0), 8_000);
        assert_eq!(beam_delay_us(2, 8_000, 1_000), 15_500);
        assert_eq!(beam_delay_us(1, 8_000, -100_000), 10_000);
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
        misses: u32,
        /// Effect schedule re-timed (a tick over half a frame late).
        retimed: u32,
    }

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Policy {
        /// beta.11: next frame - now - 3 ms.
        Slack3,
        /// [`Budget`]: next frame - now - `POST_US`.
        Fit,
    }

    fn run_loop(policy: Policy, row: u32, fx: bool, delay: u32, lo: u32, hi: u32, frames: u32) -> LoopStats {
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
            let limit = match policy {
                Policy::Slack3 => left.saturating_sub(3_000),
                Policy::Fit => Budget::frame(left).fit_us(),
            };
            let plan = if limit > 0 {
                Pacer::plan(row, read(t), 0xFFFF, 300, 30, limit)
            } else {
                None
            };
            let mut judge = Judge::new(row);
            match plan {
                Some((pacer, _)) => {
                    for k in 0..PAIRS {
                        t += pacer.wait(k, read(t));
                        let b = read(t);
                        t += 74;
                        judge.pair(k, b, read(t));
                    }
                }
                None => {
                    st.gave_up += 1;
                    for k in 0..PAIRS {
                        let b = read(t);
                        t += 74;
                        judge.pair(k, b, read(t));
                    }
                }
            }
            // The loop's work after the blit.
            t += 200 + rnd(500);
            st.blits += 1;
            st.torn += u32::from(judge.straddled);
            if t > next {
                st.misses += 1;
                free = t + 1_000; // CATCHUP_SLEEP
            } else {
                free = t;
            }
            due = next;
        }
        st
    }

    /// GIF frames (and transition and glow steps): the budget's smaller
    /// slack only makes more frames fit than beta.11's 3 ms, and costs no
    /// deadline; nothing waits with no time left.
    #[test]
    fn the_budget_keeps_the_next_frame_on_time() {
        for delay in [20_000, 40_000, 100_000] {
            let old = run_loop(Policy::Slack3, 499, false, delay, 2_000, 12_000, 500);
            let new = run_loop(Policy::Fit, 499, false, delay, 2_000, 12_000, 500);
            assert!(new.gave_up <= old.gave_up && new.torn <= old.torn, "{delay}: {new:?} vs {old:?}");
            assert_eq!(new.misses, old.misses, "{delay}");
        }
        assert_eq!(Budget::frame(5_000).fit_us(), 4_000);
        assert_eq!(Budget::frame(900).fit_us(), 0);
        assert_eq!(Budget::NONE.fit_us(), 0);
    }

    /// A slow scan (`min_refresh` low: a 16-33 ms refresh): a 20 ms GIF's
    /// blits, never worse than the 3 ms slack.
    #[test]
    fn slow_refresh_gifs_are_no_worse_than_the_old_slack() {
        for row in [1_000, 1_500, 2_062] {
            let old = run_loop(Policy::Slack3, row, false, 20_000, 2_000, 8_000, 1000);
            let new = run_loop(Policy::Fit, row, false, 20_000, 2_000, 8_000, 1000);
            println!(
                "{} us refresh, 20 ms GIF: slack 3 ms {} gave up, {} torn, {} misses; now {} gave up, {} torn, {} misses",
                row * 16,
                old.gave_up,
                old.torn,
                old.misses,
                new.gave_up,
                new.torn,
                new.misses
            );
            assert!(new.gave_up <= old.gave_up && new.torn <= old.torn);
            assert!(new.misses <= old.misses);
            assert!(new.blits == 1000 && new.retimed == 0);
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
