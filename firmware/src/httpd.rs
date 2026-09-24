//! A thin safe wrapper over ESP-IDF's `esp_http_server`.
//!
//! **Why this exists instead of `esp_idf_svc::http::server::EspHttpServer`.**
//! `httpd_config_t` has a `core_id` field, and esp-idf-svc 0.51 hardcodes it to
//! `i32::MAX` (`tskNO_AFFINITY`) with no way to override it — see
//! `esp-idf-svc-0.51.0/src/http/server.rs`, `impl From<&Configuration> for
//! Newtype<httpd_config_t>`. An unpinned httpd task runs at priority 5 against
//! the display loop's priority 1, so the scheduler is free to drop it on CPU1
//! and preempt the panel mid-frame. There is no runtime fix either:
//! `vTaskCoreAffinitySet` only exists in the SMP kernel, not in the IDF
//! FreeRTOS this build uses. Pinning the server to CPU0 is a hard requirement
//! of this design, so we start `httpd` ourselves.
//!
//! What that costs is the `embedded_svc` request/response types; what it buys,
//! besides the pinning, is exactly the streaming primitives the file server
//! wants (`httpd_resp_send_chunk` straight out of a 4 KB buffer).
//!
//! **The httpd task is a single task.** ESP-IDF services every open socket from
//! one task, so handlers never run concurrently with each other — only with the
//! display loop, on the other core. A handler that blocks blocks every other
//! client, `/api/status` included, which is why file downloads are handed to
//! [`crate::stream`] with [`Req::detach`] and why **a failed send is never
//! retried** (see [`Req::chunk`]).
//!
//! Handler contract: return `Ok(())` having sent a response (or having
//! detached the request), or `Err(_)` and the wrapper sends a 500 if nothing
//! has gone out yet, or drops the connection if something has.

use anyhow::{bail, Result};
use esp_idf_sys as sys;
use std::cell::Cell;
use std::ffi::{c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// `SO_SNDTIMEO` on every client socket, in seconds: how long one `send` may
/// wait for a client that is not reading before it fails with EAGAIN. That
/// failure ends the response (see [`Req::chunk`]), so this is also the longest
/// a stalled client can hold whichever task is sending to it.
const SEND_TIMEOUT_S: u16 = 5;
/// `SO_RCVTIMEO`, in seconds. Generous, because a browser uploading a 2 MB
/// firmware image over a weak link can stall between chunks.
const RECV_TIMEOUT_S: u16 = 10;
/// Consecutive receive timeouts [`Req::read`] sits through before it gives up
/// on an upload: 3 x 10 s of silence. It used to retry forever, which let one
/// vanished uploader hold the httpd task, and every other client, for good.
const RECV_TIMEOUTS: usize = 3;

/// The error a send failure turns into, once it has been logged. The
/// connection is dropped; nothing more can be said to that client.
#[derive(Debug)]
pub struct SendFailed;

impl std::fmt::Display for SendFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("send failed; connection dropped")
    }
}

impl std::error::Error for SendFailed {}

/// Was this error already logged where it happened?
fn already_logged(e: &anyhow::Error) -> bool {
    e.is::<SendFailed>()
}

/// Responses this device cut short after their headers went out — a send
/// that timed out, a card read that failed, the heap that did not come back
/// ([`crate::files`]'s wait) — since boot. A client that went away is not
/// counted. Every one is logged by [`Req::log_cut`].
static STREAMS_ABORTED: AtomicU32 = AtomicU32::new(0);

/// [`STREAMS_ABORTED`], for `/api/status` and the heartbeat.
pub fn streams_aborted() -> u32 {
    STREAMS_ABORTED.load(Ordering::Relaxed)
}

/// A boxed handler. `Sync` is not strictly required (one task) but it is free.
type Handler = Box<dyn Fn(&mut Req) -> Result<()> + Send + Sync + 'static>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl Method {
    fn raw(self) -> sys::httpd_method_t {
        let m = match self {
            Method::Get => sys::http_method_HTTP_GET,
            Method::Post => sys::http_method_HTTP_POST,
            Method::Put => sys::http_method_HTTP_PUT,
            Method::Delete => sys::http_method_HTTP_DELETE,
        };
        m as sys::httpd_method_t
    }
}

