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
//! detached the request, or taken its connection over), or `Err(_)` and the
//! wrapper sends a 500 if nothing has gone out yet, or drops the connection if
//! something has.
//!
//! **Uploads leave httpd altogether** ([`Req::take_over`] -> [`Upload`]). A
//! detached request is no use for one: httpd keeps a detached socket in its
//! `select` set, so body bytes arriving for it would be read by the httpd
//! task as the next request, and when the handler returns httpd reads and
//! discards whatever of the body is left (`httpd_req_delete`'s purge) — on
//! its own task, at the client's pace. So the socket is taken out of httpd's
//! hands instead: the bytes httpd already buffered are drained, and the
//! session's `recv` is overridden to report a closed connection, so httpd
//! never reads the socket again — its purge of the rest of the body, right
//! after the handler, deletes the session at once (a body httpd had already
//! read whole is not purged: that session goes when the client closes, the
//! next time `select` sees the socket). Either way httpd calls our
//! `close_fn`, which, for a taken socket, does not close it. From then on the
//! socket is the [`Upload`]'s: read and answered with plain lwip calls, and
//! closed by whichever of the two lets go last.

use anyhow::{bail, Result};
use esp_idf_sys as sys;
use std::cell::Cell;
use std::ffi::{c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config;

/// `SO_SNDTIMEO` on every client socket, in seconds: how long one `send` may
/// wait for a client that is not reading before it fails with EAGAIN. That
/// failure ends the response (see [`Req::chunk`]), so this is also the longest
/// a stalled client can hold whichever task is sending to it.
const SEND_TIMEOUT_S: u16 = 5;
/// `SO_RCVTIMEO` on every client socket (httpd's `recv_wait_timeout`), in
/// seconds: the longest the httpd task waits for a client's next bytes — the
/// rest of a request's headers, a JSON body ([`Req::read`]), or the unread
/// rest of a body httpd purges after a refusal. **One wait, never retried.**
///
/// While the httpd task waits nothing else is served: no `accept`, so lwIP's
/// listen backlog (5) fills and further SYNs are dropped (a client's connect
/// takes 1 s, 3 s: its SYN retransmissions); no refusal of a new upload, whose
/// unread frames keep the WiFi driver's receive buffers (see "The receive
/// side" in `sdkconfig.defaults`), which is what starves the body being
/// waited for in the first place. It was 10 s, retried three times in a row
/// for a body, then 10 s more for httpd's purge: one live edit stalled under
/// simultaneous uploads held every client for up to 40 s (the spare: six of
/// 190 `POST /api/play/formula` timed out at 20 s; net-stall, 2026-11-05).
/// The bodies read here are ≤ 16 KB and arrive in milliseconds on a working
/// link; one that stops for this long is answered `503` + `Retry-After` and
/// its connection dropped ([`BodyStalled`]), and the web app asks again.
/// Uploads are not read here ([`Upload`] sets its own timeouts).
const RECV_TIMEOUT_S: u16 = 2;

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

/// A body read on the httpd task ([`Req::read`]) that went [`RECV_TIMEOUT_S`]
/// without a byte: answered `503` + `Retry-After` by the wrapper, which then
/// drops the connection (no purge of the rest on the httpd task), and
/// counted ([`bodies_stalled`], `net.body_stalled` in `/api/status`).
#[derive(Debug)]
pub struct BodyStalled;

impl std::fmt::Display for BodyStalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the body stopped for {RECV_TIMEOUT_S} s")
    }
}

impl std::error::Error for BodyStalled {}

static BODIES_STALLED: AtomicU32 = AtomicU32::new(0);

/// Bodies given up on by the httpd task since boot ([`BodyStalled`]).
pub fn bodies_stalled() -> u32 {
    BODIES_STALLED.load(Ordering::Relaxed)
}

/// An upload given up on part way (a receive timeout, the total-time cap,
/// the client gone, a card write that failed), already logged by
/// [`crate::files::stream_body_to`] with the bytes received. `status` is what
/// to answer, if anything can still be said.
#[derive(Debug)]
pub struct UploadAborted {
    pub status: Option<(u16, &'static str)>,
}

impl std::fmt::Display for UploadAborted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("upload aborted")
    }
}

impl std::error::Error for UploadAborted {}

/// Why [`Upload::read`] failed: the status to answer with (a timeout is a
/// 408; a client that is gone gets nothing).
#[derive(Debug)]
pub struct BodyError {
    pub status: Option<(u16, &'static str)>,
    what: String,
}

impl std::fmt::Display for BodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.what)
    }
}

impl std::error::Error for BodyError {}

