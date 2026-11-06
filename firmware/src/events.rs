//! `GET /api/events`: what the Now view follows, pushed as Server-Sent
//! Events on ESP-IDF's `esp_http_server` (docs/research/http-stack.md §4-5;
//! docs/api.md).
//!
//! The route answers with a hand-written `text/event-stream` head and
//! returns: the request is over as far as httpd is concerned, and the session
//! sits idle in httpd's `select` set like any keep-alive connection (a
//! listener never sends on it again). So it is *not* detached and holds no
//! httpd request copy; httpd's LRU purge may evict it like any idle
//! connection when the sockets run out (the client comes back). Its fd goes
//! into [`STATE`].
//!
//! Every push runs **on the httpd task** (`httpd_queue_work`, queued by a
//! [`TICK_US`] `esp_timer` only while anyone listens and none is queued), and
//! so does httpd's `close_fn`, which takes a closed fd out of the list
//! ([`closed`]) before httpd can reuse the number: the fd-reuse race the
//! detached requests guard against with a lock cannot happen here.
//!
//! Each tick builds the event (the source in `http.rs`: a status snapshot,
//! ~0.5 KB of JSON reserved up front, none under the status's low-heap mark)
//! and sends it only when what it says changed — the clock and the position
//! are left out of that comparison ([`Event::volatile`]) — or after
//! [`KEEPALIVE_TICKS`] without one, so a client can tell a live stream from
//! a dead one. Every send is `MSG_DONTWAIT`: a push never waits for a client.
//! One event is held per listener at most: what did not fit in the socket's
//! send buffer goes first next tick, newer events are skipped meanwhile and
//! the latest goes once it is out (latest state wins). A listener stuck for
//! [`STUCK_TICKS`] is closed.

use esp_idf_sys as sys;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};

/// Listeners at once; another is a `503` + `Retry-After`. Each one is a
/// socket of httpd's [`crate::config::HTTP_MAX_SOCKETS`].
pub const MAX_LISTENERS: usize = 2;
/// How often the state is looked at while anyone listens.
const TICK_US: u64 = 250_000;
/// Ticks without a change before the state is sent anyway (3 s).
const KEEPALIVE_TICKS: u32 = 12;
/// Ticks an event may stay partly sent before the listener is closed (5 s).
const STUCK_TICKS: u32 = 20;

const HEAD: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\
X-Content-Type-Options: nosniff\r\n\r\nretry: 3000\n\n";

struct Listener {
    fd: i32,
    /// The rest of an event that did not fit, and where it stopped.
    rest: Option<(Vec<u8>, usize)>,
    stuck: u32,
    /// Owed the next event, change or not (new, or skipped one).
    owed: bool,
}

struct State {
    listeners: Vec<Listener>,
    /// The last sent event's hash, and the ticks since.
    hash: u32,
    quiet: u32,
}

static STATE: Mutex<State> = Mutex::new(State { listeners: Vec::new(), hash: 0, quiet: 0 });
static LISTENERS: AtomicU32 = AtomicU32::new(0);
static HANDLE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static QUEUED: AtomicBool = AtomicBool::new(false);
type Source = Box<dyn Fn() -> Option<Event> + Send + Sync>;
static SOURCE: OnceLock<Source> = OnceLock::new();
/// Listeners closed for not reading (`net.events.dropped`).
static DROPPED: AtomicU32 = AtomicU32::new(0);
/// Ticks with no event built for want of heap (`net.events.no_heap`).
static NO_HEAP: AtomicU32 = AtomicU32::new(0);

/// One event's text, `data: {…}\n\n`, in a buffer reserved once (a write
/// past it fails rather than grows), and the hash of what it says that
/// counts as a change.
pub struct Event {
    buf: String,
    hash: u32,
    quiet: bool,
    full: bool,
}

impl Event {
    /// Room for `json` bytes of JSON, taken fallibly.
    pub fn new(json: usize) -> Option<Event> {
        let mut buf = String::new();
        buf.try_reserve_exact(json + 8).ok()?;
        buf.push_str("data: ");
        Some(Event { buf, hash: 0x811c_9dc5, quiet: false, full: false })
    }

