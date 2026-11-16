//! The session's transport: TCP, TLS 1.2 through mbedTLS (`esp_idf_sys`,
//! no esp-tls) and the WebSocket frames on it. Spike S0's probe, unchanged
//! in what it does on the wire (`S0-RESULTS.md` in the maintainers' notes:
//! the settings were measured there).
//!
//! **TLS**: `sdkconfig.connect` turns on dynamic buffers (each record
//! buffer taken for its own size and given back once read), the asymmetric
//! content lengths (in 4096 / out 2048 by default, `CONNECT_TLS_IN` and
//! `CONNECT_TLS_OUT` in the Makefile), no kept peer certificate, client
//! session tickets and the trusted-certificate callback. One cipher suite,
//! `ECDHE-ECDSA-AES128-GCM-SHA256`; groups P-256 then P-384. TLS 1.3 is not
//! possible: IDF 5.3 cannot combine it with dynamic buffers.
//!
//! **Writes larger than a record**: the longest frame the panel sends is a
//! [`proto::MAX_TEXT_OUT`](opendmd_core::connect::proto::MAX_TEXT_OUT)
//! text (2,048 B) with its 8-byte client header, over the 2,048-byte
//! output record: `mbedtls_ssl_write` then writes one record's worth and
//! returns the count, and [`Conn::write_all`] loops on the rest.
//!
//! **Roots.** The pinned roots in `firmware/connect-roots/`: Let's Encrypt
//! ISRG Root X1 and X2, Google Trust Services GTS Root R1 and R4, SSL.com's
//! two ECC roots (the one ECDSA suite fixes the leaf's key, not the
//! chain's: an RSA root can still sign it). They stay in flash: the
//! callback ([`ca_cb`]) parses only the root whose subject is the chain's
//! top issuer, when it is asked, without a copy (`parse_der_nocopy`), and
//! mbedTLS frees it after the check. An emulator image
//! (`OPENDMD_CONNECT_TEST_ROOT` at build time) adds its local test CA.
//!
//! **Blocking**: a turn's socket reads and writes share [`BLOCK_MAX`]
//! (8 s) from the turn's start (a peer that drips a record's bytes is cut
//! off there); the TCP connect is [`CONNECT_TIMEOUT`] (5 s). DNS never runs
//! on the worker: a short-lived task (`connect-dns`, 5 KB of stack while
//! it runs) asks lwIP's resolver ([`lookup`]).

use super::{tele, Cause, End};
#[cfg(feature = "connect-measure")]
use crate::state::free_heap;
use esp_idf_sys as sys;
use opendmd_core::connect::{self as cx, Decoder};
use std::ffi::{c_int, c_uchar, c_void, CString};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::{Duration, Instant};

// --- What this build needs from the sdkconfig ---------------------------
// Each line fails to compile (no such constant) without the sdkconfig
// fragment (`firmware/sdkconfig.connect`, layered on by the Makefile
// whenever a connect feature is on).
const _: u32 = sys::CONFIG_MBEDTLS_DYNAMIC_BUFFER;
const _: u32 = sys::CONFIG_MBEDTLS_CLIENT_SSL_SESSION_TICKETS;
const _: u32 = sys::CONFIG_MBEDTLS_X509_TRUSTED_CERT_CALLBACK;
const _: u32 = sys::CONFIG_MBEDTLS_HAVE_TIME_DATE;
const _: () = assert!(sys::CONFIG_LWIP_MAX_SOCKETS >= 15, "Connect's socket: CONFIG_LWIP_MAX_SOCKETS 15");
/// The TLS record sizes this build has (the Makefile's `CONNECT_TLS_IN`,
/// `CONNECT_TLS_OUT`).
pub const TLS_IN: u32 = sys::CONFIG_MBEDTLS_SSL_IN_CONTENT_LEN;
pub const TLS_OUT: u32 = sys::CONFIG_MBEDTLS_SSL_OUT_CONTENT_LEN;
// Every message the panel reads fits one input record, so a larger record
// (the server's or anyone's) ends the session rather than growing a buffer.
const _: () = assert!(TLS_IN as usize >= cx::proto::MAX_TEXT_IN + 14);

/// The pinned roots, by name (see the module docs).
pub static ROOTS: &[(&str, &[u8])] = &[
    ("isrg_root_x2", include_bytes!("../../connect-roots/isrg_root_x2.der")),
    ("gts_root_r4", include_bytes!("../../connect-roots/gts_root_r4.der")),
    // RSA roots: the one suite fixes the leaf's key (ECDSA), not the
    // chain's, which an RSA root may sign.
    ("isrg_root_x1", include_bytes!("../../connect-roots/isrg_root_x1.der")),
    ("gts_root_r1", include_bytes!("../../connect-roots/gts_root_r1.der")),
    ("ssl_com_tls_ecc_2022", include_bytes!("../../connect-roots/ssl_com_tls_ecc_root_ca_2022.der")),
    ("ssl_com_ecc", include_bytes!("../../connect-roots/ssl_com_root_certification_authority_ecc.der")),
    #[cfg(connect_test_root)]
    ("emulator_test_ca", include_bytes!(concat!(env!("OUT_DIR"), "/connect-test-root.der"))),
];

