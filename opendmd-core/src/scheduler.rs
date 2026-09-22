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

/// Orders a set of manifest indices.
///
/// Items are referred to by their index into the caller's entry slice; the
/// scheduler never looks at the entries themselves.
#[derive(Clone, Debug)]
pub struct Scheduler {
    items: Vec<usize>,
    weights: Vec<f32>,
    mode: Mode,
    rng: Rng,
    cursor: usize,
    history: VecDeque<usize>,
    no_repeat: usize,
}

impl Scheduler {
    /// All weights default to 1.0 and the no-repeat window to 0.
    pub fn new(items: &[usize], mode: Mode, seed: u64) -> Self {
        Scheduler {
            items: items.to_vec(),
            weights: vec![1.0; items.len()],
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

    /// Relative weight for a slot (`slot` is the position in the item list, not
    /// the manifest index). Weight 0 is never chosen.
    pub fn set_weight(&mut self, slot: usize, weight: f32) {
        if let Some(w) = self.weights.get_mut(slot) {
            *w = weight.max(0.0);
        }
    }

    /// Weight for a manifest index (first matching slot).
    pub fn set_weight_for(&mut self, item: usize, weight: f32) {
        if let Some(slot) = self.items.iter().position(|i| *i == item) {
            self.set_weight(slot, weight);
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> &[usize] {
        &self.items
    }

    /// Next manifest index, or `None` if there is nothing playable (empty list,
    /// or every weight is zero in shuffle mode).
    ///
    /// Not `Iterator::next`: a `Sequential` scheduler never terminates, and the
    /// engine needs to interleave `set_weight` calls between draws.
    pub fn next_index(&mut self) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        let slot = match self.mode {
            Mode::Sequential => {
                let slot = self.cursor;
                self.cursor = (self.cursor + 1) % self.items.len();
                slot
            }
            Mode::Shuffle => self.pick_weighted()?,
        };
        self.remember(slot);
        Some(self.items[slot])
    }

    fn remember(&mut self, slot: usize) {
        if self.no_repeat == 0 {
            return;
        }
        self.history.push_back(slot);
        while self.history.len() > self.no_repeat {
            self.history.pop_front();
        }
    }

    fn pick_weighted(&mut self) -> Option<usize> {
        let playable = self.weights.iter().filter(|w| **w > 0.0).count();
        if playable == 0 {
            return None;
        }
        // Never let the window exclude every playable item.
        let window = self.no_repeat.min(playable.saturating_sub(1));
        let blocked: Vec<usize> = self
            .history
            .iter()
            .rev()
            .take(window)
            .copied()
            .collect::<Vec<_>>();

        let eligible =
            |slot: usize, weights: &[f32]| weights[slot] > 0.0 && !blocked.contains(&slot);

        let total: f64 = (0..self.items.len())
            .filter(|s| eligible(*s, &self.weights))
            .map(|s| self.weights[s] as f64)
            .sum();
        if total <= 0.0 {
            return None;
        }

        let mut target = self.rng.next_f64() * total;
        for slot in 0..self.items.len() {
            if !eligible(slot, &self.weights) {
                continue;
            }
            target -= self.weights[slot] as f64;
            if target < 0.0 {
                return Some(slot);
            }
        }
        // Float rounding: fall back to the last eligible slot.
        (0..self.items.len())
            .rev()
            .find(|s| eligible(*s, &self.weights))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take(s: &mut Scheduler, n: usize) -> Vec<usize> {
        (0..n).filter_map(|_| s.next_index()).collect()
    }

    #[test]
    fn sequential_wraps_in_manifest_order() {
        let mut s = Scheduler::new(&[3, 7, 11], Mode::Sequential, 0);
        assert_eq!(take(&mut s, 7), vec![3, 7, 11, 3, 7, 11, 3]);
    }

    #[test]
    fn shuffle_is_reproducible_for_a_seed() {
        let items: Vec<usize> = (0..20).collect();
        let mut a = Scheduler::new(&items, Mode::Shuffle, 0xDEAD_BEEF);
        let mut b = Scheduler::new(&items, Mode::Shuffle, 0xDEAD_BEEF);
        let mut c = Scheduler::new(&items, Mode::Shuffle, 0xDEAD_BEEE);
        let run_a = take(&mut a, 40);
        assert_eq!(run_a, take(&mut b, 40));
        assert_ne!(run_a, take(&mut c, 40));
        // And it actually shuffles.
        assert_ne!(run_a[..20], items[..]);
    }

    #[test]
    fn no_repeat_window_is_respected() {
        let items: Vec<usize> = (0..10).collect();
        let mut s = Scheduler::new(&items, Mode::Shuffle, 42).with_no_repeat(4);
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
        let mut s = Scheduler::new(&[1, 2], Mode::Shuffle, 7).with_no_repeat(8);
        let run = take(&mut s, 20);
        assert_eq!(run.len(), 20);
        // With a 2-item list and a window of 1, it must alternate.
        for w in run.windows(2) {
            assert_ne!(w[0], w[1]);
        }
    }

    #[test]
    fn zero_weight_is_never_chosen() {
        let items: Vec<usize> = (0..5).collect();
        let mut s = Scheduler::new(&items, Mode::Shuffle, 1);
        s.set_weight_for(2, 0.0);
        s.set_weight_for(4, 0.0);
        let run = take(&mut s, 500);
        assert_eq!(run.len(), 500);
        assert!(!run.contains(&2));
        assert!(!run.contains(&4));
        assert!(run.contains(&0) && run.contains(&1) && run.contains(&3));
    }

    #[test]
    fn weights_bias_the_draw() {
        let items: Vec<usize> = (0..2).collect();
        let mut s = Scheduler::new(&items, Mode::Shuffle, 99);
        s.set_weight_for(0, 9.0);
        let run = take(&mut s, 1000);
        let zeros = run.iter().filter(|i| **i == 0).count();
        assert!(zeros > 800 && zeros < 980, "got {zeros}/1000");
    }

    #[test]
    fn empty_and_all_zero() {
        let mut s = Scheduler::new(&[], Mode::Shuffle, 0);
        assert_eq!(s.next_index(), None);
        let mut s = Scheduler::new(&[1], Mode::Shuffle, 0);
        s.set_weight(0, 0.0);
        assert_eq!(s.next_index(), None);
    }
}