/// A running server. Dropping it stops the task and frees the handlers.
pub struct Server {
    handle: sys::httpd_handle_t,
    /// URI strings and handler boxes must outlive every request: ESP-IDF keeps
    /// the `uri` pointer in its handler table and we keep the closure behind
    /// `user_ctx`.
    uris: Vec<CString>,
    handlers: Vec<*mut Handler>,
}

// SAFETY: the raw pointers are owned allocations that nothing else touches;
// the httpd handle is used by its own task and only stopped from Drop.
unsafe impl Send for Server {}

impl Server {
    /// Start the server pinned to `core_id` with `stack_size` bytes of stack
    /// and wildcard URI matching.
    pub fn start(
        core_id: i32,
        stack_size: usize,
        max_sockets: u16,
        max_handlers: u16,
    ) -> Result<Server> {
        let mut handle: sys::httpd_handle_t = std::ptr::null_mut();
        // SAFETY: a fully initialised POD config that outlives the call;
        // `httpd_start` copies what it needs.
        let err = unsafe {
            let mut cfg: sys::httpd_config_t = std::mem::zeroed();
            cfg.task_priority = 5;
            cfg.stack_size = stack_size;
            cfg.core_id = core_id;
            cfg.task_caps = sys::MALLOC_CAP_INTERNAL | sys::MALLOC_CAP_8BIT;
            cfg.server_port = 80;
            cfg.ctrl_port = 32768;
            cfg.max_open_sockets = max_sockets;
            cfg.max_uri_handlers = max_handlers;
            cfg.max_resp_headers = 8;
            cfg.backlog_conn = 5;
            cfg.lru_purge_enable = true;
            // Applied per client socket by httpd_accept_conn (httpd_main.c:
            // setsockopt SO_RCVTIMEO / SO_SNDTIMEO from these two fields).
            cfg.recv_wait_timeout = RECV_TIMEOUT_S;
            cfg.send_wait_timeout = SEND_TIMEOUT_S;
            // Ours instead of a plain close(): it tells a detached request
            // that its connection is going away (see `close_session`).
            cfg.close_fn = Some(close_session);
            cfg.uri_match_fn = Some(sys::httpd_uri_match_wildcard);
            sys::httpd_start(&mut handle, &cfg)
        };
        if err != sys::ESP_OK {
            bail!("httpd_start failed: {err}");
        }
        log::info!(
            "httpd started on port 80, pinned to CPU{core_id}, {stack_size} B stack, \
             {max_sockets} sockets, send timeout {SEND_TIMEOUT_S} s"
        );
        Ok(Server {
            handle,
            uris: Vec::new(),
            handlers: Vec::new(),
        })
    }

    /// Register one route. Wildcard routes (`/www/*`) must be registered after
    /// the exact routes they would otherwise shadow: ESP-IDF prefers exact
    /// matches, but among wildcards the earlier registration wins.
    pub fn route<F>(&mut self, uri: &str, method: Method, handler: F) -> Result<()>
    where
        F: Fn(&mut Req) -> Result<()> + Send + Sync + 'static,
    {
        let uri_c = CString::new(uri)?;
        let boxed: *mut Handler = Box::into_raw(Box::new(Box::new(handler) as Handler));

        let desc = sys::httpd_uri_t {
            uri: uri_c.as_ptr(),
            method: method.raw(),
            handler: Some(trampoline),
            user_ctx: boxed as *mut c_void,
        };
        // SAFETY: `uri_c` and the handler box are kept alive in `self` for as
        // long as the server runs.
        let err = unsafe { sys::httpd_register_uri_handler(self.handle, &desc) };
        if err != sys::ESP_OK {
            // SAFETY: nothing took ownership, so we still own the box.
            unsafe { drop(Box::from_raw(boxed)) };
            bail!("registering {uri} failed: {err}");
        }
        self.uris.push(uri_c);
        self.handlers.push(boxed);
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // SAFETY: stop the task first, so no handler can be running when the
        // closures behind `user_ctx` are freed.
        unsafe {
            sys::httpd_stop(self.handle);
            for h in self.handlers.drain(..) {
                drop(Box::from_raw(h));
            }
        }
    }
}

