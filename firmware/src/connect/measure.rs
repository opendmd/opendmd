//! The device tests' measuring half (feature `connect-measure`, never in a
//! shipping build: what is left of spike S0's after Connect P1's last
//! branch, kept for device tests D1, D3 and D5): the `connect` telemetry in
//! `/api/status`, mbedTLS's byte counts, and the test hooks
//! `GET /api/debug/connect`, `POST /api/debug/connect/<what>` the scripts in
//! `tools/connect-device/` drive: `config` (pings, tickets, the heap gates;
//! never the server), `handshakes` (full or resumed, at an interval),
//! `stop`, `reconnect`, `reset`.
//!
//! Without the feature only the [`Tele`] type is compiled (its closures
//! are dead code) and [`tele`] runs nothing.

use super::CAUSES;
use opendmd_core::connect::Hist;
#[cfg(feature = "connect-measure")]
use {
    super::tls::{DNS_STACK_LOW, TLS_IN, TLS_IN_USE, TLS_OUT},
    crate::httpd::{json_string, query_param, Method, Respond, Server},
    std::sync::atomic::{AtomicU32, Ordering::Relaxed},
    std::sync::{LazyLock, Mutex},
    std::time::Duration,
};

// --- Run-time settings ----------------------------------------------------

/// What the client runs with. Fixed in a shipping build
/// ([`Cfg::defaults`]); a measuring build's `POST /api/debug/connect/config`
/// changes it (RAM only), but never the server: always the build's own.
#[derive(Clone)]
pub struct Cfg {
    pub host: &'static str,
    pub addr: &'static str,
    pub ping_s: u32,
    pub tickets: bool,
    /// The heap a full handshake needs free (P1-PLAN 4.4), a resumed one,
    /// the largest block for either, and PSRAM's largest block (0: not
    /// gated). Internal RAM throughout; `opendmd_core::connect::gate`.
    pub gate_full: u32,
    pub gate_resumed: u32,
    pub gate_largest: u32,
    pub gate_psram_largest: u32,
    /// The LAN quiet a handshake waits for.
    pub quiet_s: u32,
}

impl Cfg {
    #[optimize(size)]
    #[inline(never)]
    pub fn defaults() -> Cfg {
        use opendmd_core::connect::gate;
        // The S3 has three times the room: its gates are open in practice,
        // unless mbedTLS allocates from PSRAM (`psram-tls`), when the
        // internal side needs only the classic's and PSRAM is gated too.
        #[cfg(all(esp32s3, feature = "psram-tls"))]
        let g = gate::S3_PSRAM_TLS;
        #[cfg(all(esp32s3, not(feature = "psram-tls")))]
        let g = gate::S3_INTERNAL_TLS;
        #[cfg(not(esp32s3))]
        let g = gate::CLASSIC;
        Cfg {
            host: super::HOST,
            addr: super::ADDR,
            ping_s: 90,
            tickets: true,
            gate_full: g.full,
            gate_resumed: g.resumed,
            gate_largest: g.largest,
            gate_psram_largest: g.psram_largest,
            quiet_s: 10,
        }
    }

    #[cfg(feature = "connect-measure")]
    #[optimize(size)]
    #[inline(never)]
    fn json(&self) -> String {
        format!(
            r#"{{"host":{},"addr":{},"ping_s":{},"tickets":{},"gate_full":{},"gate_resumed":{},"gate_largest":{},"gate_psram_largest":{},"quiet_s":{},"tls_in":{TLS_IN},"tls_out":{TLS_OUT}}}"#,
            json_string(self.host),
            json_string(self.addr),
            self.ping_s,
            self.tickets,
            self.gate_full,
            self.gate_resumed,
            self.gate_largest,
            self.gate_psram_largest,
            self.quiet_s,
        )
    }
}

