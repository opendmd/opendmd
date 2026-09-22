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
//! display loop, on the other core.
//!
//! Handler contract: return `Ok(())` having sent a response, or `Err(_)` and
//! the wrapper sends a 500 if nothing has gone out yet.

use anyhow::{anyhow, bail, Result};
use esp_idf_sys as sys;
use std::ffi::{c_void, CStr, CString};

/// How many times a chunk send is retried before the connection is given up on,
/// and how long to wait between tries.
const SEND_RETRIES: usize = 8;
const SEND_RETRY_DELAY_MS: u64 = 25;

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
            // Generous, because a browser uploading a 2 MB firmware image over
            // a weak link can stall between chunks.
            cfg.recv_wait_timeout = 10;
            cfg.send_wait_timeout = 10;
            cfg.uri_match_fn = Some(sys::httpd_uri_match_wildcard);
            sys::httpd_start(&mut handle, &cfg)
        };
        if err != sys::ESP_OK {
            bail!("httpd_start failed: {err}");
        }
        log::info!("httpd started on port 80, pinned to CPU{core_id}, {stack_size} B stack");
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
    match (*handler)(&mut req) {
        Ok(()) => {
            // A handler that streamed chunks must terminate the response; do
            // it for them if they forgot, or the socket hangs.
            if req.chunked && !req.finished {
                let _ = req.finish();
            }
            sys::ESP_OK
        }
        Err(e) => {
            log::warn!("{} {}: {e:#}", req.method_name(), req.uri());
            if req.responded {
                // Half a response has already gone out; the only honest thing
                // left is to drop the connection.
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
}

impl Req {
    fn new(raw: *mut sys::httpd_req_t) -> Req {
        Req {
            raw,
            keep: Vec::new(),
            responded: false,
            chunked: false,
            finished: false,
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
    /// A socket timeout is retried rather than reported, which is what makes a
    /// slow uploader work.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        loop {
            // SAFETY: `buf` is a live mutable slice; httpd writes at most
            // `buf.len()` bytes into it.
            let n = unsafe { sys::httpd_req_recv(self.raw, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n >= 0 {
                return Ok(n as usize);
            }
            if n == sys::HTTPD_SOCK_ERR_TIMEOUT {
                continue;
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
        // SAFETY: `body` is a live slice read synchronously by httpd.
        let err = unsafe {
            sys::httpd_resp_send(self.raw, body.as_ptr() as *const _, body.len() as isize)
        };
        if err != sys::ESP_OK {
            bail!("resp_send: {err}");
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

    /// Send one chunk. Empty chunks are skipped — an empty
    /// `httpd_resp_send_chunk` is what *terminates* the response.
    /// Send one chunk, retrying a send that failed for want of memory.
    ///
    /// Under concurrent load this board runs the heap down to a few KB, and
    /// lwip then fails a `send` with EAGAIN — which arrives here as
    /// `ESP_ERR_HTTPD_RESP_SEND` and, untreated, truncates the response
    /// mid-stream (observed on the rig against a 110 KB directory listing).
    /// A short sleep is enough for the player to finish a frame and for the
    /// stack to drain, so a slow response beats a broken one.
    pub fn chunk(&mut self, buf: &[u8]) -> Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        for attempt in 0..SEND_RETRIES {
            // SAFETY: `buf` is a live slice, written out synchronously.
            let err = unsafe {
                sys::httpd_resp_send_chunk(self.raw, buf.as_ptr() as *const _, buf.len() as isize)
            };
            if err == sys::ESP_OK {
                return Ok(());
            }
            if err != sys::ESP_ERR_HTTPD_RESP_SEND || attempt + 1 == SEND_RETRIES {
                bail!("resp_send_chunk: {err}");
            }
            std::thread::sleep(std::time::Duration::from_millis(SEND_RETRY_DELAY_MS));
        }
        unreachable!()
    }

    /// Terminate a chunked response.
    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        // SAFETY: the null/zero chunk is httpd's end-of-response marker.
        let err = unsafe { sys::httpd_resp_send_chunk(self.raw, std::ptr::null(), 0) };
        if err != sys::ESP_OK {
            return Err(anyhow!("resp_send_chunk(end): {err}"));
        }
        Ok(())
    }
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