// --- mbedTLS constants (stable across 3.x) --------------------------------
const SSL_IS_CLIENT: c_int = 0;
const SSL_TRANSPORT_STREAM: c_int = 0;
const SSL_PRESET_DEFAULT: c_int = 0;
const SSL_VERIFY_REQUIRED: c_int = 2;
pub const ERR_WANT_READ: c_int = -0x6900;
pub const ERR_WANT_WRITE: c_int = -0x6880;
pub const ERR_PEER_CLOSE_NOTIFY: c_int = -0x7880;
const ERR_NET_RECV_FAILED: c_int = -0x004C;
const ERR_NET_SEND_FAILED: c_int = -0x004E;
const ERR_X509_ALLOC_FAILED: c_int = -0x2880;
const BADCERT_EXPIRED: u32 = 0x01;
const BADCERT_FUTURE: u32 = 0x0200;
/// The one suite offered: TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256.
static CIPHERS: [c_int; 2] = [0xC02B, 0];
/// secp256r1, secp384r1 (IANA group ids), 0-terminated.
static GROUPS: [u16; 3] = [23, 24, 0];

// --- Timing ------------------------------------------------------------------

/// How long one turn works before giving the worker back.
pub const SLICE: Duration = Duration::from_millis(250);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The socket's own timeout for one read or write call; the calls of a
/// turn together stop at [`BLOCK_MAX`].
const IO_TIMEOUT: Duration = Duration::from_secs(1);
/// The most a turn's socket reads and writes may block the worker.
pub const BLOCK_MAX: Duration = Duration::from_secs(8);
/// A handshake not done this long after it began has failed.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a lookup is waited for before it counts as failed (lwIP's
/// worst case, 3 servers × 7 s, and some): `opendmd_core::connect::dns`.
pub use cx::dns::DNS_WAIT;

/// [`DNS_WAIT`], or an emulator lane's shorter one (`PUT /api/connect`
/// `{"test_dns_wait_ms": n}`, `connect-test` builds; 0: the usual).
#[optimize(size)]
#[inline(never)]
pub fn dns_wait() -> Duration {
    #[cfg(feature = "connect-test")]
    match super::TEST_DNS_WAIT_MS.load(Relaxed) {
        0 => {}
        ms => return Duration::from_millis(u64::from(ms)),
    }
    DNS_WAIT
}

#[optimize(size)]
#[inline(never)]
pub fn now_us() -> u64 {
    // SAFETY: a plain clock read.
    unsafe { sys::esp_timer_get_time() as u64 }
}

#[optimize(size)]
#[inline(never)]
pub fn rand_u32() -> u32 {
    // SAFETY: the hardware RNG; no preconditions.
    unsafe { sys::esp_random() }
}

/// `N` bytes from the hardware RNG (with WiFi up, a true RNG).
#[optimize(size)]
#[inline(never)]
pub fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    // SAFETY: N bytes.
    unsafe { sys::esp_fill_random(b.as_mut_ptr() as *mut c_void, N) };
    b
}

// --- mbedTLS's heap, counted (measuring builds) ---------------------------------

/// Bytes mbedTLS holds now (all of its users: WPA-Enterprise and SAE too;
/// what was allocated before the counting began is not in it, so only
/// differences mean anything), and the most since [`mark_tls_peak`].
/// Counted only in a `connect-measure` build (its allocator).
pub static TLS_IN_USE: AtomicI32 = AtomicI32::new(0);
pub static TLS_PEAK: AtomicI32 = AtomicI32::new(0);

/// Of [`TLS_IN_USE`], the bytes in PSRAM (`psram-tls`, an ESP32-S3
/// measurement build), and the allocations that wanted PSRAM and got
/// internal RAM (its `connect.heap.psram`; a shipping build's only
/// [`tls_calloc`] call, the verify callback's root, is not counted).
#[cfg(all(esp32s3, feature = "psram-tls", feature = "connect-measure"))]
pub static TLS_PSRAM: AtomicI32 = AtomicI32::new(0);
#[cfg(all(esp32s3, feature = "psram-tls", feature = "connect-measure"))]
pub static TLS_FELL_BACK: AtomicU32 = AtomicU32::new(0);
#[cfg(all(feature = "psram-tls", not(esp32s3)))]
compile_error!("psram-tls is an ESP32-S3 measurement switch (VARIANT=s3)");
// IDF's own mbedTLS allocator must say PSRAM too (sdkconfig's line, which
// the Makefile's switch adds with the feature).
#[cfg(all(esp32s3, feature = "psram-tls"))]
const _: u32 = sys::CONFIG_MBEDTLS_EXTERNAL_MEM_ALLOC;