#[cfg(feature = "connect-measure")]
static CFG: LazyLock<Mutex<Cfg>> = LazyLock::new(|| Mutex::new(Cfg::defaults()));
/// Bumped by every config change: the client reconnects to apply it.
#[cfg(feature = "connect-measure")]
pub static CFG_GEN: AtomicU32 = AtomicU32::new(0);

#[optimize(size)]
#[inline(never)]
pub fn cfg() -> Cfg {
    #[cfg(feature = "connect-measure")]
    return CFG.lock().unwrap_or_else(|p| p.into_inner()).clone();
    #[cfg(not(feature = "connect-measure"))]
    Cfg::defaults()
}

#[optimize(size)]
#[inline(never)]
pub fn cfg_gen() -> u32 {
    #[cfg(feature = "connect-measure")]
    return CFG_GEN.load(Relaxed);
    #[cfg(not(feature = "connect-measure"))]
    0
}

// --- What the debug API asks -------------------------------------------------

#[derive(Clone, Copy)]
pub enum Ask {
    /// Full handshakes (`resume` false: tickets off; D5, under LAN load)
    /// or resumed ones (`resume` true: from the last session; D3).
    #[cfg_attr(not(feature = "connect-measure"), allow(dead_code))]
    Handshakes { n: u32, interval: std::time::Duration, resume: bool },
    Stop,
    Reconnect,
    Reset,
}

impl Ask {
    #[optimize(size)]
    #[inline(never)]
    pub fn name(&self) -> &'static str {
        match self {
            Ask::Handshakes { resume: false, .. } => "full handshakes",
            Ask::Handshakes { resume: true, .. } => "resumed handshakes",
            Ask::Stop => "stop",
            Ask::Reconnect => "reconnect",
            Ask::Reset => "reset",
        }
    }
}

/// The waiting requests and their numbers (each POST's `ask`; a job
/// reports the number that started it as `seq`). Up to [`ASK_MAX`] wait,
/// taken one a turn, in order; one more is a 409.
#[cfg(feature = "connect-measure")]
static ASK: Mutex<std::collections::VecDeque<(u32, Ask)>> = Mutex::new(std::collections::VecDeque::new());
#[cfg(feature = "connect-measure")]
static ASK_SEQ: AtomicU32 = AtomicU32::new(0);
/// The last request the client took (`connect.ask_taken`).
#[cfg(feature = "connect-measure")]
static ASK_TAKEN: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "connect-measure")]
const ASK_MAX: usize = 4;
#[cfg(feature = "connect-measure")]
const BUSY: &str = "busy: requests still waiting for the client; try again in a second";

#[cfg(feature = "connect-measure")]
#[optimize(size)]
#[inline(never)]
fn ask(a: Ask) -> Result<u32, String> {
    let mut q = ASK.lock().unwrap_or_else(|p| p.into_inner());
    if q.len() >= ASK_MAX {
        return Err(BUSY.into());
    }
    let seq = ASK_SEQ.fetch_add(1, Relaxed) + 1;
    q.push_back((seq, a));
    Ok(seq)
}

/// The next request, if one waits (the client's turn).
#[cfg(feature = "connect-measure")]
#[optimize(size)]
#[inline(never)]
pub fn take_ask() -> Option<(u32, Ask)> {
    let a = ASK.lock().unwrap_or_else(|p| p.into_inner()).pop_front();
    if let Some((seq, _)) = a {
        ASK_TAKEN.store(seq, Relaxed);
    }
    a
}

// --- Telemetry -------------------------------------------------------------------

