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

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub use std::sync::mpsc::{RecvTimeoutError, SendError, TryRecvError};

struct State<T> {
    queue: VecDeque<T>,
    senders: usize,
    receiver: bool,
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