/// Was this error already logged where it happened?
fn already_logged(e: &anyhow::Error) -> bool {
    e.is::<SendFailed>() || e.is::<UploadAborted>()
}

/// The JSON responses a handler sends, on an httpd request ([`Req`]) or on a
/// connection taken over for an upload ([`Upload`]) alike.
pub trait Respond {
    /// A complete `application/json` response with this status line.
    fn respond_json(&mut self, status: &str, body: &str) -> Result<()>;

    fn json(&mut self, body: &str) -> Result<()> {
        self.respond_json("200 OK", body)
    }

    /// A JSON error body with the given status, e.g. `(404, "404 Not Found")`.
    fn error(&mut self, _code: u16, status: &str, message: &str) -> Result<()> {
        let body = format!(r#"{{"error":{}}}"#, json_string(message));
        self.respond_json(status, &body)
    }

    fn not_found(&mut self, message: &str) -> Result<()> {
        self.error(404, "404 Not Found", message)
    }

    fn bad_request(&mut self, message: &str) -> Result<()> {
        self.error(400, "400 Bad Request", message)
    }
}

impl Respond for Req {
    fn respond_json(&mut self, status: &str, body: &str) -> Result<()> {
        self.send_status(0, status, "application/json", body.as_bytes())
    }
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

// --- The request gate (`opendmd_core::access`, docs/api.md "Security") ----

/// The API key (`api_key` in the settings), empty = none. Set at start-up
/// and by `PUT /api/config`.
static API_KEY: Mutex<String> = Mutex::new(String::new());
/// The device is the setup access point: no key is asked, so a lost key can
/// be cleared from the setup network, and only the setup routes answer
/// (`opendmd_core::access::setup_route`).
static SETUP_MODE: AtomicBool = AtomicBool::new(false);
/// The names a request's `Host` may carry besides the fixed ones
/// (`opendmd_core::access::host_allowed`): the configured name, the one
/// mDNS answers to, the current address. Kept by [`set_names`].
static NAMES: Mutex<[String; 4]> = Mutex::new([String::new(), String::new(), String::new(), String::new()]);

pub fn set_api_key(key: &str) {
    *API_KEY.lock().unwrap_or_else(|p| p.into_inner()) = key.to_string();
}

pub fn set_setup_mode(on: bool) {
    SETUP_MODE.store(on, Ordering::Relaxed);
}

/// Whether the device is the setup network now (the settings handlers ask).
pub fn setup_mode() -> bool {
    SETUP_MODE.load(Ordering::Relaxed)
}

/// The panel's own names, from the status (`hostname`, `mdns`, `ip`):
/// called wherever the network worker changes one of them. A rename keeps
/// the name before it too (`access::Names::was`), until the restart.
#[optimize(size)]
pub fn set_names(hostname: &str, mdns: &str, ip: &str) {
    // (No request can arrive before the network worker's first `publish`,
    // which calls this: the names are never stale at a request.)
    let mut n = NAMES.lock().unwrap_or_else(|p| p.into_inner());
    if n[0] != hostname {
        n[3] = std::mem::replace(&mut n[0], hostname.to_string());
    }
    n[1] = mdns.to_string();
    n[2] = ip.to_string();
}

/// Answer a refused request; `close` drops the connection after it.
fn refuse(req: &mut Req, close: bool, code: u16, status: &str, why: &str) -> Option<bool> {
    if close {
        let _ = req.header("Connection", "close");
    }
    let _ = req.error(code, status, why);
    Some(close)
}

/// Every request passes here before its handler (see [`trampoline`]): a
/// `Host` that is not one of the panel's names (421, DNS rebinding), the
/// setup network's short list of routes (403), the API key where one is set
/// (401), then a mutating request from a page on another host (403).
/// `Some(close)` if it was refused (the answer is sent; `close` drops the
/// connection rather than letting httpd read the rest of a large body on
/// its own task). Never inlined: its header buffers (257 B each) are off
/// the stack before the handler runs, whose depth is what the httpd task's
/// ~1 KB of headroom is kept for (`config::HTTP_STACK`).
#[inline(never)]
#[optimize(size)]
fn gate(req: &mut Req) -> Option<bool> {
    use opendmd_core::access::{self, Setup};
    let path = req.path().to_string();
    let close = req.content_len() > 8192;
    let setup = SETUP_MODE.load(Ordering::Relaxed);
    let get = req.method_name() == "GET";
    // Where the setup network sends a page it does not serve.
    let portal = || {
        let ip = NAMES.lock().unwrap_or_else(|p| p.into_inner())[2].clone();
        format!("http://{}/", if ip.is_empty() { "192.168.4.1" } else { &ip })
    };
    // A `Host` too long to read is refused like a foreign one; none at all
    // (an HTTP/1.0 script) is not a browser, so not a rebinding.
    let host = req.request_header(c"Host");
    if req.header_len(c"Host") > 0 {
        let host = host.as_deref().unwrap_or_default();
        let refused = {
            let n = NAMES.lock().unwrap_or_else(|p| p.into_inner());
            let names = access::Names {
                hostname: &n[0],
                mdns: &n[1],
                ip: &n[2],
                was: &n[3],
            };
            !access::host_allowed(host, &names)
        };
        if refused {
            // The setup network answers every name with itself (its DNS):
            // a phone's captive-portal probe for another host is sent to
            // the setup page.
            if setup && get {
                let _ = req.redirect(&portal());
                return Some(false);
            }
            log::warn!("{path}: 421, Host {host}");
            return refuse(req, close, 421, "421 Misdirected Request", access::MISDIRECTED);
        }
    }
    if setup {
        match access::setup_route(req.method_name(), &path) {
            Setup::Allow => {}
            Setup::Portal => {
                let _ = req.redirect(&portal());
                return Some(false);
            }
            Setup::Refuse => {
                return refuse(
                    req,
                    close,
                    403,
                    "403 Forbidden",
                    "the setup network only sets up WiFi",
                );
            }
        }
    }
    if access::needs_key(&path) && !setup {
        let key = API_KEY.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if !key.is_empty() {
            let auth = req.request_header(c"Authorization");
            let q = query_param(req.query(), "key");
            if !access::key_ok(&key, auth.as_deref(), q.as_deref()) {
                log::info!("{} {path}: 401, no or wrong API key", req.method_name());
                let _ = req.header("WWW-Authenticate", "Bearer");
                return refuse(req, close, 401, "401 Unauthorized", "API key required");
            }
        }
    }
    if !get {
        if let Some(origin) = req.request_header(c"Origin") {
            let host = host.unwrap_or_default();
            if !access::same_origin(&origin, &host) {
                log::warn!(
                    "{} {path}: 403, a page from {origin} (the device is {host})",
                    req.method_name()
                );
                return refuse(req, close, 403, "403 Forbidden", "cross-origin request refused");
            }
        }
    }
    None
}

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
            // Enough for the most any response sets: the web app's five
            // (Content-Encoding, ETag, Cache-Control, CSP, nosniff), a card
            // file's four or five (Cache-Control, nosniff, CSP,
            // Content-Disposition, Retry-After). 8 B of heap a slot.
            cfg.max_resp_headers = 8;
            // lwIP drops a SYN once this many connections wait for
            // `accept` (handshaking, or established and not yet taken): the
            // client's connect then takes 1 s (its first SYN retransmission).
            // Five uploads at once and a status poll are six; more than the
            // accept mailbox holds would be reset instead (lwIP aborts an
            // established connection it cannot post), so exactly that.
            cfg.backlog_conn = sys::CONFIG_LWIP_TCP_ACCEPTMBOX_SIZE as u16;
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