/// # Safety
/// Called by ESP-IDF's httpd task with a live `httpd_req_t` whose `user_ctx` is
/// the handler box registered above.
unsafe extern "C" fn trampoline(raw: *mut sys::httpd_req_t) -> sys::esp_err_t {
    let handler = (*raw).user_ctx as *const Handler;
    if handler.is_null() {
        return sys::ESP_FAIL;
    }
    let mut req = Req::new(raw);
    let result = (*handler)(&mut req);
    stack_low_water("httpd", &HTTPD_STACK_LOW);
    match result {
        Ok(()) => {
            // A handler that streamed chunks must terminate the response; do
            // it for them if they forgot, or the socket hangs. A sized body
            // that came up short is cut: the client must not wait for the rest.
            if let Err(e) = req.complete() {
                if !already_logged(&e) {
                    req.log_failure(&e);
                }
                return sys::ESP_FAIL;
            }
            sys::ESP_OK
        }
        Err(e) => {
            if !already_logged(&e) {
                req.log_failure(&e);
            }
            if req.responded {
                // Half a response has already gone out; the only honest thing
                // left is to drop the connection, which is what ESP_FAIL makes
                // httpd do.
                sys::ESP_FAIL
            } else {
                let _ = req.send_status(
                    500,
                    "500 Internal Server Error",
                    "application/json",
                    b"{\"error\":\"internal error\"}",
                );
                sys::ESP_OK
            }
        }
    }
}

/// The least stack the httpd task has had left, in bytes.
static HTTPD_STACK_LOW: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);

/// [`stack_low_water`] for the httpd task, from a handler that will not
/// return (a reboot).
pub fn note_httpd_stack() {
    stack_low_water("httpd", &HTTPD_STACK_LOW);
}

/// Log the calling task's stack high-water mark when it reaches a new low.
/// A scan of the task's unused stack (~30 µs), cheap next to a request.
pub fn stack_low_water(task: &str, low: &std::sync::atomic::AtomicU32) {
    // SAFETY: a query about the calling task.
    let free = unsafe { sys::uxTaskGetStackHighWaterMark(std::ptr::null_mut()) };
    if free < low.load(Ordering::Relaxed) {
        low.store(free, Ordering::Relaxed);
        log::info!("{task} task: stack high-water {free} B unused");
    }
}

/// One in-flight request.
///
/// `keep` exists because `httpd_resp_set_status`, `_set_type` and `_set_hdr`
/// store the *pointer* they are given and read it when the response is
/// serialised — the strings have to outlive the send, not just the call.
pub struct Req {
    raw: *mut sys::httpd_req_t,
    keep: Vec<CString>,
    responded: bool,
    chunked: bool,
    finished: bool,
    /// Body bytes sent so far (chunks only), for the log line when a send fails.
    sent: u64,
    /// The declared `Content-Length` of a sized response ([`Req::begin_sized`]).
    total: Option<u64>,
    /// `Field: value\r\n` for every [`Req::header`], for a sized response,
    /// whose head is written here rather than by httpd.
    extra_headers: String,
    /// The client closed its end: a failed send is its doing, not a fault.
    client_gone: Cell<bool>,
    /// Set when this is a detached request running on another task: every
    /// send goes through the session's lock (see [`Session`]).
    session: Option<Arc<Session>>,
}

impl Req {
    fn new(raw: *mut sys::httpd_req_t) -> Req {
        Req {
            raw,
            keep: Vec::new(),
            responded: false,
            chunked: false,
            finished: false,
            sent: 0,
            total: None,
            extra_headers: String::new(),
            client_gone: Cell::new(false),
            session: None,
        }
    }

    /// The request target, query string included.
    pub fn uri(&self) -> &str {
        // SAFETY: `uri` is an inline NUL-terminated 513-byte array in the
        // request struct, valid for as long as the handler runs.
        unsafe { CStr::from_ptr((*self.raw).uri.as_ptr()) }
            .to_str()
            .unwrap_or("")
    }