pub struct Tele {
    pub state: &'static str,
    pub since_s: u32,
    pub clock: &'static str,
    pub attempts: u32,
    pub opened: u32,
    pub full: Hist,
    pub resumed: Hist,
    /// Resumption asked for, a full handshake given.
    pub resume_missed: u32,
    pub dns_ms: u32,
    pub tcp_ms: u32,
    pub upgrade_ms: u32,
    /// From the lookup to the upgrade's answer, the last attempt that opened.
    pub connect_ms: u32,
    /// The last handshake's time inside mbedTLS's calls.
    pub hs_cpu_ms: u32,
    pub hs_cpu_max_ms: u32,
    /// Sessions that ended (not the forced ones), by cause; attempts that
    /// failed, by stage.
    pub reconnects: u32,
    pub causes: [u32; CAUSES.len()],
    pub gate_waits: u32,
    pub quiet_waits: u32,
    pub backoff_s: u32,
    /// The last sessions: (seconds open, cause, close code).
    pub sessions: [(u32, super::Cause, u16); 8],
    pub sessions_n: usize,
    pub session_max_s: u32,
    pub heap_at_connect: u32,
    pub heap_hs_low: u32,
    pub heap_low: u32,
    pub heap_low_largest: u32,
    pub heap_open: u32,
    /// mbedTLS's own bytes: the most over the last handshake, and held once
    /// open, above what it held before the attempt.
    pub tls_hs_peak: i32,
    pub tls_hs_peak_max: i32,
    pub tls_open: i32,
    /// The Rust side of a session (its boxes).
    pub rust_bytes: u32,
    /// The network worker's stack never used, bytes (its whole life).
    pub stack_unused: u32,
    pub pings: u32,
    pub pongs: u32,
    pub missed: u32,
    pub rtt_ms: Option<u32>,
    pub rtt_max_ms: u32,
    pub rx: u64,
    pub tx: u64,
    pub root: &'static str,
    pub cert_flags: u32,
    pub job: &'static str,
    pub job_seq: u32,
    pub job_done: u32,
    pub job_of: u32,
    pub job_failed: u32,
    /// Signed commands: checked, accepted, refused; the slowest check.
    pub commands: u32,
    pub accepted: u32,
    pub refused: u32,
    pub check_max_us: u32,
    pub last_error: Option<String>,
    pub last_error_s: u32,
    pub errors: u32,
    pub crash_boot: bool,
}

impl Tele {
    #[optimize(size)]
    #[inline(never)]
    pub fn new() -> Tele {
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
            quiet_waits: 0,
            backoff_s: 0,
            sessions: [(0, super::Cause::Forced, 0); 8],
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
            commands: 0,
            accepted: 0,
            refused: 0,
            check_max_us: 0,
            last_error: None,
            last_error_s: 0,
            errors: 0,
            crash_boot: false,
        }
    }
}

#[cfg(feature = "connect-measure")]
static TELE: LazyLock<Mutex<Tele>> = LazyLock::new(|| Mutex::new(Tele::new()));

/// Run `f` on the telemetry (a `connect-measure` build); without it,
/// nothing runs and the default comes back.
#[cfg(feature = "connect-measure")]
#[optimize(size)]
#[inline(never)]
pub fn tele<R: Default>(f: impl FnOnce(&mut Tele) -> R) -> R {
    f(&mut TELE.lock().unwrap_or_else(|p| p.into_inner()))
}

#[cfg(not(feature = "connect-measure"))]
#[inline(always)]
pub fn tele<R: Default>(_: impl FnOnce(&mut Tele) -> R) -> R {
    R::default()
}

/// `Ask::Reset`: the counters from zero, the state kept.
#[cfg(feature = "connect-measure")]
#[optimize(size)]
#[inline(never)]
pub fn reset() {
    tele(|t| {
        let keep = (t.state, t.since_s, t.clock, t.stack_unused, t.rust_bytes, t.crash_boot);
        *t = Tele::new();
        (t.state, t.since_s, t.clock, t.stack_unused, t.rust_bytes, t.crash_boot) = keep;
    });
}

#[cfg(feature = "connect-measure")]
#[optimize(size)]
#[inline(never)]
fn opt<T: ToString>(v: Option<T>) -> String {
    v.map_or_else(|| "null".to_string(), |v| v.to_string())
}

