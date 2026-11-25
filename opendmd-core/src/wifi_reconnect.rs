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
//! - **A heap that never recovers**: after [`HEAP_STUCK_MS`] in all,
//!   this outage, with no room for the scan (held back, or only the pinned
//!   connect tried), [`Step::Restart`] `{ heap: true }`: the firmware
//!   restarts, recorded (`wifi_heap`), and rejoins on a fresh heap. In all,
//!   not in a row: a heap that has room for the scan now and then, for a
//!   look or two, does not start the count again (a test panel's heap
//!   hovered at the scan's limit for hours, 2026-11-21, offline all along).
//! - **A driver that stops hearing the network**: after [`RESET_ROUNDS`]
//!   failed rounds in a row, and every [`RESET_ROUNDS`] more,
//!   [`Step::Reset`]: the radio's driver stopped and started again, then
//!   the scan. Only with room for the setup page's scan ([`Heap::scan_ok`]):
//!   the start takes the driver's buffers again.
//! - **Down for good**: [`DOWN_STUCK_MS`] since the outage began (the
//!   driver restarted several times by then, if the heap had room),
//!   [`Step::Restart`] `{ heap: false }`: the firmware restarts, recorded
//!   (`wifi_stuck`): what brought the same test panel back at once. Once
//!   per outage: the firmware tells the boot after it
//!   ([`Reconnect::no_stuck_restart`]), so a network that is really away
//!   does not restart the panel every hour.
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
/// Failed rounds in a row before the driver is restarted ([`Step::Reset`]),
/// and between one restart and the next: at the longest pause, about every
/// two minutes.
pub const RESET_ROUNDS: u32 = 6;
/// The WiFi down this long, whatever the heap: restart ([`Step::Restart`]
/// `{ heap: false }`), once an outage.
pub const DOWN_STUCK_MS: u32 = 60 * 60 * 1000;

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
    /// Restart the radio's driver (stop, start), then [`Step::Scan`]'s scan
    /// and ladder; a failure is a failed round, as the scan's.
    Reset,
    /// Not yet: `ms` more (`heap`: held back for the heap, not a pause).
    Wait { ms: u32, heap: bool },
    /// Restart the firmware. `heap`: the heap has not let the WiFi back for
    /// [`HEAP_STUCK_MS`]; otherwise down [`DOWN_STUCK_MS`].
    Restart { heap: bool },
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
    /// How long (ms) this outage has had no room for the scan, in all.
    low_ms: u32,
    /// The last look (uptime ms), and whether it had no room for the scan.
    last_look: Option<(u32, bool)>,
    /// When the outage's first look came (uptime ms).
    down_since: Option<u32>,
    /// Driver restarts this outage, and the round of the last one.
    resets: u32,
    reset_round: u32,
    /// The once-an-outage firmware restart is spent (the boot after one).
    stuck_spent: bool,
}

impl Reconnect {
    pub fn new() -> Reconnect {
        Reconnect::default()
    }

    /// Rounds failed in a row.
    pub fn rounds(&self) -> u32 {
        self.rounds
    }

    /// Driver restarts this outage.
    pub fn resets(&self) -> u32 {
        self.resets
    }