    /// The path with any `?query` removed.
    pub fn path(&self) -> &str {
        let uri = self.uri();
        uri.split('?').next().unwrap_or(uri)
    }

    /// The raw query string, if any.
    pub fn query(&self) -> Option<&str> {
        self.uri().split_once('?').map(|(_, q)| q)
    }

    fn method_name(&self) -> &'static str {
        // SAFETY: a plain field read on a live request.
        match unsafe { (*self.raw).method } as u32 {
            sys::http_method_HTTP_GET => "GET",
            sys::http_method_HTTP_POST => "POST",
            sys::http_method_HTTP_PUT => "PUT",
            sys::http_method_HTTP_DELETE => "DELETE",
            _ => "?",
        }
    }

    pub fn content_len(&self) -> usize {
        // SAFETY: a plain field read on a live request.
        unsafe { (*self.raw).content_len }
    }

    /// Read up to `buf.len()` body bytes. `Ok(0)` means the body is complete.
    /// A socket timeout is retried up to [`RECV_TIMEOUTS`] times in a row,
    /// which is what makes a slow uploader work without letting a vanished
    /// one hold the httpd task forever.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let mut timeouts = 0;
        loop {
            // SAFETY: `buf` is a live mutable slice; httpd writes at most
            // `buf.len()` bytes into it.
            let n = unsafe { sys::httpd_req_recv(self.raw, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n >= 0 {
                return Ok(n as usize);
            }
            if n == sys::HTTPD_SOCK_ERR_TIMEOUT {
                timeouts += 1;
                if timeouts < RECV_TIMEOUTS {
                    continue;
                }
                bail!(
                    "client sent nothing for {} s",
                    RECV_TIMEOUTS * RECV_TIMEOUT_S as usize
                );
            }
            bail!("socket read error {n}");
        }
    }

    /// Read the whole body, refusing anything over `limit`.
    pub fn read_to_vec(&mut self, limit: usize) -> Result<Vec<u8>> {
        let declared = self.content_len();
        if declared > limit {
            bail!("body of {declared} B exceeds the {limit} B limit");
        }
        let mut out = Vec::with_capacity(declared.min(limit));
        let mut buf = [0u8; 1024];
        loop {
            let n = self.read(&mut buf)?;
            if n == 0 {
                break;
            }
            if out.len() + n > limit {
                bail!("body exceeds the {limit} B limit");
            }
            out.extend_from_slice(&buf[..n]);
        }
        Ok(out)
    }

    fn set_status(&mut self, status: &str) -> Result<()> {
        let s = CString::new(status)?;
        // SAFETY: `s` is parked in `keep`, so the pointer stays valid until the
        // response has been serialised.
        let err = unsafe { sys::httpd_resp_set_status(self.raw, s.as_ptr()) };
        self.keep.push(s);
        if err != sys::ESP_OK {
            bail!("set_status: {err}");
        }
        Ok(())
    }

    fn set_type(&mut self, content_type: &str) -> Result<()> {
        let s = CString::new(content_type)?;
        // SAFETY: as above.
        let err = unsafe { sys::httpd_resp_set_type(self.raw, s.as_ptr()) };
        self.keep.push(s);
        if err != sys::ESP_OK {
            bail!("set_type: {err}");
        }
        Ok(())
    }

    /// Append a response header. Must be called before the first chunk.
    pub fn header(&mut self, field: &str, value: &str) -> Result<()> {
        let f = CString::new(field)?;
        let v = CString::new(value)?;
        // SAFETY: both strings are parked in `keep` for the same reason.
        let err = unsafe { sys::httpd_resp_set_hdr(self.raw, f.as_ptr(), v.as_ptr()) };
        self.keep.push(f);
        self.keep.push(v);
        if err != sys::ESP_OK {
            bail!("set_hdr: {err}");
        }
        self.extra_headers.push_str(field);
        self.extra_headers.push_str(": ");
        self.extra_headers.push_str(value);
        self.extra_headers.push_str("\r\n");
        Ok(())
    }

    /// Send a complete response in one call.
    pub fn send_status(
        &mut self,
        _code: u16,
        status: &str,
        content_type: &str,
        body: &[u8],
    ) -> Result<()> {
        self.set_status(status)?;
        self.set_type(content_type)?;
        self.responded = true;
        let raw = self.raw;
        // SAFETY: `body` is a live slice read synchronously by httpd.
        let err = self.guarded(|| unsafe {
            sys::httpd_resp_send(raw, body.as_ptr() as *const _, body.len() as isize)
        })?;
        if err != sys::ESP_OK {
            return Err(self.send_failed("resp_send", err));
        }
        Ok(())
    }

    pub fn ok(&mut self, content_type: &str, body: &[u8]) -> Result<()> {
        self.send_status(200, "200 OK", content_type, body)
    }

    pub fn json(&mut self, body: &str) -> Result<()> {
        self.ok("application/json", body.as_bytes())
    }

    /// A JSON error body with the given status, e.g. `(404, "404 Not Found")`.
    pub fn error(&mut self, code: u16, status: &str, message: &str) -> Result<()> {
        let body = format!(r#"{{"error":{}}}"#, json_string(message));
        self.send_status(code, status, "application/json", body.as_bytes())
    }

    pub fn not_found(&mut self, message: &str) -> Result<()> {
        self.error(404, "404 Not Found", message)
    }

    pub fn bad_request(&mut self, message: &str) -> Result<()> {
        self.error(400, "400 Bad Request", message)
    }

    pub fn redirect(&mut self, location: &str) -> Result<()> {
        self.header("Location", location)?;
        // Without this a browser caches the 302 and keeps following it after
        // the device has moved to STA mode.
        self.header("Cache-Control", "no-store")?;
        self.send_status(302, "302 Found", "text/plain", b"")
    }

    /// Begin a chunked response. Headers must already be set.
    pub fn begin_chunked(&mut self, status: &str, content_type: &str) -> Result<()> {
        self.set_status(status)?;
        self.set_type(content_type)?;
        self.responded = true;
        self.chunked = true;
        Ok(())
    }

    /// Begin a response whose body is exactly `len` bytes, with a
    /// `Content-Length` — what a file download is, so a client can tell a
    /// body that ended early (curl: "transfer closed with N bytes remaining")
    /// and a browser can show progress. httpd has no streaming call for a
    /// sized body (`httpd_resp_send` wants it all in one buffer), so the head
    /// is written here, in the shape httpd writes its own, with the headers
    /// set by [`Req::header`]; the body then goes out with [`Req::chunk`].
    /// Fewer than `len` bytes by the time the handler returns cuts the
    /// connection (see [`Req::complete`]).
    pub fn begin_sized(&mut self, status: &str, content_type: &str, len: u64) -> Result<()> {
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {len}\r\n{}\r\n",
            self.extra_headers
        );
        self.responded = true;
        self.total = Some(len);
        self.send_all(head.as_bytes(), "send(head)")
    }

    /// Write `buf` to the socket, all of it, through the session guard.
    fn send_all(&mut self, mut buf: &[u8], what: &str) -> Result<()> {
        let raw = self.raw;
        while !buf.is_empty() {
            // SAFETY: `buf` is a live slice, written out synchronously.
            let n = self
                .guarded(|| unsafe { sys::httpd_send(raw, buf.as_ptr() as *const _, buf.len()) })?;
            if n <= 0 {
                return Err(self.send_failed(what, n));
            }
            buf = &buf[(n as usize).min(buf.len())..];
        }
        Ok(())
    }

    /// Send one chunk. Empty chunks are skipped — an empty
    /// `httpd_resp_send_chunk` is what *terminates* the response.
    ///
    /// **A failed send ends the response; it is never retried.** The socket
    /// is blocking with `SO_SNDTIMEO` = [`SEND_TIMEOUT_S`], so lwip does not
    /// fail a send for want of memory — it waits — and a failure means the
    /// client took nothing for that long, or is gone. This used to retry 8
    /// times, and with the old 10 s timeout one client that stopped reading
    /// held the single httpd task for ~80 s (EAGAIN, `error in send : 11`,
    /// every ~10 s) while every other request, `/api/status` included,
    /// queued behind it. The failure is logged here, once, and comes back as
    /// [`SendFailed`]; the caller's `?` drops the file and the wrapper drops
    /// the connection.
    pub fn chunk(&mut self, buf: &[u8]) -> Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        if let Some(total) = self.total {
            // A sized body: raw bytes, never past what was declared.
            let room = total.saturating_sub(self.sent) as usize;
            if buf.len() > room {
                bail!("body is longer than the {total} B declared");
            }
            self.send_all(buf, "send")?;
            self.sent += buf.len() as u64;
            return Ok(());
        }
        let raw = self.raw;
        // SAFETY: `buf` is a live slice, written out synchronously.
        let err = self.guarded(|| unsafe {
            sys::httpd_resp_send_chunk(raw, buf.as_ptr() as *const _, buf.len() as isize)
        })?;
        if err != sys::ESP_OK {
            return Err(self.send_failed("resp_send_chunk", err));
        }
        self.sent += buf.len() as u64;
        Ok(())
    }

    /// Terminate a chunked response.
    pub fn finish(&mut self) -> Result<()> {
        if self.finished || self.total.is_some() {
            return Ok(());
        }
        self.finished = true;
        let raw = self.raw;
        // SAFETY: the null/zero chunk is httpd's end-of-response marker.
        let err =
            self.guarded(|| unsafe { sys::httpd_resp_send_chunk(raw, std::ptr::null(), 0) })?;
        if err != sys::ESP_OK {
            return Err(self.send_failed("resp_send_chunk(end)", err));
        }
        Ok(())
    }

    /// End the response properly once the handler is done with it: the
    /// closing chunk of a chunked one; for a sized one, nothing — unless the
    /// body came up short, which is an error (the connection must be cut, or
    /// the client waits for bytes that are not coming).
    fn complete(&mut self) -> Result<()> {
        if self.chunked && !self.finished {
            return self.finish();
        }
        if let Some(total) = self.total {
            if self.sent < total {
                bail!("the handler stopped at {} of {total} B", self.sent);
            }
        }
        Ok(())
    }

    /// Log a handler's error: a response cut short ([`Req::log_cut`]) if
    /// anything went out, else one line naming the request.
    fn log_failure(&self, e: &anyhow::Error) {
        if self.streaming() {
            self.log_cut(&format!("{e:#}"));
        } else {
            log::warn!("{} {}: {e:#}", self.method_name(), self.uri());
        }
    }

    /// A streamed body (chunked or sized) has begun: a failure from here on
    /// leaves the client with part of it.
    fn streaming(&self) -> bool {
        self.chunked || self.total.is_some()
    }

    /// The one line for a response this device cut short, and the count of
    /// them: the path, how far it got, and the heap at that moment — so a
    /// client that received fewer bytes than declared can always be matched
    /// to a reason.
    fn log_cut(&self, why: &str) {
        STREAMS_ABORTED.fetch_add(1, Ordering::Relaxed);
        let total = self
            .total
            .map_or_else(|| "?".to_string(), |t| t.to_string());
        log::warn!(
            "{} {}: response cut short at {} of {total} B ({} B free): {why}",
            self.method_name(),
            self.path(),
            self.sent,
            crate::state::free_heap()
        );
    }

    /// Log a failed send once and turn it into [`SendFailed`].
    fn send_failed(&self, what: &str, err: i32) -> anyhow::Error {
        let gone = self.client_gone.get()
            || self
                .session
                .as_ref()
                .is_some_and(|s| s.closing.load(Ordering::Acquire));
        if gone {
            self.client_gone.set(true);
            log::info!(
                "{} {}: client went away after {} B",
                self.method_name(),
                self.path(),
                self.sent
            );
        } else if self.streaming() {
            self.log_cut(&format!("{what} failed ({err}); dropping the connection"));
        } else {
            log::warn!(
                "{} {}: {what} failed ({err}); dropping the connection",
                self.method_name(),
                self.path(),
            );
        }
        SendFailed.into()
    }

    /// Run one httpd send call. On the httpd task that is just the call; on a
    /// detached request it happens under the session lock, and not at all if
    /// httpd has closed the connection (see [`Session`]).
    fn guarded<F: FnOnce() -> sys::esp_err_t>(&self, send: F) -> Result<sys::esp_err_t> {
        let Some(session) = &self.session else {
            return Ok(send());
        };
        let gone = session.gone.lock().unwrap_or_else(|p| p.into_inner());
        if *gone {
            self.client_gone.set(true);
            log::info!(
                "{} {}: client went away after {} B",
                self.method_name(),
                self.path(),
                self.sent
            );
            return Err(SendFailed.into());
        }
        Ok(send())
    }

    /// Hand this request to another task and return, so the httpd task can
    /// serve other clients while the response is produced there
    /// (`httpd_req_async_handler_begin`). The handler must send nothing more
    /// on `self` once this succeeds and must return `Ok(())`; the other task
    /// runs the response with [`run_detached`], which completes the request.
    pub fn detach(&mut self) -> Result<Detached> {
        let mut copy: *mut sys::httpd_req_t = std::ptr::null_mut();
        // SAFETY: a live request on the httpd task; ESP-IDF copies it (and
        // its aux/header block) into allocations we own until
        // `httpd_req_async_handler_complete`, and marks the socket in use so
        // the LRU purge leaves it alone.
        let err = unsafe { sys::httpd_req_async_handler_begin(self.raw, &mut copy) };
        if err != sys::ESP_OK {
            bail!("httpd_req_async_handler_begin: {err}");
        }
        // SAFETY: both are plain reads of the live request.
        let (fd, handle) = unsafe { (sys::httpd_req_to_sockfd(self.raw), (*self.raw).handle) };
        let session = Arc::new(Session {
            fd,
            handle,
            closing: AtomicBool::new(false),
            gone: Mutex::new(false),
        });
        DETACHED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(session.clone());
        Ok(Detached { raw: copy, session })
    }
}