#[cfg(all(esp32s3, feature = "psram-tls", feature = "connect-measure"))]
#[optimize(size)]
#[inline(never)]
fn is_psram(p: *mut c_void) -> bool {
    (0x3C00_0000..0x3E00_0000).contains(&(p as usize))
}

#[optimize(size)]
#[inline(never)]
pub unsafe extern "C" fn tls_calloc(n: usize, size: usize) -> *mut c_void {
    // As IDF's own (CONFIG_MBEDTLS_INTERNAL_MEM_ALLOC): internal RAM.
    #[cfg(not(all(esp32s3, feature = "psram-tls")))]
    let p = sys::heap_caps_calloc(n, size, sys::MALLOC_CAP_INTERNAL | sys::MALLOC_CAP_8BIT);
    // `psram-tls`: PSRAM, internal RAM only if that fails.
    #[cfg(all(esp32s3, feature = "psram-tls"))]
    let p = {
        let p = sys::heap_caps_calloc(n, size, sys::MALLOC_CAP_SPIRAM | sys::MALLOC_CAP_8BIT);
        if p.is_null() && n.saturating_mul(size) > 0 {
            #[cfg(feature = "connect-measure")]
            TLS_FELL_BACK.fetch_add(1, Relaxed);
            sys::heap_caps_calloc(n, size, sys::MALLOC_CAP_INTERNAL | sys::MALLOC_CAP_8BIT)
        } else {
            p
        }
    };
    #[cfg(feature = "connect-measure")]
    if !p.is_null() {
        let size = sys::heap_caps_get_allocated_size(p) as i32;
        let now = TLS_IN_USE.fetch_add(size, Relaxed) + size;
        TLS_PEAK.fetch_max(now, Relaxed);
        #[cfg(all(esp32s3, feature = "psram-tls"))]
        if is_psram(p) {
            TLS_PSRAM.fetch_add(size, Relaxed);
        }
    }
    p
}

#[optimize(size)]
#[inline(never)]
pub unsafe extern "C" fn tls_free(p: *mut c_void) {
    if !p.is_null() {
        #[cfg(feature = "connect-measure")]
        {
            let size = sys::heap_caps_get_allocated_size(p) as i32;
            TLS_IN_USE.fetch_sub(size, Relaxed);
            #[cfg(all(esp32s3, feature = "psram-tls"))]
            if is_psram(p) {
                TLS_PSRAM.fetch_sub(size, Relaxed);
            }
        }
        sys::heap_caps_free(p);
    }
}

#[optimize(size)]
#[inline(never)]
pub fn mark_tls_peak() -> i32 {
    let now = TLS_IN_USE.load(Relaxed);
    TLS_PEAK.store(now, Relaxed);
    now
}

#[optimize(size)]
#[inline(never)]
pub fn tls_in_use() -> i32 {
    TLS_IN_USE.load(Relaxed)
}

// --- The TLS pieces ---------------------------------------------------------------

/// What the I/O callbacks need: the socket, and the heap low they sample.
pub struct Bio {
    fd: c_int,
    pub hs_low: u32,
    pub rx: u64,
    pub tx: u64,
    /// This turn's reads and writes block no later than this ([`BLOCK_MAX`]).
    pub deadline: Instant,
}

impl Bio {
    #[optimize(size)]
    #[inline(never)]
    fn sample(&mut self) {
        crate::netstat::sample(crate::netstat::At::Connect);
        // The rest feeds only a measuring build's telemetry.
        #[cfg(feature = "connect-measure")]
        self.measure();
    }

    #[cfg(feature = "connect-measure")]
    fn measure(&mut self) {
        let f = free_heap();
        if f < self.hs_low {
            self.hs_low = f;
        }
        let low = tele(|t| (f < t.heap_low).then(|| t.heap_low = f).is_some());
        if low {
            let largest = crate::state::largest_free_block() as u32;
            tele(|t| t.heap_low_largest = largest);
        }
    }
}

#[optimize(size)]
#[inline(never)]
fn would_block() -> bool {
    std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock
}

#[optimize(size)]
#[inline(never)]
unsafe extern "C" fn bio_send(ctx: *mut c_void, buf: *const c_uchar, len: usize) -> c_int {
    // SAFETY: `ctx` is the connection's boxed `Bio`, alive while its TLS is.
    let b = &mut *(ctx as *mut Bio);
    b.sample();
    let n = sys::lwip_send(b.fd, buf as *const c_void, len, 0);
    if n >= 0 {
        b.tx += n as u64;
        return n as c_int;
    }
    // EAGAIN: the socket's 1 s timeout; again until the turn's deadline.
    if would_block() && Instant::now() < b.deadline {
        ERR_WANT_WRITE
    } else {
        ERR_NET_SEND_FAILED
    }
}