    /// Write what `f` writes without counting it as a change (the clock,
    /// the position in the file).
    pub fn volatile(&mut self, f: impl FnOnce(&mut Self)) {
        self.quiet = true;
        f(self);
        self.quiet = false;
    }

    /// Count `s` as part of the state without sending it.
    pub fn mark(&mut self, s: &str) {
        self.fold(s.as_bytes());
    }

    fn fold(&mut self, b: &[u8]) {
        for &c in b {
            self.hash = (self.hash ^ c as u32).wrapping_mul(0x0100_0193);
        }
    }
}

impl std::fmt::Write for Event {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        // (Two bytes kept for the closing "\n\n".)
        if self.buf.len() + s.len() + 2 > self.buf.capacity() {
            self.full = true;
            return Err(std::fmt::Error);
        }
        self.buf.push_str(s);
        if !self.quiet {
            self.fold(s.as_bytes());
        }
        Ok(())
    }
}

/// Start the push timer. `source` makes one event, or `None` when the heap
/// cannot spare it (that tick is skipped).
pub fn start(handle: sys::httpd_handle_t, source: impl Fn() -> Option<Event> + Send + Sync + 'static) {
    HANDLE.store(handle, Relaxed);
    let _ = SOURCE.set(Box::new(source));
    // SAFETY: plain esp_timer calls; the timer lives for the program's life.
    unsafe {
        let args = sys::esp_timer_create_args_t {
            callback: Some(tick),
            arg: std::ptr::null_mut(),
            dispatch_method: sys::esp_timer_dispatch_t_ESP_TIMER_TASK,
            name: c"events".as_ptr(),
            skip_unhandled_events: true,
        };
        let mut timer: sys::esp_timer_handle_t = std::ptr::null_mut();
        if sys::esp_timer_create(&args, &mut timer) != sys::ESP_OK
            || sys::esp_timer_start_periodic(timer, TICK_US) != sys::ESP_OK
        {
            log::warn!("events: no push timer");
        }
    }
}

/// Queue a push on the httpd task if anyone listens and none is queued.
fn queue() {
    if LISTENERS.load(Relaxed) == 0 || QUEUED.swap(true, Relaxed) {
        return;
    }
    // SAFETY: the handle is the running server's; the work fn is ours.
    if unsafe { sys::httpd_queue_work(HANDLE.load(Relaxed), Some(push), std::ptr::null_mut()) } != sys::ESP_OK {
        QUEUED.store(false, Relaxed);
    }
}

/// The timer (esp_timer task, CPU0).
unsafe extern "C" fn tick(_: *mut c_void) {
    queue();
}

/// `"events":{…}` for the status's `net`: listeners now, those closed for
/// not reading, ticks skipped for want of heap.
pub fn status_json() -> String {
    format!(
        r#""events":{{"listeners":{},"dropped":{},"no_heap":{}}}"#,
        LISTENERS.load(Relaxed),
        DROPPED.load(Relaxed),
        NO_HEAP.load(Relaxed)
    )
}

/// `GET /api/events`, on the httpd task.
#[optimize(size)]
pub fn open(req: &mut crate::httpd::Req) -> anyhow::Result<()> {
    use crate::httpd::Respond;
    let mut st = STATE.lock().unwrap_or_else(|p| p.into_inner());
    if st.listeners.len() >= MAX_LISTENERS || st.listeners.try_reserve_exact(MAX_LISTENERS).is_err() {
        drop(st);
        req.header("Retry-After", "5")?;
        return req.error(503, "503 Service Unavailable", "too many event listeners");
    }
    let fd = req.hold_open(HEAD)?;
    st.listeners.push(Listener { fd, rest: None, stuck: 0, owed: true });
    LISTENERS.store(st.listeners.len() as u32, Relaxed);
    drop(st);
    // Its first event at once, not at the next tick.
    queue();
    Ok(())
}