/// A request handed off the httpd task by [`Req::detach`]. Must end up in
/// [`run_detached`], which releases it.
pub struct Detached {
    raw: *mut sys::httpd_req_t,
    session: Arc<Session>,
}

// SAFETY: the copy belongs to whoever holds this, and every send on it is
// serialised against httpd's close of the socket by the session lock.
unsafe impl Send for Detached {}

/// One detached request's connection, as seen from both sides.
///
/// **Why the lock.** httpd keeps the socket in its `select` set while the
/// request is detached, and a detached request's sends go through the
/// session's *slot* (`sock_db`), not a copy of the fd. So if the client closes
/// its end, httpd deletes the session, closes the fd and frees the slot, and
/// the next `accept` can hand both to a new client — whose socket the
/// streaming task would then write the rest of a GIF into. httpd calls our
/// [`close_session`] before it closes anything; that marks the session `gone`
/// under this lock, which every send on a detached request holds. So a send
/// either finishes before the close, or never starts.
///
/// **Why `closing`.** The lock would make httpd wait for a send in progress,
/// and a send to a client that stopped reading takes up to
/// [`SEND_TIMEOUT_S`]. So `close_session` first sets `closing` and shuts the
/// socket down, which aborts that send at once (lwip is built with
/// `LWIP_NETCONN_FULLDUPLEX`, so a shutdown from another task cancels a
/// blocked write); `closing` also lets the failed send be logged as a client
/// going away rather than as a fault.
struct Session {
    fd: i32,
    handle: sys::httpd_handle_t,
    closing: AtomicBool,
    gone: Mutex<bool>,
}

