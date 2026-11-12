//! OpenDMD Connect, spike S0: a measurement probe, not a feature (feature
//! `connect-probe`, off in every other build and compiled out entirely;
//! `/srv/scratch/connect-design/CONNECT.md` sections 5 and 10, the
//! private notes' `docs/research/connect.md` once written).
//!
//! **What it is.** One outbound WebSocket over TLS 1.2 (`wss://`, port 443)
//! to the scratch echo Worker (`connect/s0-echo/`), held open with pings,
//! reconnected with backoff. No protocol, no pairing: it exists to measure
//! what a secure outbound session costs this device (heap, flash, CPU0
//! time, the display's frame deadlines) beside play.
//!
//! **Where it runs.** Inside the network worker (CPU0, priority 5, its 11 KB
//! stack), as a polled client: [`Probe::tick`] is called from the worker's
//! loop and does at most a slice of work (~250 ms: a handshake step that
//! is not waiting, a few records read), and [`Probe::wait`] shortens the
//! loop's sleep while there is something to do. No task and no stack of its
//! own (CONNECT.md 5.5's first placement): an own task would take 8-10 KB
//! of stack for good, which is the whole of the idle-floor gate's margin.
//! Nothing here runs on CPU1, takes a lock the display waits on, or writes
//! flash. The handshake's blocking parts are bounded: DNS and the TCP
//! connect (5 s), a record's body once its first byte has arrived (10 s).
//!
//! **TLS** (mbedTLS through `esp_idf_sys`, no esp-tls), as CONNECT.md 5.2
//! has it: `sdkconfig.connect-probe` turns on dynamic buffers (each record
//! buffer taken for its own size and given back once read), the asymmetric
//! content lengths (in 4096 / out 2048 by default, `CONNECT_TLS_IN` and
//! `CONNECT_TLS_OUT` in the Makefile), no kept peer certificate, client
//! session tickets and the trusted-certificate callback. One cipher suite,
//! `ECDHE-ECDSA-AES128-GCM-SHA256`; groups P-256 then P-384. TLS 1.3 is not
//! possible: IDF 5.3 cannot combine it with dynamic buffers.
//!
//! **Roots.** The pinned ECDSA roots in `firmware/connect-roots/` (Let's
//! Encrypt ISRG Root X2, Google Trust Services GTS Root R4, SSL.com's two
//! ECC roots: every ECDSA chain Cloudflare's universal certificates use;
//! the one suite offered means an ECDSA chain). They stay in flash: the
//! callback ([`ca_cb`]) parses only the root whose subject is the
//! chain's top issuer, when it is asked, without a copy
//! (`parse_der_nocopy`), and mbedTLS frees it after the check.
//!
//! **The clock** (CONNECT.md 9.4). The first handshake waits for SNTP; with
//! no sync 60 s after the network came up, certificate dates are checked
//! against the build date instead (a certificate that ended before it is
//! refused; "not yet valid" is ignored), and `connect.clock` says
//! `"build date"`.
//!
//! **Test hooks**: `GET /api/debug/connect`, `POST /api/debug/connect/<what>`
//! ([`routes`]). Not in `docs/api.md`: they exist only in this build.
//! **Telemetry**: `/api/status` `connect` ([`json`]).

use crate::httpd::{json_string, query_param, Method, Respond, Server};
use crate::state::{free_heap, largest_free_block};
use esp_idf_sys as sys;
use opendmd_core::connect::{self as cx, Backoff, Decoder, Event, Fnv, Hist, Payload};
use std::ffi::{c_int, c_uchar, c_void, CString};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering::Relaxed};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

// --- What this build needs from the sdkconfig ---------------------------
// Each line fails to compile (no such constant) without the probe's
// sdkconfig fragment (`firmware/sdkconfig.connect-probe`, layered on by
// the Makefile whenever the feature is on).
const _: u32 = sys::CONFIG_MBEDTLS_DYNAMIC_BUFFER;
const _: u32 = sys::CONFIG_MBEDTLS_CLIENT_SSL_SESSION_TICKETS;
const _: u32 = sys::CONFIG_MBEDTLS_X509_TRUSTED_CERT_CALLBACK;
const _: u32 = sys::CONFIG_MBEDTLS_HAVE_TIME_DATE;
const _: () = assert!(sys::CONFIG_LWIP_MAX_SOCKETS >= 15, "the probe's socket: CONFIG_LWIP_MAX_SOCKETS 15");
/// The TLS record sizes this build has (the Makefile's `CONNECT_TLS_IN`,
/// `CONNECT_TLS_OUT`).
pub const TLS_IN: u32 = sys::CONFIG_MBEDTLS_SSL_IN_CONTENT_LEN;
pub const TLS_OUT: u32 = sys::CONFIG_MBEDTLS_SSL_OUT_CONTENT_LEN;

// --- Build-time settings -------------------------------------------------

/// The server, its path and the shared secret (the echo Worker's `?k=`):
/// `OPENDMD_CONNECT_HOST`, `_PATH`, `_TOKEN` at build time, or
/// `POST /api/debug/connect/config` at run time (RAM only: nothing is
/// written to flash). The token is never logged or reported.
const DEFAULT_HOST: &str = match option_env!("OPENDMD_CONNECT_HOST") {
    Some(h) => h,
    None => "connect-s0.opendmd.dev",
};
const DEFAULT_PATH: &str = match option_env!("OPENDMD_CONNECT_PATH") {
    Some(p) => p,
    None => "/ws",
};
const DEFAULT_TOKEN: &str = match option_env!("OPENDMD_CONNECT_TOKEN") {
    Some(t) => t,
    None => "",
};
/// Connect here instead of looking the host up (`ip:port`; the host still
/// names the certificate): the emulator lane's local server.
const DEFAULT_ADDR: &str = match option_env!("OPENDMD_CONNECT_ADDR") {
    Some(a) => a,
    None => "",
};
/// The build's date (Unix seconds, `build.rs`): the clock floor.
const BUILD_UNIX: &str = env!("OPENDMD_BUILD_UNIX");

/// The pinned roots, by name (see the module docs). An emulator image
/// (`OPENDMD_CONNECT_TEST_ROOT` at build time) adds its local test CA.
static ROOTS: &[(&str, &[u8])] = &[
    ("isrg_root_x2", include_bytes!("../connect-roots/isrg_root_x2.der")),
    ("gts_root_r4", include_bytes!("../connect-roots/gts_root_r4.der")),
    ("ssl_com_tls_ecc_2022", include_bytes!("../connect-roots/ssl_com_tls_ecc_root_ca_2022.der")),
    ("ssl_com_ecc", include_bytes!("../connect-roots/ssl_com_root_certification_authority_ecc.der")),
    #[cfg(connect_test_root)]
    ("emulator_test_ca", include_bytes!(concat!(env!("OUT_DIR"), "/connect-test-root.der"))),
];

// --- mbedTLS constants (stable across 3.x) --------------------------------
const SSL_IS_CLIENT: c_int = 0;
const SSL_TRANSPORT_STREAM: c_int = 0;
const SSL_PRESET_DEFAULT: c_int = 0;
const SSL_VERIFY_REQUIRED: c_int = 2;
const ERR_WANT_READ: c_int = -0x6900;
const ERR_WANT_WRITE: c_int = -0x6880;
const ERR_PEER_CLOSE_NOTIFY: c_int = -0x7880;
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

/// How long one [`Probe::tick`] works before giving the worker back.
const SLICE: Duration = Duration::from_millis(250);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// A record's body once its first byte is in; a write.
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const UPGRADE_TIMEOUT: Duration = Duration::from_secs(10);
const CLOCK_WAIT: Duration = Duration::from_secs(60);
/// How long a close waits for the server's own close (so the server's end
/// holds the TIME_WAIT pcb, not ours: CLAUDE.md's "close second").
const CLOSE_WAIT: Duration = Duration::from_millis(1500);
const STREAM_TIMEOUT: Duration = Duration::from_secs(180);
/// A session that ran this long counts as having run (backoff forgotten).
const RAN: Duration = Duration::from_secs(60);
/// After a ping, wait this long right there for its pong (an exact RTT;
/// a later pong is still taken, its RTT then coarse).
const PONG_WAIT: Duration = Duration::from_millis(300);

fn now_us() -> u64 {
    // SAFETY: a plain clock read.
    unsafe { sys::esp_timer_get_time() as u64 }
}

fn uptime_s() -> u32 {
    (now_us() / 1_000_000) as u32
}

fn rand_u32() -> u32 {
    // SAFETY: the hardware RNG; no preconditions.
    unsafe { sys::esp_random() }
}

// --- Run-time settings ----------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PingKind {
    /// A WebSocket ping frame: the runtime answers it, the object sleeps on.
    Ws,
    /// The text `ping`: the object's auto-response (`pong`), no wake.
    Text,
    /// `echo <n>`: the object wakes and echoes it.
    Echo,
}

impl PingKind {
    fn as_str(self) -> &'static str {
        match self {
            PingKind::Ws => "ws",
            PingKind::Text => "text",
            PingKind::Echo => "echo",
        }
    }
}

