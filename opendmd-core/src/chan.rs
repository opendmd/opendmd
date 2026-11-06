//! A multi-producer, single-consumer channel on a `Mutex` and a `Condvar`:
//! what the firmware uses instead of `std::sync::mpsc`, whose `channel()`
//! corrupts the calling task's registers once in a while on this target.
//!
//! **Why not `std::sync::mpsc`.** Its channel is built on a type aligned to
//! 64 bytes (`CachePadded`), constructed on the stack, so every
//! `std::sync::mpmc::channel::<T>` instantiation starts by realigning its
//! stack pointer: `entry a1, 352`, then `a1 += 64 - (a1 & 63)`, a plain `add`
//! (the esp Rust toolchain's LLVM). The windowed ABI does not allow that:
//! the caller's `a0`-`a3` are spilled to, and reloaded from, the 16 bytes
//! below the callee's `a1`, and moving `a1` needs `MOVSP` (which moves them).
//! An interrupt in the few instructions between `entry` and the `add` (an
//! interrupt spills every live window; the function is in flash, so a cache
//! miss there stretches the gap to microseconds) stores the caller's
//! registers below the old `a1`; `retw` reloads them from below the new one,
//! i.e. from stale stack. The caller then runs with garbage in `a0`-`a7`.
//! That was the `/api/play` panic (CLAUDE.md, Gotchas: "No
//! `std::sync::mpsc` at all"): the handler's `Arc<Ctx>` pointer came back 0
//! and its return address 0xa right after `mpsc::channel()`; the `diag-heap`
//! build's stress run hit it again as a double exception spilling a window
//! with `a1` = 1. Every request that waits for the main task (`/api/play`,
//! `/api/config`, `/api/pause`, …) made a channel, so each was a small
//! chance of a panic.
//!
//! This one has no over-aligned type in it (`tools/check-stack-realign.py`,
//! run by `make fw-build`/`fw-release`, fails a build that has any function
//! realigning its stack), and it is smaller: an `Arc`, a `Mutex`, a `Condvar`
//! and a `VecDeque` that grows to the most messages queued at once, against
//! std's 64-byte-aligned counter and 31-slot blocks.
//!
//! The API is the subset of `std::sync::mpsc` the firmware used, with std's
//! error types, so call sites read the same. Disconnection works as in std:
//! `send` fails once the receiver is gone; `try_recv`/`recv_timeout` report
//! `Disconnected` once every sender is gone and the queue is empty.
//!
//! Differences from std, deliberate: the channel is always unbounded (no
//! `sync_channel`), there is no `iter`/`try_iter`, and `recv_timeout` is
//! loop-based so a spurious wakeup never ends it early. `recv` is here for
//! completeness and the tests; the firmware only uses `recv_timeout` and
//! `try_recv`. Messages still queued when the receiver drops are dropped
//! then, as in std.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub use std::sync::mpsc::{RecvError, RecvTimeoutError, SendError, TryRecvError};

struct State<T> {
    queue: VecDeque<T>,
    senders: usize,
    receiver: bool,
    /// A sender said it is doing the work ([`Sender::claim`]).
    claimed: bool,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    ready: Condvar,
}

impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The sending half; clone it for more producers.
pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

/// The receiving half (one consumer).
pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
}

/// A new channel, unbounded like `std::sync::mpsc::channel`.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            queue: VecDeque::new(),
            senders: 1,
            receiver: true,
            claimed: false,
        }),
        ready: Condvar::new(),
    });
    (
        Sender {
            shared: shared.clone(),
        },
        Receiver { shared },
    )
}

impl<T> Sender<T> {
    /// Queue `t`, or hand it back if the receiver is gone.
    pub fn send(&self, t: T) -> Result<(), SendError<T>> {
        let mut state = self.shared.lock();
        if !state.receiver {
            return Err(SendError(t));
        }
        state.queue.push_back(t);
        drop(state);
        self.shared.ready.notify_one();
        Ok(())
    }
}