/// httpd's `close_fn` for every client socket, on the httpd task, before the
/// fd is closed and can be reused.
pub fn closed(fd: i32) {
    if LISTENERS.load(Relaxed) == 0 {
        return;
    }
    let mut st = STATE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(i) = st.listeners.iter().position(|l| l.fd == fd) {
        st.listeners.swap_remove(i);
        LISTENERS.store(st.listeners.len() as u32, Relaxed);
        if st.listeners.is_empty() {
            // (No list kept while nobody listens.)
            st.listeners = Vec::new();
        }
    }
}

/// Send what fits without waiting: the bytes sent, or `None` for a dead socket.
fn send_some(fd: i32, buf: &[u8]) -> Option<usize> {
    // SAFETY: `fd` is a live httpd session (we are on the httpd task, and
    // `closed` removes it before httpd closes it).
    let n = unsafe { sys::lwip_send(fd, buf.as_ptr() as *const c_void, buf.len(), sys::MSG_DONTWAIT as i32) };
    if n >= 0 {
        return Some(n as usize);
    }
    // SAFETY: lwIP's errno for this task.
    let e = unsafe { *sys::__errno() };
    (e == sys::EAGAIN as i32 || e == sys::EWOULDBLOCK as i32).then_some(0)
}

/// Keep what did not fit of an event for the next tick (taken fallibly):
/// `false` for a listener to close.
fn keep(l: &mut Listener, tail: &[u8]) -> bool {
    let mut v = Vec::new();
    if v.try_reserve_exact(tail.len()).is_err() {
        return false;
    }
    v.extend_from_slice(tail);
    l.rest = Some((v, 0));
    true
}

/// The push, on the httpd task (`httpd_queue_work`).
unsafe extern "C" fn push(_: *mut c_void) {
    QUEUED.store(false, Relaxed);
    let handle = HANDLE.load(Relaxed);
    let mut close = [-1i32; MAX_LISTENERS];
    let mut st = STATE.lock().unwrap_or_else(|p| p.into_inner());
    if st.listeners.is_empty() {
        return;
    }
    // A listener still sending an older event: the rest of it first.
    for (k, l) in st.listeners.iter_mut().enumerate() {
        let Some((rest, at)) = l.rest.take() else { continue };
        l.stuck += 1;
        match send_some(l.fd, &rest[at..]) {
            None => close[k] = l.fd,
            Some(n) if at + n == rest.len() => l.stuck = 0,
            Some(_) if l.stuck >= STUCK_TICKS => {
                DROPPED.fetch_add(1, Relaxed);
                log::warn!("events: fd {} stopped reading, closed", l.fd);
                close[k] = l.fd;
            }
            Some(n) => l.rest = Some((rest, at + n)),
        }
    }
    st.quiet += 1;
    let ev = SOURCE.get().and_then(|f| f());
    match ev {
        None => {
            NO_HEAP.fetch_add(1, Relaxed);
        }
        Some(mut ev) if !ev.full => {
            ev.buf.push_str("\n\n");
            let due = ev.hash != st.hash || st.quiet >= KEEPALIVE_TICKS;
            if due {
                st.hash = ev.hash;
                st.quiet = 0;
            }
            for (k, l) in st.listeners.iter_mut().enumerate() {
                if close[k] >= 0 || !(due || l.owed) {
                    continue;
                }
                if l.rest.is_some() {
                    // Latest state wins: this one is skipped, the next owed.
                    l.owed = true;
                    continue;
                }
                l.owed = false;
                let buf = ev.buf.as_bytes();
                match send_some(l.fd, buf) {
                    Some(n) if n == buf.len() => {}
                    Some(n) if keep(l, &buf[n..]) => {}
                    _ => close[k] = l.fd,
                }
            }
        }
        Some(_) => {
            // (Bigger than its reservation: a bug in the source's estimate.)
            NO_HEAP.fetch_add(1, Relaxed);
        }
    }
    drop(st);
    for fd in close.into_iter().filter(|&fd| fd >= 0) {
        // `close_fn` (and so `closed`) runs when httpd gets to it.
        sys::httpd_sess_trigger_close(handle, fd);
    }
}
