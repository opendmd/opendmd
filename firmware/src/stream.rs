//! The file-streaming task: card files go out from here, not from the httpd
//! task.
//!
//! ESP-IDF's httpd runs every handler on one task, so while it streams a 4 MB
//! GIF (20-30 s at the card's ~150-300 KB/s) nothing else is answered — and
//! the web app's Now view polls `/api/status` every 500 ms. So a `GET` for a
//! card file (`/gifs/`, `/odm/`, `/sets/`, `/filelist*`, the web app under
//! `/www/` and the `/*` catch-all) is *detached* on the httpd task
//! ([`Req::detach`], i.e. `httpd_req_async_handler_begin`) and queued here;
//! the handler returns at once and httpd goes back to its sockets. This task
//! opens the file (the directory scan — up to ~550 ms in `gifs/Consoles` —
//! happens here too) and streams it in [`config::IO_CHUNK`] pieces with the
//! same yield-and-heap-wait rules as before ([`files::serve`]).
//!
//! **One stream at a time, [`QUEUE_DEPTH`] waiting.** Serving two files at
//! once would not be faster (one SPI bus, one radio) and would double the
//! lwip buffers in flight on a heap with ~50 KB to spare. A request that finds
//! the queue full waits on the httpd task up to [`QUEUE_WAIT`] for a place,
//! then gets `503` + `Retry-After: 1`. Waiting rather than refusing at once is
//! for the web app's page load: `index.html` pulls in a stylesheet, a script
//! and a dozen ES modules at once, a browser does not retry a module that got
//! a 503, and those files are small enough that the queue drains in well
//! under the wait.
//!
//! **Why the depth is tied to the socket count.** A queued or streaming
//! request's socket is marked in use, and httpd's LRU purge skips those. If
//! every socket were so marked, a new connection could not be accepted and
//! could not evict anyone either — and httpd's accept path then spins on the
//! listening socket without yielding. So at most `1 + QUEUE_DEPTH` sockets
//! may be detached, and [`config::HTTP_MAX_SOCKETS`] is one more than that.
//!
//! Uploads (`PUT /api/files/…`, manifests, `POST /api/ota`) and the JSON
//! listings stay on the httpd task.

use anyhow::Result;
use esp_idf_hal::cpu::Core;
use esp_idf_hal::task::thread::ThreadSpawnConfiguration;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::config;
use crate::files;
use crate::httpd::{self, Detached, Req};
use crate::state::free_heap;

/// Requests waiting behind the one being streamed.
pub const QUEUE_DEPTH: usize = 2;
/// How long a request that finds the queue full waits (on the httpd task)
/// for a place before it is answered 503.
const QUEUE_WAIT: Duration = Duration::from_secs(2);
/// Below the httpd task (5) and the network worker (5), above idle: the
/// stream only runs when nothing on CPU0 that answers a client is ready to.
const PRIORITY: u8 = 2;

// The invariant in the module docs: one socket is never detached.
const _: () = assert!(config::HTTP_MAX_SOCKETS as usize > 1 + QUEUE_DEPTH);

/// What the task runs for one request: produce the whole response on the
/// `Req` it is given (usually [`files::serve`] plus what to say on a miss).
type Work = Box<dyn FnOnce(&mut Req) -> Result<()> + Send>;

struct Job {
    req: Detached,
    work: Work,
}

/// The waiting jobs, and the condition the task sleeps on while there are
/// none.
///
/// **Not `std::sync::mpsc`.** A bounded std channel is lock-free: a sender
/// that finds a slot the receiver has claimed but not yet released spins
/// (`Backoff::spin_heavy`, then `yield_now`) until it is released. Here the
/// sender is the httpd task (priority 5) and the receiver this task
/// (priority 2), on the same core, and a FreeRTOS yield only goes to tasks of
/// equal or higher priority — so when the receiver was preempted in that
/// window, httpd spun forever, starved IDLE0 (task watchdog every 5 s) and
/// took WiFi down with it. Found on the rig under a page-load burst. A mutex
/// blocks instead of spinning, and FreeRTOS mutexes inherit priority.
struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    ready: Condvar,
}

static QUEUE: OnceLock<Queue> = OnceLock::new();
/// Set once the task is running; until then [`hand_off`] serves inline.
static STARTED: AtomicBool = AtomicBool::new(false);
/// The least stack this task has had left, in bytes.
static STACK_LOW: AtomicU32 = AtomicU32::new(u32::MAX);

/// Start the task, pinned to CPU0. Called by `http::start`, i.e. from the main
/// task before WiFi, while the heap still has a contiguous block for the
/// stack. If it cannot start, [`hand_off`] serves on the httpd task instead.
pub fn start() -> Result<()> {
    let before = free_heap();
    let queue = QUEUE.get_or_init(|| Queue {
        jobs: Mutex::new(VecDeque::with_capacity(QUEUE_DEPTH)),
        ready: Condvar::new(),
    });

    let previous = ThreadSpawnConfiguration::get();
    ThreadSpawnConfiguration {
        name: Some(b"stream\0"),
        stack_size: config::STREAM_STACK,
        priority: PRIORITY,
        pin_to_core: Some(Core::Core0),
        ..Default::default()
    }
    .set()?;
    let handle = thread::Builder::new()
        .name("stream".into())
        .stack_size(config::STREAM_STACK)
        .spawn(move || run(queue));
    if let Some(previous) = previous {
        let _ = previous.set();
    } else {
        let _ = ThreadSpawnConfiguration::default().set();
    }
    handle?;
    STARTED.store(true, Ordering::Release);
    log::info!(
        "stream task started on CPU0: {} B stack, queue {QUEUE_DEPTH}, cost {} B of heap",
        config::STREAM_STACK,
        before.saturating_sub(free_heap())
    );
    Ok(())
}

/// Answer `req` on the streaming task with `work`, and return at once.
///
/// Checks the player's heap reserve first ([`files::affordable`]), as every
/// file route did. `work` runs with a detached `Req`: it may send anything a
/// handler could, and its errors are handled the way the httpd wrapper
/// handles a handler's.
pub fn hand_off<F>(req: &mut Req, work: F) -> Result<()>
where
    F: FnOnce(&mut Req) -> Result<()> + Send + 'static,
{
    if !files::affordable(req)? {
        return Ok(());
    }
    let queue = match QUEUE.get() {
        Some(q) if STARTED.load(Ordering::Acquire) => q,
        // No task: the old way, on the httpd task.
        _ => return work(req),
    };
    let deadline = Instant::now() + QUEUE_WAIT;
    // Wait for a place *before* detaching, so no more than 1 + QUEUE_DEPTH
    // sockets are ever marked in use.
    loop {
        {
            let mut jobs = queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
            if jobs.len() < QUEUE_DEPTH {
                jobs.push_back(Job {
                    req: req.detach()?,
                    work: Box::new(work),
                });
                queue.ready.notify_one();
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            log::warn!("{}: stream queue full, 503", req.path());
            req.header("Retry-After", "1")?;
            return req.error(
                503,
                "503 Service Unavailable",
                "busy sending other files; retry shortly",
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn run(queue: &'static Queue) {
    loop {
        let Job { req, work } = {
            let mut jobs = queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if let Some(job) = jobs.pop_front() {
                    break job;
                }
                jobs = queue.ready.wait(jobs).unwrap_or_else(|p| p.into_inner());
            }
        };
        httpd::run_detached(req, work);
        // The FAT lock and the SPI bus are shared with the player: a turn for
        // everyone between files as well as between chunks.
        thread::yield_now();
        httpd::stack_low_water("stream", &STACK_LOW);
    }
}