impl<T> Sender<T> {
    /// Say the work this channel will answer is being done now: `false` if
    /// the receiver has withdrawn it already ([`Receiver::answer_within`]),
    /// and then the work must not be done at all. Atomic with the
    /// withdrawal: exactly one of the two wins.
    #[inline(never)]
    pub fn claim(&self) -> bool {
        let mut state = self.shared.lock();
        if !state.receiver {
            return false;
        }
        state.claimed = true;
        true
    }
}

/// What [`Receiver::answer_within`] came back with.
#[derive(Debug, PartialEq, Eq)]
pub enum Handoff<T> {
    /// The answer.
    Answered(T),
    /// Claimed and under way, not answered in time: it will be done.
    Taken,
    /// Not claimed in time (or never can be: every sender gone), and now
    /// withdrawn: it will not be done.
    Withdrawn,
}

impl<T> Receiver<T> {
    /// A request handed to another task that may be busy: wait at most
    /// `take` for the answer; if none came, withdraw the request unless it
    /// has been claimed ([`Sender::claim`]: a later `claim` then fails, so
    /// nothing is done), and if it has, wait at most `answer` more. For a
    /// caller that must not be held long by a task that is slow to look at
    /// its queue (the httpd task waiting on the main loop). Built on
    /// `recv_timeout`, so it adds little code per message type.
    pub fn answer_within(&self, take: Duration, answer: Duration) -> Handoff<T> {
        if let Ok(t) = self.recv_timeout(take) {
            return Handoff::Answered(t);
        }
        if self.withdraw() {
            return Handoff::Withdrawn;
        }
        match self.recv_timeout(answer) {
            Ok(t) => Handoff::Answered(t),
            Err(_) => Handoff::Taken,
        }
    }

    /// Withdraw the request unless a sender has claimed it: `true` if
    /// withdrawn.
    fn withdraw(&self) -> bool {
        let mut state = self.shared.lock();
        if state.claimed {
            return false;
        }
        state.receiver = false;
        true
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.shared.lock().senders += 1;
        Sender {
            shared: self.shared.clone(),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.senders -= 1;
        let last = state.senders == 0;
        drop(state);
        if last {
            // A receiver waiting for a message that cannot come now.
            self.shared.ready.notify_all();
        }
    }
}

impl<T> Receiver<T> {
    /// The next message if there is one, without waiting.
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let mut state = self.shared.lock();
        match state.queue.pop_front() {
            Some(t) => Ok(t),
            None if state.senders == 0 => Err(TryRecvError::Disconnected),
            None => Err(TryRecvError::Empty),
        }
    }

    /// The next message, waiting until one arrives or every sender is gone.
    pub fn recv(&self) -> Result<T, RecvError> {
        let mut state = self.shared.lock();
        loop {
            if let Some(t) = state.queue.pop_front() {
                return Ok(t);
            }
            if state.senders == 0 {
                return Err(RecvError);
            }
            state = self
                .shared
                .ready
                .wait(state)
                .unwrap_or_else(|p| p.into_inner());
        }
    }

    /// The next message, waiting at most `timeout` for one.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        let deadline = Instant::now().checked_add(timeout);
        let mut state = self.shared.lock();
        loop {
            if let Some(t) = state.queue.pop_front() {
                return Ok(t);
            }
            if state.senders == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            // No deadline (a timeout too long to add): wait in long steps.
            let left = match deadline {
                Some(d) => match d.checked_duration_since(Instant::now()) {
                    Some(left) if !left.is_zero() => left,
                    _ => return Err(RecvTimeoutError::Timeout),
                },
                None => Duration::from_secs(3600),
            };
            state = self
                .shared
                .ready
                .wait_timeout(state, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        // Senders fail from now on; what was queued goes, outside the lock.
        let queued = {
            let mut state = self.shared.lock();
            state.receiver = false;
            std::mem::take(&mut state.queue)
        };
        drop(queued);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    /// Run `f` on a thread and fail (rather than hang) if it takes over 10 s.
    fn bounded<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(10))
            .expect("operation hung")
    }

    #[test]
    fn answer_within_withdraws_unclaimed_work() {
        let (tx, rx) = channel::<u8>();
        assert_eq!(rx.answer_within(Duration::from_millis(30), Duration::from_secs(5)), Handoff::Withdrawn);
        // The worker comes round late: refused, and nothing is to be done.
        assert!(!tx.claim());
        assert!(tx.send(1).is_err());
    }