/// The S3's PSRAM side of `connect.heap`. Empty on the classic.
#[cfg(feature = "connect-measure")]
#[optimize(size)]
#[inline(never)]
fn psram_json() -> String {
    #[cfg(esp32s3)]
    {
        use esp_idf_sys as sys;
        // SAFETY: read-only queries with a capability mask.
        let (free, low, largest) = unsafe {
            (
                sys::heap_caps_get_free_size(sys::MALLOC_CAP_SPIRAM),
                sys::heap_caps_get_minimum_free_size(sys::MALLOC_CAP_SPIRAM),
                sys::heap_caps_get_largest_free_block(sys::MALLOC_CAP_SPIRAM),
            )
        };
        #[cfg(feature = "psram-tls")]
        let tls = format!(
            r#","tls_psram":{},"tls_fell_back":{}"#,
            super::tls::TLS_PSRAM.load(Relaxed),
            super::tls::TLS_FELL_BACK.load(Relaxed)
        );
        #[cfg(not(feature = "psram-tls"))]
        let tls = r#","tls_psram":null"#;
        format!(r#","internal":true,"psram":{{"free":{free},"largest":{largest},"low":{low}{tls}}}"#)
    }
    #[cfg(not(esp32s3))]
    String::new()
}

/// `/api/status` `connect` in a measuring build: everything measured, and
/// the shipping build's summary fields (`super::lite_fields`) after it.
#[cfg(feature = "connect-measure")]
#[optimize(size)]
#[inline(never)]
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
    let (on, registered) = super::shared(|s| (s.on, s.identity.is_some())).unwrap_or((false, false));
    format!(
        concat!(
            r#"{{"enabled":{},"registered":{},"crash_boot":{},"ask_taken":{},"dns_stack_unused":{},"host":{},"tickets":{},"ping_s":{},"tls_in":{},"tls_out":{},"clock":"{}","#,
            r#""attempts":{},"full":{},"resumed_hist":{},"resume_missed":{},"#,
            r#""dns_ms":{},"tcp_ms":{},"upgrade_ms":{},"connect_ms":{},"hs_cpu_ms":{},"hs_cpu_max_ms":{},"#,
            r#""causes":{{{}}},"gate_waits":{},"quiet_waits":{},"backoff_s":{},"sessions":[{}],"session_max_s":{},"#,
            r#""heap":{{"at_connect":{},"hs_low":{},"low":{},"low_largest":{},"open":{},"tls_hs_peak":{},"tls_hs_peak_max":{},"tls_open":{},"tls_in_use":{},"rust":{}{}}},"#,
            r#""stack_unused":{},"ping":{{"sent":{},"pongs":{},"missed":{},"rtt_ms":{},"rtt_max_ms":{}}},"rx":{},"tx":{},"root":{},"cert_flags":{},"#,
            r#""job":{{"kind":"{}","seq":{},"done":{},"of":{},"failed":{}}},"commands":{{"n":{},"accepted":{},"refused":{},"check_max_us":{}}},"errors":{},{}}}"#
        ),
        on,
        registered,
        t.crash_boot,
        ASK_TAKEN.load(Relaxed),
        match DNS_STACK_LOW.load(Relaxed) {
            u32::MAX => "null".to_string(),
            v => v.to_string(),
        },
        json_string(c.host),
        c.tickets,
        c.ping_s,
        TLS_IN,
        TLS_OUT,
        t.clock,
        t.attempts,
        t.full.json(),
        t.resumed.json(),
        t.resume_missed,
        t.dns_ms,
        t.tcp_ms,
        t.upgrade_ms,
        t.connect_ms,
        t.hs_cpu_ms,
        t.hs_cpu_max_ms,
        causes.join(","),
        t.gate_waits,
        t.quiet_waits,
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
        psram_json(),
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
        t.commands,
        t.accepted,
        t.refused,
        t.check_max_us,
        t.errors,
        super::lite_fields(),
    )
}

// --- The debug API ---------------------------------------------------------------------