// SAFETY: `handle` is only passed back to httpd's thread-safe
// `httpd_sess_trigger_close`.
unsafe impl Send for Session {}
unsafe impl Sync for Session {}

/// Every detached request whose connection is still open. At most a few
/// entries (see `stream::QUEUE_DEPTH`), and each fd appears once: an entry
/// leaves when its connection closes, before the fd can be reused.
static DETACHED: Mutex<Vec<Arc<Session>>> = Mutex::new(Vec::new());

/// httpd's `close_fn`: called on the httpd task by `httpd_sess_delete` for
/// every client socket it closes, before the fd or the session slot are
/// released.
unsafe extern "C" fn close_session(_hd: sys::httpd_handle_t, fd: i32) {
    let hit = {
        let mut detached = DETACHED.lock().unwrap_or_else(|p| p.into_inner());
        detached
            .iter()
            .position(|s| s.fd == fd)
            .map(|i| detached.swap_remove(i))
    };
    if let Some(session) = hit {
        session.closing.store(true, Ordering::Release);
        // SAFETY: `fd` is still open (we close it below); a shutdown aborts a
        // send blocked on it from the streaming task.
        sys::lwip_shutdown(fd, sys::SHUT_RDWR as i32);
        *session.gone.lock().unwrap_or_else(|p| p.into_inner()) = true;
    }
    // SAFETY: httpd hands us the fd to close; this is what it would have done.
    sys::close(fd);
}