/// All of `len`, or nothing yet. mbedTLS's dynamic buffers read a record's
/// 5-byte header into a buffer on the stack (`esp_mbedtls_add_rx_buffer`),
/// so a header split over two calls by a `WANT_READ` would lose its first
/// part: this answers `WANT_READ` only when nothing at all has arrived, and
/// once something has, blocks for the rest, up to the turn's deadline
/// ([`BLOCK_MAX`] in all, whatever the peer drips): then the session fails.
#[optimize(size)]
#[inline(never)]
unsafe extern "C" fn bio_recv(ctx: *mut c_void, buf: *mut c_uchar, len: usize) -> c_int {
    // SAFETY: as in `bio_send`.
    let b = &mut *(ctx as *mut Bio);
    b.sample();
    let mut one = 0u8;
    let n = sys::lwip_recv(b.fd, &mut one as *mut u8 as *mut c_void, 1, (sys::MSG_PEEK | sys::MSG_DONTWAIT) as c_int);
    if n == 0 {
        return 0;
    }
    if n < 0 {
        return if would_block() { ERR_WANT_READ } else { ERR_NET_RECV_FAILED };
    }
    let mut got = 0usize;
    while got < len {
        if Instant::now() >= b.deadline {
            return ERR_NET_RECV_FAILED;
        }
        let n = sys::lwip_recv(b.fd, buf.add(got) as *mut c_void, len - got, 0);
        if n > 0 {
            got += n as usize;
        } else if n == 0 {
            break;
        } else if !would_block() {
            return ERR_NET_RECV_FAILED;
        }
    }
    b.rx += got as u64;
    got as c_int
}

#[optimize(size)]
#[inline(never)]
unsafe extern "C" fn rng(_: *mut c_void, out: *mut c_uchar, len: usize) -> c_int {
    sys::esp_fill_random(out as *mut c_void, len);
    0
}

/// Which pinned root the last chain ended at (index + 1; 0 none yet).
pub static ROOT_USED: AtomicU32 = AtomicU32::new(0);

/// mbedTLS asks for the trusted parents of `child`: the pinned root whose
/// subject is its issuer, parsed in place from flash, or none.
#[optimize(size)]
#[inline(never)]
unsafe extern "C" fn ca_cb(_: *mut c_void, child: *const sys::mbedtls_x509_crt, out: *mut *mut sys::mbedtls_x509_crt) -> c_int {
    *out = std::ptr::null_mut();
    let raw = &(*child).issuer_raw;
    if raw.p.is_null() {
        return 0;
    }
    let issuer = std::slice::from_raw_parts(raw.p, raw.len);
    for (k, (_, der)) in ROOTS.iter().enumerate() {
        if cx::cert_subject(der) != Some(issuer) {
            continue;
        }
        // Freed by mbedTLS (mbedtls_x509_crt_free, then mbedtls_free:
        // with the measuring allocator ours, `tls_free`, which takes any
        // heap block).
        let crt = tls_calloc(1, std::mem::size_of::<sys::mbedtls_x509_crt>()) as *mut sys::mbedtls_x509_crt;
        if crt.is_null() {
            return ERR_X509_ALLOC_FAILED;
        }
        sys::mbedtls_x509_crt_init(crt);
        if sys::mbedtls_x509_crt_parse_der_nocopy(crt, der.as_ptr(), der.len()) != 0 {
            sys::mbedtls_x509_crt_free(crt);
            tls_free(crt as *mut c_void);
            continue;
        }
        #[cfg(feature = "connect-measure")]
        ROOT_USED.store(k as u32 + 1, Relaxed);
        #[cfg(not(feature = "connect-measure"))]
        let _ = k;
        *out = crt;
        return 0;
    }
    0
}

/// What the certificate check saw.
pub struct Verify {
    /// Called at all: a full handshake (a resumed one checks nothing).
    pub called: bool,
    /// The clock floor in force (Unix seconds), when SNTP has not set it.
    floor: Option<i64>,
    /// The build's date (Unix seconds): no certificate that ended before it.
    build: i64,
    pub flags: u32,
}

