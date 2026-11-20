//! The WiFi's way back after a drop (firmware `net_worker.rs`, `wifi.rs`):
//! which step next, and when, so a network that has gone away is not
//! scanned for in a loop, never into a heap in crumbs, and a heap that
//! never recovers does not leave the panel offline for good.
//!
//! - **Straight back first.** The access point, channel and security of
//!   the last good connection are known ([`Reconnect::next`]'s `learned`):
//!   the outage's first step is a connect pinned to them, no scan (the
//!   driver looks on that one channel only).
//! - **Then a scan**, for the network's name only (the firmware's scan is
//!   directed at the SSID: the driver keeps only its access points), then
//!   the boot's ladder of connect stages. Every later round starts with
//!   the scan: the access point may have moved.
//! - **Then a pause** before the next round, growing with each round that
//!   failed ([`backoff_ms`]: none for the first two, then 1, 2, 4, 8 s, at
//!   most [`BACKOFF_MAX_MS`]), so a network that is away costs a scan every
//!   ~20 s rather than every 3.
//! - **Never into a low heap**: the reconnect's scan wants
//!   [`RESCAN_MIN_FREE`] free and a [`RESCAN_MIN_BLOCK`] block, a pinned
//!   connect [`DIRECT_MIN_FREE`] and [`DIRECT_MIN_BLOCK`] (the driver's
//!   association buffers). With room for the pinned connect only, that is
//!   tried again (it costs less); with room for neither, the step waits
//!   [`HEAP_RECHECK_MS`] and looks again. The setup page's scan of every
//!   network asks more ([`Heap::scan_ok`]). The display's own allocations
//!   come first: the heap is what it plays from.
//! - **A heap that never recovers**: after [`HEAP_STUCK_MS`] with no room
//!   for the scan (held back, or only the pinned connect tried, and the
//!   WiFi still down), [`Step::Restart`]: the firmware restarts, recorded
//!   (`wifi_heap`), and rejoins on a fresh heap.
//!
//! Times are milliseconds of uptime, compared wrapping. Pure: the firmware
//! passes the clock and the heap in.

/// Free heap and largest block the setup page's scan of every network
/// needs (`GET /api/wifi/scan`: the driver keeps every answering access
/// point on the heap until it is read).
pub const SCAN_MIN_FREE: u32 = 16 * 1024;
pub const SCAN_MIN_BLOCK: u32 = 4 * 1024;
/// Free heap and largest block the reconnect's scan needs: directed at the
/// network's name, it keeps that network's access points only.
pub const RESCAN_MIN_FREE: u32 = 11 * 1024;
pub const RESCAN_MIN_BLOCK: u32 = 2 * 1024 + 512;
/// Free heap and largest block a pinned connect needs.
pub const DIRECT_MIN_FREE: u32 = 10 * 1024;
pub const DIRECT_MIN_BLOCK: u32 = 2 * 1024;
/// How long a step held back for the heap waits before it looks again.
pub const HEAP_RECHECK_MS: u32 = 5_000;
/// The longest pause between rounds.
pub const BACKOFF_MAX_MS: u32 = 15_000;
/// No room for the reconnect's scan this long, the WiFi down all the
/// while: restart ([`Step::Restart`]).
pub const HEAP_STUCK_MS: u32 = 10 * 60 * 1000;

/// The heap as the step sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heap {
    pub free: u32,
    pub largest: u32,
}

impl Heap {
    /// Room for the setup page's scan of every network
    /// (`GET /api/wifi/scan`).
    pub fn scan_ok(self) -> bool {
        self.free >= SCAN_MIN_FREE && self.largest >= SCAN_MIN_BLOCK
    }

    /// Room for the reconnect's scan, directed at the network's name.
    pub fn rescan_ok(self) -> bool {
        self.free >= RESCAN_MIN_FREE && self.largest >= RESCAN_MIN_BLOCK
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
    /// The heap has not let the WiFi back for [`HEAP_STUCK_MS`]: restart.
    Restart,
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
    /// Rounds failed in a row (a round ends with a failed scan, or a
    /// failed pinned connect tried for want of room to scan).
    rounds: u32,
    /// The pinned connect failed this round: the scan is next.
    direct_failed: bool,
    /// No step before this (uptime ms), while `pause` is set.
    not_before: u32,
    pause: bool,
    /// The pause is a heap recheck's, not a backoff's.
    pause_heap: bool,
    /// Since when (uptime ms) there has been no room for the scan.
    low_since: Option<u32>,
}

impl Reconnect {
    pub fn new() -> Reconnect {
        Reconnect::default()
    }