/// Complete a detached request, and drop its connection if `abort`.
///
/// Under the session lock, so the connection is still this request's while we
/// touch it. `httpd_req_async_handler_complete` clears the socket's in-use
/// mark through the session slot; when the connection is already gone that
/// slot may belong to a newer connection, whose mark it then clears too. The
/// only effect is that the LRU purge could pick that connection — rare, and
/// harmless beyond one dropped request, so it is accepted rather than worked
/// around through httpd's private structs.
fn release(raw: *mut sys::httpd_req_t, session: &Arc<Session>, abort: bool) {
    {
        let gone = session.gone.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: `raw` is the copy from `httpd_req_async_handler_begin`,
        // released exactly once, here.
        unsafe { sys::httpd_req_async_handler_complete(raw) };
        if abort && !*gone {
            // Half a response went out. Shut the socket so the client sees
            // the end now, and have httpd close the session; the fd is still
            // this connection's, because `gone` is false under the lock.
            // SAFETY: plain socket and httpd calls on a live fd.
            unsafe {
                sys::lwip_shutdown(session.fd, sys::SHUT_RDWR as i32);
                sys::httpd_sess_trigger_close(session.handle, session.fd);
            }
        }
    }
    let mut detached = DETACHED.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(i) = detached.iter().position(|s| Arc::ptr_eq(s, session)) {
        detached.swap_remove(i);
    }
}