#[optimize(size)]
#[inline(never)]
unsafe extern "C" fn verify_cb(ctx: *mut c_void, crt: *mut sys::mbedtls_x509_crt, _depth: c_int, flags: *mut u32) -> c_int {
    // SAFETY: `ctx` is the connection's boxed `Verify`.
    let v = &mut *(ctx as *mut Verify);
    v.called = true;
    // No clock (or one before the build date, `client::clock_ready`):
    // "not yet valid" means nothing then.
    if v.floor.is_some() {
        *flags &= !BADCERT_FUTURE;
    }
    // "Ended" is judged against the build date always, as well as the
    // clock: a time server that says it is years ago cannot bring an
    // expired certificate back.
    let t = &(*crt).valid_to;
    let to = cx::civil_to_unix(t.year.into(), t.mon as u32, t.day as u32, t.hour as u32, t.min as u32, t.sec as u32);
    if to < v.build {
        *flags |= BADCERT_EXPIRED;
    }
    v.flags |= *flags;
    0
}

/// An mbedTLS error as its code and, for the ones a session meets, a few
/// words. Not `mbedtls_strerror`: its tables of every module's messages
/// are ~10 KB of flash.
#[optimize(size)]
#[inline(never)]
pub fn tls_error(r: c_int) -> String {
    let n = r.unsigned_abs();
    let words = |c: u32| match c {
        0x7200 => "invalid record (one over this build's TLS input size?)",
        0x7280 => "connection ended",
        0x7300 => "decode error",
        0x7700 => "unexpected message",
        0x7780 => "fatal alert from the server",
        0x7880 => "close_notify",
        0x7F00 | 0x2880 | 0x4D80 | 0x3F80 | 0x0010 => "out of memory",
        0x7100 => "bad input",
        0x6E80 => "bad protocol version",
        0x6E00 => "handshake failure",
        0x6C00 => "internal error",
        0x6800 => "timeout",
        0x6700 => "unexpected record",
        0x6600 => "illegal parameter",
        0x5E80 => "bad config",
        0x2700 => "certificate not trusted or invalid",
        0x2600 => "unknown signature algorithm",
        0x2180 | 0x2780 => "certificate format",
        0x4E00 => "signature check failed",
        0x004C => "receive failed",
        0x004E => "send failed",
        _ => "",
    };
    let (high, low) = (n & 0xFF80, n & 0x007F);
    let (wh, wl) = (words(high), words(low));
    match (wh.is_empty(), wl.is_empty() || low == 0) {
        (false, true) => format!("-0x{n:04x} {wh}"),
        (false, false) => format!("-0x{n:04x} {wh}; {wl}"),
        (true, false) => format!("-0x{n:04x} {wl}"),
        (true, true) => format!("-0x{n:04x}"),
    }
}

pub struct Tls {
    ssl: sys::mbedtls_ssl_context,
    conf: sys::mbedtls_ssl_config,
}

impl Drop for Tls {
    #[optimize(size)]
    #[inline(never)]
    fn drop(&mut self) {
        // SAFETY: both were initialised (zeroed, then _init) before any use.
        unsafe {
            sys::mbedtls_ssl_free(&mut self.ssl);
            sys::mbedtls_ssl_config_free(&mut self.conf);
        }
    }
}

/// A saved session (its ticket) for the next connection's resumption.
pub struct Ticket(sys::mbedtls_ssl_session);

impl Ticket {
    #[optimize(size)]
    #[inline(never)]
    pub fn new() -> Box<Ticket> {
        // SAFETY: an all-zero session is what _init makes; _init then.
        let mut s: Box<Ticket> = unsafe { Box::new_zeroed().assume_init() };
        unsafe { sys::mbedtls_ssl_session_init(&mut s.0) };
        s
    }
}

impl Drop for Ticket {
    #[optimize(size)]
    #[inline(never)]
    fn drop(&mut self) {
        // SAFETY: initialised in `new`.
        unsafe { sys::mbedtls_ssl_session_free(&mut self.0) };
    }
}

// --- A connection -------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Handshake,
    /// `POST /v1/register` written, its answer being read.
    Request,
    Upgrade,
    Open,
}

/// What a connection is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purpose {
    /// One `POST /v1/register`, then closed.
    Register,
    /// The WebSocket session.
    Session,
}

pub struct Ping {
    pub seq: u32,
    pub sent: Option<(u32, Instant)>,
    pub next: Instant,
    pub missed: u32,
}

/// The longest binary message kept: one byte over the longest command
/// frame or transfer chunk, so one too long is still seen as such (a
/// command's key id answered `too-big`; a chunk refused whole, never taken
/// cut short).
pub const BIN_KEEP: usize = max(cx::proto::COMMAND_HEAD + cx::proto::MAX_BODY, cx::xfer::CHUNK_HEAD + cx::xfer::MAX_CHUNK) + 1;

const fn max(a: usize, b: usize) -> usize {
    if a > b {
        a
    } else {
        b
    }
}

