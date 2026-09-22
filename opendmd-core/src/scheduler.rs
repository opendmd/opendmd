//! Playlist ordering: sequential or weighted shuffle, deterministic per seed.
//!
//! No `rand` dependency — the firmware has no entropy source worth the code
//! size and tests need reproducibility, so the caller supplies a seed and the
//! generator is splitmix64 inline.

use std::collections::VecDeque;

/// splitmix64. Tiny, passes BigCrush, exactly reproducible.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        // Top 53 bits — the exactly-representable range of an f64 mantissa.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Sequential,
    Shuffle,
}

/// Orders a catalog of `len` items, referred to by their index `0..len`.
///
/// The scheduler never stores the item list: on a 10k-entry card a dense
/// `Vec<usize>` of items plus a dense `Vec<f32>` of weights would be 120 KB of
/// DRAM for something entirely implied by `len`. Weights are sparse, allocated
/// only for the indices `set_weight` is actually called on, so the default
/// (everything weight 1.0) costs nothing and the draw is O(1) memory.
#[derive(Clone, Debug)]
pub struct Scheduler {
    len: usize,
    /// `(index, weight)`, unique by index, empty until `set_weight` is called.
    weights: Vec<(usize, f32)>,
    mode: Mode,
    rng: Rng,
    cursor: usize,
    /// Ring of the last `no_repeat` indices drawn: 32 x 4 B by default.
    history: VecDeque<usize>,
    no_repeat: usize,
}

/// Rejection-sampling attempts before the weighted draw gives up and falls
/// back to a deterministic scan. Only reached when almost every index is
/// zero-weighted or blocked, which the fallback then handles exactly.
const DRAW_ATTEMPTS: usize = 1024;

impl Scheduler {
    /// Schedules over `0..len`. All weights default to 1.0 and the no-repeat
    /// window to 0.
    pub fn new(len: usize, mode: Mode, seed: u64) -> Self {
        Scheduler {
            len,
            weights: Vec::new(),
            mode,
            rng: Rng::new(seed),
            cursor: 0,
            history: VecDeque::new(),
            no_repeat: 0,
        }
    }

    /// Don't replay any of the last `n` items. Clamped at pick time so it can
    /// never starve a short playlist.
    pub fn with_no_repeat(mut self, n: usize) -> Self {
        self.no_repeat = n;
        self
    }

    /// Rewind to the start of the order with a fresh seed, keeping `len`, the
    /// mode and any weights. The caller gets a new shuffle without rebuilding
    /// (or re-reading) the catalog.
    pub fn reset(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
        self.cursor = 0;
        self.history.clear();
    }

