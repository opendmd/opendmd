//! The WiFi's way back after a drop (firmware `net_worker.rs`, `wifi.rs`):
//! which step next, and when, so a network that has gone away is not
//! scanned for in a loop, and never into a heap in crumbs.
//!
//! - **Straight back first.** The access point, channel and security of
//!   the last good connection are known ([`Reconnect::next`]'s `learned`):
//!   the first step is a connect pinned to them, no scan (the driver looks
//!   on that one channel only).
//! - **Then a scan**, for the network's name only (the firmware's scan is
//!   directed at the SSID and keeps nothing of the rest), then the boot's
//!   ladder of connect stages.
//! - **Then a pause** before the next round, growing with each round that
//!   failed ([`backoff_ms`]: none for the first two, then 1, 2, 4, 8 s, at
//!   most [`BACKOFF_MAX_MS`]), so a network that is away costs a scan every
//!   ~30 s rather than every 3.
//! - **Never into a low heap**: a scan wants [`SCAN_MIN_FREE`] free and a
//!   [`SCAN_MIN_BLOCK`] block, a pinned connect [`DIRECT_MIN_FREE`] and
//!   [`DIRECT_MIN_BLOCK`] (the driver's association buffers); short of that
//!   the step waits [`HEAP_RECHECK_MS`] and looks again. The display's own
//!   allocations come first: the heap is what it plays from.
//!
//! Times are milliseconds of uptime, compared wrapping. Pure: the firmware
//! passes the clock and the heap in.

/// Free heap a scan needs (the driver keeps every answering access point
/// on the heap until it is read).
pub const SCAN_MIN_FREE: u32 = 16 * 1024;
/// The largest free block a scan needs.
pub const SCAN_MIN_BLOCK: u32 = 4 * 1024;
/// Free heap and largest block a pinned connect needs.
pub const DIRECT_MIN_FREE: u32 = 10 * 1024;
pub const DIRECT_MIN_BLOCK: u32 = 2 * 1024;
/// How long a step held back for the heap waits before it looks again.
pub const HEAP_RECHECK_MS: u32 = 5_000;
/// The longest pause between rounds.
pub const BACKOFF_MAX_MS: u32 = 15_000;

/// The heap as the step sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heap {
    pub free: u32,
    pub largest: u32,
}

impl Heap {
    /// Room for a scan (`GET /api/wifi/scan` asks the same).
    pub fn scan_ok(self) -> bool {
        self.free >= SCAN_MIN_FREE && self.largest >= SCAN_MIN_BLOCK
    }

    fn direct_ok(self) -> bool {
        self.free >= DIRECT_MIN_FREE && self.largest >= DIRECT_MIN_BLOCK
    }
}

/// The next step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Connect pinned to the last good access point, channel and security.
    Direct,
    /// Scan for the network, then the connect ladder (as at boot).
    Scan,
    /// Not yet: `ms` more (`heap`: held back for the heap, not a pause).
    Wait { ms: u32, heap: bool },
}

/// The pause after `rounds` failed rounds in a row.
pub fn backoff_ms(rounds: u32) -> u32 {
    match rounds {
        0 | 1 => 0,
        n => (1000u32 << (n - 2).min(4)).min(BACKOFF_MAX_MS),
    }
}

/// One outage's progress: from a drop (or a boot that did not connect) to
/// the next connection ([`Reconnect::connected`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reconnect {
    /// Rounds failed in a row (a round ends with a failed scan).
    rounds: u32,
    /// The pinned connect failed this round: the scan is next.
    direct_failed: bool,
    /// No step before this (uptime ms), when `pause` is set.
    not_before: u32,
    pause: bool,
}

impl Reconnect {
    pub fn new() -> Reconnect {
        Reconnect::default()
    }

    /// Rounds failed in a row.
    pub fn rounds(&self) -> u32 {
        self.rounds
    }

    /// What to do at `now` (ms). `learned`: the last good connection is
    /// known for this network.
    pub fn next(&self, now: u32, learned: bool, heap: Heap) -> Step {
        if self.pause {
            let left = self.not_before.wrapping_sub(now) as i32;
            if left > 0 {
                return Step::Wait { ms: left as u32, heap: false };
            }
        }
        let wait = Step::Wait { ms: HEAP_RECHECK_MS, heap: true };
        if learned && !self.direct_failed {
            return if heap.direct_ok() { Step::Direct } else { wait };
        }
        if heap.scan_ok() {
            Step::Scan
        } else if learned && heap.direct_ok() {
            // No room to scan: the pinned connect again, which costs less.
            Step::Direct
        } else {
            wait
        }
    }

