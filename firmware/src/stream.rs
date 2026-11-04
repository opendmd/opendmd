//! The file-streaming task: card files go out from here, not from the httpd
//! task.
//!
//! ESP-IDF's httpd runs every handler on one task, so while it streams a 4 MB
//! GIF (20-30 s at the card's ~150-300 KB/s) nothing else is answered — and
//! the web app's Now view polls `/api/status` every 500 ms. So a `GET` for a
//! card file (`/gifs/`, `/cache/`, `/sets/`, `/filelist*`) or web app file
//! (`/`, `/www/` and the `/*` catch-all, from flash) is *detached* on the httpd task
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
//! **Uploads are received here too** (`PUT /api/files/…`, `/api/manifest`,
//! `/api/sets/…`, and so a playlist's `.meta`): on the httpd task a client
//! that trickled its body held every other request — `/api/status` went
//! unanswered for ~90 s. The handler takes the connection out of httpd's
//! hands ([`Req::take_over`], see [`crate::httpd`]) and queues it like a
//! download, in the same queue: one transfer at a time, either way.
//!
//! **One upload at a time, none waiting** ([`MAX_UPLOADS`]). A waiting
//! upload's client has sent its window already, and each of those frames
//! stays in one of the WiFi driver's six receive buffers until its turn
//! comes (see "The receive side" in `sdkconfig.defaults`). The window caps
//! the bytes, not the frames: in the emulator a waiting upload held five
//! buffers for 2.6 KB of small segments, and the one being received starved
//! on the one left (a 408 after 5 s of nothing, as on the spare under two
//! uploads at once). So a second upload is refused at once with a `503` and
//! `Retry-After` (never waited for on the httpd task either, where its frames
//! would be held as well), and the web app asks again; a refused upload
//! keeps its place in line for the next free slot ([`LINE`]). Nor does an
//! upload wait behind a download: behind six of a waiting upload's frames a
//! listing's ACKs could not come in and it was cut short. So an upload is
//! taken only with the task idle, and while one waits its turn the downloads
//! that are asked again anyway (GIFs, copies, listings) are refused too, so
//! the task runs dry ([`upload_waiting`]). Reading
//! gives up after [`config::UPLOAD_IDLE_TIMEOUT`] without data or
//! [`config::UPLOAD_MAX_TIME`] in all; the temp file goes and one warn line
//! says how much of the `Content-Length` came ([`files::stream_body_to`]).
//! A taken socket is outside httpd's [`config::HTTP_MAX_SOCKETS`], so lwip
//! needs `1 + QUEUE_DEPTH` more (asserted below).
//!
//! The web app (`/`, `/www/…` and the `/*` catch-all) is sent from here too,
//! out of the www pack in flash (`www.rs`), and a new pack (`PUT /api/www`)
//! is received here: one job at a time, so a pack is never rewritten while
//! one of its files is going out.
//!
//! **The listings go out from here too** (`GET /api/files`,
//! `/api/cache/list`, `/api/cache/failed`): each reads card directories or
//! files under the FAT lock for as long as it streams — `gifs/Consoles` took
//! 2.3 s on a 20 MHz card — and on the httpd task that held `/api/status`
//! (and every other API call) for the whole time (the spare, 2026-10-26).
//! Here a listing takes ~0.3 s longer (the same spare, its marginal 64 GB
//! card at 20 MHz: `gifs/Consoles` 2.6-2.7 s against 2.2-2.3 s on the httpd
//! task, this task being below httpd), and `/api/status` stayed under 80 ms
//! throughout (it was 2-2.4 s).
//! They queue like a file: a full queue is a 503 + `Retry-After: 1`, which
//! the web app retries; they do not wait for a place (`hand_off_within`
//! with `Duration::ZERO`), so a burst of them never holds the httpd task.
//! The heap is checked again when one starts (`files::affordable`), as for a
//! file, since the queue may have held it while the heap dipped.
//! **They interleave with other tasks' card work, by design.** A listing
//! reads its directory between the httpd task's `DELETE` (a file, or
//! `remove_tree` under `cache/`) and the builder's writes, and
//! `/api/cache/failed` reads `/cache/.failed` while `files::rename_line` may
//! swap it (a move): each read is FatFs's own, under its volume lock, so a
//! listing can be stale (a name just deleted, or one just made missing) or
//! end early at a swapped file, never show a corrupt entry. A client wanting
//! the truth lists again. The other JSON routes stay on the httpd task.

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
use crate::httpd::{self, Detached, Req, Respond, Upload, UploadAborted};
use crate::state::free_heap;