#[cfg(feature = "connect-measure")]
#[optimize(size)]
#[inline(never)]
fn num<T: std::str::FromStr>(q: Option<&str>, key: &str, default: T) -> Result<T, String> {
    match query_param(q, key) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|_| format!("{key}: not a number")),
    }
}

#[cfg(feature = "connect-measure")]
#[optimize(size)]
#[inline(never)]
fn ranged(v: u64, lo: u64, hi: u64, key: &str) -> Result<u64, String> {
    if (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(format!("{key}: {lo}..{hi}"))
    }
}

/// `POST /api/debug/connect/<what>`'s work; the JSON to answer.
#[cfg(feature = "connect-measure")]
#[optimize(size)]
#[inline(never)]
fn post(what: &str, q: Option<&str>) -> Result<String, String> {
    let queued = |a| ask(a).map(|seq| format!(r#"{{"ok":true,"ask":{seq}}}"#));
    match what {
        "config" => {
            let mut c = cfg();
            if query_param(q, "host").is_some() || query_param(q, "addr").is_some() {
                return Err("host, addr: a measuring build talks to its own server only".into());
            }
            c.ping_s = ranged(num(q, "ping_s", u64::from(c.ping_s))?, 1, 3600, "ping_s")? as u32;
            c.tickets = num(q, "tickets", u8::from(c.tickets))? != 0;
            c.gate_full = ranged(num(q, "gate_full", u64::from(c.gate_full))?, 0, 400_000, "gate_full")? as u32;
            c.gate_resumed = ranged(num(q, "gate_resumed", u64::from(c.gate_resumed))?, 0, 400_000, "gate_resumed")? as u32;
            c.gate_largest = ranged(num(q, "gate_largest", u64::from(c.gate_largest))?, 0, 200_000, "gate_largest")? as u32;
            c.gate_psram_largest = ranged(num(q, "gate_psram_largest", u64::from(c.gate_psram_largest))?, 0, 8_000_000, "gate_psram_largest")? as u32;
            c.quiet_s = ranged(num(q, "quiet_s", u64::from(c.quiet_s))?, 0, 600, "quiet_s")? as u32;
            let json = c.json();
            *CFG.lock().unwrap_or_else(|p| p.into_inner()) = c;
            CFG_GEN.fetch_add(1, Relaxed);
            Ok(format!(r#"{{"ok":true,"cfg":{json}}}"#))
        }
        "handshakes" => {
            let n = ranged(num(q, "n", 200)?, 1, 10_000, "n")? as u32;
            let ms = ranged(num(q, "interval_ms", num(q, "interval_s", 10u64)? * 1000)?, 200, 3_600_000, "interval_ms")?;
            let resume = num(q, "resume", 0u8)? != 0;
            let seq = ask(Ask::Handshakes { n, interval: Duration::from_millis(ms), resume })?;
            Ok(format!(r#"{{"ok":true,"ask":{seq},"n":{n},"interval_ms":{ms},"resume":{resume}}}"#))
        }
        "stop" => queued(Ask::Stop),
        "reconnect" => queued(Ask::Reconnect),
        "reset" => queued(Ask::Reset),
        _ => Err(format!("{what}: config, handshakes, stop, reconnect or reset")),
    }
}

/// The test hooks, registered before the catch-all (`http::start`).
#[cfg(feature = "connect-measure")]
#[optimize(size)]
#[inline(never)]
pub fn routes(server: &mut Server) -> anyhow::Result<()> {
    server.route("/api/debug/connect", Method::Get, |req| {
        req.json(&format!(r#"{{"connect":{},"cfg":{}}}"#, json(), cfg().json()))
    })?;
    server.route("/api/debug/connect/*", Method::Post, |req| {
        let what = req.path().trim_start_matches("/api/debug/connect/").to_string();
        match post(&what, req.query()) {
            Ok(body) => req.json(&body),
            Err(e) if e == BUSY => req.error(409, "409 Conflict", &e),
            Err(e) => req.bad_request(&e),
        }
    })?;
    Ok(())
}