#[derive(Clone)]
struct Cfg {
    enabled: bool,
    host: String,
    addr: String,
    path: String,
    token: String,
    ping_s: u32,
    ping: PingKind,
    tickets: bool,
    gate_free: u32,
    gate_largest: u32,
}

impl Cfg {
    fn defaults() -> Cfg {
        Cfg {
            enabled: true,
            host: DEFAULT_HOST.into(),
            addr: DEFAULT_ADDR.into(),
            path: DEFAULT_PATH.into(),
            token: DEFAULT_TOKEN.into(),
            ping_s: 45,
            ping: PingKind::Ws,
            tickets: true,
            // CONNECT.md 5.3: start a handshake only with this much free.
            gate_free: 60 * 1024,
            gate_largest: 16 * 1024,
        }
    }

    fn json(&self) -> String {
        format!(
            r#"{{"enabled":{},"host":{},"addr":{},"path":{},"token":{},"ping_s":{},"ping":"{}","tickets":{},"gate_free":{},"gate_largest":{},"tls_in":{TLS_IN},"tls_out":{TLS_OUT}}}"#,
            self.enabled,
            json_string(&self.host),
            json_string(&self.addr),
            json_string(&self.path),
            if self.token.is_empty() { "\"unset\"" } else { "\"set\"" },
            self.ping_s,
            self.ping.as_str(),
            self.tickets,
            self.gate_free,
            self.gate_largest,
        )
    }
}

static CFG: LazyLock<Mutex<Cfg>> = LazyLock::new(|| Mutex::new(Cfg::defaults()));
/// Bumped by every config change: the probe reconnects to apply it.
static CFG_GEN: AtomicU32 = AtomicU32::new(0);

fn cfg() -> Cfg {
    CFG.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

// --- What the debug API asks -------------------------------------------------

#[derive(Clone, Copy)]
enum Ask {
    /// M4 (`resume` false: tickets off, every one a full handshake) and M5
    /// (`resume` true: every one resumed from the last session).
    Handshakes { n: u32, interval: Duration, resume: bool },
    /// M9.
    Ed25519 { n: u32 },
    /// M10.
    Stream { bytes: u64, chunk: u32, window: u32, seed: u32 },
    Stop,
    Reconnect,
    Reset,
}

impl Ask {
    fn name(&self) -> &'static str {
        match self {
            Ask::Handshakes { resume: false, .. } => "full handshakes",
            Ask::Handshakes { resume: true, .. } => "resumed handshakes",
            Ask::Ed25519 { .. } => "ed25519",
            Ask::Stream { .. } => "stream",
            Ask::Stop => "stop",
            Ask::Reconnect => "reconnect",
            Ask::Reset => "reset",
        }
    }
}

/// The last request and its number (each POST's `ask`; a job, M9 and M10
/// report the number that started them as `seq`, so a script tells its own
/// run from the one before: the probe takes a request at its next turn).
static ASK: Mutex<Option<(u32, Ask)>> = Mutex::new(None);
static ASK_SEQ: AtomicU32 = AtomicU32::new(0);

fn ask(a: Ask) -> u32 {
    let seq = ASK_SEQ.fetch_add(1, Relaxed) + 1;
    *ASK.lock().unwrap_or_else(|p| p.into_inner()) = Some((seq, a));
    seq
}

// --- mbedTLS's heap, counted ---------------------------------------------------

/// Bytes mbedTLS holds now (all of its users: WPA-Enterprise and SAE too;
/// what was allocated before the counting began is not in it, so only
/// differences mean anything), and the most since [`mark_tls_peak`].
static TLS_IN_USE: AtomicI32 = AtomicI32::new(0);
static TLS_PEAK: AtomicI32 = AtomicI32::new(0);

unsafe extern "C" fn tls_calloc(n: usize, size: usize) -> *mut c_void {
    // As IDF's own (CONFIG_MBEDTLS_INTERNAL_MEM_ALLOC): internal RAM.
    let p = sys::heap_caps_calloc(n, size, sys::MALLOC_CAP_INTERNAL | sys::MALLOC_CAP_8BIT);
    if !p.is_null() {
        let size = sys::heap_caps_get_allocated_size(p) as i32;
        let now = TLS_IN_USE.fetch_add(size, Relaxed) + size;
        TLS_PEAK.fetch_max(now, Relaxed);
    }
    p
}

unsafe extern "C" fn tls_free(p: *mut c_void) {
    if !p.is_null() {
        TLS_IN_USE.fetch_sub(sys::heap_caps_get_allocated_size(p) as i32, Relaxed);
        sys::heap_caps_free(p);
    }
}

extern "C" {
    // mbedtls/platform.h (MBEDTLS_PLATFORM_MEMORY, which IDF defines).
    fn mbedtls_platform_set_calloc_free(
        calloc: unsafe extern "C" fn(usize, usize) -> *mut c_void,
        free: unsafe extern "C" fn(*mut c_void),
    ) -> c_int;
}

fn mark_tls_peak() -> i32 {
    let now = TLS_IN_USE.load(Relaxed);
    TLS_PEAK.store(now, Relaxed);
    now
}

// --- Telemetry -------------------------------------------------------------------

/// Why a session ended or an attempt failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
enum Cause {
    /// A test hook closed it (M4, M5, reconnect, config).
    Forced,
    /// The server sent a WebSocket close.
    ServerClose,
    /// TLS close_notify or TCP end without a WebSocket close.
    Eof,
    /// Two pings unanswered.
    PongTimeout,
    /// A TLS error on an open session.
    Tls,
    /// A WebSocket protocol error.
    Ws,
    /// WiFi went away.
    Wifi,
    /// A write or read failed or timed out.
    Io,
    /// Attempts that never opened: by stage.
    Dns,
    Tcp,
    Handshake,
    Upgrade,
}

const CAUSES: [&str; 12] = [
    "forced", "server_close", "eof", "pong_timeout", "tls", "ws", "wifi", "io", "dns", "tcp", "handshake", "upgrade",
];

impl Cause {
    fn as_str(self) -> &'static str {
        CAUSES[self as usize]
    }
}

#[derive(Default, Clone)]
struct M9 {
    seq: u32,
    n: u32,
    of: u32,
    min_us: u32,
    max_us: u32,
    sum_us: u64,
    failed: u32,
    stack_unused: u32,
    done: bool,
}

#[derive(Default, Clone)]
struct M10 {
    seq: u32,
    state: &'static str,
    bytes: u64,
    of: u64,
    chunk: u32,
    window: u32,
    frames: u32,
    mismatches: u32,
    ms: u32,
    fnv: u32,
    server: Option<(u64, u32)>,
    ok: Option<bool>,
    heap_low: u32,
    largest_at_low: u32,
    credits: u32,
}

struct Tele {
    state: &'static str,
    since_s: u32,
    clock: &'static str,
    attempts: u32,
    opened: u32,
    full: Hist,
    resumed: Hist,
    /// Resumption asked for, a full handshake given.
    resume_missed: u32,
    dns_ms: u32,
    tcp_ms: u32,
    upgrade_ms: u32,
    /// From the lookup to the upgrade's answer, the last attempt that opened.
    connect_ms: u32,
    /// The last handshake's time inside mbedTLS's calls (its CPU0 cost,
    /// with the reads of records already arriving).
    hs_cpu_ms: u32,
    hs_cpu_max_ms: u32,
    /// Sessions that ended (not the forced ones), by cause; attempts that
    /// failed, by stage.
    reconnects: u32,
    causes: [u32; CAUSES.len()],
    gate_waits: u32,
    backoff_s: u32,
    /// The last sessions: (seconds open, cause, close code).
    sessions: [(u32, Cause, u16); 8],
    sessions_n: usize,
    session_max_s: u32,
    /// Free heap just before an attempt; the least sampled at each TLS read
    /// and write during the last handshake and since boot, with the largest
    /// block then; the free heap once open.
    heap_at_connect: u32,
    heap_hs_low: u32,
    heap_low: u32,
    heap_low_largest: u32,
    heap_open: u32,
    /// mbedTLS's own bytes: the most over the last handshake, and held once
    /// open, above what it held before the attempt.
    tls_hs_peak: i32,
    tls_hs_peak_max: i32,
    tls_open: i32,
    /// The Rust side of a session (its boxes).
    rust_bytes: u32,
    /// The network worker's stack never used, bytes (its whole life).
    stack_unused: u32,
    pings: u32,
    pongs: u32,
    missed: u32,
    rtt_ms: Option<u32>,
    rtt_max_ms: u32,
    rx: u64,
    tx: u64,
    root: &'static str,
    cert_flags: u32,
    job: &'static str,
    job_seq: u32,
    job_done: u32,
    job_of: u32,
    job_failed: u32,
    m9: Option<M9>,
    m10: Option<M10>,
    last_error: Option<String>,
    last_error_s: u32,
    errors: u32,
}