    /// Relative weight for an item index. Weight 0 is never chosen.
    pub fn set_weight(&mut self, index: usize, weight: f32) {
        if index >= self.len {
            return;
        }
        let weight = weight.max(0.0);
        match self.weights.iter_mut().find(|(i, _)| *i == index) {
            Some(slot) => slot.1 = weight,
            None => self.weights.push((index, weight)),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Weight of one index; anything `set_weight` was never called on is 1.0.
    pub fn weight(&self, index: usize) -> f32 {
        self.weights
            .iter()
            .find(|(i, _)| *i == index)
            .map_or(1.0, |(_, w)| *w)
    }

    /// Next item index, or `None` if there is nothing playable (empty catalog,
    /// or every weight is zero in shuffle mode).
    ///
    /// Not `Iterator::next`: a `Sequential` scheduler never terminates, and the
    /// engine needs to interleave `set_weight` calls between draws.
    pub fn next_index(&mut self) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        let index = match self.mode {
            Mode::Sequential => {
                let index = self.cursor;
                self.cursor = (self.cursor + 1) % self.len;
                index
            }
            Mode::Shuffle => self.pick_weighted()?,
        };
        self.remember(index);
        Some(index)
    }

    fn remember(&mut self, index: usize) {
        if self.no_repeat == 0 {
            return;
        }
        self.history.push_back(index);
        while self.history.len() > self.no_repeat {
            self.history.pop_front();
        }
    }

    /// How many indices could be drawn at all: everything except the ones
    /// explicitly weighted to zero.
    fn playable(&self) -> usize {
        self.len - self.weights.iter().filter(|(_, w)| *w <= 0.0).count()
    }

    fn blocked(&self, index: usize, window: usize) -> bool {
        self.history.iter().rev().take(window).any(|i| *i == index)
    }

    /// Rejection sampling against the sparse weight table: draw an index
    /// uniformly, accept it with probability `weight / max_weight`. Exactly the
    /// same distribution as a cumulative-sum walk over a dense weight vector,
    /// in O(1) memory and — for the unweighted case, which is every real
    /// playlist — one RNG draw per pick.
    fn pick_weighted(&mut self) -> Option<usize> {
        let playable = self.playable();
        if playable == 0 {
            return None;
        }
        // Never let the window exclude every playable item.
        let window = self.no_repeat.min(playable.saturating_sub(1));
        let max_weight = self
            .weights
            .iter()
            .fold(1.0f32, |acc, (_, w)| if *w > acc { *w } else { acc });

        for _ in 0..DRAW_ATTEMPTS {
            let index = (self.rng.next_u64() % self.len as u64) as usize;
            if self.blocked(index, window) {
                continue;
            }
            let weight = self.weight(index);
            if weight <= 0.0 {
                continue;
            }
            if weight >= max_weight || self.rng.next_f64() < (weight / max_weight) as f64 {
                return Some(index);
            }
        }

        // Pathological table (a handful of playable indices in a huge catalog):
        // scan from a random start for the first eligible index, then ignore
        // the no-repeat window rather than return nothing.
        let from = (self.rng.next_u64() % self.len as u64) as usize;
        let scan = |blocking: bool, s: &Self| {
            (0..s.len)
                .map(|k| (from + k) % s.len)
                .find(|i| s.weight(*i) > 0.0 && !(blocking && s.blocked(*i, window)))
        };
        scan(true, self).or_else(|| scan(false, self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take(s: &mut Scheduler, n: usize) -> Vec<usize> {
        (0..n).filter_map(|_| s.next_index()).collect()
    }

    #[test]
    fn sequential_wraps_in_catalog_order() {
        let mut s = Scheduler::new(3, Mode::Sequential, 0);
        assert_eq!(take(&mut s, 7), vec![0, 1, 2, 0, 1, 2, 0]);
    }

    #[test]
    fn shuffle_is_reproducible_for_a_seed() {
        let mut a = Scheduler::new(20, Mode::Shuffle, 0xDEAD_BEEF);
        let mut b = Scheduler::new(20, Mode::Shuffle, 0xDEAD_BEEF);
        let mut c = Scheduler::new(20, Mode::Shuffle, 0xDEAD_BEEE);
        let run_a = take(&mut a, 40);
        assert_eq!(run_a, take(&mut b, 40));
        assert_ne!(run_a, take(&mut c, 40));
        // And it actually shuffles.
        let ordered: Vec<usize> = (0..20).collect();
        assert_ne!(run_a[..20], ordered[..]);
    }

    #[test]
    fn reset_rewinds_and_reseeds_without_rebuilding() {
        let mut s = Scheduler::new(20, Mode::Shuffle, 7).with_no_repeat(4);
        let first = take(&mut s, 40);
        s.reset(7);
        assert_eq!(take(&mut s, 40), first, "same seed must replay exactly");
        s.reset(8);
        assert_ne!(take(&mut s, 40), first);

        let mut q = Scheduler::new(5, Mode::Sequential, 0);
        assert_eq!(take(&mut q, 3), vec![0, 1, 2]);
        q.reset(0);
        assert_eq!(take(&mut q, 3), vec![0, 1, 2]);
    }

    #[test]
    fn no_repeat_window_is_respected() {
        let mut s = Scheduler::new(10, Mode::Shuffle, 42).with_no_repeat(4);
        let run = take(&mut s, 200);
        assert_eq!(run.len(), 200);
        for w in run.windows(5) {
            let mut seen = w.to_vec();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen.len(), 5, "repeat inside the window: {w:?}");
        }
    }

    #[test]
    fn no_repeat_window_cannot_starve_a_short_list() {
        let mut s = Scheduler::new(2, Mode::Shuffle, 7).with_no_repeat(8);
        let run = take(&mut s, 20);
        assert_eq!(run.len(), 20);
        // With a 2-item list and a window of 1, it must alternate.
        for w in run.windows(2) {
            assert_ne!(w[0], w[1]);
        }
    }

    #[test]
    fn zero_weight_is_never_chosen() {
        let mut s = Scheduler::new(5, Mode::Shuffle, 1);
        s.set_weight(2, 0.0);
        s.set_weight(4, 0.0);
        let run = take(&mut s, 500);
        assert_eq!(run.len(), 500);
        assert!(!run.contains(&2));
        assert!(!run.contains(&4));
        assert!(run.contains(&0) && run.contains(&1) && run.contains(&3));
    }

    #[test]
    fn weights_bias_the_draw() {
        let mut s = Scheduler::new(2, Mode::Shuffle, 99);
        s.set_weight(0, 9.0);
        let run = take(&mut s, 1000);
        let zeros = run.iter().filter(|i| **i == 0).count();
        assert!(zeros > 800 && zeros < 980, "got {zeros}/1000");
    }

    #[test]
    fn a_single_playable_index_in_a_large_catalog_is_still_found() {
        // Rejection sampling would time out here; the deterministic fallback
        // is what must produce the answer.
        let mut s = Scheduler::new(4000, Mode::Shuffle, 5).with_no_repeat(32);
        for i in 0..4000 {
            if i != 1234 {
                s.set_weight(i, 0.0);
            }
        }
        assert_eq!(take(&mut s, 3), vec![1234, 1234, 1234]);
    }

    #[test]
    fn weights_stay_sparse() {
        let mut s = Scheduler::new(10_000, Mode::Shuffle, 0);
        assert!(s.weights.is_empty());
        s.set_weight(42, 0.5);
        s.set_weight(42, 0.25);
        s.set_weight(10_001, 0.0); // out of range, ignored
        assert_eq!(s.weights.len(), 1);
        assert_eq!(s.weight(42), 0.25);
        assert_eq!(s.weight(43), 1.0);
    }

    #[test]
    fn empty_and_all_zero() {
        let mut s = Scheduler::new(0, Mode::Shuffle, 0);
        assert_eq!(s.next_index(), None);
        let mut s = Scheduler::new(1, Mode::Shuffle, 0);
        s.set_weight(0, 0.0);
        assert_eq!(s.next_index(), None);
    }
}
