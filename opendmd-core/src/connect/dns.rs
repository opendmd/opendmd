//! The server's lookup, as the Connect worker paces it (host-testable; the
//! lookup itself runs on the firmware's `connect-dns` task).
//!
//! lwIP asks each configured DNS server in turn, up to 7 s each
//! (`CONFIG_LWIP_DNS_MAX_SERVERS` = 3: 21 s at worst), so [`DNS_WAIT`] is
//! longer than that. A lookup not answered by then counts as one failure;
//! its task cannot be stopped, so it is left to finish ("given up"). Its
//! late answer then counts for nothing more: a late address is still used
//! at the next attempt, a late error is dropped and a fresh lookup starts.
//! Only one lookup task runs at a time: one still running at the next
//! attempt is reported as stuck (that attempt's one failure). DNS failures
//! back off no further than [`DNS_BACKOFF_CAP`]: a lookup costs the
//! server nothing, and a resolver that comes back should be used soon.

use core::time::Duration;

/// How long a lookup is waited for before it counts as failed (lwIP's
/// worst case is 3 × 7 s).
pub const DNS_WAIT: Duration = Duration::from_secs(25);
/// The most a DNS failure backs off.
pub const DNS_BACKOFF_CAP: Duration = Duration::from_secs(30);

/// The lookup task's record as the worker finds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Job {
    /// For the host the worker wants now.
    pub same_host: bool,
    /// `Some(true)`: an address; `Some(false)`: an error; `None`: running.
    pub answer: Option<bool>,
    /// Already counted as failed (it ran past [`DNS_WAIT`]).
    pub given_up: bool,
    /// Since it started.
    pub elapsed: Duration,
}

/// What the worker does with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// No usable record: start a lookup.
    Start,
    /// Running, inside the wait: look again shortly.
    Poll,
    /// The address: use it (late or not).
    Use,
    /// An error answered in time: one failure.
    Fail,
    /// Running past the wait: mark it given up, one failure.
    GiveUp,
    /// Given up and still running at the next attempt: no new task can
    /// start; report it, one failure (this attempt's).
    Stuck,
}

/// The step for `job` (`None`: no record), a lookup being waited for
/// `wait` ([`DNS_WAIT`]; an emulator lane's shorter one).
#[cfg_attr(target_arch = "xtensa", optimize(size))]
pub fn step(job: Option<Job>, wait: Duration) -> Step {
    let Some(j) = job else { return Step::Start };
    match j.answer {
        Some(true) if j.same_host => Step::Use,
        Some(false) if j.same_host && !j.given_up => Step::Fail,
        // A late error (counted already) or another host's answer.
        Some(_) => Step::Start,
        None if j.given_up => Step::Stuck,
        None if j.elapsed > wait => Step::GiveUp,
        None => Step::Poll,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(answer: Option<bool>, given_up: bool, secs: u64) -> Option<Job> {
        Some(Job { same_host: true, answer, given_up, elapsed: Duration::from_secs(secs) })
    }

    /// A lookup that runs lwIP's worst case out: waited for, then one
    /// failure; its late error is not a second; the next attempt starts a
    /// fresh lookup.
    #[test]
    fn a_slow_failure_counts_once() {
        assert!(DNS_WAIT > Duration::from_secs(3 * 7));
        let mut fails = 0;
        let mut given_up = false;
        for s in 0..=21 {
            match step(job(None, given_up, s), DNS_WAIT) {
                Step::Poll => {}
                other => panic!("{s} s: {other:?}"),
            }
        }
        assert_eq!(step(job(None, false, 26), DNS_WAIT), Step::GiveUp);
        fails += 1;
        given_up = true;
        // The late error arrives during the backoff; the next attempt.
        match step(job(Some(false), given_up, 30), DNS_WAIT) {
            Step::Fail | Step::GiveUp | Step::Stuck => fails += 1,
            Step::Start => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(fails, 1);
    }

    #[test]
    fn a_late_address_is_used() {
        assert_eq!(step(job(Some(true), true, 40), DNS_WAIT), Step::Use);
        assert_eq!(step(job(Some(true), false, 1), DNS_WAIT), Step::Use);
    }

    #[test]
    fn an_error_in_time_fails_once() {
        assert_eq!(step(job(Some(false), false, 3), DNS_WAIT), Step::Fail);
    }

    /// Given up and still running at the next attempt: reported as stuck,
    /// never shown as a lookup in progress (`Poll`).
    #[test]
    fn a_stuck_lookup_is_reported_not_polled() {
        assert_eq!(step(job(None, true, 31), DNS_WAIT), Step::Stuck);
        assert_eq!(step(job(None, true, 600), DNS_WAIT), Step::Stuck);
    }

    #[test]
    fn another_hosts_answer_starts_a_fresh_lookup() {
        let other = Some(Job { same_host: false, answer: Some(true), given_up: false, elapsed: Duration::from_secs(1) });
        assert_eq!(step(other, DNS_WAIT), Step::Start);
        assert_eq!(step(None, DNS_WAIT), Step::Start);
        // Another host's lookup still running: one task at a time.
        let running = Some(Job { same_host: false, answer: None, given_up: false, elapsed: Duration::from_secs(1) });
        assert_eq!(step(running, DNS_WAIT), Step::Poll);
    }
}