    /// Rounds failed in a row.
    pub fn rounds(&self) -> u32 {
        self.rounds
    }

    /// A pause under way at `now` (a backoff, or a held step's recheck),
    /// as its [`Step::Wait`]: asked before the heap is read, so a pause
    /// costs no heap walk.
    pub fn waiting(&self, now: u32) -> Option<Step> {
        if !self.pause {
            return None;
        }
        let left = self.not_before.wrapping_sub(now) as i32;
        (left > 0).then_some(Step::Wait { ms: left as u32, heap: self.pause_heap })
    }

    /// What to do at `now` (ms). `learned`: the last good connection is
    /// known for this network.
    pub fn next(&mut self, now: u32, learned: bool, heap: Heap) -> Step {
        if let Some(w) = self.waiting(now) {
            return w;
        }
        self.pause = false;
        let direct_ok = learned && heap.direct_ok();
        let step = if direct_ok && self.rounds == 0 && !self.direct_failed {
            Step::Direct
        } else if heap.rescan_ok() {
            Step::Scan
        } else if direct_ok {
            // No room to scan: the pinned connect again, which costs less.
            Step::Direct
        } else {
            Step::Wait { ms: HEAP_RECHECK_MS, heap: true }
        };
        if heap.rescan_ok() {
            self.low_since = None;
        } else {
            let since = *self.low_since.get_or_insert(now);
            if now.wrapping_sub(since) >= HEAP_STUCK_MS {
                return Step::Restart;
            }
        }
        if let Step::Wait { ms, .. } = step {
            self.pause = true;
            self.pause_heap = true;
            self.not_before = now.wrapping_add(ms);
        }
        step
    }