pub struct Conn {
    tcp: TcpStream,
    tls: Box<Tls>,
    pub bio: Box<Bio>,
    pub verify: Box<Verify>,
    pub phase: Phase,
    pub purpose: Purpose,
    pub t0: Instant,
    pub hs_t0: Instant,
    pub hs_cpu_us: u64,
    pub resume: bool,
    pub tickets: bool,
    pub tls_before: i32,
    /// The upgrade's key (base64), once the request is written.
    pub key: String,
    /// The answer's head (and, for the register request, its body).
    pub head: Vec<u8>,
    pub dec: Box<Decoder>,
    pub buf: Box<[u8; 512]>,
    pub opened: Option<Instant>,
    pub ping: Ping,
    /// The binary message being received (up to [`BIN_KEEP`] of it), its
    /// whole length so far, and whether one is under way.
    pub bin: Vec<u8>,
    pub bin_len: u64,
    pub in_bin: bool,
}

/// The Rust side's bytes for an open session (reported with mbedTLS's;
/// the upgrade's head buffer, up to 1 KB, is gone by then).
#[optimize(size)]
#[inline(never)]
pub fn rust_bytes() -> u32 {
    (std::mem::size_of::<Conn>()
        + std::mem::size_of::<Tls>()
        + std::mem::size_of::<Bio>()
        + std::mem::size_of::<Verify>()
        + std::mem::size_of::<Decoder>()
        + 512) as u32
}

impl Conn {
    #[optimize(size)]
    #[inline(never)]
    fn fd(&self) -> c_int {
        self.tcp.as_raw_fd()
    }

    #[optimize(size)]
    #[inline(never)]
    pub fn ssl(&mut self) -> *mut sys::mbedtls_ssl_context {
        &mut self.tls.ssl
    }

    /// Wait up to `ms` for something to read.
    #[optimize(size)]
    #[inline(never)]
    pub fn poll(&self, ms: u32) -> bool {
        let mut p = sys::pollfd { fd: self.fd(), events: sys::POLLIN as i16, revents: 0 };
        // SAFETY: one valid pollfd.
        unsafe { sys::poll(&mut p, 1, ms as c_int) > 0 }
    }

    /// Write all of `data` through TLS (blocking, to the turn's deadline).
    /// More than one output record ([`TLS_OUT`]) is written a record at a
    /// time: `mbedtls_ssl_write` takes what fits and says how much.
    #[optimize(size)]
    #[inline(never)]
    pub fn write_all(&mut self, mut data: &[u8]) -> Result<(), End> {
        let ssl = self.ssl();
        while !data.is_empty() {
            if Instant::now() > self.bio.deadline {
                return Err(End::new(Cause::Io, "write timed out"));
            }
            // SAFETY: an initialised, set-up context; a valid buffer.
            let r = unsafe { sys::mbedtls_ssl_write(ssl, data.as_ptr(), data.len()) };
            if r > 0 {
                data = &data[r as usize..];
            } else if r == ERR_WANT_WRITE || r == ERR_WANT_READ {
                std::thread::sleep(Duration::from_millis(5));
            } else {
                return Err(End::new(Cause::Io, format!("write: {}", tls_error(r))));
            }
        }
        Ok(())
    }

    #[optimize(size)]
    #[inline(never)]
    pub fn send(&mut self, opcode: u8, payload: &[u8]) -> Result<(), End> {
        let mut f = Vec::with_capacity(payload.len() + 8);
        cx::encode_frame(opcode, payload, random::<4>(), &mut f);
        self.write_all(&f)
    }

    /// One text message. One longer than the protocol lets the panel send
    /// is not sent (logged): the server would drop it anyway.
    #[optimize(size)]
    #[inline(never)]
    pub fn text(&mut self, s: &str) -> Result<(), End> {
        debug_assert!(s.len() <= cx::proto::MAX_TEXT_OUT, "a {} B text over MAX_TEXT_OUT", s.len());
        if s.len() > cx::proto::MAX_TEXT_OUT {
            log::warn!("connect: a {} B message not sent (over {} B)", s.len(), cx::proto::MAX_TEXT_OUT);
            return Ok(());
        }
        self.send(cx::OP_TEXT, s.as_bytes())
    }

    /// One `mbedtls_ssl_read` into the connection's buffer.
    #[optimize(size)]
    #[inline(never)]
    pub fn read(&mut self) -> c_int {
        let (ssl, buf) = (&mut self.tls.ssl as *mut _, self.buf.as_mut_ptr());
        // SAFETY: an open context and the connection's buffer.
        unsafe { sys::mbedtls_ssl_read(ssl, buf, 512) }
    }

    /// TLS's own end (the server may have gone already).
    #[optimize(size)]
    #[inline(never)]
    pub fn close_notify(&mut self) {
        // SAFETY: an open context.
        let _ = unsafe { sys::mbedtls_ssl_close_notify(self.ssl()) };
    }