impl Tele {
    fn new() -> Tele {
        Tele {
            state: "starting",
            since_s: 0,
            clock: "waiting",
            attempts: 0,
            opened: 0,
            full: Hist::default(),
            resumed: Hist::default(),
            resume_missed: 0,
            dns_ms: 0,
            tcp_ms: 0,
            upgrade_ms: 0,
            connect_ms: 0,
            hs_cpu_ms: 0,
            hs_cpu_max_ms: 0,
            reconnects: 0,
            causes: [0; CAUSES.len()],
            gate_waits: 0,
            backoff_s: 0,
            sessions: [(0, Cause::Forced, 0); 8],
            sessions_n: 0,
            session_max_s: 0,
            heap_at_connect: 0,
            heap_hs_low: 0,
            heap_low: u32::MAX,
            heap_low_largest: 0,
            heap_open: 0,
            tls_hs_peak: 0,
            tls_hs_peak_max: 0,
            tls_open: 0,
            rust_bytes: 0,
            stack_unused: u32::MAX,
            pings: 0,
            pongs: 0,
            missed: 0,
            rtt_ms: None,
            rtt_max_ms: 0,
            rx: 0,
            tx: 0,
            root: "",
            cert_flags: 0,
            job: "none",
            job_seq: 0,
            job_done: 0,
            job_of: 0,
            job_failed: 0,
            m9: None,
            m10: None,
            last_error: None,
            last_error_s: 0,
            errors: 0,
        }
    }
}

static TELE: LazyLock<Mutex<Tele>> = LazyLock::new(|| Mutex::new(Tele::new()));

fn tele<R>(f: impl FnOnce(&mut Tele) -> R) -> R {
    f(&mut TELE.lock().unwrap_or_else(|p| p.into_inner()))
}

fn set_state(s: &'static str) {
    tele(|t| {
        if t.state != s {
            t.state = s;
            t.since_s = uptime_s();
        }
    });
}

fn error(e: String) {
    log::warn!("connect-probe: {e}");
    tele(|t| {
        t.last_error = Some(e);
        t.last_error_s = uptime_s();
        t.errors += 1;
    });
}

fn opt<T: ToString>(v: Option<T>) -> String {
    v.map_or_else(|| "null".to_string(), |v| v.to_string())
}