/// Requests waiting behind the one being streamed.
pub const QUEUE_DEPTH: usize = 2;
/// How long a request that finds the queue full waits (on the httpd task)
/// for a place before it is answered 503.
pub const QUEUE_WAIT: Duration = Duration::from_secs(2);
/// Uploads in this task's hands at once, queued or being received (see the
/// module docs): one, and only when the task is idle ([`BUSY`]).
pub const MAX_UPLOADS: u32 = 1;
/// The `Retry-After` of an upload refused because [`MAX_UPLOADS`] are here:
/// about the time a 300 KB upload takes at the card's pace.
const UPLOAD_RETRY_AFTER: &str = "2";
/// Uploads queued or being received.
static UPLOADS: AtomicU32 = AtomicU32::new(0);
/// The emulator's low watch (`emu-census`): what the httpd task (low byte:
/// 1 status, 2 an upload refused, 3 another handler) and this task (second
/// byte: 1 an upload, 3 a download) are doing.
#[cfg(feature = "emu-census")]
static PHASE: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "emu-census")]
pub fn set_phase(shift: u32, v: u32) {
    let _ = PHASE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |p| Some((p & !(0xff << shift)) | (v << shift)));
}
#[cfg(not(feature = "emu-census"))]
#[inline(always)]
pub fn set_phase(_shift: u32, _v: u32) {}
#[cfg(feature = "emu-census")]
pub fn phase() -> u32 {
    PHASE.load(Ordering::Relaxed)
}
/// Uploads queued or being received (`net.heap_low`, `misses.recent.net`).
pub fn uploads() -> u32 {
    UPLOADS.load(Ordering::Relaxed)
}
/// A transfer is under way on the task: set when a job is taken (under the
/// queue's lock), cleared when it is done, for an upload as soon as its
/// reply is out (its connection's close is still awaited, but nothing more
/// comes in on it). An upload is taken only when this is clear and nothing
/// waits ([`receive`]).
static BUSY: AtomicBool = AtomicBool::new(false);
/// Uploads refused for want of the slot, oldest first: `"<method> <path>"`
/// and when it was last refused. A refused upload keeps its place: while
/// the first of these is still asking (refused within [`LINE_HOLD`]), a free
/// slot goes to it, and any other upload is refused too. Without it a
/// client uploading back to back took the slot each time it came free,
/// before the refused one asked again (the API lane's five loads: a save
/// into `/gifs/Consoles` refused twelve times in a row). Touched on the
/// httpd task only.
static LINE: Mutex<VecDeque<(String, Instant)>> = Mutex::new(VecDeque::new());
/// Is a refused upload waiting for its turn (the first of [`LINE`], still
/// asking)? Then the downloads a client asks for again anyway are refused
/// too, so the task runs dry and the upload gets it: a page of thumbnails
/// would otherwise keep it busy for as long as it lasts.
fn upload_waiting() -> bool {
    let mut line = LINE.lock().unwrap_or_else(|p| p.into_inner());
    line.retain(|(_, at)| at.elapsed() < LINE_HOLD);
    !line.is_empty()
}

/// Refused uploads remembered at most (a few hundred bytes of paths).
const LINE_LEN: usize = 4;
/// How long a refused upload keeps its place without asking again (it is
/// dropped when another upload comes and finds it this old): the web app
/// asks after 2-4 s (`Retry-After` and its jitter), curl's scripts after
/// `Retry-After`; a client that gave up holds the slot back this long.
const LINE_HOLD: Duration = Duration::from_secs(12);

/// Below the httpd task (5) and the network worker (5), above idle: the
/// stream only runs when nothing on CPU0 that answers a client is ready to.
const PRIORITY: u8 = 2;

// The invariant in the module docs: one socket is never detached.
const _: () = assert!(config::HTTP_MAX_SOCKETS as usize > 1 + QUEUE_DEPTH);
// lwip's sockets: httpd's clients, its listening and control sockets plus
// the one it opens to post a control message, one more (the captive DNS in
// AP mode, the RePlayOS client in STA mode), and the uploads taken out of
// httpd, one streaming and QUEUE_DEPTH waiting.
const _: () = assert!(
    esp_idf_sys::CONFIG_LWIP_MAX_SOCKETS as usize
        >= config::HTTP_MAX_SOCKETS as usize + 3 + 1 + 1 + QUEUE_DEPTH
);

/// What the task runs for one request: produce the whole response on the
/// `Req` it is given (usually [`files::serve`] plus what to say on a miss).
type Work = Box<dyn FnOnce(&mut Req) -> Result<()> + Send>;
/// What the task runs for one upload: read the body and answer it.
type UploadWork = Box<dyn FnOnce(&mut Upload) -> Result<()> + Send>;