    /// A new ticket for the next resumption, into `t`.
    #[optimize(size)]
    #[inline(never)]
    pub fn save_ticket(&mut self, t: &mut Ticket) -> Result<(), String> {
        // SAFETY: an open context and an initialised session.
        let r = unsafe { sys::mbedtls_ssl_get_session(&self.tls.ssl, &mut t.0) };
        if r != 0 {
            return Err(format!("no session to keep: {}", tls_error(r)));
        }
        Ok(())
    }
}

/// TCP to `addr` (looked up already), and the TLS context ready for its
/// handshake to `host`.
#[optimize(size)]
#[inline(never)]
pub fn connect(host: &str, addr: SocketAddr, purpose: Purpose, tickets: bool, ticket: Option<&Ticket>, floor: Option<i64>, free: u32) -> Result<Box<Conn>, End> {
    let t0 = Instant::now();
    let tcp = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| End::new(Cause::Tcp, format!("{}: {e}", v4_ip(&addr))))?;
    let tcp_ms = t0.elapsed().as_millis() as u32;
    let _ = tcp.set_nodelay(true);
    let _ = tcp.set_read_timeout(Some(IO_TIMEOUT));
    let _ = tcp.set_write_timeout(Some(IO_TIMEOUT));
    tele(|t| {
        t.tcp_ms = tcp_ms;
        t.heap_hs_low = free;
    });
    let tls_before = mark_tls_peak();
    let host = CString::new(host).map_err(|_| End::new(Cause::Dns, "host has a NUL"))?;
    let fd = tcp.as_raw_fd();
    let mut conn = Box::new(Conn {
        tcp,
        // SAFETY: zeroed mbedTLS contexts, _init'ed below before any use.
        tls: unsafe { Box::new_zeroed().assume_init() },
        bio: Box::new(Bio { fd, hs_low: free, rx: 0, tx: 0, deadline: Instant::now() + BLOCK_MAX }),
        verify: Box::new(Verify { called: false, floor, build: env!("OPENDMD_BUILD_UNIX").parse().unwrap_or(0), flags: 0 }),
        phase: Phase::Handshake,
        purpose,
        t0,
        hs_t0: Instant::now(),
        hs_cpu_us: 0,
        resume: ticket.is_some(),
        tickets,
        tls_before,
        key: String::new(),
        head: Vec::new(),
        dec: Box::new(Decoder::new()),
        buf: Box::new([0; 512]),
        opened: None,
        ping: Ping { seq: 0, sent: None, next: Instant::now(), missed: 0 },
        bin: Vec::new(),
        bin_len: 0,
        in_bin: false,
    });
    let Conn { tls, bio, verify, .. } = &mut *conn;
    // SAFETY: the contexts live in their boxes for the connection's life;
    // the callbacks' contexts (bio, verify) too; the strings and lists
    // they keep pointers to are statics, or copied (the host name).
    let r = unsafe {
        sys::mbedtls_ssl_init(&mut tls.ssl);
        sys::mbedtls_ssl_config_init(&mut tls.conf);
        let mut r = sys::mbedtls_ssl_config_defaults(&mut tls.conf, SSL_IS_CLIENT, SSL_TRANSPORT_STREAM, SSL_PRESET_DEFAULT);
        if r == 0 {
            sys::mbedtls_ssl_conf_authmode(&mut tls.conf, SSL_VERIFY_REQUIRED);
            sys::mbedtls_ssl_conf_rng(&mut tls.conf, Some(rng), std::ptr::null_mut());
            sys::mbedtls_ssl_conf_ciphersuites(&mut tls.conf, CIPHERS.as_ptr());
            sys::mbedtls_ssl_conf_groups(&mut tls.conf, GROUPS.as_ptr());
            sys::mbedtls_ssl_conf_ca_cb(&mut tls.conf, Some(ca_cb), std::ptr::null_mut());
            sys::mbedtls_ssl_conf_verify(&mut tls.conf, Some(verify_cb), &mut **verify as *mut Verify as *mut c_void);
            sys::mbedtls_ssl_conf_session_tickets(&mut tls.conf, c_int::from(tickets));
            r = sys::mbedtls_ssl_setup(&mut tls.ssl, &tls.conf);
        }
        if r == 0 {
            r = sys::mbedtls_ssl_set_hostname(&mut tls.ssl, host.as_ptr());
        }
        if r == 0 {
            sys::mbedtls_ssl_set_bio(&mut tls.ssl, &mut **bio as *mut Bio as *mut c_void, Some(bio_send), Some(bio_recv), None);
            if let Some(s) = ticket {
                let rs = sys::mbedtls_ssl_set_session(&mut tls.ssl, &s.0);
                if rs != 0 {
                    log::warn!("connect: saved session refused: {}", tls_error(rs));
                }
            }
        }
        r
    };
    if r != 0 {
        return Err(End::new(Cause::Handshake, format!("setup: {}", tls_error(r))));
    }
    conn.hs_t0 = Instant::now();
    Ok(conn)
}