/// Produce the response to a detached request on the calling task, with the
/// same contract as a handler on the httpd task: `Ok(())` having responded (an
/// unfinished chunked response is finished here), or `Err` — a 500 if nothing
/// went out, a dropped connection otherwise. Then the request is released.
pub fn run_detached<F>(detached: Detached, handler: F)
where
    F: FnOnce(&mut Req) -> Result<()>,
{
    let Detached { raw, session } = detached;
    // A client that left while this waited in the queue (a page reload, a
    // cancelled preview): do not open its file at all.
    if *session.gone.lock().unwrap_or_else(|p| p.into_inner()) {
        release(raw, &session, false);
        return;
    }
    let mut req = Req::new(raw);
    req.session = Some(session.clone());
    let abort = match handler(&mut req) {
        Ok(()) => match req.complete() {
            Ok(()) => false,
            Err(e) => {
                if !already_logged(&e) {
                    req.log_failure(&e);
                }
                true
            }
        },
        Err(e) => {
            if !already_logged(&e) {
                req.log_failure(&e);
            }
            // Half a response out: drop the connection. Nothing out: a 500,
            // and drop it only if even that fails.
            req.responded
                || req
                    .send_status(
                        500,
                        "500 Internal Server Error",
                        "application/json",
                        b"{\"error\":\"internal error\"}",
                    )
                    .is_err()
        }
    };
    drop(req);
    release(raw, &session, abort);
}

/// Quote and escape a string as a JSON value.
///
/// Small enough to do by hand, and it keeps `serde_json::to_string` off the
/// hot paths that build JSON a field at a time.
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Pull one `key=value` out of a query string, percent-decoded.
pub fn query_param(query: Option<&str>, key: &str) -> Option<String> {
    let q = query?;
    q.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

/// Percent-decode, treating `+` as a space (form encoding) and passing
/// malformed escapes through unchanged.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    None => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