enum Job {
    Download { req: Detached, work: Work },
    Upload { up: Upload, work: UploadWork },
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

/// The least stack the task has had left after a job, bytes (`None` before
/// its first): `stream_stack` in `/api/status`, to measure the margin of
/// [`config::STREAM_STACK`] on a device without a console.
pub fn stack_low() -> Option<u32> {
    let v = STACK_LOW.load(Ordering::Relaxed);
    (v != u32::MAX).then_some(v)
}

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
    hand_off_within(req, QUEUE_WAIT, work)
}

/// [`hand_off`], waiting at most `wait` for a place in a full queue before
/// the 503. The web app's own files (and the playlists) wait [`QUEUE_WAIT`]
/// (a browser does not retry a module); the listings and the card's GIFs
/// and copies (`/gifs/`, `/cache/`) wait `Duration::ZERO`: the web app
/// retries those, and each 2 s wait would be 2 s of the httpd task — and
/// `/api/status` — spent waiting for the stream task (five folders opened
/// at once with thumbnails held status 1.1-2.0 s on the spare, listing-c).
pub fn hand_off_within<F>(req: &mut Req, wait: Duration, work: F) -> Result<()>
where
    F: FnOnce(&mut Req) -> Result<()> + Send + 'static,
{
    hand_off_boxed(req, wait, Box::new(work))
}