    /// [`Server::route`] for a handler that reads a JSON body: a non-empty
    /// body must come as `Content-Type: application/json`, else 415 (see
    /// `opendmd_core::access`).
    pub fn route_json<F>(&mut self, uri: &str, method: Method, handler: F) -> Result<()>
    where
        F: Fn(&mut Req) -> Result<()> + Send + Sync + 'static,
    {
        self.route(uri, method, #[optimize(size)] move |req| {
            if req.content_len() > 0 {
                let ct = req.request_header(c"Content-Type").unwrap_or_default();
                if !opendmd_core::access::is_json(&ct) {
                    return req.error(
                        415,
                        "415 Unsupported Media Type",
                        "Content-Type must be application/json",
                    );
                }
            }
            handler(req)
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
        #[cfg(feature = "diag-heap")]
        guard::register(boxed);
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
    crate::diag::check("httpd: handler start");
    #[cfg(feature = "diag-heap")]
    guard::verify(handler, "handler start");
    let mut req = Req::new(raw);
    if let Some(close) = gate(&mut req) {
        return if close { sys::ESP_FAIL } else { sys::ESP_OK };
    }
    crate::netstat::sample(crate::netstat::At::Request);
    crate::stream::set_phase(0, if req.path() == "/api/status" { 1 } else { 3 });
    let result = (*handler)(&mut req);
    crate::stream::set_phase(0, 0);
    stack_low_water("httpd", &HTTPD_STACK_LOW);
    crate::diag::check("httpd: handler end");
    #[cfg(feature = "diag-heap")]
    guard::verify(handler, "handler end");
    if req.taken {
        // The connection is an `Upload`'s now; httpd must let the session
        // go without touching the socket. With body bytes still to come,
        // returning OK sends httpd into its purge, whose first read (our
        // override) reports the connection closed and deletes the session.
        // With none, the session stays until the client closes (or the LRU
        // purge picks it), and goes the same way. Not `ESP_FAIL`, which
        // would delete it now too, but with an IDF warning line per upload.
        if let Err(e) = &result {
            if !already_logged(e) {
                req.log_failure(e);
            }
        }
        return sys::ESP_OK;
    }
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
            } else if e.is::<BodyStalled>() {
                // Busy, not broken: asked again by the web app (`sent`). Then
                // the connection goes at once (ESP_FAIL): httpd's purge of the
                // rest would wait on this task for a body that has stopped.
                BODIES_STALLED.fetch_add(1, Ordering::Relaxed);
                let _ = req.header("Retry-After", "2");
                let _ = req.header("Connection", "close");
                let _ = req.error(
                    503,
                    "503 Service Unavailable",
                    "the request's body stopped arriving; retry shortly",
                );
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

/// `diag-heap` only: every registered handler's box — the `Box<dyn Fn>`
/// fat pointer that `user_ctx` points at — as it was when registered,
/// checked on every request. The `/api/play` crash (CLAUDE.md) faulted
/// loading the closure's `Arc<Ctx>` through a null environment pointer.
#[cfg(feature = "diag-heap")]
mod guard {
    use super::Handler;
    use std::sync::Mutex;

    static TABLE: Mutex<Vec<(usize, [usize; 2])>> = Mutex::new(Vec::new());

    fn words(h: *const Handler) -> [usize; 2] {
        // SAFETY: `h` is a live `Box<Box<dyn Fn>>` allocation; its two
        // words are the inner box's data and vtable pointers.
        unsafe { std::ptr::read(h.cast::<[usize; 2]>()) }
    }

    pub fn register(h: *mut Handler) {
        let w = words(h);
        TABLE.lock().unwrap().push((h as usize, w));
    }

    /// Every entry unchanged, and `h` one of them.
    pub fn verify(h: *const Handler, site: &str) {
        let table = TABLE.lock().unwrap();
        let mut known = false;
        for &(at, was) in table.iter() {
            known |= at == h as usize;
            let now = words(at as *const Handler);
            if now != was {
                // SAFETY: NUL-terminated format; plain integers.
                unsafe {
                    esp_idf_sys::esp_rom_printf(
                        c"\nDIAG: handler box at 0x%08x changed: data 0x%08x -> 0x%08x, vtable 0x%08x -> 0x%08x\n".as_ptr(),
                        at as u32, was[0] as u32, now[0] as u32, was[1] as u32, now[1] as u32,
                    );
                    crate::restarts::diag("handler box changed");
                    esp_idf_sys::abort();
                }
            }
        }
        if !known {
            log::error!("DIAG: {site}: user_ctx {:p} is not a registered handler", h);
            crate::restarts::diag("unregistered handler");
            // SAFETY: plain abort.
            unsafe { esp_idf_sys::abort() };
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

/// `uri` for the log: a query with a `key=` in it is never written down.
pub fn log_uri(uri: &str) -> &str {
    match uri.find("key=") {
        Some(_) => uri.split('?').next().unwrap_or(""),
        None => uri,
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
    /// Set by [`Req::take_over`]: the connection is an [`Upload`]'s.
    taken: bool,
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
            taken: false,
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

    /// A request header's length, 0 if it is absent.
    pub fn header_len(&self, field: &CStr) -> usize {
        // SAFETY: a live request and a NUL-terminated field name.
        unsafe { sys::httpd_req_get_hdr_value_len(self.raw, field.as_ptr()) }
    }

    /// A request header's value (at most 256 B; longer is treated as absent).
    pub fn request_header(&self, field: &CStr) -> Option<String> {
        // SAFETY: a live request and a NUL-terminated field name.
        let len = unsafe { sys::httpd_req_get_hdr_value_len(self.raw, field.as_ptr()) };
        if len == 0 || len > 256 {
            return None;
        }
        let mut buf = [0u8; 257];
        // SAFETY: `buf` holds `len` bytes and the NUL.
        let err = unsafe {
            sys::httpd_req_get_hdr_value_str(
                self.raw,
                field.as_ptr(),
                buf.as_mut_ptr().cast(),
                len + 1,
            )
        };
        if err != sys::ESP_OK {
            return None;
        }
        Some(String::from_utf8_lossy(&buf[..len]).into_owned())
    }

    /// Whether a request header names `want`: equal to it, or one of a
    /// comma-separated list, a weak `W/` form too (`If-None-Match`). Read
    /// into a 64 B stack buffer, no heap: this runs on the httpd task
    /// (~1 KB free at its deepest, `config::HTTP_STACK`), and a longer value
    /// counts as not naming it.
    pub fn header_names(&self, field: &CStr, want: &str) -> bool {
        // SAFETY: a live request and a NUL-terminated field name.
        let len = unsafe { sys::httpd_req_get_hdr_value_len(self.raw, field.as_ptr()) };
        let mut buf = [0u8; 64];
        if len == 0 || len >= buf.len() {
            return false;
        }
        // SAFETY: `buf` holds `len` bytes and the NUL.
        let err = unsafe {
            sys::httpd_req_get_hdr_value_str(
                self.raw,
                field.as_ptr(),
                buf.as_mut_ptr().cast(),
                len + 1,
            )
        };
        err == sys::ESP_OK
            && std::str::from_utf8(&buf[..len]).is_ok_and(|v| {
                v.split(',')
                    .map(str::trim)
                    .any(|t| t == want || t.strip_prefix("W/") == Some(want))
            })
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
    /// Only the small bodies (JSON ≤ 8 KB, a compiled Formula ≤ 16 KB) are
    /// read this way, on the httpd task; uploads are [`Upload`]s, read on
    /// the stream task. A socket timeout ([`RECV_TIMEOUT_S`]) is not
    /// retried: it is a [`BodyStalled`], which the wrapper answers `503` +
    /// `Retry-After` before it drops the connection.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // SAFETY: `buf` is a live mutable slice; httpd writes at most
        // `buf.len()` bytes into it.
        let n = unsafe { sys::httpd_req_recv(self.raw, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n >= 0 {
            return Ok(n as usize);
        }
        if n == sys::HTTPD_SOCK_ERR_TIMEOUT {
            return Err(BodyStalled.into());
        }
        bail!("socket read error {n}");
    }

    /// Read the whole body, refusing anything over `limit`; `Ok(None)`
    /// when the heap has no block for it (taken fallibly: `with_capacity`
    /// would abort the board on a fragmented heap).
    pub fn read_to_vec(&mut self, limit: usize) -> Result<Option<Vec<u8>>> {
        let declared = self.content_len();
        if declared > limit {
            bail!("body of {declared} B exceeds the {limit} B limit");
        }
        let mut out = Vec::new();
        if out.try_reserve_exact(declared).is_err() {
            return Ok(None);
        }
        let mut buf = [0u8; 1024];
        loop {
            let n = self.read(&mut buf)?;
            if n == 0 {
                break;
            }
            if out.len() + n > limit {
                bail!("body exceeds the {limit} B limit");
            }
            if out.try_reserve(n).is_err() {
                return Ok(None);
            }
            out.extend_from_slice(&buf[..n]);
        }
        Ok(Some(out))
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

    /// The web app's security headers (`opendmd_core::access::APP_CSP`):
    /// its policy (`frame-ancestors 'none'`: never in a frame), no type
    /// sniffing.
    pub fn app_headers(&mut self, csp: &'static str) -> Result<()> {
        self.header("Content-Security-Policy", csp)?;
        self.header("X-Content-Type-Options", "nosniff")
    }

    /// A card file's: no sniffing and a sandbox whatever it holds; not an
    /// image (`media` false), a download as well.
    pub fn card_headers(&mut self, media: bool) -> Result<()> {
        self.header("X-Content-Type-Options", "nosniff")?;
        self.header("Content-Security-Policy", "sandbox; default-src 'none'")?;
        if !media {
            self.header("Content-Disposition", "attachment")?;
        }
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
            log::warn!("{} {}: {e:#}", self.method_name(), log_uri(self.uri()));
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

    /// Take this request's connection away from httpd, for an upload whose
    /// body is to be received on another task (see the module docs).
    ///
    /// The body bytes httpd read along with the headers (at most 128, its
    /// `pending_data`) are drained into the [`Upload`] first; the session's
    /// `recv` is then overridden to report a closed connection, so nothing on
    /// the httpd task reads the socket again, and the session is deleted as
    /// soon as the handler returns — which the handler must do at once,
    /// having sent nothing on `self`.
    pub fn take_over(&mut self) -> Result<Upload> {
        // SAFETY: plain reads of the live request.
        let (fd, handle) = unsafe { (sys::httpd_req_to_sockfd(self.raw), (*self.raw).handle) };
        let len = self.content_len();
        // SAFETY: `handle` and `fd` are this live request's.
        let err = unsafe { sys::httpd_sess_set_recv_override(handle, fd, Some(recv_pending_only)) };
        if err != sys::ESP_OK {
            bail!("httpd_sess_set_recv_override: {err}");
        }
        let mut prefix = vec![0u8; len.min(PENDING_MAX)];
        let mut got = 0;
        if !prefix.is_empty() {
            // SAFETY: `prefix` is a live buffer of the length given. With the
            // override, this returns httpd's buffered bytes and never waits.
            let n = unsafe {
                sys::httpd_req_recv(self.raw, prefix.as_mut_ptr() as *mut _, prefix.len())
            };
            got = n.max(0) as usize;
        }
        prefix.truncate(got);
        // SAFETY: as above.
        unsafe { sys::httpd_sess_set_recv_override(handle, fd, Some(recv_closed)) };
        TAKEN.lock().unwrap_or_else(|p| p.into_inner()).push(Taken {
            fd,
            httpd_done: false,
            owner_done: false,
        });
        self.taken = true;
        self.responded = true;
        Ok(Upload {
            fd,
            method: self.method_name(),
            path: self.path().to_string(),
            len: len as u64,
            received: 0,
            prefix,
            pos: 0,
            started: Instant::now(),
            recv_timeout: Duration::from_secs(RECV_TIMEOUT_S.into()),
            extra_headers: String::new(),
            responded: false,
            read_failed: false,
        })
    }
}

/// httpd's `pending_data` (`PARSER_BLOCK_SIZE` in `esp_httpd_priv.h`): the
/// most body it can have read along with the headers.
const PENDING_MAX: usize = 128;
/// How long a finished [`Upload`] waits for the client to close first.
const UPLOAD_LINGER: Duration = Duration::from_secs(1);
/// How long an [`Upload`] refused before its body was all in keeps reading
/// (and dropping) the body after its reply, for the client to stop.
const REFUSED_LINGER: Duration = Duration::from_secs(2);

/// A taken-over session's `recv` while its buffered bytes are drained: never
/// touch the socket (httpd then returns what it had buffered).
unsafe extern "C" fn recv_pending_only(
    _hd: sys::httpd_handle_t,
    _fd: i32,
    _buf: *mut core::ffi::c_char,
    _len: usize,
    _flags: i32,
) -> i32 {
    sys::HTTPD_SOCK_ERR_TIMEOUT
}

/// A taken-over session's `recv` from then on: the connection is closed, as
/// far as httpd is concerned.
unsafe extern "C" fn recv_closed(
    _hd: sys::httpd_handle_t,
    _fd: i32,
    _buf: *mut core::ffi::c_char,
    _len: usize,
    _flags: i32,
) -> i32 {
    0
}

/// `SO_RCVTIMEO` on a socket.
fn set_recv_timeout(fd: i32, timeout: Duration) {
    let tv = sys::timeval {
        tv_sec: timeout.as_secs() as _,
        tv_usec: timeout.subsec_micros() as _,
    };
    // SAFETY: a live socket and a `timeval` that outlives the call.
    unsafe {
        sys::lwip_setsockopt(
            fd,
            sys::SOL_SOCKET as i32,
            sys::SO_RCVTIMEO as i32,
            &tv as *const _ as *const c_void,
            std::mem::size_of::<sys::timeval>() as u32,
        );
    }
}

/// A socket taken over by [`Req::take_over`]: who has let go of it. httpd
/// lets go when it deletes the session (our `close_fn`), the [`Upload`] when
/// it is dropped; whichever is second closes the fd. Until then the fd stays
/// open, so its number cannot be handed to a new connection while either
/// side might still use it.
struct Taken {
    fd: i32,
    httpd_done: bool,
    owner_done: bool,
}

static TAKEN: Mutex<Vec<Taken>> = Mutex::new(Vec::new());

/// The request's connection, out of httpd's hands: an upload whose body is
/// read (and whose reply is sent) with plain socket calls, on whichever task
/// holds this — the stream task, normally.
///
/// **Reading gives up** after [`config::UPLOAD_IDLE_TIMEOUT`] without data,
/// or once the body has taken [`config::UPLOAD_MAX_TIME`] since
/// [`Upload::start_clock`], however steadily a trickle keeps it alive. Every
/// response closes the connection (`Connection: close`): it may be sent
/// before the body is all in.
///
/// **A refusal goes out at once**, not after the body: a handler that
/// answers before it has read everything (a bad signature at the bundle's
/// 140-byte header, a pack header that is wrong, no card, …) simply replies
/// and returns. curl stops sending when a 4xx/5xx arrives mid-upload and
/// closes ("HTTP error before end of send, stop sending"); the drop then
/// reads and discards whatever is still in flight for up to
/// [`REFUSED_LINGER`], until the client's FIN, before it closes, because
/// closing a socket with unread data makes lwip send a reset, which could
/// overtake the reply. A client that keeps sending (a browser's XHR) gets
/// that reset after [`REFUSED_LINGER`]; Chromium then reads the reply that
/// came before it.
pub struct Upload {
    fd: i32,
    method: &'static str,
    path: String,
    len: u64,
    received: u64,
    /// What httpd had already read of the body, handed over first.
    prefix: Vec<u8>,
    pos: usize,
    started: Instant,
    recv_timeout: Duration,
    extra_headers: String,
    responded: bool,
    /// A read failed (cut short, timed out, the client went): nothing more
    /// will come worth waiting for.
    read_failed: bool,
}

impl Upload {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn method(&self) -> &'static str {
        self.method
    }

    /// The declared `Content-Length`.
    pub fn content_len(&self) -> u64 {
        self.len
    }

    /// Body bytes read so far.
    pub fn received(&self) -> u64 {
        self.received
    }

    /// How long reception has been going.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn responded(&self) -> bool {
        self.responded
    }

    /// Start the [`config::UPLOAD_MAX_TIME`] clock: when reception starts,
    /// not when the request was queued.
    pub fn start_clock(&mut self) {
        self.started = Instant::now();
    }

    /// Read up to `buf.len()` body bytes; `Ok(0)` once all `Content-Length`
    /// bytes are in. A failure is a [`BodyError`].
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let r = self.read_inner(buf);
        if r.is_err() {
            self.read_failed = true;
        }
        r
    }

    fn read_inner(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.received >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let want = ((self.len - self.received) as usize).min(buf.len());
        if self.pos < self.prefix.len() {
            let n = want.min(self.prefix.len() - self.pos);
            buf[..n].copy_from_slice(&self.prefix[self.pos..self.pos + n]);
            self.pos += n;
            self.received += n as u64;
            return Ok(n);
        }
        let timeout_408 = Some((408, "408 Request Timeout"));
        let left = config::UPLOAD_MAX_TIME.saturating_sub(self.started.elapsed());
        if left.is_zero() {
            return Err(BodyError {
                status: timeout_408,
                what: format!(
                    "the body took longer than the {} s cap",
                    config::UPLOAD_MAX_TIME.as_secs()
                ),
            }
            .into());
        }
        // Each read waits at most the idle timeout, and never past the cap.
        let wait = left
            .min(config::UPLOAD_IDLE_TIMEOUT)
            .max(Duration::from_millis(10));
        if wait != self.recv_timeout {
            set_recv_timeout(self.fd, wait);
            self.recv_timeout = wait;
        }
        // SAFETY: `buf` is a live buffer of at least `want` bytes; the socket
        // is ours (see `Taken`).
        let n = unsafe { sys::lwip_recv(self.fd, buf.as_mut_ptr() as *mut c_void, want, 0) };
        if n > 0 {
            self.received += n as u64;
            return Ok(n as usize);
        }
        if n == 0 {
            return Err(BodyError {
                status: None,
                what: "the client closed the connection".into(),
            }
            .into());
        }
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if errno == sys::EAGAIN as i32 {
            let what = if wait < config::UPLOAD_IDLE_TIMEOUT {
                format!(
                    "the body took longer than the {} s cap",
                    config::UPLOAD_MAX_TIME.as_secs()
                )
            } else {
                format!("no data for {} s", config::UPLOAD_IDLE_TIMEOUT.as_secs())
            };
            return Err(BodyError {
                status: timeout_408,
                what,
            }
            .into());
        }
        Err(BodyError {
            status: None,
            what: format!("socket read error (errno {errno})"),
        }
        .into())
    }

    /// Close at once when dropped, even after an early reply: for a refusal
    /// made on the httpd task, which must not wait [`REFUSED_LINGER`].
    pub fn no_linger(&mut self) {
        self.read_failed = true;
    }

    /// Append a response header (before the response).
    pub fn header(&mut self, field: &str, value: &str) {
        self.extra_headers.push_str(field);
        self.extra_headers.push_str(": ");
        self.extra_headers.push_str(value);
        self.extra_headers.push_str("\r\n");
    }

    /// Write all of `buf` to the socket (`SO_SNDTIMEO` is httpd's 5 s).
    fn send_all(&self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            // SAFETY: a live slice; the socket is ours.
            let n = unsafe { sys::lwip_send(self.fd, buf.as_ptr() as *const c_void, buf.len(), 0) };
            if n <= 0 {
                bail!("send failed after {} B", self.received);
            }
            buf = &buf[(n as usize).min(buf.len())..];
        }
        Ok(())
    }
}

impl Respond for Upload {
    fn respond_json(&mut self, status: &str, body: &str) -> Result<()> {
        if self.responded {
            return Ok(());
        }
        self.responded = true;
        let msg = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{body}",
            body.len(),
            self.extra_headers
        );
        if let Err(e) = self.send_all(msg.as_bytes()) {
            log::info!(
                "{} {}: the reply was not delivered: {e}",
                self.method,
                self.path
            );
            return Err(SendFailed.into());
        }
        Ok(())
    }
}

impl Drop for Upload {
    /// **The client closes first, if it will.** Whichever end closes a TCP
    /// connection first keeps it in TIME_WAIT for 2 x `TCP_MSL` (120 s), and
    /// this lwip allocates its pcbs from the heap with no limit on how many
    /// wait: when the device closed first, a `card-sync` of 71 files (~100
    /// uploads in two minutes, ~200 B each) took the free heap to 13 KB with
    /// a 1.6 KB largest block, and the next 2 KB chunk buffer aborted the
    /// board. So after a complete exchange the socket is read until the
    /// client's FIN (it closes once it has the reply: `Connection: close`),
    /// for at most [`UPLOAD_LINGER`]; only an upload cut short — body bytes
    /// may still be arriving — is shut down at once.
    fn drop(&mut self) {
        let complete = self.received >= self.len;
        if self.responded && (complete || !self.read_failed) {
            // Complete: wait for the client's FIN. Refused early (see the
            // type's docs): read off the rest of the body as it stops, so
            // the close does not reset the connection under the reply.
            let linger = if complete {
                UPLOAD_LINGER
            } else {
                REFUSED_LINGER
            };
            set_recv_timeout(self.fd, linger);
            let until = Instant::now() + linger;
            let mut sink = [0u8; 256];
            let mut dropped = 0usize;
            loop {
                // SAFETY: a live buffer; the socket is ours.
                let n = unsafe {
                    sys::lwip_recv(self.fd, sink.as_mut_ptr() as *mut c_void, sink.len(), 0)
                };
                if n <= 0 || Instant::now() >= until {
                    break;
                }
                dropped += n as usize;
            }
            if !complete {
                log::info!(
                    "{} {}: refused at {} of {} B; {dropped} B more read off before closing",
                    self.method,
                    self.path,
                    self.received,
                    self.len
                );
                // SAFETY: as below.
                unsafe { sys::lwip_shutdown(self.fd, sys::SHUT_RDWR as i32) };
            }
        } else {
            // SAFETY: the socket is still open (see `Taken`): end the
            // connection now, whoever closes the fd.
            unsafe { sys::lwip_shutdown(self.fd, sys::SHUT_RDWR as i32) };
        }
        let close = {
            let mut taken = TAKEN.lock().unwrap_or_else(|p| p.into_inner());
            match taken.iter().position(|t| t.fd == self.fd) {
                Some(i) if taken[i].httpd_done => {
                    taken.swap_remove(i);
                    true
                }
                Some(i) => {
                    taken[i].owner_done = true;
                    false
                }
                None => false,
            }
        };
        if close {
            // SAFETY: httpd has let go of it and so have we.
            unsafe { sys::close(self.fd) };
        }
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
    // A socket taken over by an upload: httpd is done with it, but it is
    // closed here only if the upload is done with it too.
    let taken = {
        let mut taken = TAKEN.lock().unwrap_or_else(|p| p.into_inner());
        taken.iter().position(|t| t.fd == fd).map(|i| {
            if taken[i].owner_done {
                taken.swap_remove(i);
                true
            } else {
                taken[i].httpd_done = true;
                false
            }
        })
    };
    match taken {
        Some(true) => {
            sys::close(fd);
            return;
        }
        Some(false) => return,
        None => {}
    }
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
    // A client that closed its end with some of our reply unacknowledged
    // (it gave up: a poll's timeout) is aborted, not left retransmitting
    // to nobody with that reply held in the heap (net-wedge2).
    crate::netstat::close_dead(fd);
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
    crate::diag::check("stream: download start");
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
    crate::diag::check("stream: download end");
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