    /// `step` failed at `now`.
    pub fn failed(&mut self, now: u32, step: Step) {
        match step {
            Step::Direct if !self.direct_failed => self.direct_failed = true,
            Step::Direct | Step::Scan => {
                self.direct_failed = false;
                self.rounds = self.rounds.saturating_add(1);
                let pause = backoff_ms(self.rounds);
                self.pause = pause > 0;
                self.not_before = now.wrapping_add(pause);
            }
            Step::Wait { .. } => {}
        }
    }

    /// Connected: the next outage starts afresh.
    pub fn connected(&mut self) {
        *self = Reconnect::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOMY: Heap = Heap { free: 40_000, largest: 20_000 };
    /// A test panel's last heartbeat before its out-of-memory crash
    /// (1.1.0-beta.2).
    const CRUMBS: Heap = Heap { free: 9_676, largest: 432 };

    #[test]
    fn straight_back_first_then_a_scan() {
        let mut r = Reconnect::new();
        assert_eq!(r.next(0, true, ROOMY), Step::Direct);
        r.failed(0, Step::Direct);
        assert_eq!(r.next(15_000, true, ROOMY), Step::Scan);
        // A network never joined this run (a boot that did not connect):
        // the scan and the ladder, as at boot.
        assert_eq!(Reconnect::new().next(0, false, ROOMY), Step::Scan);
        r.connected();
        assert_eq!(r, Reconnect::new());
    }

    #[test]
    fn rounds_back_off() {
        let mut r = Reconnect::new();
        let mut now = 0u32;
        let mut scans = Vec::new();
        // The network is away for ten minutes.
        while now < 600_000 {
            match r.next(now, true, ROOMY) {
                Step::Wait { ms, heap } => {
                    assert!(!heap);
                    now += ms;
                }
                s => {
                    if s == Step::Scan {
                        scans.push(now);
                    }
                    // A pinned connect waits out its 15 s; a scan ~3 s.
                    now += if s == Step::Direct { 15_000 } else { 3_000 };
                    r.failed(now, s);
                }
            }
        }
        let gaps: Vec<u32> = scans.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.windows(2).all(|g| g[1] >= g[0]), "never faster: {gaps:?}");
        assert_eq!(*gaps.last().unwrap(), 15_000 + 3_000 + BACKOFF_MAX_MS);
        assert!(scans.len() < 25, "{} scans in 10 min", scans.len());
        // The first two rounds go at once (a boot's two tries before the
        // setup network are not held back).
        assert_eq!((backoff_ms(0), backoff_ms(1), backoff_ms(2)), (0, 0, 1000));
        assert_eq!(backoff_ms(100), BACKOFF_MAX_MS);
    }

    #[test]
    fn never_into_a_heap_in_crumbs() {
        let mut r = Reconnect::new();
        assert_eq!(r.next(0, true, CRUMBS), Step::Wait { ms: HEAP_RECHECK_MS, heap: true });
        assert_eq!(r.next(0, false, CRUMBS), Step::Wait { ms: HEAP_RECHECK_MS, heap: true });
        // Room for a pinned connect but not a scan: the connect again.
        let tight = Heap { free: 12_000, largest: 3_000 };
        r.failed(0, Step::Direct);
        assert_eq!(r.next(0, true, tight), Step::Direct);
        assert_eq!(r.next(0, false, tight), Step::Wait { ms: HEAP_RECHECK_MS, heap: true });
        // A wait is not a failure.
        let before = r;
        r.failed(0, Step::Wait { ms: 1, heap: true });
        assert_eq!(r, before);
        assert!(!CRUMBS.scan_ok() && ROOMY.scan_ok());
    }

    #[test]
    fn the_clock_wraps() {
        let mut r = Reconnect::new();
        let near = u32::MAX - 500;
        for _ in 0..3 {
            r.failed(near, Step::Scan);
        }
        assert!(matches!(r.next(near, true, ROOMY), Step::Wait { heap: false, .. }));
        assert_eq!(r.next(near.wrapping_add(backoff_ms(3)), true, ROOMY), Step::Direct);
    }
}