/// [`hand_off_within`]'s body, once: not monomorphised for every route.
fn hand_off_boxed(req: &mut Req, wait: Duration, work: Work) -> Result<()> {
    if !files::affordable(req)? {
        return Ok(());
    }
    let queue = match QUEUE.get() {
        Some(q) if STARTED.load(Ordering::Acquire) => q,
        // No task: the old way, on the httpd task.
        _ => return work(req),
    };
    // The ones that are asked again (no wait: GIFs, copies, listings) step
    // aside for an upload waiting its turn; a page's own files do not (a
    // browser does not ask again for a module).
    if wait.is_zero() && upload_waiting() {
        log::warn!("{}: an upload is waiting its turn, 503", req.path());
        req.header("Retry-After", "1")?;
        return req.error(
            503,
            "503 Service Unavailable",
            "busy with an upload; retry shortly",
        );
    }
    let deadline = Instant::now() + wait;
    // Wait for a place *before* detaching, so no more than 1 + QUEUE_DEPTH
    // sockets are ever marked in use.
    loop {
        {
            let mut jobs = queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
            if jobs.len() < QUEUE_DEPTH {
                jobs.push_back(Job::Download {
                    req: req.detach()?,
                    work,
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

/// Receive an upload's body on the streaming task with `work`, and return
/// at once: the connection is taken out of httpd's hands first
/// ([`Req::take_over`]), so nothing of the body is read on the httpd task.
/// With the task busy, an upload here already or another refused one ahead
/// of it, it is a `503` with `Retry-After` at once: nothing waits, on the
/// httpd task or in the queue, with its body arriving (see the module docs).
pub fn receive<F>(req: &mut Req, work: F) -> Result<()>
where
    F: FnOnce(&mut Upload) -> Result<()> + Send + 'static,
{
    receive_boxed(req, Box::new(work))
}

/// [`receive`]'s body, once: not monomorphised for every upload route.
fn receive_boxed(req: &mut Req, work: UploadWork) -> Result<()> {
    let up = req.take_over()?;
    let queue = match QUEUE.get() {
        Some(q) if STARTED.load(Ordering::Acquire) => q,
        // No task: on the httpd task, as before (the connection is still
        // ours, and still has its timeouts).
        _ => {
            run_upload(up, work, false);
            return Ok(());
        }
    };
    let key = format!("{} {}", up.method(), up.path());
    let mut line = LINE.lock().unwrap_or_else(|p| p.into_inner());
    // Its own place first, however long it took to ask again: dropping it
    // as stale here let two uploads that each came back late swap places
    // for ever, the slot free (the emulator, whose clock runs ahead of the
    // client's).
    if let Some(place) = line.iter_mut().find(|(k, _)| *k == key) {
        place.1 = Instant::now();
    }
    line.retain(|(_, at)| at.elapsed() < LINE_HOLD);
    let ahead = line.front().is_some_and(|(k, _)| *k != key);
    let mut refused = {
        let mut jobs = queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
        // Under the queue's lock: the count only goes up here, and the
        // stream task takes it down (after the upload) without this lock,
        // so a stale read can only refuse, never let a second one in.
        // Idle only: nothing being sent or received, nothing waiting. A
        // waiting upload's unread frames starve whatever is ahead of it, a
        // download as much as an upload (the emulator: a listing's ACKs
        // could not come in behind six of them, and it was cut short).
        let idle = jobs.is_empty() && !BUSY.load(Ordering::Acquire);
        if !ahead && idle && UPLOADS.load(Ordering::Acquire) < MAX_UPLOADS {
            UPLOADS.fetch_add(1, Ordering::AcqRel);
            jobs.push_back(Job::Upload { up, work });
            queue.ready.notify_one();
            drop(jobs);
            line.retain(|(k, _)| *k != key);
            return Ok(());
        }
        up
    };
    if line.len() < LINE_LEN && !line.iter().any(|(k, _)| *k == key) {
        line.push_back((key, Instant::now()));
    }
    let waiting = line.len();
    drop(line);
    log::warn!(
        "{} {}: {} upload here already, {waiting} refused in line; 503",
        refused.method(),
        refused.path(),
        UPLOADS.load(Ordering::Relaxed)
    );
    // On the httpd task: no lingering over the rest of the body. Closing
    // with the body unread resets the connection after the reply; curl and
    // Chromium read a reply that came before the reset.
    set_phase(0, 2);
    refused.no_linger();
    refused.header("Retry-After", UPLOAD_RETRY_AFTER);
    let _ = refused.error(
        503,
        "503 Service Unavailable",
        "busy with another upload; retry shortly",
    );
    crate::netstat::sample(crate::netstat::At::Refusal);
    Ok(())
}

/// One upload, start to finish: `work` reads the body and answers; an abort
/// (already logged) is answered with its status if one can still be sent,
/// any other failure with a 500 if nothing was sent yet. Dropping `up` ends
/// the connection.
fn run_upload(mut up: Upload, work: UploadWork, counted: bool) {
    crate::diag::check("stream: upload start");
    run_upload_inner(&mut up, work);
    // The reply is out and nothing more is read but the client's close:
    // the slot is the next upload's while this one's drop waits for that
    // (up to a second). Held until the drop, a client sending its uploads
    // one after another met a 503 for every one (the API lane: ~10 s
    // between two small PUTs).
    if counted {
        UPLOADS.fetch_sub(1, Ordering::AcqRel);
        BUSY.store(false, Ordering::Release);
    }
    drop(up);
    crate::diag::check("stream: upload end");
}

fn run_upload_inner(up: &mut Upload, work: UploadWork) {
    up.start_clock();
    // As for a download: a transfer starts only with room to finish. The
    // chunk buffer is taken fallibly as well (`files::stream_body_to`).
    let free = free_heap();
    if free < config::STREAM_START_HEAP {
        log::warn!(
            "refusing {} {}: {free} B free, a transfer needs {} B to start",
            up.method(),
            up.path(),
            config::STREAM_START_HEAP
        );
        up.header("Retry-After", "2");
        let _ = up.error(
            503,
            "503 Service Unavailable",
            "the player needs the heap right now; retry shortly",
        );
        return;
    }
    if let Err(e) = work(up) {
        if let Some(aborted) = e.downcast_ref::<UploadAborted>() {
            if let Some((code, status)) = aborted.status {
                let why = if code == 503 {
                    "the player needs the heap right now; retry shortly".to_string()
                } else {
                    format!(
                        "upload aborted at {} of {} B",
                        up.received(),
                        up.content_len()
                    )
                };
                let _ = up.error(code, status, &why);
            }
        } else if !e.is::<httpd::SendFailed>() {
            log::warn!("{} {}: {e:#}", up.method(), up.path());
            if !up.responded() {
                let _ = up.error(500, "500 Internal Server Error", &format!("{e:#}"));
            }
        }
    } else if !up.responded() {
        let _ = up.error(500, "500 Internal Server Error", "no response");
    }
}

fn run(queue: &'static Queue) {
    loop {
        let job = {
            let mut jobs = queue.jobs.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if let Some(job) = jobs.pop_front() {
                    BUSY.store(true, Ordering::Release);
                    break job;
                }
                jobs = queue.ready.wait(jobs).unwrap_or_else(|p| p.into_inner());
            }
        };
        {
            // The cache builder stops while a file goes out or comes in.
            let _busy = crate::cache_builder::busy();
            match job {
                Job::Download { req, work } => {
                    set_phase(8, 3);
                    httpd::run_detached(req, work);
                    BUSY.store(false, Ordering::Release);
                }
                Job::Upload { up, work } => {
                    set_phase(8, 1);
                    run_upload(up, work, true)
                }
            }
            set_phase(8, 0);
        }
        // The FAT lock and the SPI bus are shared with the player: a turn for
        // everyone between files as well as between chunks.
        thread::yield_now();
        httpd::stack_low_water("stream", &STACK_LOW);
    }
}