    /// `step` failed at `now`.
    pub fn failed(&mut self, now: u32, step: Step) {
        match step {
            Step::Direct if !self.direct_failed && self.rounds == 0 => self.direct_failed = true,
            Step::Direct | Step::Scan => {
                self.direct_failed = false;
                self.rounds = self.rounds.saturating_add(1);
                let pause = backoff_ms(self.rounds);
                self.pause = pause > 0;
                self.pause_heap = false;
                self.not_before = now.wrapping_add(pause);
            }
            Step::Wait { .. } | Step::Restart => {}
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
    /// Room for a pinned connect, not for a scan.
    const TIGHT: Heap = Heap { free: 10_500, largest: 2_200 };
    const HELD: Step = Step::Wait { ms: HEAP_RECHECK_MS, heap: true };

    #[test]
    fn straight_back_first_then_scans() {
        let mut r = Reconnect::new();
        assert_eq!(r.next(0, true, ROOMY), Step::Direct);
        r.failed(0, Step::Direct);
        assert_eq!(r.next(15_000, true, ROOMY), Step::Scan);
        // Later rounds start with the scan: the access point may have moved.
        r.failed(18_000, Step::Scan);
        assert_eq!(r.next(18_000, true, ROOMY), Step::Scan);
        r.failed(21_000, Step::Scan);
        assert_eq!(r.next(21_000 + backoff_ms(2), true, ROOMY), Step::Scan);
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
                    assert_ne!(s, Step::Restart);
                    if s == Step::Scan {
                        scans.push(now);
                    }
                    // A pinned connect waits out its 5 s; a scan ~3 s.
                    now += if s == Step::Direct { 5_000 } else { 3_000 };
                    r.failed(now, s);
                }
            }
        }
        let gaps: Vec<u32> = scans.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.windows(2).all(|g| g[1] >= g[0]), "never faster: {gaps:?}");
        assert_eq!(*gaps.last().unwrap(), 3_000 + BACKOFF_MAX_MS);
        assert!(scans.len() < 40, "{} scans in 10 min", scans.len());
        assert_eq!((backoff_ms(0), backoff_ms(1), backoff_ms(2)), (0, 0, 1000));
        assert_eq!(backoff_ms(100), BACKOFF_MAX_MS);
    }

    #[test]
    fn never_into_a_heap_in_crumbs() {
        let mut r = Reconnect::new();
        assert_eq!(r.next(0, true, CRUMBS), HELD);
        // The recheck: no step (and no heap walk needed) until it is due.
        assert_eq!(r.waiting(1_000), Some(Step::Wait { ms: HEAP_RECHECK_MS - 1_000, heap: true }));
        assert_eq!(r.next(1_000, true, ROOMY), Step::Wait { ms: HEAP_RECHECK_MS - 1_000, heap: true });
        assert_eq!(r.waiting(HEAP_RECHECK_MS), None);
        assert_eq!(r.next(HEAP_RECHECK_MS, true, ROOMY), Step::Direct);
        assert_eq!(Reconnect::new().next(0, false, CRUMBS), HELD);
        // Room for a pinned connect but not a scan: the connect, again
        // after it failed, never the scan.
        let mut t = Reconnect::new();
        assert_eq!(t.next(0, true, TIGHT), Step::Direct);
        t.failed(0, Step::Direct);
        assert_eq!(t.next(0, true, TIGHT), Step::Direct);
        assert_eq!(Reconnect::new().next(0, false, TIGHT), HELD);
        // The reconnect's directed scan asks less than the setup page's.
        let mid = Heap { free: 12_000, largest: 3_000 };
        assert!(mid.rescan_ok() && !mid.scan_ok());
        t.failed(0, Step::Direct);
        assert_eq!(t.next(60_000, true, mid), Step::Scan);
        // A wait or a restart is not a failure.
        let before = t;
        t.failed(0, HELD);
        t.failed(0, Step::Restart);
        assert_eq!(t, before);
        assert!(!CRUMBS.scan_ok() && ROOMY.scan_ok());
    }

    #[test]
    fn a_heap_that_never_recovers_restarts() {
        // Held back all along: a restart after HEAP_STUCK_MS, not before.
        let mut r = Reconnect::new();
        let mut now = 0u32;
        let restart_at = loop {
            match r.next(now, true, CRUMBS) {
                Step::Restart => break now,
                Step::Wait { heap: true, ms } => now += ms,
                s => panic!("{s:?} in a heap in crumbs"),
            }
        };
        assert!((HEAP_STUCK_MS..HEAP_STUCK_MS + HEAP_RECHECK_MS).contains(&restart_at), "{restart_at}");
        // Only the pinned connect possible, and failing: a restart too.
        let mut t = Reconnect::new();
        let mut now = 0u32;
        let mut tries = 0;
        let restart_at = loop {
            match t.next(now, true, TIGHT) {
                Step::Restart => break now,
                Step::Wait { ms, .. } => now += ms,
                Step::Direct => {
                    tries += 1;
                    now += 5_000;
                    t.failed(now, Step::Direct);
                }
                Step::Scan => panic!("a scan without the room for one"),
            }
        };
        assert!(restart_at >= HEAP_STUCK_MS && tries > 10, "{restart_at} {tries}");
        // Room to scan once in the ten minutes: the count starts again.
        let mut u = Reconnect::new();
        assert_eq!(u.next(0, false, CRUMBS), HELD);
        assert_eq!(u.next(HEAP_STUCK_MS - 10_000, false, ROOMY), Step::Scan);
        u.failed(HEAP_STUCK_MS - 7_000, Step::Scan);
        assert_eq!(u.next(HEAP_STUCK_MS + 1_000, false, CRUMBS), HELD);
    }

    #[test]
    fn a_backoff_pause_is_not_a_heap_wait() {
        let mut r = Reconnect::new();
        for t in [0, 3_000] {
            r.failed(t, Step::Scan);
        }
        assert_eq!(r.waiting(3_500), Some(Step::Wait { ms: 500, heap: false }));
    }

    #[test]
    fn the_clock_wraps() {
        let mut r = Reconnect::new();
        let near = u32::MAX - 500;
        for _ in 0..3 {
            r.failed(near, Step::Scan);
        }
        assert!(matches!(r.next(near, true, ROOMY), Step::Wait { heap: false, .. }));
        assert_eq!(r.next(near.wrapping_add(backoff_ms(3)), true, ROOMY), Step::Scan);
    }
}