/// Handshake steps until done (`Ok(true)`), waiting, or the slice is over.
#[optimize(size)]
#[inline(never)]
pub fn step_handshake(conn: &mut Conn, started: Instant) -> Result<bool, End> {
    let ssl = conn.ssl();
    loop {
        if conn.hs_t0.elapsed() > HANDSHAKE_TIMEOUT {
            return Err(End::new(Cause::Handshake, format!("no handshake within {} s", HANDSHAKE_TIMEOUT.as_secs())));
        }
        #[cfg(feature = "connect-measure")]
        let t = now_us();
        // SAFETY: a set-up context.
        let r = unsafe { sys::mbedtls_ssl_handshake(ssl) };
        #[cfg(feature = "connect-measure")]
        {
            conn.hs_cpu_us += now_us() - t;
        }
        if r == 0 {
            return Ok(true);
        }
        if r != ERR_WANT_READ && r != ERR_WANT_WRITE {
            let flags = conn.verify.flags;
            let extra = if flags != 0 { format!(" (certificate flags 0x{flags:x})") } else { String::new() };
            return Err(End::new(Cause::Handshake, format!("{}{extra}", tls_error(r))));
        }
        let left = SLICE.saturating_sub(started.elapsed());
        if left.is_zero() {
            return Ok(false);
        }
        conn.poll((left.as_millis() as u32).min(50));
    }
}

/// The address's IPv4 part as text (IPv4 only: `IpAddr`'s own Display
/// would link the IPv6 formatter, ~1.5 KB, for nothing).
#[optimize(size)]
#[inline(never)]
fn v4_ip(addr: &SocketAddr) -> String {
    match addr {
        SocketAddr::V4(a) => a.ip().to_string(),
        SocketAddr::V6(_) => "an IPv6 address".into(),
    }
}

// --- DNS, on a task of its own ------------------------------------------------------

/// A lookup on its own task (`connect-dns`): the host, and its answer once
/// there (the address, or why not) with how long it took.
pub struct Dns {
    pub host: String,
    pub answer: Option<(Result<SocketAddr, String>, u32)>,
    pub started: Instant,
    /// Counted failed already (it ran past [`dns_wait`]): its late
    /// answer counts for nothing more.
    pub given_up: bool,
}

pub static DNS: Mutex<Option<Dns>> = Mutex::new(None);
/// The least of `connect-dns`'s stack never used (bytes; `u32::MAX` before
/// a lookup has run).
pub static DNS_STACK_LOW: AtomicU32 = AtomicU32::new(u32::MAX);

#[optimize(size)]
#[inline(never)]
pub fn lookup(host: String) -> Result<(), String> {
    *DNS.lock().unwrap_or_else(|p| p.into_inner()) = Some(Dns { host: host.clone(), answer: None, started: Instant::now(), given_up: false });
    let job = Box::new(move || {
        let t0 = Instant::now();
        let r = (host.as_str(), 443)
            .to_socket_addrs()
            .map_err(|e| format!("{host}: {e}"))
            .and_then(|mut a| a.find(SocketAddr::is_ipv4).ok_or_else(|| format!("{host}: no IPv4 address")));
        let ms = t0.elapsed().as_millis() as u32;
        if let Some(d) = DNS.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            d.answer = Some((r, ms));
        }
        // SAFETY: the calling task's own high-water mark (bytes on IDF).
        let unused = unsafe { sys::uxTaskGetStackHighWaterMark(std::ptr::null_mut()) };
        DNS_STACK_LOW.fetch_min(unused, Relaxed);
    });
    crate::task::spawn_core0(b"connect-dns\0", 5 * 1024, 2, job).map_err(|e| {
        *DNS.lock().unwrap_or_else(|p| p.into_inner()) = None;
        format!("no task for the lookup: {e}")
    })
}

#[cfg(feature = "connect-measure")]
extern "C" {
    // mbedtls/platform.h (MBEDTLS_PLATFORM_MEMORY, which IDF defines).
    fn mbedtls_platform_set_calloc_free(
        calloc: unsafe extern "C" fn(usize, usize) -> *mut c_void,
        free: unsafe extern "C" fn(*mut c_void),
    ) -> c_int;
}

/// A measuring build counts mbedTLS's bytes through its own allocator.
#[optimize(size)]
#[inline(never)]
pub fn install_allocator() {
    // SAFETY: once, before any TLS use of ours (mbedTLS's other users may
    // already hold blocks: `tls_free` takes any heap block).
    #[cfg(feature = "connect-measure")]
    unsafe {
        mbedtls_platform_set_calloc_free(tls_calloc, tls_free)
    };
}