    #[test]
    fn answer_within_answered_or_taken() {
        let (tx, rx) = channel::<u8>();
        tx.send(3).unwrap();
        assert_eq!(rx.answer_within(Duration::ZERO, Duration::ZERO), Handoff::Answered(3));
        // Claimed while waiting, answered in the second wait.
        let (tx, rx) = channel::<u8>();
        let h = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            assert!(tx.claim());
            thread::sleep(Duration::from_millis(20));
            tx.send(4).unwrap();
        });
        assert_eq!(bounded(move || rx.answer_within(Duration::from_secs(5), Duration::from_secs(5))), Handoff::Answered(4));
        h.join().unwrap();
        // Claimed, slow to answer: taken, and the late answer still goes.
        let (tx, rx) = channel::<u8>();
        assert!(tx.claim());
        assert_eq!(rx.answer_within(Duration::from_millis(20), Duration::from_millis(20)), Handoff::Taken);
        assert!(tx.send(5).is_ok());
        // Every sender gone unclaimed: withdrawn at once.
        let (tx, rx) = channel::<u8>();
        drop(tx);
        assert_eq!(bounded(move || rx.answer_within(Duration::from_secs(5), Duration::from_secs(5))), Handoff::Withdrawn);
    }

    #[test]
    fn send_recv_in_order() {
        let (tx, rx) = channel();
        for i in 0..100 {
            tx.send(i).unwrap();
        }
        for i in 0..100 {
            assert_eq!(rx.recv().unwrap(), i);
            if i == 49 {
                tx.send(1000).unwrap();
            }
        }
        assert_eq!(rx.try_recv().unwrap(), 1000);
    }

    #[test]
    fn try_recv_empty_then_disconnected() {
        let (tx, rx) = channel::<u8>();
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
        drop(tx);
        assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
        assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn recv_timeout_times_out_then_succeeds() {
        let (tx, rx) = channel();
        let t0 = Instant::now();
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Timeout)
        );
        assert!(t0.elapsed() >= Duration::from_millis(50));
        tx.send(7).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_millis(50)), Ok(7));
        let h = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            tx.send(8).unwrap();
            tx
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(8));
        drop(h.join().unwrap());
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(10)),
            Err(RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn recv_timeout_zero_and_huge() {
        let (tx, rx) = channel();
        assert_eq!(
            rx.recv_timeout(Duration::ZERO),
            Err(RecvTimeoutError::Timeout)
        );
        tx.send(1).unwrap();
        assert_eq!(rx.recv_timeout(Duration::ZERO), Ok(1));
        // A timeout too long to add to `Instant` still waits, and wakes on a send.
        let h = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            tx.send(2).unwrap();
        });
        assert_eq!(rx.recv_timeout(Duration::MAX), Ok(2));
        h.join().unwrap();
    }

    #[test]
    fn recv_returns_err_when_last_sender_drops_while_blocked() {
        let (tx, rx) = channel::<u32>();
        let started = Arc::new(AtomicBool::new(false));
        let s2 = started.clone();
        let h = thread::spawn(move || {
            s2.store(true, Ordering::SeqCst);
            rx.recv()
        });
        while !started.load(Ordering::SeqCst) {
            thread::yield_now();
        }
        thread::sleep(Duration::from_millis(50)); // let it block
        drop(tx);
        let r = bounded(move || h.join().unwrap());
        assert_eq!(r, Err(RecvError));
    }

    #[test]
    fn last_sender_drop_wakes_recv_timeout() {
        let (tx, rx) = channel::<u32>();
        let h = thread::spawn(move || rx.recv_timeout(Duration::from_secs(60)));
        thread::sleep(Duration::from_millis(50));
        let t0 = Instant::now();
        drop(tx);
        let r = bounded(move || h.join().unwrap());
        assert_eq!(r, Err(RecvTimeoutError::Disconnected));
        assert!(t0.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn cloned_senders_keep_channel_alive_until_last_drops() {
        let (tx, rx) = channel::<usize>();
        let clones: Vec<_> = (0..1000).map(|_| tx.clone()).collect();
        drop(tx);
        let mut clones = clones;
        while clones.len() > 1 {
            clones.pop();
            assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
        }
        clones[0].send(5).unwrap();
        assert_eq!(rx.try_recv(), Ok(5));
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
        clones.pop();
        assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn send_after_receiver_drop_returns_value() {
        let (tx, rx) = channel();
        drop(rx);
        let e = tx.send(String::from("back")).unwrap_err();
        assert_eq!(e.0, "back");
        // Clones fail too.
        assert!(tx.clone().send(String::new()).is_err());
    }

    #[test]
    fn queued_messages_delivered_before_disconnect() {
        let (tx, rx) = channel();
        for i in 0..5 {
            tx.send(i).unwrap();
        }
        drop(tx);
        for i in 0..5 {
            assert_eq!(rx.try_recv(), Ok(i));
        }
        assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
        // Same through recv and recv_timeout.
        let (tx, rx) = channel();
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        drop(tx);
        assert_eq!(rx.recv(), Ok(1));
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)), Ok(2));
        assert_eq!(rx.recv(), Err(RecvError));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)),
            Err(RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn receiver_drop_drops_queued_messages() {
        let live = Arc::new(());
        let (tx, rx) = channel();
        for _ in 0..3 {
            tx.send(live.clone()).unwrap();
        }
        assert_eq!(Arc::strong_count(&live), 4);
        drop(rx);
        assert_eq!(Arc::strong_count(&live), 1);
    }

    #[test]
    fn four_producers_ten_thousand_each_exactly_once_in_order() {
        const PRODUCERS: usize = 4;
        const N: usize = 10_000;
        let (tx, rx) = channel::<(usize, usize)>();
        let handles: Vec<_> = (0..PRODUCERS)
            .map(|p| {
                let tx = tx.clone();
                thread::spawn(move || {
                    for i in 0..N {
                        tx.send((p, i)).unwrap();
                    }
                })
            })
            .collect();
        drop(tx);
        let got = bounded(move || {
            let mut next = [0usize; PRODUCERS];
            let mut total = 0usize;
            while let Ok((p, i)) = rx.recv() {
                assert_eq!(i, next[p], "producer {p} out of order or duplicated");
                next[p] += 1;
                total += 1;
            }
            (total, next)
        });
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(got.0, PRODUCERS * N);
        assert_eq!(got.1, [N; PRODUCERS]);
    }

    #[test]
    fn recv_timeout_does_not_return_early_on_wakeups() {
        // A poker thread sends and steals messages back on the same channel,
        // waking the waiter (a wakeup that may find the queue empty again).
        // The waiter must not report Timeout before its deadline.
        let (tx, rx) = channel::<u32>();
        let rx = Arc::new(rx);
        let stop = Arc::new(AtomicBool::new(false));
        let poker = {
            let (tx, rx, stop) = (tx.clone(), rx.clone(), stop.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    tx.send(0).unwrap();
                    // Steal it before the waiter can (may lose the race, fine).
                    let _ = rx.try_recv();
                }
            })
        };
        let want = Duration::from_millis(200);
        let deadline = Instant::now() + want;
        let t0 = Instant::now();
        // Whatever the waiter gets, it may only return before the deadline
        // with a message; loop until a Timeout and check it came late enough.
        loop {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(_) => {
                    assert!(t0.elapsed() < want + Duration::from_secs(5));
                }
                Err(RecvTimeoutError::Timeout) => {
                    assert!(
                        Instant::now() >= deadline,
                        "Timeout returned {:?} early",
                        deadline - Instant::now()
                    );
                    break;
                }
                Err(e) => panic!("unexpected {e:?}"),
            }
        }
        stop.store(true, Ordering::SeqCst);
        poker.join().unwrap();
        // With no traffic at all, the wait lasts the whole timeout.
        while rx.try_recv().is_ok() {}
        let t1 = Instant::now();
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(150)),
            Err(RecvTimeoutError::Timeout)
        );
        assert!(t1.elapsed() >= Duration::from_millis(150));
        drop(tx);
    }
}