    /// The boot after a [`Step::Restart`] `{ heap: false }` (`wifi_stuck`),
    /// still down: no second one this outage (the heap's still holds).
    pub fn no_stuck_restart(&mut self) {
        self.stuck_spent = true;
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
        let low = !heap.rescan_ok();
        // Since the last look, counted as low if that look was.
        if let Some((at, true)) = self.last_look {
            self.low_ms = self.low_ms.saturating_add(now.wrapping_sub(at));
        }
        self.last_look = Some((now, low));
        let since = *self.down_since.get_or_insert(now);
        if low && self.low_ms >= HEAP_STUCK_MS {
            return Step::Restart { heap: true };
        }
        if !self.stuck_spent && now.wrapping_sub(since) >= DOWN_STUCK_MS {
            return Step::Restart { heap: false };
        }
        let direct_ok = learned && heap.direct_ok();
        let reset_due = self.rounds >= RESET_ROUNDS && self.rounds - self.reset_round >= RESET_ROUNDS;
        let step = if direct_ok && self.rounds == 0 && !self.direct_failed {
            Step::Direct
        } else if reset_due && heap.scan_ok() {
            Step::Reset
        } else if !low {
            Step::Scan
        } else if direct_ok {
            // No room to scan: the pinned connect again, which costs less.
            Step::Direct
        } else {
            Step::Wait { ms: HEAP_RECHECK_MS, heap: true }
        };
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
            Step::Direct | Step::Scan | Step::Reset => {
                if step == Step::Reset {
                    self.resets = self.resets.saturating_add(1);
                    self.reset_round = self.rounds;
                }
                self.direct_failed = false;
                self.rounds = self.rounds.saturating_add(1);
                let pause = backoff_ms(self.rounds);
                self.pause = pause > 0;
                self.pause_heap = false;
                self.not_before = now.wrapping_add(pause);
            }
            Step::Wait { .. } | Step::Restart { .. } => {}
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
                    assert!(!matches!(s, Step::Restart { .. }));
                    // (A driver restart's round starts with the scan too.)
                    if matches!(s, Step::Scan | Step::Reset) {
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
        t.failed(0, Step::Restart { heap: true });
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
                Step::Restart { heap: true } => break now,
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
                Step::Restart { heap: true } => break now,
                Step::Wait { ms, .. } => now += ms,
                Step::Direct => {
                    tries += 1;
                    now += 5_000;
                    t.failed(now, Step::Direct);
                }
                s => panic!("{s:?} without the room for a scan"),
            }
        };
        assert!(restart_at >= HEAP_STUCK_MS && tries > 10, "{restart_at} {tries}");
        // Room to scan once in the ten minutes: the time it had room is not
        // counted, the time before and after is.
        let mut u = Reconnect::new();
        assert_eq!(u.next(0, false, CRUMBS), HELD);
        assert_eq!(u.next(HEAP_STUCK_MS - 10_000, false, ROOMY), Step::Scan);
        u.failed(HEAP_STUCK_MS - 7_000, Step::Scan);
        assert_eq!(u.next(HEAP_STUCK_MS + 1_000, false, CRUMBS), HELD);
        assert_eq!(u.next(HEAP_STUCK_MS + 1_000 + HEAP_RECHECK_MS, false, CRUMBS), HELD);
        assert_eq!(u.next(HEAP_STUCK_MS + 10_000 + HEAP_RECHECK_MS, false, CRUMBS), Step::Restart { heap: true });
    }

    /// The heap at a test panel's `wifi_heap` restart (2026-11-21), after
    /// hours offline: room for a pinned connect, a block short of a scan.
    const RECORDED: Heap = Heap { free: 11_684, largest: 2_304 };
    /// Just enough for the reconnect's scan.
    const EDGE: Heap = Heap { free: 11_800, largest: 2_600 };

    #[test]
    fn a_heap_at_the_scans_limit_restarts_within_the_half_hour() {
        // The heap now with room for the scan, now without, at every other
        // look: the old rule (ten minutes in a row) never fired.
        let mut r = Reconnect::new();
        let (mut now, mut looks) = (0u32, 0u32);
        let restart_at = loop {
            if let Some(Step::Wait { ms, .. }) = r.waiting(now) {
                now += ms;
                continue;
            }
            let heap = if looks % 2 == 0 { RECORDED } else { EDGE };
            looks += 1;
            match r.next(now, true, heap) {
                Step::Restart { heap: true } => break now,
                Step::Restart { heap: false } => panic!("the hour's restart first, at {now}"),
                Step::Wait { ms, .. } => now += ms,
                // A pinned connect's 5 s; a scan that does not find it, ~2 s.
                s => {
                    now += if s == Step::Direct { 5_000 } else { 2_000 };
                    r.failed(now, s);
                }
            }
        };
        assert!((HEAP_STUCK_MS..3 * HEAP_STUCK_MS).contains(&restart_at), "{restart_at}");
        assert!(looks > 50, "{looks} looks");
    }

    #[test]
    fn a_driver_that_hears_nothing_is_restarted() {
        let mut r = Reconnect::new();
        let mut now = 0u32;
        let mut at = Vec::new();
        while now < 20 * 60 * 1000 {
            match r.next(now, true, ROOMY) {
                Step::Wait { ms, .. } => now += ms,
                s => {
                    if s == Step::Reset {
                        at.push(r.rounds());
                    }
                    now += if s == Step::Direct { 5_000 } else { 2_000 };
                    r.failed(now, s);
                }
            }
        }
        assert_eq!(&at[..3], &[RESET_ROUNDS, 2 * RESET_ROUNDS, 3 * RESET_ROUNDS]);
        assert_eq!(r.resets() as usize, at.len());
        // Not without room for a full scan: the scan alone.
        let mid = Heap { free: 12_000, largest: 3_000 };
        let mut m = Reconnect::new();
        for _ in 0..3 * RESET_ROUNDS {
            let s = m.next(now, false, mid);
            assert_eq!(s, Step::Scan);
            m.failed(now, s);
            now += BACKOFF_MAX_MS;
        }
        assert_eq!(m.next(now, false, ROOMY), Step::Reset);
    }

    #[test]
    fn down_for_an_hour_restarts_once() {
        let run = |r: &mut Reconnect, until: u32| -> Option<u32> {
            let mut now = 0u32;
            while now < until {
                match r.next(now, true, ROOMY) {
                    Step::Restart { heap: false } => return Some(now),
                    Step::Restart { heap: true } => panic!("a heap restart in a roomy heap"),
                    Step::Wait { ms, .. } => now += ms,
                    s => {
                        now += if s == Step::Direct { 5_000 } else { 2_000 };
                        r.failed(now, s);
                    }
                }
            }
            None
        };
        let mut r = Reconnect::new();
        let at = run(&mut r, 2 * DOWN_STUCK_MS).expect("a restart");
        assert!((DOWN_STUCK_MS..DOWN_STUCK_MS + BACKOFF_MAX_MS + 2_000).contains(&at), "{at}");
        assert!(r.resets() >= 10, "{} driver restarts first", r.resets());
        // The boot after it: no second one, however long.
        let mut b = Reconnect::new();
        b.no_stuck_restart();
        assert_eq!(run(&mut b, 3 * DOWN_STUCK_MS), None);
        // Joined meanwhile: the next outage may restart again.
        b.connected();
        assert_eq!(b, Reconnect::new());
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