/// `/api/status` `connect`: everything the probe measures.
pub fn json() -> String {
    let t = TELE.lock().unwrap_or_else(|p| p.into_inner());
    let c = cfg();
    let causes: Vec<String> = CAUSES.iter().zip(t.causes).map(|(n, v)| format!(r#""{n}":{v}"#)).collect();
    let sessions: Vec<String> = (0..t.sessions_n.min(8))
        .map(|k| {
            let (s, cause, code) = t.sessions[(t.sessions_n - 1 - k) % 8];
            format!(r#"[{s},"{}",{code}]"#, cause.as_str())
        })
        .collect();
    let m9 = t.m9.as_ref().map(|m| {
        format!(
            r#"{{"seq":{},"n":{},"of":{},"done":{},"min_us":{},"mean_us":{},"max_us":{},"failed":{},"stack_unused":{}}}"#,
            m.seq,
            m.n,
            m.of,
            m.done,
            m.min_us,
            if m.n > 0 { m.sum_us / u64::from(m.n) } else { 0 },
            m.max_us,
            m.failed,
            m.stack_unused
        )
    });
    let m10 = t.m10.as_ref().map(|m| {
        let kbps = if m.ms > 0 { m.bytes * 1000 / u64::from(m.ms) / 1024 } else { 0 };
        format!(
            r#"{{"seq":{},"state":"{}","bytes":{},"of":{},"chunk":{},"window":{},"frames":{},"credits":{},"ms":{},"kb_s":{kbps},"mismatches":{},"fnv":"{:08x}","server":{},"checksum_ok":{},"heap_low":{},"largest_at_low":{}}}"#,
            m.seq,
            m.state,
            m.bytes,
            m.of,
            m.chunk,
            m.window,
            m.frames,
            m.credits,
            m.ms,
            m.mismatches,
            m.fnv,
            m.server.map_or_else(|| "null".to_string(), |(b, f)| format!(r#"{{"bytes":{b},"fnv":"{f:08x}"}}"#)),
            opt(m.ok),
            m.heap_low,
            m.largest_at_low
        )
    });
    format!(
        concat!(
            r#"{{"state":"{}","since_s":{},"host":{},"tickets":{},"ping_mode":"{}","ping_s":{},"tls_in":{},"tls_out":{},"clock":"{}","#,
            r#""attempts":{},"opened":{},"full":{},"resumed":{},"resume_missed":{},"#,
            r#""dns_ms":{},"tcp_ms":{},"upgrade_ms":{},"connect_ms":{},"hs_cpu_ms":{},"hs_cpu_max_ms":{},"#,
            r#""reconnects":{},"causes":{{{}}},"gate_waits":{},"backoff_s":{},"sessions":[{}],"session_max_s":{},"#,
            r#""heap":{{"at_connect":{},"hs_low":{},"low":{},"low_largest":{},"open":{},"tls_hs_peak":{},"tls_hs_peak_max":{},"tls_open":{},"tls_in_use":{},"rust":{}}},"#,
            r#""stack_unused":{},"ping":{{"sent":{},"pongs":{},"missed":{},"rtt_ms":{},"rtt_max_ms":{}}},"rx":{},"tx":{},"root":{},"cert_flags":{},"#,
            r#""job":{{"kind":"{}","seq":{},"done":{},"of":{},"failed":{}}},"m9":{},"m10":{},"errors":{},"last_error":{},"last_error_s":{}}}"#
        ),
        t.state,
        t.since_s,
        json_string(&c.host),
        c.tickets,
        c.ping.as_str(),
        c.ping_s,
        TLS_IN,
        TLS_OUT,
        t.clock,
        t.attempts,
        t.opened,
        t.full.json(),
        t.resumed.json(),
        t.resume_missed,
        t.dns_ms,
        t.tcp_ms,
        t.upgrade_ms,
        t.connect_ms,
        t.hs_cpu_ms,
        t.hs_cpu_max_ms,
        t.reconnects,
        causes.join(","),
        t.gate_waits,
        t.backoff_s,
        sessions.join(","),
        t.session_max_s,
        t.heap_at_connect,
        t.heap_hs_low,
        if t.heap_low == u32::MAX { 0 } else { t.heap_low },
        t.heap_low_largest,
        t.heap_open,
        t.tls_hs_peak,
        t.tls_hs_peak_max,
        t.tls_open,
        TLS_IN_USE.load(Relaxed),
        t.rust_bytes,
        if t.stack_unused == u32::MAX { "null".to_string() } else { t.stack_unused.to_string() },
        t.pings,
        t.pongs,
        t.missed,
        opt(t.rtt_ms),
        t.rtt_max_ms,
        t.rx,
        t.tx,
        if t.root.is_empty() { "null".to_string() } else { json_string(t.root) },
        t.cert_flags,
        t.job,
        t.job_seq,
        t.job_done,
        t.job_of,
        t.job_failed,
        opt(m9),
        opt(m10),
        t.errors,
        t.last_error.as_deref().map_or_else(|| "null".to_string(), json_string),
        t.last_error_s,
    )
}

// --- The TLS pieces ---------------------------------------------------------------

/// What the I/O callbacks need: the socket, and the heap low they sample.
struct Bio {
    fd: c_int,
    hs_low: u32,
    rx: u64,
    tx: u64,
}

impl Bio {
    fn sample(&mut self) {
        let f = free_heap();
        if f < self.hs_low {
            self.hs_low = f;
        }
        crate::netstat::sample(crate::netstat::At::Connect);
        let low = tele(|t| (f < t.heap_low).then(|| t.heap_low = f).is_some());
        if low {
            let largest = largest_free_block() as u32;
            tele(|t| t.heap_low_largest = largest);
        }
    }
}

fn would_block() -> bool {
    std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock
}

unsafe extern "C" fn bio_send(ctx: *mut c_void, buf: *const c_uchar, len: usize) -> c_int {
    // SAFETY: `ctx` is the connection's boxed `Bio`, alive while its TLS is.
    let b = &mut *(ctx as *mut Bio);
    b.sample();
    let n = sys::lwip_send(b.fd, buf as *const c_void, len, 0);
    if n >= 0 {
        b.tx += n as u64;
        return n as c_int;
    }
    // A blocking socket: EAGAIN here is its send timeout.
    ERR_NET_SEND_FAILED
}

/// All of `len`, or nothing yet. mbedTLS's dynamic buffers read a record's
/// 5-byte header into a buffer on the stack (`esp_mbedtls_add_rx_buffer`),
/// so a header split over two calls by a `WANT_READ` would lose its first
/// part: this answers `WANT_READ` only when nothing at all has arrived, and
/// once something has, blocks (up to [`IO_TIMEOUT`]) for the rest.
unsafe extern "C" fn bio_recv(ctx: *mut c_void, buf: *mut c_uchar, len: usize) -> c_int {
    // SAFETY: as in `bio_send`.
    let b = &mut *(ctx as *mut Bio);
    b.sample();
    let mut one = 0u8;
    let n = sys::lwip_recv(
        b.fd,
        &mut one as *mut u8 as *mut c_void,
        1,
        (sys::MSG_PEEK | sys::MSG_DONTWAIT) as c_int,
    );
    if n == 0 {
        return 0;
    }
    if n < 0 {
        return if would_block() { ERR_WANT_READ } else { ERR_NET_RECV_FAILED };
    }
    let mut got = 0usize;
    while got < len {
        let n = sys::lwip_recv(b.fd, buf.add(got) as *mut c_void, len - got, 0);
        if n > 0 {
            got += n as usize;
        } else if n == 0 {
            break;
        } else {
            return ERR_NET_RECV_FAILED;
        }
    }
    b.rx += got as u64;
    got as c_int
}

unsafe extern "C" fn rng(_: *mut c_void, out: *mut c_uchar, len: usize) -> c_int {
    sys::esp_fill_random(out as *mut c_void, len);
    0
}

/// Which pinned root the last chain ended at (index + 1; 0 none yet).
static ROOT_USED: AtomicU32 = AtomicU32::new(0);

/// mbedTLS asks for the trusted parents of `child`: the pinned root whose
/// subject is its issuer, parsed in place from flash, or none.
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
        // ours, `tls_free`, which takes any heap block).
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
        ROOT_USED.store(k as u32 + 1, Relaxed);
        *out = crt;
        return 0;
    }
    0
}

/// What the certificate check saw.
struct Verify {
    /// Called at all: a full handshake (a resumed one checks nothing).
    called: bool,
    /// The clock floor in force (Unix seconds), when SNTP has not set it.
    floor: Option<i64>,
    flags: u32,
}

unsafe extern "C" fn verify_cb(ctx: *mut c_void, crt: *mut sys::mbedtls_x509_crt, _depth: c_int, flags: *mut u32) -> c_int {
    // SAFETY: `ctx` is the connection's boxed `Verify`.
    let v = &mut *(ctx as *mut Verify);
    v.called = true;
    if let Some(floor) = v.floor {
        // The clock reads 1970: "not yet valid" means nothing; "ended" is
        // judged against the build date.
        *flags &= !BADCERT_FUTURE;
        let t = &(*crt).valid_to;
        let to = cx::civil_to_unix(t.year.into(), t.mon as u32, t.day as u32, t.hour as u32, t.min as u32, t.sec as u32);
        if to < floor {
            *flags |= BADCERT_EXPIRED;
        }
    }
    v.flags |= *flags;
    0
}

/// An mbedTLS error as its code and, for the ones a session meets, a few
/// words. Not `mbedtls_strerror`: its tables of every module's messages
/// are ~10 KB of flash.
fn tls_error(r: c_int) -> String {
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

struct Tls {
    ssl: sys::mbedtls_ssl_context,
    conf: sys::mbedtls_ssl_config,
}

impl Drop for Tls {
    fn drop(&mut self) {
        // SAFETY: both were initialised (zeroed, then _init) before any use.
        unsafe {
            sys::mbedtls_ssl_free(&mut self.ssl);
            sys::mbedtls_ssl_config_free(&mut self.conf);
        }
    }
}

/// A saved session (its ticket) for the next connection's resumption.
struct Session(sys::mbedtls_ssl_session);

impl Session {
    fn new() -> Box<Session> {
        // SAFETY: an all-zero session is what _init makes; _init then.
        let mut s: Box<Session> = unsafe { Box::new_zeroed().assume_init() };
        unsafe { sys::mbedtls_ssl_session_init(&mut s.0) };
        s
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: initialised in `new`.
        unsafe { sys::mbedtls_ssl_session_free(&mut self.0) };
    }
}

// --- A connection -------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Handshake,
    Upgrade,
    Open,
}

struct Stream {
    want: u64,
    window: u32,
    got: u64,
    frames: u32,
    ungranted: u32,
    credits: u32,
    gen: Payload,
    fnv: Fnv,
    mismatches: u32,
    t0: Instant,
    heap_low: u32,
    largest_at_low: u32,
    server: Option<(u64, u32)>,
}

struct Ping {
    seq: u32,
    sent: Option<(u32, Instant)>,
    next: Instant,
    missed: u32,
}

struct Conn {
    tcp: TcpStream,
    tls: Box<Tls>,
    bio: Box<Bio>,
    verify: Box<Verify>,
    phase: Phase,
    t0: Instant,
    hs_t0: Instant,
    hs_cpu_us: u64,
    resume: bool,
    tickets: bool,
    tls_before: i32,
    key: String,
    head: Vec<u8>,
    dec: Box<Decoder>,
    buf: Box<[u8; 512]>,
    opened: Option<Instant>,
    ping: Ping,
    stream: Option<Stream>,
}

/// The Rust side's bytes for an open session (reported with mbedTLS's;
/// the upgrade's head buffer, up to 1 KB, is gone by then).
fn rust_bytes() -> u32 {
    (std::mem::size_of::<Conn>()
        + std::mem::size_of::<Tls>()
        + std::mem::size_of::<Bio>()
        + std::mem::size_of::<Verify>()
        + std::mem::size_of::<Decoder>()
        + 512) as u32
}

/// How a session ended.
struct End {
    cause: Cause,
    code: u16,
    why: String,
}

impl End {
    fn new(cause: Cause, why: impl Into<String>) -> End {
        End { cause, code: 0, why: why.into() }
    }
}

impl Conn {
    fn fd(&self) -> c_int {
        self.tcp.as_raw_fd()
    }

    fn ssl(&mut self) -> *mut sys::mbedtls_ssl_context {
        &mut self.tls.ssl
    }

    /// Wait up to `ms` for something to read.
    fn poll(&self, ms: u32) -> bool {
        let mut p = sys::pollfd { fd: self.fd(), events: sys::POLLIN as i16, revents: 0 };
        // SAFETY: one valid pollfd.
        unsafe { sys::poll(&mut p, 1, ms as c_int) > 0 }
    }

    /// Write all of `data` through TLS (blocking, [`IO_TIMEOUT`]).
    fn write_all(&mut self, mut data: &[u8]) -> Result<(), End> {
        let ssl = self.ssl();
        let until = Instant::now() + IO_TIMEOUT;
        while !data.is_empty() {
            if Instant::now() > until {
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

    fn send(&mut self, opcode: u8, payload: &[u8]) -> Result<(), End> {
        let mut f = Vec::with_capacity(payload.len() + 8);
        let mut mask = [0u8; 4];
        // SAFETY: four bytes.
        unsafe { sys::esp_fill_random(mask.as_mut_ptr() as *mut c_void, 4) };
        cx::encode_frame(opcode, payload, mask, &mut f);
        self.write_all(&f)
    }

    fn text(&mut self, s: &str) -> Result<(), End> {
        self.send(cx::OP_TEXT, s.as_bytes())
    }
}

/// What the probe is doing besides holding the session.
#[derive(Clone, Copy, Debug)]
enum Job {
    None,
    Handshakes { n: u32, done: u32, failed: u32, interval: Duration, resume: bool, next: Instant },
}

/// M9 in progress (beside anything else).
struct M9Run {
    n: u32,
    done: u32,
    msg: Box<[u8; cx::ED25519_MSG_LEN]>,
}

// --- The probe ---------------------------------------------------------------------

/// Owned by the network worker's loop (`net_worker::run`).
pub struct Probe {
    conn: Option<Box<Conn>>,
    backoff: Backoff,
    /// No attempt before this.
    not_before: Instant,
    session: Option<Box<Session>>,
    job: Job,
    m9: Option<M9Run>,
    /// A stream asked for, sent once the session is open.
    stream_ask: Option<(u32, u64, u32, u32, u32)>,
    online_since: Option<Instant>,
    cfg_gen: u32,
    floor: Option<i64>,
    /// With no connection: the next attempt waits for a time (backoff, a
    /// job's interval, the memory gate), not for the network or the clock.
    timed: bool,
}

impl Probe {
    pub fn new() -> Probe {
        // SAFETY: once, before any TLS use of ours (mbedTLS's other users
        // may already hold blocks: `tls_free` takes any heap block).
        unsafe { mbedtls_platform_set_calloc_free(tls_calloc, tls_free) };
        log::info!(
            "connect-probe: on (host {DEFAULT_HOST}, path {DEFAULT_PATH}, token {}, TLS in {TLS_IN} out {TLS_OUT}, {} pinned roots)",
            if DEFAULT_TOKEN.is_empty() { "unset" } else { "set" },
            ROOTS.len()
        );
        tele(|t| t.rust_bytes = rust_bytes());
        Probe {
            conn: None,
            backoff: Backoff::default(),
            not_before: Instant::now(),
            session: None,
            job: Job::None,
            m9: None,
            stream_ask: None,
            online_since: None,
            cfg_gen: CFG_GEN.load(Relaxed),
            floor: None,
            timed: false,
        }
    }

    /// How long the worker may sleep before the next [`Probe::tick`].
    /// Never less than 5 ms; never more than the worker's own second, so
    /// the loop's cadence is unchanged unless something is in progress
    /// (a handshake, an upgrade, a stream, M9: each tick then works for up
    /// to [`SLICE`] and the loop comes straight back).
    pub fn wait(&self) -> Duration {
        let busy = Duration::from_millis(5);
        if self.m9.is_some() {
            return busy;
        }
        let at = match self.conn.as_deref() {
            Some(c) if c.phase != Phase::Open || c.stream.is_some() => return busy,
            Some(c) => Some(c.ping.next),
            // Waiting for something that is not a time (the network, the
            // clock, switched on): the worker's own pace.
            None if !self.timed => None,
            None => match self.job {
                Job::Handshakes { next, .. } => Some(next),
                Job::None => Some(self.not_before),
            },
        };
        at.map_or(Duration::from_secs(1), |t| t.saturating_duration_since(Instant::now()).clamp(busy, Duration::from_secs(1)))
    }

    /// The worker's turn. `online`: a station with an address.
    pub fn tick(&mut self, online: bool) {
        self.turn(online);
        self.note_stack();
    }

    fn turn(&mut self, online: bool) {
        let started = Instant::now();
        self.timed = false;
        self.take_ask();
        self.run_m9(started);
        let c = cfg();
        let gen = CFG_GEN.load(Relaxed);
        if gen != self.cfg_gen {
            self.cfg_gen = gen;
            if self.conn.is_some() {
                self.close(Cause::Forced, "settings changed");
            }
            self.not_before = Instant::now();
        }
        if !c.enabled {
            if self.conn.is_some() {
                self.close(Cause::Forced, "switched off");
            }
            set_state("off");
            return;
        }
        if !online {
            self.online_since = None;
            if self.conn.is_some() {
                self.end(End::new(Cause::Wifi, "WiFi went away"));
            }
            set_state("waiting for network");
            return;
        }
        let since = *self.online_since.get_or_insert_with(Instant::now);
        if self.conn.is_none() && !self.clock_ready(since) {
            set_state("waiting for clock");
            return;
        }
        if self.conn.is_none() {
            self.timed = true;
            self.maybe_connect(&c);
        }
        let Some(conn) = self.conn.as_mut() else { return };
        let result = match conn.phase {
            Phase::Handshake => step_handshake(conn, started),
            Phase::Upgrade => step_upgrade(conn, &c, started),
            Phase::Open => Ok(()),
        };
        if let Err(e) = result {
            return self.end(e);
        }
        let conn = self.conn.as_mut().expect("held");
        if conn.phase == Phase::Open && conn.opened.is_none() {
            self.opened(&c);
        }
        let conn = self.conn.as_mut().expect("held");
        if conn.phase == Phase::Open {
            // A job's connection is closed as soon as it is open.
            if let Job::Handshakes { .. } = self.job {
                return self.close(Cause::Forced, "");
            }
            if let Some((seq, bytes, chunk, window, seed)) = self.stream_ask.take() {
                if let Err(e) = start_stream(conn, seq, bytes, chunk, window, seed) {
                    return self.end(e);
                }
            }
            if let Err(e) = service(conn, &c, started) {
                self.end(e);
            }
        }
    }

    fn clock_ready(&mut self, online_since: Instant) -> bool {
        if crate::clock::synced_ago_secs().is_some() {
            self.floor = None;
            tele(|t| t.clock = "sntp");
            return true;
        }
        if online_since.elapsed() >= CLOCK_WAIT {
            if self.floor.is_none() {
                error("no clock 60 s after the network came up (UDP 123 blocked?): certificate dates checked against the build date".into());
            }
            self.floor = Some(BUILD_UNIX.parse().unwrap_or(0));
            tele(|t| t.clock = "build date");
            return true;
        }
        false
    }

    fn note_stack(&self) {
        // SAFETY: the calling task's own high-water mark (bytes on IDF).
        let unused = unsafe { sys::uxTaskGetStackHighWaterMark(std::ptr::null_mut()) };
        tele(|t| t.stack_unused = t.stack_unused.min(unused));
    }

    fn take_ask(&mut self) {
        let Some((seq, a)) = ASK.lock().unwrap_or_else(|p| p.into_inner()).take() else { return };
        log::info!("connect-probe: asked {}", a.name());
        match a {
            Ask::Handshakes { n, interval, resume } => {
                self.stream_ask = None;
                if self.conn.is_some() {
                    self.close(Cause::Forced, "");
                }
                if resume && self.session.is_none() {
                    // The first one makes the ticket the rest resume.
                    log::info!("connect-probe: no saved session yet: the first is a full handshake");
                }
                self.job = Job::Handshakes { n, done: 0, failed: 0, interval, resume, next: Instant::now() };
                tele(|t| {
                    t.job = if resume { "resumed" } else { "full" };
                    t.job_seq = seq;
                    t.job_done = 0;
                    t.job_of = n;
                    t.job_failed = 0;
                });
            }
            Ask::Ed25519 { n } => {
                let mut msg = Box::new([0u8; cx::ED25519_MSG_LEN]);
                *msg = cx::ed25519_msg();
                self.m9 = Some(M9Run { n, done: 0, msg });
                tele(|t| t.m9 = Some(M9 { seq, of: n, min_us: u32::MAX, ..M9::default() }));
            }
            Ask::Stream { bytes, chunk, window, seed } => {
                if let Some(c) = self.conn.as_mut().filter(|c| c.phase == Phase::Open && c.stream.is_none()) {
                    if let Err(e) = start_stream(c, seq, bytes, chunk, window, seed) {
                        self.end(e);
                    }
                } else {
                    self.stream_ask = Some((seq, bytes, chunk, window, seed));
                    tele(|t| t.m10 = Some(M10 { seq, state: "waiting for the session", of: bytes, chunk, window, ..M10::default() }));
                }
            }
            Ask::Stop => {
                self.job = Job::None;
                self.m9 = None;
                self.stream_ask = None;
                if let Some(c) = self.conn.as_mut() {
                    if c.stream.take().is_some() {
                        tele(|t| {
                            if let Some(m) = t.m10.as_mut() {
                                m.state = "stopped";
                            }
                        });
                    }
                }
                tele(|t| t.job = "none");
            }
            Ask::Reconnect => {
                if self.conn.is_some() {
                    self.close(Cause::Forced, "");
                }
                self.backoff.reset();
                self.not_before = Instant::now();
            }
            Ask::Reset => tele(|t| {
                let keep = (t.state, t.since_s, t.clock, t.stack_unused, t.rust_bytes);
                *t = Tele::new();
                (t.state, t.since_s, t.clock, t.stack_unused, t.rust_bytes) = keep;
            }),
        }
    }

    fn run_m9(&mut self, started: Instant) {
        let Some(run) = self.m9.as_mut() else { return };
        while run.done < run.n && started.elapsed() < SLICE {
            let t = now_us();
            let ok = cx::ed25519_vector(&run.msg[..]);
            let us = (now_us() - t) as u32;
            run.done += 1;
            tele(|t| {
                if let Some(m) = t.m9.as_mut() {
                    m.n += 1;
                    m.min_us = m.min_us.min(us);
                    m.max_us = m.max_us.max(us);
                    m.sum_us += u64::from(us);
                    if !ok {
                        m.failed += 1;
                    }
                }
            });
        }
        if run.done >= run.n {
            self.m9 = None;
            // SAFETY: the calling task's own high-water mark.
            let unused = unsafe { sys::uxTaskGetStackHighWaterMark(std::ptr::null_mut()) };
            tele(|t| {
                t.stack_unused = t.stack_unused.min(unused);
                if let Some(m) = t.m9.as_mut() {
                    m.done = true;
                    m.stack_unused = t.stack_unused;
                    log::info!("connect-probe: M9 {} verifies, {}-{} us, {} failed", m.n, m.min_us, m.max_us, m.failed);
                }
            });
        }
    }

    /// Start an attempt if it is time and the heap allows.
    fn maybe_connect(&mut self, c: &Cfg) {
        let now = Instant::now();
        // Tickets offered at all, and the saved session offered back.
        let tickets = match self.job {
            Job::Handshakes { next, resume, .. } => {
                if now < next {
                    set_state(if resume { "job: next resumed" } else { "job: next full" });
                    return;
                }
                resume
            }
            Job::None => {
                if now < self.not_before {
                    set_state("backoff");
                    return;
                }
                c.tickets
            }
        };
        let (free, largest) = (free_heap(), largest_free_block() as u32);
        if free < c.gate_free || largest < c.gate_largest {
            set_state("waiting for memory");
            tele(|t| t.gate_waits += 1);
            self.not_before = now + Duration::from_secs(2);
            if let Job::Handshakes { ref mut next, .. } = self.job {
                *next = now + Duration::from_secs(2);
            }
            return;
        }
        if let Job::Handshakes { ref mut next, interval, .. } = self.job {
            *next = now + interval;
        }
        set_state("connecting");
        tele(|t| {
            t.attempts += 1;
            t.heap_at_connect = free;
        });
        let session = if tickets { self.session.as_deref() } else { None };
        match connect(c, tickets, session, self.floor, free) {
            Ok(conn) => self.conn = Some(conn),
            Err(e) => self.failed(e),
        }
    }

    /// The session is open: record the handshake.
    fn opened(&mut self, c: &Cfg) {
        let conn = self.conn.as_mut().expect("held");
        conn.opened = Some(Instant::now());
        conn.ping.next = Instant::now() + Duration::from_secs(u64::from(c.ping_s.max(1)));
        let full = conn.verify.called;
        let connect_ms = conn.t0.elapsed().as_millis() as u32;
        let free = free_heap();
        let tls_open = TLS_IN_USE.load(Relaxed) - conn.tls_before;
        // A new ticket for the next resumption.
        if conn.tickets {
            let s = self.session.get_or_insert_with(Session::new);
            // SAFETY: an open context and an initialised session.
            let r = unsafe { sys::mbedtls_ssl_get_session(&conn.tls.ssl, &mut s.0) };
            if r != 0 {
                error(format!("no session to keep: {}", tls_error(r)));
                self.session = None;
            }
        }
        let root = match ROOT_USED.load(Relaxed) {
            0 => "",
            k => ROOTS.get(k as usize - 1).map_or("", |r| r.0),
        };
        let (resume, flags) = (conn.resume, conn.verify.flags);
        tele(|t| {
            t.opened += 1;
            t.connect_ms = connect_ms;
            t.heap_open = free;
            t.tls_open = tls_open;
            t.cert_flags = flags;
            if full {
                t.root = root;
            }
            if resume && full {
                t.resume_missed += 1;
            }
        });
        set_state("open");
        log::info!(
            "connect-probe: open ({} handshake, heap {} -> {} B, lowest {} B, mbedTLS +{} B peak, +{} B held)",
            if full { "full" } else { "resumed" },
            tele(|t| t.heap_at_connect),
            free,
            tele(|t| t.heap_hs_low),
            tele(|t| t.tls_hs_peak),
            tls_open
        );
        if let Job::Handshakes { ref mut done, n, .. } = self.job {
            *done += 1;
            let d = *done;
            tele(|t| t.job_done = d);
            if d >= n {
                log::info!("connect-probe: job done ({d} handshakes)");
                self.job = Job::None;
                tele(|t| t.job = "none");
                // Back to holding a session, at once.
                self.not_before = Instant::now();
                self.backoff.reset();
            }
        }
    }

    /// An attempt that never opened.
    fn failed(&mut self, e: End) {
        tele(|t| t.causes[e.cause as usize] += 1);
        error(format!("{}: {}", e.cause.as_str(), e.why));
        if let Job::Handshakes { ref mut failed, n, .. } = self.job {
            *failed += 1;
            let f = *failed;
            tele(|t| t.job_failed = f);
            if f > n.max(10) {
                error("job stopped: more failures than handshakes asked".into());
                self.job = Job::None;
                tele(|t| t.job = "none");
            }
            set_state("job: failed, next at its interval");
            return;
        }
        let wait = self.backoff.failed(rand_u32());
        self.not_before = Instant::now() + wait;
        tele(|t| t.backoff_s = wait.as_secs() as u32);
        set_state("backoff");
    }

    /// The session (or attempt) ended for `e`: drop it, count it, wait.
    fn end(&mut self, e: End) {
        let Some(conn) = self.conn.take() else { return };
        let Some(opened) = conn.opened else {
            drop(conn);
            if e.cause == Cause::Forced {
                // Stopped by a test hook or a settings change mid-attempt.
                self.not_before = Instant::now();
                set_state("closed");
                return;
            }
            // It never opened: an attempt that failed.
            return self.failed(e);
        };
        self.finish_stream(&conn, e.cause.as_str());
        let secs = opened.elapsed().as_secs() as u32;
        let (rx, tx) = (conn.bio.rx, conn.bio.tx);
        drop(conn);
        tele(|t| {
            t.rx += rx;
            t.tx += tx;
            t.causes[e.cause as usize] += 1;
            if e.cause != Cause::Forced {
                t.reconnects += 1;
            }
            t.sessions[t.sessions_n % 8] = (secs, e.cause, e.code);
            t.sessions_n += 1;
            t.session_max_s = t.session_max_s.max(secs);
        });
        if e.cause != Cause::Forced {
            error(format!("session ended after {secs} s: {} {}", e.cause.as_str(), e.why));
        }
        let wait = if e.cause == Cause::Forced {
            Duration::ZERO
        } else if opened.elapsed() >= RAN || secs >= RAN.as_secs() as u32 {
            self.backoff.dropped(rand_u32())
        } else {
            self.backoff.failed(rand_u32())
        };
        self.not_before = Instant::now() + wait;
        tele(|t| t.backoff_s = wait.as_secs() as u32);
        set_state(if e.cause == Cause::Forced { "closed" } else { "backoff" });
    }

    /// Close cleanly (a WebSocket close, the server's answer awaited), then
    /// [`Probe::end`].
    fn close(&mut self, cause: Cause, why: &str) {
        if let Some(conn) = self.conn.as_mut() {
            if conn.phase == Phase::Open {
                set_state("closing");
                if conn.send(cx::OP_CLOSE, &cx::close_payload(1000, "")).is_ok() {
                    let until = Instant::now() + CLOSE_WAIT;
                    let ssl = conn.ssl();
                    while Instant::now() < until {
                        // SAFETY: an open context and the connection's buffer.
                        let r = unsafe { sys::mbedtls_ssl_read(ssl, conn.buf.as_mut_ptr(), conn.buf.len()) };
                        if r == ERR_WANT_READ {
                            conn.poll(50);
                            continue;
                        }
                        if r <= 0 {
                            break;
                        }
                    }
                }
            }
        }
        self.end(End::new(cause, why));
    }

    fn finish_stream(&self, conn: &Conn, cause: &'static str) {
        if conn.stream.is_some() {
            tele(|t| {
                if let Some(m) = t.m10.as_mut() {
                    m.state = cause;
                    m.ok = Some(false);
                }
            });
        }
    }
}

impl Default for Probe {
    fn default() -> Self {
        Self::new()
    }
}

/// DNS, TCP, and the TLS context ready for its handshake.
fn connect(c: &Cfg, tickets: bool, session: Option<&Session>, floor: Option<i64>, free: u32) -> Result<Box<Conn>, End> {
    let t0 = Instant::now();
    let addr: SocketAddr = if !c.addr.is_empty() {
        SocketAddr::V4(c.addr.parse::<std::net::SocketAddrV4>().map_err(|_| End::new(Cause::Dns, format!("bad address {}", c.addr)))?)
    } else {
        (c.host.as_str(), 443)
            .to_socket_addrs()
            .map_err(|e| End::new(Cause::Dns, format!("{}: {e}", c.host)))?
            .find(SocketAddr::is_ipv4)
            .ok_or_else(|| End::new(Cause::Dns, format!("{}: no IPv4 address", c.host)))?
    };
    let dns_ms = t0.elapsed().as_millis() as u32;
    let t1 = Instant::now();
    let tcp = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| End::new(Cause::Tcp, format!("{}: {e}", addr.ip())))?;
    let tcp_ms = t1.elapsed().as_millis() as u32;
    let _ = tcp.set_nodelay(true);
    let _ = tcp.set_read_timeout(Some(IO_TIMEOUT));
    let _ = tcp.set_write_timeout(Some(IO_TIMEOUT));
    tele(|t| {
        t.dns_ms = dns_ms;
        t.tcp_ms = tcp_ms;
        t.heap_hs_low = free;
    });
    let tls_before = mark_tls_peak();
    let host = CString::new(c.host.as_str()).map_err(|_| End::new(Cause::Dns, "host has a NUL"))?;
    let fd = tcp.as_raw_fd();
    let mut conn = Box::new(Conn {
        tcp,
        // SAFETY: zeroed mbedTLS contexts, _init'ed below before any use.
        tls: unsafe { Box::new_zeroed().assume_init() },
        bio: Box::new(Bio { fd, hs_low: free, rx: 0, tx: 0 }),
        verify: Box::new(Verify { called: false, floor, flags: 0 }),
        phase: Phase::Handshake,
        t0,
        hs_t0: Instant::now(),
        hs_cpu_us: 0,
        resume: session.is_some(),
        tickets,
        tls_before,
        key: String::new(),
        head: Vec::new(),
        dec: Box::new(Decoder::new()),
        buf: Box::new([0; 512]),
        opened: None,
        ping: Ping { seq: 0, sent: None, next: Instant::now(), missed: 0 },
        stream: None,
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
            if let Some(s) = session {
                let rs = sys::mbedtls_ssl_set_session(&mut tls.ssl, &s.0);
                if rs != 0 {
                    log::warn!("connect-probe: saved session refused: {}", tls_error(rs));
                }
            }
        }
        r
    };
    if r != 0 {
        return Err(End::new(Cause::Handshake, format!("setup: {}", tls_error(r))));
    }
    set_state("handshake");
    conn.hs_t0 = Instant::now();
    Ok(conn)
}

/// Handshake steps until done, waiting, or the slice is over.
fn step_handshake(conn: &mut Conn, started: Instant) -> Result<(), End> {
    let ssl = conn.ssl();
    loop {
        let t = now_us();
        // SAFETY: a set-up context.
        let r = unsafe { sys::mbedtls_ssl_handshake(ssl) };
        conn.hs_cpu_us += now_us() - t;
        if r == 0 {
            let ms = conn.hs_t0.elapsed().as_millis() as u32;
            let cpu = (conn.hs_cpu_us / 1000) as u32;
            let peak = TLS_PEAK.load(Relaxed) - conn.tls_before;
            let (full, low) = (conn.verify.called, conn.bio.hs_low);
            tele(|t| {
                if full {
                    t.full.add(ms);
                } else {
                    t.resumed.add(ms);
                }
                t.hs_cpu_ms = cpu;
                t.hs_cpu_max_ms = t.hs_cpu_max_ms.max(cpu);
                t.heap_hs_low = low;
                t.tls_hs_peak = peak;
                t.tls_hs_peak_max = t.tls_hs_peak_max.max(peak);
            });
            conn.phase = Phase::Upgrade;
            set_state("upgrading");
            return Ok(());
        }
        if r != ERR_WANT_READ && r != ERR_WANT_WRITE {
            let flags = conn.verify.flags;
            let extra = if flags != 0 { format!(" (certificate flags 0x{flags:x})") } else { String::new() };
            return Err(End::new(Cause::Handshake, format!("{}{extra}", tls_error(r))));
        }
        let left = SLICE.saturating_sub(started.elapsed());
        if left.is_zero() {
            return Ok(());
        }
        conn.poll((left.as_millis() as u32).min(50));
    }
}

/// The HTTP upgrade: send it, read the answer's head.
fn step_upgrade(conn: &mut Conn, c: &Cfg, started: Instant) -> Result<(), End> {
    if conn.key.is_empty() {
        let mut k = [0u8; 16];
        // SAFETY: sixteen bytes.
        unsafe { sys::esp_fill_random(k.as_mut_ptr() as *mut c_void, 16) };
        conn.key = cx::base64(&k);
        let path = if c.token.is_empty() {
            c.path.clone()
        } else {
            format!("{}{}k={}", c.path, if c.path.contains('?') { '&' } else { '?' }, c.token)
        };
        let req = cx::upgrade_request(&c.host, &path, &conn.key);
        conn.write_all(req.as_bytes())?;
        conn.hs_t0 = Instant::now();
    }
    let ssl = conn.ssl();
    loop {
        // SAFETY: an open context and the connection's buffer.
        let r = unsafe { sys::mbedtls_ssl_read(ssl, conn.buf.as_mut_ptr(), conn.buf.len()) };
        if r > 0 {
            conn.head.extend_from_slice(&conn.buf[..r as usize]);
            if let Some(end) = cx::head_end(&conn.head) {
                cx::check_upgrade(&conn.head[..end], &conn.key).map_err(|e| End::new(Cause::Upgrade, e))?;
                let rest = conn.head.split_off(end);
                conn.head = Vec::new();
                tele(|t| t.upgrade_ms = conn.hs_t0.elapsed().as_millis() as u32);
                conn.phase = Phase::Open;
                if !rest.is_empty() {
                    return feed(conn, &rest, c);
                }
                return Ok(());
            }
            if conn.head.len() > cx::MAX_UPGRADE_HEAD {
                return Err(End::new(Cause::Upgrade, "upgrade answer's head too long"));
            }
            continue;
        }
        if r != ERR_WANT_READ {
            return Err(End::new(Cause::Upgrade, format!("reading the answer: {}", tls_error(r))));
        }
        if conn.hs_t0.elapsed() > UPGRADE_TIMEOUT {
            return Err(End::new(Cause::Upgrade, "no answer to the upgrade"));
        }
        let left = SLICE.saturating_sub(started.elapsed());
        if left.is_zero() {
            return Ok(());
        }
        conn.poll((left.as_millis() as u32).min(50));
    }
}

fn start_stream(conn: &mut Conn, seq: u32, bytes: u64, chunk: u32, window: u32, seed: u32) -> Result<(), End> {
    conn.text(&format!("stream {bytes} {chunk} {window} {seed}"))?;
    let free = free_heap();
    conn.stream = Some(Stream {
        want: bytes,
        window,
        got: 0,
        frames: 0,
        ungranted: 0,
        credits: 0,
        gen: Payload::new(seed),
        fnv: Fnv::default(),
        mismatches: 0,
        t0: Instant::now(),
        heap_low: free,
        largest_at_low: largest_free_block() as u32,
        server: None,
    });
    tele(|t| t.m10 = Some(M10 { seq, state: "running", of: bytes, chunk, window, heap_low: free, ..M10::default() }));
    log::info!("connect-probe: M10 stream of {bytes} B asked ({chunk} B chunks, window {window})");
    Ok(())
}

/// What the server's frames asked of us, acted on after the decoder.
#[derive(Default)]
struct Acts {
    pong: Option<Vec<u8>>,
    pong_seen: Option<u32>,
    close: Option<Option<u16>>,
    stream_end: bool,
}

/// Feed the first `n` bytes of the connection's buffer to its decoder.
fn feed_buf(conn: &mut Conn, n: usize, c: &Cfg) -> Result<(), End> {
    let Conn { dec, stream, ping, buf, .. } = conn;
    let acts = decode(dec, stream, ping, &buf[..n])?;
    act(conn, acts, c)
}

/// Feed bytes from elsewhere (what followed the upgrade's head).
fn feed(conn: &mut Conn, data: &[u8], c: &Cfg) -> Result<(), End> {
    let Conn { dec, stream, ping, .. } = conn;
    let acts = decode(dec, stream, ping, data)?;
    act(conn, acts, c)
}

fn decode(dec: &mut Decoder, stream: &mut Option<Stream>, ping: &mut Ping, data: &[u8]) -> Result<Acts, End> {
    let mut acts = Acts::default();
    let r = dec.feed(data, &mut |e| match e {
        Event::Text(t) => {
            if let Some((b, f)) = cx::parse_end(t) {
                if let Some(s) = stream.as_mut() {
                    s.server = Some((b, f));
                    acts.stream_end = true;
                }
            } else if t == b"pong" {
                acts.pong_seen = Some(ping.seq);
            } else if let Some(n) = t.strip_prefix(b"echo ") {
                acts.pong_seen = std::str::from_utf8(n).ok().and_then(|n| n.parse().ok());
            }
        }
        Event::BinaryStart(_) => {}
        Event::Binary(b) => {
            if let Some(s) = stream.as_mut() {
                s.got += b.len() as u64;
                s.mismatches += s.gen.check(b);
                s.fnv.update(b);
            }
        }
        Event::BinaryEnd { .. } => {
            if let Some(s) = stream.as_mut() {
                s.frames += 1;
                s.ungranted += 1;
            }
        }
        Event::Ping(p) => acts.pong = Some(p.to_vec()),
        Event::Pong(p) => {
            acts.pong_seen = (p.len() == 4).then(|| u32::from_be_bytes([p[0], p[1], p[2], p[3]]));
        }
        Event::Close { code, .. } => acts.close = Some(code),
    });
    if let Err(e) = r {
        return Err(End::new(Cause::Ws, e.as_str()));
    }
    Ok(acts)
}

/// Act on what the frames asked: answer pings and closes, take pongs,
/// grant the stream credit, finish it.
fn act(conn: &mut Conn, acts: Acts, c: &Cfg) -> Result<(), End> {
    if let Some(p) = acts.pong {
        conn.send(cx::OP_PONG, &p)?;
    }
    if let (Some(seq), Some((sent_seq, at))) = (acts.pong_seen, conn.ping.sent) {
        if seq == sent_seq || c.ping == PingKind::Text {
            let ms = at.elapsed().as_millis() as u32;
            conn.ping.sent = None;
            conn.ping.missed = 0;
            tele(|t| {
                t.pongs += 1;
                t.rtt_ms = Some(ms);
                t.rtt_max_ms = t.rtt_max_ms.max(ms);
            });
        }
    }
    if let Some(code) = acts.close {
        // Answer the close (RFC 6455 5.5.1), then end.
        let _ = conn.send(cx::OP_CLOSE, &code.map(|c| c.to_be_bytes().to_vec()).unwrap_or_default());
        return Err(End { cause: Cause::ServerClose, code: code.unwrap_or(1005), why: format!("code {}", code.unwrap_or(1005)) });
    }
    let free = free_heap();
    if let Some(s) = conn.stream.as_mut() {
        if free < s.heap_low {
            s.heap_low = free;
            s.largest_at_low = largest_free_block() as u32;
        }
        let every = (s.window / 2).max(1);
        if s.ungranted >= every && s.got < s.want {
            let n = s.ungranted;
            s.ungranted = 0;
            s.credits += n;
            conn.text(&format!("credit {n}"))?;
        }
    }
    let s = conn.stream.as_ref();
    let snapshot = s.map(|s| (s.got, s.frames, s.mismatches, s.fnv.0, s.heap_low, s.largest_at_low, s.credits, s.t0));
    if let Some((got, frames, mism, fnv, low, largest, credits, t0)) = snapshot {
        tele(|t| {
            if let Some(m) = t.m10.as_mut() {
                m.bytes = got;
                m.frames = frames;
                m.mismatches = mism;
                m.fnv = fnv;
                m.heap_low = low;
                m.largest_at_low = largest;
                m.credits = credits;
                m.ms = t0.elapsed().as_millis() as u32;
            }
        });
    }
    if acts.stream_end {
        if let Some(s) = conn.stream.take() {
            let ok = s.server == Some((s.got, s.fnv.0)) && s.mismatches == 0 && s.got == s.want;
            let ms = s.t0.elapsed().as_millis() as u32;
            log::info!(
                "connect-probe: M10 {} B in {ms} ms, {} frames, {} mismatches, fnv {:08x} (server {:08x}): {}",
                s.got,
                s.frames,
                s.mismatches,
                s.fnv.0,
                s.server.map_or(0, |(_, f)| f),
                if ok { "ok" } else { "BAD" }
            );
            tele(|t| {
                if let Some(m) = t.m10.as_mut() {
                    m.state = "done";
                    m.server = s.server;
                    m.ok = Some(ok);
                    m.ms = ms;
                }
            });
        }
    }
    Ok(())
}

/// An open session's turn: read what came, ping when due.
fn service(conn: &mut Conn, c: &Cfg, started: Instant) -> Result<(), End> {
    let ssl = conn.ssl();
    loop {
        // SAFETY: an open context and the connection's buffer.
        let r = unsafe { sys::mbedtls_ssl_read(ssl, conn.buf.as_mut_ptr(), conn.buf.len()) };
        if r > 0 {
            feed_buf(conn, r as usize, c)?;
            if started.elapsed() >= SLICE {
                break;
            }
            continue;
        }
        if r == 0 {
            return Err(End::new(Cause::Eof, "the connection ended"));
        }
        if r == ERR_PEER_CLOSE_NOTIFY {
            return Err(End::new(Cause::Eof, "TLS close_notify"));
        }
        if r != ERR_WANT_READ && r != ERR_WANT_WRITE {
            return Err(End::new(Cause::Tls, tls_error(r)));
        }
        // Nothing more now. A stream keeps reading for the slice.
        if let Some(s) = conn.stream.as_ref() {
            if s.t0.elapsed() > STREAM_TIMEOUT {
                conn.stream = None;
                tele(|t| {
                    if let Some(m) = t.m10.as_mut() {
                        m.state = "timed out";
                        m.ok = Some(false);
                    }
                });
                return Ok(());
            }
            let left = SLICE.saturating_sub(started.elapsed());
            if !left.is_zero() {
                conn.poll((left.as_millis() as u32).min(50));
                continue;
            }
        }
        break;
    }
    if Instant::now() >= conn.ping.next {
        if conn.ping.sent.is_some() {
            conn.ping.missed += 1;
            tele(|t| t.missed += 1);
            if conn.ping.missed >= 2 {
                return Err(End::new(Cause::PongTimeout, "two pings unanswered"));
            }
        }
        conn.ping.seq = conn.ping.seq.wrapping_add(1);
        let seq = conn.ping.seq;
        match c.ping {
            PingKind::Ws => conn.send(cx::OP_PING, &seq.to_be_bytes())?,
            PingKind::Text => conn.text("ping")?,
            PingKind::Echo => conn.text(&format!("echo {seq}"))?,
        }
        conn.ping.sent = Some((seq, Instant::now()));
        conn.ping.next = Instant::now() + Duration::from_secs(u64::from(c.ping_s.max(1)));
        tele(|t| t.pings += 1);
        // The pong, right here if it comes soon.
        if conn.poll(PONG_WAIT.as_millis() as u32) {
            return service_once(conn, c);
        }
    }
    Ok(())
}

/// Read what is there now (after a ping).
fn service_once(conn: &mut Conn, c: &Cfg) -> Result<(), End> {
    let ssl = conn.ssl();
    loop {
        // SAFETY: as in `service`.
        let r = unsafe { sys::mbedtls_ssl_read(ssl, conn.buf.as_mut_ptr(), conn.buf.len()) };
        if r > 0 {
            feed_buf(conn, r as usize, c)?;
            continue;
        }
        if r == 0 || r == ERR_PEER_CLOSE_NOTIFY {
            return Err(End::new(Cause::Eof, "the connection ended"));
        }
        if r != ERR_WANT_READ && r != ERR_WANT_WRITE {
            return Err(End::new(Cause::Tls, tls_error(r)));
        }
        return Ok(());
    }
}

// --- The debug API ---------------------------------------------------------------------

fn num<T: std::str::FromStr>(q: Option<&str>, key: &str, default: T) -> Result<T, String> {
    match query_param(q, key) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|_| format!("{key}: not a number")),
    }
}

fn ranged(v: u64, lo: u64, hi: u64, key: &str) -> Result<u64, String> {
    if (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(format!("{key}: {lo}..{hi}"))
    }
}

/// `POST /api/debug/connect/<what>`'s work; the JSON to answer.
fn post(what: &str, q: Option<&str>) -> Result<String, String> {
    match what {
        "config" => {
            let mut c = cfg();
            if let Some(v) = query_param(q, "host") {
                c.host = v;
            }
            if let Some(v) = query_param(q, "addr") {
                c.addr = v;
            }
            if let Some(v) = query_param(q, "path") {
                c.path = v;
            }
            if let Some(v) = query_param(q, "token") {
                c.token = v;
            }
            c.ping_s = ranged(num(q, "ping_s", u64::from(c.ping_s))?, 1, 3600, "ping_s")? as u32;
            if let Some(v) = query_param(q, "ping") {
                c.ping = match v.as_str() {
                    "ws" => PingKind::Ws,
                    "text" => PingKind::Text,
                    "echo" => PingKind::Echo,
                    _ => return Err("ping: ws, text or echo".into()),
                };
            }
            c.tickets = num(q, "tickets", u8::from(c.tickets))? != 0;
            c.enabled = num(q, "enabled", u8::from(c.enabled))? != 0;
            c.gate_free = ranged(num(q, "gate_free", u64::from(c.gate_free))?, 0, 200_000, "gate_free")? as u32;
            c.gate_largest = ranged(num(q, "gate_largest", u64::from(c.gate_largest))?, 0, 100_000, "gate_largest")? as u32;
            let json = c.json();
            *CFG.lock().unwrap_or_else(|p| p.into_inner()) = c;
            CFG_GEN.fetch_add(1, Relaxed);
            Ok(format!(r#"{{"ok":true,"cfg":{json}}}"#))
        }
        "handshakes" => {
            let n = ranged(num(q, "n", 200)?, 1, 10_000, "n")? as u32;
            let ms = ranged(num(q, "interval_ms", num(q, "interval_s", 10u64)? * 1000)?, 200, 3_600_000, "interval_ms")?;
            let resume = num(q, "resume", 0u8)? != 0;
            let seq = ask(Ask::Handshakes { n, interval: Duration::from_millis(ms), resume });
            Ok(format!(r#"{{"ok":true,"ask":{seq},"n":{n},"interval_ms":{ms},"resume":{resume}}}"#))
        }
        "ed25519" => {
            let n = ranged(num(q, "n", 100)?, 1, 10_000, "n")? as u32;
            let seq = ask(Ask::Ed25519 { n });
            Ok(format!(r#"{{"ok":true,"ask":{seq},"n":{n}}}"#))
        }
        "stream" => {
            let bytes = ranged(num(q, "bytes", 524_288)?, 1, 16 << 20, "bytes")?;
            let chunk = ranged(num(q, "chunk", 1000)?, 1, 16_384, "chunk")? as u32;
            let window = ranged(num(q, "window", 4)?, 1, 64, "window")? as u32;
            let seed = num(q, "seed", 1u32)?;
            // What may be in flight must fit one record of TLS_IN even if
            // the edge puts it all in one (each frame's header up to 4 B).
            if u64::from(window) * u64::from(chunk + 4) > u64::from(TLS_IN) {
                return Err(format!("window x (chunk + 4) must be at most {TLS_IN} (this build's TLS input record)"));
            }
            let seq = ask(Ask::Stream { bytes, chunk, window, seed });
            Ok(format!(r#"{{"ok":true,"ask":{seq},"bytes":{bytes},"chunk":{chunk},"window":{window},"seed":{seed}}}"#))
        }
        "stop" => {
            let seq = ask(Ask::Stop);
            Ok(format!(r#"{{"ok":true,"ask":{seq}}}"#))
        }
        "reconnect" => {
            let seq = ask(Ask::Reconnect);
            Ok(format!(r#"{{"ok":true,"ask":{seq}}}"#))
        }
        "reset" => {
            let seq = ask(Ask::Reset);
            Ok(format!(r#"{{"ok":true,"ask":{seq}}}"#))
        }
        _ => Err(format!("{what}: config, handshakes, ed25519, stream, stop, reconnect or reset")),
    }
}

/// The test hooks, registered before the catch-all (`http::start`).
pub fn routes(server: &mut Server) -> anyhow::Result<()> {
    server.route("/api/debug/connect", Method::Get, |req| {
        req.json(&format!(r#"{{"connect":{},"cfg":{}}}"#, json(), cfg().json()))
    })?;
    server.route("/api/debug/connect/*", Method::Post, |req| {
        let what = req.path().trim_start_matches("/api/debug/connect/").to_string();
        match post(&what, req.query()) {
            Ok(body) => req.json(&body),
            Err(e) => req.bad_request(&e),
        }
    })?;
    Ok(())
}
