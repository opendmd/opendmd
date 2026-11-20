//! The session: when to connect, registration, the upgrade, `hello`, what
//! the server's frames ask, keepalive, status, and how a session ends
//! (P1-PLAN 4.4's session policy, from spike S0's measurements).
//!
//! | Topic | Rule |
//! |---|---|
//! | Full handshake | only with 48 KB free and a 16 KB block ([`Cfg`], `opendmd_core::connect::gate`), and 10 s without a LAN upload, stream or heavy request (`super::lan_busy`). Otherwise wait: "waiting for memory" (`waiting` names the unmet condition), "waiting for quiet" |
//! | Resumed handshake (a TLS ticket held) | 44 KB until it is measured (D3); the same quiet |
//! | S3 | 100 KB: open in practice. With mbedTLS in PSRAM (`psram-tls`): the classic's internal free gates, an 8 KB internal block, and a 64 KB PSRAM block |
//! | Looking up the server | on its own task, one at a time, waited for 25 s ("looking up the server"; `opendmd_core::connect::dns`). Unanswered by then: one failure (its late error is not another; a late address is used next time). Still running at the next attempt: reported as stuck, one failure. DNS failures back off no further than 30 s |
//! | After a server close | 0-30 s spread, then 5 s doubling to 15 min, with jitter (`Backoff`) |
//! | Refused (4403, or HTTP 403 at the upgrade) | straight to the 15 min cap; `refused` (Settings: set up again) |
//! | The id unknown (HTTP 404 at the upgrade) | with no controllers (the server drops a registration that never said `hello` within 10 min): register again, at most 3 a day, on the usual backoff. With controllers: refused, as 403, nothing erased (Forget on the LAN starts again) |
//! | Too old (4426, or 426 at registration) | `too_old`, again a day later. Too new (4406) is transient: the usual backoff |
//! | An upgrade answered without `odc1` | a failed attempt: the usual backoff (never "update needed"). A close right after an upgrade that named it is judged by its code, as above |
//! | Stay away (4429, or 429) | the minutes it says (1 to a day; 15 if it says none) |
//! | Too much (1009: a frame over the server's size; 4429 "1": over its text budget) | the usual backoff, and the panel keeps under the budgets itself: text ≤ 2,048 B (`Conn::text`), `keys`/`info` at most one a second after `hello`'s two, status one every 2 s, acks no faster than the commands' 4 a second per key, 8 per panel |
//! | After boot | [`HOLD_S`] (180 s). After a crash boot 15 min; after two crash boots in a row off until a clean boot or a switch-on in Settings (a brownout boot is neither a crash nor clean: power, not firmware) |
//! | Switched on in Settings | at once: no hold, no backoff |
//! | Clock | SNTP first; after 60 s without it, the build date as the floor |
//! | An update under way | closed, held until the restart (`super::updating`) |
//! | No WiFi, setup mode, `network` off | nothing (the worker does not run this) |
//!
//! Close codes are acted on only as a close frame after a completed
//! upgrade; `hello` goes in the same turn as the upgrade's answer (the
//! server drops a socket that says nothing for 3 s).
//!
//! Keepalive: a WebSocket ping every 90 s (the server's runtime answers
//! it without waking anything); two in a row unanswered end the session.
//! Status goes only while the server says a controller watches: on change
//! at most one every 2 s, and one every 30 s regardless.
//!
//! Pairing ([`Client::pairing`], after the turn, at the worker's top
//! level: the MACs hash and the key store waits on the main task, neither
//! of which may sit under the session's read path, where the stack is
//! tightest): a guest's `pair` is queued by the read path (at most
//! [`PAIR_QUEUE`]) and checked here against the open window; the key is
//! stored, `keys` sent, `paired` answered, and `pair-close` said. A window
//! that dies any other way (expired, three wrong MACs, cancelled or
//! replaced on the LAN, switched off) is closed at the server the same
//! turn; one announced before a reconnect that died meanwhile, at the next
//! session's start. `pair-open` counts against the server's budget for
//! `keys` and `info` (1 a second, a burst of 4).

use super::dispatch::{Link, Pending};
use super::measure::{cfg, cfg_gen, Cfg};
use super::tls::{self, now_us, rand_u32, random, Conn, Phase, Purpose, Ticket, BIN_KEEP, BLOCK_MAX, DNS, ERR_PEER_CLOSE_NOTIFY, ERR_WANT_READ, ERR_WANT_WRITE, ROOTS, ROOT_USED, SLICE, TLS_PEAK};
use super::{error, lite, set_state, shared, tele, Cause, End, Store};
use crate::state::{free_heap, largest_free_block};
use opendmd_core::connect::gate;
#[cfg(any(feature = "connect-measure", feature = "connect-test"))]
use esp_idf_sys as sys;
use opendmd_core::connect::keys::{KeyId, KeyList};
use opendmd_core::connect::qr::Qr;
use opendmd_core::connect::proto::{self, Caps, Dev, ServerMsg, Session};
use opendmd_core::connect::{self as cx, dns, Backoff, Event, BACKOFF_CAP};
use std::net::SocketAddr;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

/// No attempt of its own before this much uptime (seconds;
/// `OPENDMD_CONNECT_HOLD_S` at build time, the emulator lane's shorter).
#[cfg(feature = "connect-test")]
pub const HOLD_S: u64 = match option_env!("OPENDMD_CONNECT_HOLD_S") {
    Some(v) => parse_u64(v),
    None => 180,
};
#[cfg(not(feature = "connect-test"))]
pub const HOLD_S: u64 = 180;
/// After a crash boot.
const CRASH_HOLD_S: u64 = 15 * 60;

/// A build-time number (`OPENDMD_CONNECT_HOLD_S`, `_PAIR_TTL_S`).
#[cfg(feature = "connect-test")]
pub(super) const fn parse_u64(v: &str) -> u64 {
    let b = v.as_bytes();
    let mut n = 0u64;
    let mut k = 0;
    while k < b.len() {
        assert!(b[k].is_ascii_digit(), "OPENDMD_CONNECT_*_S: digits only");
        n = n * 10 + (b[k] - b'0') as u64;
        k += 1;
    }
    n
}

/// The build's date (Unix seconds, `build.rs`): the clock floor.
const BUILD_UNIX: &str = env!("OPENDMD_BUILD_UNIX");
/// How long a looked-up address is used.
const DNS_KEEP: Duration = Duration::from_secs(3600);
/// The state a handshake waits in for the LAN to be quiet.
const WAITING_FOR_QUIET: &str = "waiting for quiet";
/// While a lookup runs, the worker's pace (not the busy 5 ms).
const DNS_POLL: Duration = Duration::from_millis(200);
const UPGRADE_TIMEOUT: Duration = Duration::from_secs(10);
const CLOCK_WAIT: Duration = Duration::from_secs(60);
/// How long a close waits for the server's own close (so the server's end
/// holds the TIME_WAIT pcb, not ours: CLAUDE.md's "close second").
const CLOSE_WAIT: Duration = Duration::from_millis(1500);
/// A session that ran this long counts as having run (backoff forgotten).
const RAN: Duration = Duration::from_secs(60);
/// After a ping, wait this long right there for its pong.
const PONG_WAIT: Duration = Duration::from_millis(300);
/// Status while watched: at most this often on change, and at least this
/// often anyway.
#[cfg(esp32s3)]
const STATUS_MIN: Duration = Duration::from_secs(1);
#[cfg(not(esp32s3))]
const STATUS_MIN: Duration = Duration::from_secs(2);
const STATUS_EVERY: Duration = Duration::from_secs(30);
/// A server that says the panel is too old: asked again a day later.
const TOO_OLD_WAIT: Duration = Duration::from_secs(24 * 3600);
/// Registrations made again for a dropped id, a day, before the panel
/// says refused instead.
const REREGISTER_MAX: u32 = 3;
/// The register answer's longest, head and body.
const REGISTER_MAX: usize = 2048;
/// Guests' `pair` messages held for the next top-level turn (the server
/// lets two guests in at a time); more meanwhile are answered `busy`.
pub(super) const PAIR_QUEUE: usize = 2;

/// A guest's `pair`, as read: its slot, key, label (as sent) and MAC.
pub(super) struct PairReq {
    /// The window it came for (a `pair` for another one is `closed`).
    pub seq: Option<u32>,
    pub g: u8,
    pub public: [u8; 32],
    pub label: String,
    pub mac: [u8; 32],
}

/// What the client last asked the main task to show.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Shown {
    None,
    /// The QR code of window `seq`.
    Qr(u32),
    Added,
}

#[optimize(size)]
#[inline(never)]
fn uptime_s() -> u64 {
    now_us() / 1_000_000
}

/// The core's clock: a `Duration` since boot (it never reads one itself).
#[optimize(size)]
#[inline(never)]
pub(super) fn core_now() -> Duration {
    Duration::from_micros(now_us())
}

/// What the client is doing besides holding the session (a measuring
/// build's handshake jobs).
#[derive(Clone, Copy, Debug)]
enum Job {
    None,
    #[cfg(feature = "connect-measure")]
    Handshakes { n: u32, done: u32, failed: u32, interval: Duration, resume: bool, next: Instant },
}

/// What the server's frames asked, acted on after the decoder.
#[derive(Default)]
struct Acts {
    pong: Option<Vec<u8>>,
    pong_seen: Option<u32>,
    close: Option<(Option<u16>, Vec<u8>)>,
    texts: Vec<ServerMsg>,
    frames: Vec<Vec<u8>>,
}

/// Owned by the network worker's loop (`net_worker::run`).
pub struct Client {
    pub(super) conn: Option<Box<Conn>>,
    backoff: Backoff,
    /// No attempt before this.
    not_before: Instant,
    ticket: Option<Box<Ticket>>,
    job: Job,
    online_since: Option<Instant>,
    cfg_gen: u32,
    floor: Option<i64>,
    /// With no connection: the next attempt waits for a time (backoff, a
    /// job's interval, the memory gate), not for the network or the clock.
    timed: bool,
    /// The host's address and when it was looked up.
    resolved: Option<(String, SocketAddr, Instant)>,
    /// The server the saved ticket and the address are for.
    target: (String, String),
    /// No attempt before this uptime (the boot's or a crash's hold).
    hold_s: Option<u64>,
    /// Two crash boots in a row: off until a switch-on in Settings.
    crash_off: bool,
    /// When the LAN was last seen busy.
    quiet_since: Instant,
    /// The secret of a registration under way.
    reg_secret: Option<[u8; 32]>,
    /// The shared state's copy the client works from.
    pub(super) identity: Option<(Dev, [u8; 32])>,
    pub(super) keys: KeyList,
    name: String,
    need_keys: bool,
    need_info: bool,
    /// The shared state was read once (the first turn takes all of it).
    synced: bool,
    /// A forget asked on the LAN, not yet acted on.
    forget: bool,
    /// Registrations made again for a dropped id: how many since `at`
    /// (at most [`REREGISTER_MAX`] a day, then refused).
    reregs: (u32, Instant),
    /// The server no longer knows this id (HTTP 404 at the upgrade) and no
    /// controller does either (a registration that never said `hello`,
    /// deleted): register anew.
    reregister: bool,
    /// `keys`, `info` and `pair-open` against the server's budget for them
    /// (1 a second, a burst of 4): a bucket of `meta_burst` tokens, the
    /// last refilled at `meta_at`.
    meta_at: Instant,
    meta_burst: u8,
    /// The open session's protocol state (its `sess`, the counters).
    pub(super) sess: Option<Session>,
    watched: bool,
    status_sent: Option<(Instant, u32)>,
    pub(super) link: Link,
    /// A configuration change the player has not answered yet.
    pub(super) pending: Option<Pending>,
    /// Remote config commands: one a second.
    pub(super) config_rate: Vec<(KeyId, proto::Bucket)>,
    /// The settings' card copy (`settings_card`) is due then: once remote
    /// config commands stop (one card write for a drag), or at the
    /// session's end.
    pub(super) card_due: Option<Instant>,
    /// Each key's last accepted command this boot.
    pub(super) last_use: Vec<(KeyId, Instant)>,
    /// Guests' `pair` messages for the next top-level turn.
    pub(super) pair_reqs: Vec<PairReq>,
    /// The window the server was told of (`pair-open`), by `pair_id`.
    pub(super) announced: Option<[u8; 10]>,
    /// A new session: say again what is open (and close what died).
    pub(super) reannounce: bool,
    /// The open window's number and `pair_id` (hashed once a window), and
    /// its QR code once made.
    pub(super) window: Option<(u32, [u8; 10], Option<Box<Qr>>)>,
    /// `paired` answers waiting for `keys` to go first (the budget; at
    /// most [`PAIR_QUEUE`]).
    pub(super) paired_due: Vec<String>,
    /// A pairing whose key store was refused `busy` (a LAN store at the
    /// same moment): tried once more at the next turn before answering.
    pub(super) pair_store: Option<super::pair::PairStore>,
    /// "Remote control added" up until then, with the device's name.
    pub(super) added: Option<(String, Instant)>,
    pub(super) shown: Shown,
    /// A GIF being sent (`xfer`): the slot, what the read path queued, the
    /// `.part` open. Boxed: the client lives on the network worker's
    /// stack, and the transfer's hash states are ~0.5 KB.
    pub(super) xfer: Box<super::xfer::Xfer>,
}

impl Client {
    #[optimize(size)]
    #[inline(never)]
    pub fn new(link: Link) -> Client {
        tls::install_allocator();
        let crashes = crate::restarts::crash_boots_in_a_row();
        let (hold_s, crash_off) = match crashes {
            0 => (Some(HOLD_S), false),
            1 => (Some(uptime_s() + CRASH_HOLD_S), false),
            _ => (None, true),
        };
        log::info!(
            "connect: compiled in (server {}, TLS in {} out {}, {} pinned roots){}",
            super::HOST,
            tls::TLS_IN,
            tls::TLS_OUT,
            ROOTS.len(),
            match crashes {
                0 => String::new(),
                1 => "; a crash boot since the last clean boot: held 15 minutes".into(),
                n => format!("; {n} crash boots in a row: off until switched on in Settings"),
            }
        );
        tele(|t| {
            t.rust_bytes = tls::rust_bytes();
            t.crash_boot = crashes > 0;
        });
        Client {
            conn: None,
            backoff: Backoff::default(),
            not_before: Instant::now(),
            ticket: None,
            job: Job::None,
            online_since: None,
            cfg_gen: cfg_gen(),
            floor: None,
            timed: false,
            resolved: None,
            target: (super::HOST.into(), super::ADDR.into()),
            hold_s,
            crash_off,
            quiet_since: Instant::now(),
            reg_secret: None,
            identity: None,
            keys: KeyList::new(),
            name: String::new(),
            need_keys: false,
            need_info: false,
            synced: false,
            forget: false,
            reregs: (0, Instant::now()),
            reregister: false,
            meta_at: Instant::now(),
            meta_burst: 0,
            sess: None,
            watched: false,
            status_sent: None,
            link,
            pending: None,
            config_rate: Vec::new(),
            card_due: None,
            last_use: Vec::new(),
            pair_reqs: Vec::new(),
            announced: None,
            reannounce: false,
            window: None,
            paired_due: Vec::new(),
            pair_store: None,
            added: None,
            shown: Shown::None,
            xfer: Box::new(super::xfer::Xfer::new()),
        }
    }

    /// How long the worker may sleep before the next [`Client::tick`].
    /// Never less than 5 ms; never more than the worker's own second.
    #[optimize(size)]
    #[inline(never)]
    pub fn wait(&self) -> Duration {
        let busy = Duration::from_millis(5);
        let at = match self.conn.as_deref() {
            Some(c) if c.phase != Phase::Open => return busy,
            Some(c) => Some(c.ping.next),
            None if !self.timed => None,
            None => match self.job {
                #[cfg(feature = "connect-measure")]
                Job::Handshakes { next, .. } => Some(next),
                Job::None => Some(self.not_before),
            },
        };
        let w = at.map_or(Duration::from_secs(1), |t| t.saturating_duration_since(Instant::now()).clamp(busy, Duration::from_secs(1)));
        if self.pending.is_some() || !self.paired_due.is_empty() || self.pair_store.is_some() {
            return w.min(Duration::from_millis(200));
        }
        // A send's chunks or credit wait for the next turn (a chunk held
        // back by a LAN upload, an update or the heap: not at 200 Hz).
        if let Some(d) = self.xfer.due() {
            return w.min(d.max(busy));
        }
        // The card's copy is due: the next turn makes it.
        if self.card_due.is_some_and(|t| Instant::now() >= t) {
            return busy;
        }
        w
    }

    /// The worker's turn. `online`: a station with an address.
    #[optimize(size)]
    #[inline(never)]
    pub fn tick(&mut self, online: bool) {
        self.turn(online);
        // Pairing, at the top level (never under the read path).
        self.pairing();
        // A send's hashing and card writes, likewise.
        self.transfers();
        // The worker's stack margin, where something reads it (a measuring
        // build's telemetry, an emulator image's lane).
        #[cfg(any(feature = "connect-measure", feature = "connect-test"))]
        {
            // SAFETY: the calling task's own high-water mark (bytes on IDF).
            let unused = unsafe { sys::uxTaskGetStackHighWaterMark(std::ptr::null_mut()) };
            tele(|t| t.stack_unused = t.stack_unused.min(unused));
            #[cfg(feature = "connect-test")]
            lite(|l| l.stack_unused = l.stack_unused.min(unused));
        }
    }

    #[optimize(size)]
    #[inline(never)]
    fn turn(&mut self, online: bool) {
        let started = Instant::now();
        self.timed = false;
        // This turn's blocking budget, before anything can write.
        if let Some(conn) = self.conn.as_mut() {
            conn.bio.deadline = started + BLOCK_MAX;
        }
        #[cfg(feature = "connect-measure")]
        {
            // A request that closed the session ends the turn: the next
            // attempt is a turn of its own.
            if self.take_ask() {
                return;
            }
        }
        let c = cfg();
        let gen = cfg_gen();
        if gen != self.cfg_gen {
            self.cfg_gen = gen;
            let target = (c.host.to_string(), c.addr.to_string());
            if target != self.target {
                // Another server: its ticket and address are not this one's.
                self.ticket = None;
                self.resolved = None;
                self.target = target;
            }
            self.not_before = Instant::now();
            if self.conn.is_some() {
                return self.close(Cause::Forced, "settings changed");
            }
        }
        if super::lan_busy() {
            self.quiet_since = Instant::now();
        }
        self.poll_pending();
        self.save_card(false);
        let Some(on) = self.sync_shared() else { return };
        // (Latched: a forget taken in by a command's sync is acted on here.
        // Offline, it never reaches the server: the old record lingers there,
        // harmless, never used again.)
        if std::mem::take(&mut self.forget) {
            // The old identity's last word; the next attempt registers anew.
            // (Its window first, as at a switch-off.)
            self.paired_due.clear();
            self.pair_store = None;
            let _ = self.announce(None);
            if let Some(conn) = self.conn.as_mut().filter(|c| c.phase == Phase::Open) {
                let _ = conn.text(proto::FORGET);
            }
            if self.conn.is_some() {
                return self.close(Cause::Forced, "forgotten");
            }
        }
        if !on {
            if self.conn.is_some() {
                // The window went with the switch: the server hears first.
                self.paired_due.clear();
                self.pair_store = None;
                let _ = self.announce(None);
                self.close(Cause::Forced, "switched off");
            }
            set_state("off");
            return;
        }
        if super::updating() {
            if self.conn.is_some() {
                self.close(Cause::Forced, "an update is under way");
            }
            set_state("held for an update");
            return;
        }
        if self.crash_off {
            set_state("off after two crashes in a row");
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
            if let Some(h) = self.hold_s {
                if uptime_s() < h {
                    set_state("waiting after boot");
                    return;
                }
                self.hold_s = None;
            }
            self.timed = true;
            // An attempt (DNS on its own task, the TCP connect here) is a
            // turn of its own: the handshake starts at the next.
            return self.maybe_connect(&c);
        }
        let phase = self.conn.as_ref().map(|c| c.phase);
        let r = match phase {
            Some(Phase::Handshake) => self.step_handshake(started),
            Some(Phase::Request) => self.step_register(started),
            Some(Phase::Upgrade) => self.step_upgrade(started),
            _ => Ok(()),
        };
        if let Err(e) = r {
            return self.end(e);
        }
        if self.conn.as_ref().is_some_and(|c| c.phase == Phase::Open) {
            #[cfg(feature = "connect-measure")]
            if let Job::Handshakes { .. } = self.job {
                // A job's connection is closed as soon as it is open.
                return self.close(Cause::Forced, "");
            }
            if let Err(e) = self.service(&c, started) {
                self.end(e);
            }
        }
    }

    /// Take in what the LAN routes and the commands changed: the switch,
    /// the identity, the keys, the name. `Some((on, forget))`.
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn sync_shared(&mut self) -> Option<bool> {
        let all = !std::mem::replace(&mut self.synced, true);
        let (on, identity, forget, start_now, keys, name) = shared(|s| {
            let keys = (std::mem::take(&mut s.keys_changed) || all).then(|| s.keys.clone());
            let name = (std::mem::take(&mut s.name_changed) || all).then(|| s.name.clone());
            (s.on, s.identity, std::mem::take(&mut s.forget), std::mem::take(&mut s.start_now), keys, name)
        })?;
        if start_now {
            // Switched on in Settings: at once.
            self.hold_s = None;
            self.crash_off = false;
            self.backoff.reset();
            self.not_before = Instant::now();
            lite(|l| {
                l.refused = false;
                l.too_old = false;
            });
        }
        if let Some(k) = keys {
            // A removed key's config bucket goes with it (else 16 keys
            // over time would fill the table for good).
            self.config_rate.retain(|(x, _)| k.find(x).is_some());
            self.keys = k;
            self.need_keys = true;
        }
        if let Some(n) = name {
            self.name = n;
            self.need_info = true;
        }
        self.identity = identity;
        self.forget |= forget;
        Some(on)
    }

    #[optimize(size)]
    #[inline(never)]
    fn clock_ready(&mut self, online_since: Instant) -> bool {
        // A clock before the build date is wrong (a spoofed or broken
        // time server): certificates are then judged against the build date.
        let build: i64 = BUILD_UNIX.parse().unwrap_or(0);
        if crate::clock::synced_ago_secs().is_some() && crate::clock::unix_valid().is_some_and(|t| t >= build) {
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

    /// Take the next measuring request; `true` if it closed the session.
    #[cfg(feature = "connect-measure")]
    #[optimize(size)]
    #[inline(never)]
    fn take_ask(&mut self) -> bool {
        use super::measure::Ask;
        let Some((seq, a)) = super::measure::take_ask() else { return false };
        log::info!("connect: asked {}", a.name());
        let mut closed = false;
        match a {
            Ask::Handshakes { n, interval, resume } => {
                if self.conn.is_some() {
                    self.close(Cause::Forced, "");
                    closed = true;
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
            Ask::Stop => {
                self.job = Job::None;
                tele(|t| t.job = "none");
            }
            Ask::Reconnect => {
                if self.conn.is_some() {
                    self.close(Cause::Forced, "");
                    closed = true;
                }
                self.backoff.reset();
                self.not_before = Instant::now();
            }
            Ask::Reset => super::measure::reset(),
        }
        closed
    }

    /// Start an attempt if it is time, the LAN is quiet and the heap allows.
    #[optimize(size)]
    #[inline(never)]
    fn maybe_connect(&mut self, c: &Cfg) {
        let now = Instant::now();
        // Tickets offered at all, and the saved session offered back.
        let tickets = match self.job {
            #[cfg(feature = "connect-measure")]
            Job::Handshakes { next, resume, .. } => {
                if now < next {
                    set_state(if resume { "job: next resumed" } else { "job: next full" });
                    return;
                }
                resume
            }
            Job::None => {
                if now < self.not_before {
                    let (refused, too_old, state) = lite(|l| (l.refused, l.too_old, l.state));
                    // The gates' own 2 s retries keep their state (and
                    // `waiting`): a turn between two is not a backoff.
                    let retry = state == super::WAITING_FOR_MEMORY || state == WAITING_FOR_QUIET;
                    set_state(if refused {
                        "refused: set Remote control up again"
                    } else if too_old {
                        "update needed"
                    } else if retry {
                        state
                    } else {
                        "backoff"
                    });
                    return;
                }
                c.tickets
            }
        };
        // A LAN upload, stream or heavy request always wins: no handshake
        // until it has been quiet a while (S0's M6: one under load took the
        // heap to 5 KB).
        if self.quiet_since.elapsed() < Duration::from_secs(u64::from(c.quiet_s)) {
            set_state(WAITING_FOR_QUIET);
            tele(|t| t.quiet_waits += 1);
            self.not_before = now + Duration::from_secs(2);
            return;
        }
        let Some(addr) = self.address(c) else { return };
        let resumed = tickets && self.ticket.is_some();
        let gates = gate::Gates { full: c.gate_full, resumed: c.gate_resumed, largest: c.gate_largest, psram_largest: c.gate_psram_largest };
        let free = free_heap();
        let heap = gate::Heap { free, largest: largest_free_block() as u32, psram_largest: psram_largest(gates.psram_largest) };
        if let Err(unmet) = gate::check(&gates, &heap, resumed) {
            super::set_waiting(unmet);
            tele(|t| t.gate_waits += 1);
            self.not_before = now + Duration::from_secs(2);
            #[cfg(feature = "connect-measure")]
            if let Job::Handshakes { ref mut next, .. } = self.job {
                *next = now + Duration::from_secs(2);
            }
            return;
        }
        #[cfg(feature = "connect-measure")]
        if let Job::Handshakes { ref mut next, interval, .. } = self.job {
            *next = now + interval;
        }
        let purpose = if self.identity.is_some() && !self.reregister { Purpose::Session } else { Purpose::Register };
        set_state(if purpose == Purpose::Register { "registering" } else { "connecting" });
        tele(|t| {
            t.attempts += 1;
            t.heap_at_connect = free;
        });
        if purpose == Purpose::Register {
            self.reg_secret = Some(random::<32>());
        }
        let ticket = if tickets { self.ticket.as_deref() } else { None };
        match tls::connect(c.host, addr, purpose, tickets, ticket, self.floor, free) {
            Ok(conn) => {
                set_state("handshake");
                self.conn = Some(conn);
            }
            Err(e) => {
                if e.cause == Cause::Tcp {
                    // Perhaps the address moved: look it up again.
                    self.resolved = None;
                }
                self.failed(e)
            }
        }
    }

    /// The server's address: the configured one, a recent lookup, or
    /// `None` while one runs on its own task (or failed: counted once).
    /// The pacing is `opendmd_core::connect::dns`'s.
    #[optimize(size)]
    #[inline(never)]
    fn address(&mut self, c: &Cfg) -> Option<SocketAddr> {
        #[cfg(feature = "connect-test")]
        let fixed = !c.addr.is_empty() && !super::TEST_LOOKUP.load(Relaxed);
        #[cfg(not(feature = "connect-test"))]
        let fixed = !c.addr.is_empty();
        if fixed {
            return match c.addr.parse::<std::net::SocketAddrV4>() {
                Ok(a) => Some(SocketAddr::V4(a)),
                Err(_) => {
                    self.failed(End::new(Cause::Dns, format!("bad address {}", c.addr)));
                    None
                }
            };
        }
        if let Some((h, a, at)) = &self.resolved {
            if *h == c.host && at.elapsed() < DNS_KEEP {
                return Some(*a);
            }
        }
        let wait = tls::dns_wait();
        let (step, answer, ran) = {
            let mut d = DNS.lock().unwrap_or_else(|p| p.into_inner());
            let job = d.as_ref().map(|j| dns::Job {
                same_host: j.host == c.host,
                answer: j.answer.as_ref().map(|(r, _)| r.is_ok()),
                given_up: j.given_up,
                elapsed: j.started.elapsed(),
            });
            let ran = job.map_or(0, |j| j.elapsed.as_secs());
            let step = dns::step(job, wait);
            let answer = match step {
                dns::Step::Use | dns::Step::Fail => d.take().and_then(|j| j.answer),
                dns::Step::GiveUp => {
                    if let Some(j) = d.as_mut() {
                        j.given_up = true;
                    }
                    None
                }
                // A late error, or another host's answer: dropped.
                dns::Step::Start => {
                    *d = None;
                    None
                }
                dns::Step::Poll | dns::Step::Stuck => None,
            };
            (step, answer, ran)
        };
        match (step, answer) {
            (dns::Step::Use, Some((Ok(a), ms))) => {
                tele(|t| t.dns_ms = ms);
                self.resolved = Some((c.host.to_string(), a, Instant::now()));
                Some(a)
            }
            (dns::Step::Fail, Some((Err(e), ms))) => {
                tele(|t| t.dns_ms = ms);
                self.failed(End::new(Cause::Dns, e));
                None
            }
            (dns::Step::GiveUp, _) => {
                self.failed(End::new(Cause::Dns, format!("{}: no answer within {} s", c.host, wait.as_secs_f32())));
                None
            }
            (dns::Step::Stuck, _) => {
                // One lookup task at a time: none starts until it ends.
                self.failed(End::new(Cause::Dns, format!("{}: the last lookup is stuck ({ran} s, no answer); none new until it ends", c.host)));
                None
            }
            (dns::Step::Poll, _) => {
                set_state("looking up the server");
                self.not_before = Instant::now() + DNS_POLL;
                None
            }
            _ => {
                if let Err(e) = tls::lookup(c.host.to_string()) {
                    self.failed(End::new(Cause::Dns, e));
                } else {
                    set_state("looking up the server");
                    self.not_before = Instant::now() + DNS_POLL;
                }
                None
            }
        }
    }

    /// The TLS handshake's steps; once done, the request or the upgrade.
    #[optimize(size)]
    #[inline(never)]
    fn step_handshake(&mut self, started: Instant) -> Result<(), End> {
        let c = cfg();
        let conn = self.conn.as_mut().expect("held");
        if !tls::step_handshake(conn, started)? {
            return Ok(());
        }
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
        lite(|l| {
            l.handshake_ms = Some(ms);
            if !full {
                l.resumed += 1;
            }
        });
        // A new ticket for the next resumption.
        if conn.tickets {
            let t = self.ticket.get_or_insert_with(Ticket::new);
            if let Err(e) = conn.save_ticket(t) {
                error(e);
                self.ticket = None;
            }
        }
        let conn = self.conn.as_mut().expect("held");
        conn.hs_t0 = Instant::now();
        match conn.purpose {
            Purpose::Register => {
                let Some(secret) = self.reg_secret else {
                    return Err(End::new(Cause::Register, "no secret"));
                };
                let req = cx::post_request(c.host, super::REGISTER_PATH, &proto::register_body(&secret));
                conn.write_all(req.as_bytes())?;
                conn.phase = Phase::Request;
                set_state("registering");
            }
            Purpose::Session => {
                conn.phase = Phase::Upgrade;
                set_state("upgrading");
            }
        }
        Ok(())
    }

    /// `POST /v1/register`'s answer: read it, and once it is all here keep
    /// the id with the secret (through the main task) and close.
    #[optimize(size)]
    #[inline(never)]
    fn step_register(&mut self, started: Instant) -> Result<(), End> {
        let conn = self.conn.as_mut().expect("held");
        let ended = loop {
            let r = conn.read();
            if r > 0 {
                conn.head.extend_from_slice(&conn.buf[..r as usize]);
                match cx::response(&conn.head, REGISTER_MAX) {
                    cx::Response::TooLong => return Err(End::new(Cause::Register, "the answer is too long")),
                    cx::Response::Complete { .. } => break false,
                    cx::Response::Partial => continue,
                }
            }
            if r == 0 || r == ERR_PEER_CLOSE_NOTIFY {
                break true;
            }
            if r != ERR_WANT_READ && r != ERR_WANT_WRITE {
                return Err(End::new(Cause::Register, format!("reading the answer: {}", tls::tls_error(r))));
            }
            if conn.hs_t0.elapsed() > UPGRADE_TIMEOUT {
                return Err(End::new(Cause::Register, "no answer"));
            }
            let left = SLICE.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Ok(());
            }
            conn.poll((left.as_millis() as u32).min(50));
        };
        // The body: by its length (bounded by `response`), or what came
        // before the connection ended.
        let (end, body) = match cx::response(&conn.head, REGISTER_MAX) {
            cx::Response::Complete { end, len } => (end, &conn.head[end..end + len]),
            _ if ended => match cx::head_end(&conn.head) {
                Some(end) => (end, &conn.head[end..]),
                None => return Err(End::new(Cause::Register, "the answer ended early")),
            },
            _ => return Err(End::new(Cause::Register, "the answer is too long")),
        };
        let status = cx::http_status(&conn.head[..end]).unwrap_or(0);
        if status != 200 && status != 201 {
            let mut e = End::new(Cause::Register, format!("registration refused: HTTP {status}"));
            e.code = status;
            return Err(e);
        }
        let Some(dev) = proto::parse_register_answer(body) else {
            return Err(End::new(Cause::Register, "the answer has no id"));
        };
        conn.close_notify();
        // Done with this connection (not a session: nothing counted).
        self.conn = None;
        let Some(secret) = self.reg_secret.take() else {
            return Err(End::new(Cause::Register, "no secret"));
        };
        // A new identity starts with the image's first list (a panel only
        // registers again when it has no controllers: `policy`).
        let keys = super::first_keys();
        let blob = keys.encode();
        let stored = super::change(
            &self.link.cmd,
            move |_| Ok((Store::Identity { dev: dev.0, secret, keys: blob }, ())),
            move |s, ()| {
                s.identity = Some((dev, secret));
                s.keys = keys;
                s.keys_changed = true;
            },
        );
        match stored {
            Ok(()) => {
                log::info!("connect: registered");
                self.reregister = false;
                set_state("registered");
                // The backoff is forgotten only by a session that ran: a
                // server that registers and then refuses the id cannot make
                // a loop of it.
                self.not_before = Instant::now();
                let _ = self.sync_shared();
                Ok(())
            }
            // The server's record is never used: a new one next time.
            Err(e) => {
                self.failed(End::new(Cause::Register, format!("registered, but the identity was not stored: {e}")));
                Ok(())
            }
        }
    }

    /// The HTTP upgrade with the version and the id as subprotocols: send
    /// it, read the answer's head; then `hello`, `keys`, `info`.
    #[optimize(size)]
    #[inline(never)]
    fn step_upgrade(&mut self, started: Instant) -> Result<(), End> {
        let Some((dev, _)) = self.identity else {
            return Err(End::new(Cause::Forced, "no identity"));
        };
        let c = cfg();
        let conn = self.conn.as_mut().expect("held");
        if conn.key.is_empty() {
            conn.key = cx::base64(&random::<16>());
            let protocols = format!("{}, {}", proto::subprotocol(proto::VERSION), dev.subprotocol());
            let req = cx::upgrade_request(c.host, super::PANEL_PATH, &conn.key, &protocols);
            conn.write_all(req.as_bytes())?;
            conn.hs_t0 = Instant::now();
        }
        loop {
            let r = conn.read();
            if r > 0 {
                conn.head.extend_from_slice(&conn.buf[..r as usize]);
                if let Some(end) = cx::head_end(&conn.head) {
                    match cx::check_upgrade(&conn.head[..end], &conn.key) {
                        Ok(p) if p.as_deref().and_then(proto::parse_subprotocol) == Some(proto::VERSION) => {}
                        Ok(p) => return Err(End::new(Cause::Upgrade, format!("the server chose {p:?}, not odc{}", proto::VERSION))),
                        Err(e) => {
                            let mut fail = End::new(Cause::Upgrade, e);
                            fail.code = cx::http_status(&conn.head[..end]).unwrap_or(0);
                            return Err(fail);
                        }
                    }
                    let rest = conn.head.split_off(end);
                    conn.head = Vec::new();
                    tele(|t| t.upgrade_ms = conn.hs_t0.elapsed().as_millis() as u32);
                    conn.phase = Phase::Open;
                    self.opened(&c)?;
                    if !rest.is_empty() {
                        let conn = self.conn.as_mut().expect("held");
                        let acts = decode(conn, Some(&rest), 0)?;
                        return self.act(acts);
                    }
                    return Ok(());
                }
                if conn.head.len() > cx::MAX_UPGRADE_HEAD {
                    return Err(End::new(Cause::Upgrade, "upgrade answer's head too long"));
                }
                continue;
            }
            if r != ERR_WANT_READ {
                return Err(End::new(Cause::Upgrade, format!("reading the answer: {}", tls::tls_error(r))));
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

    /// The session is open: a new `sess`, then `hello`, `keys`, `info`.
    #[optimize(size)]
    #[inline(never)]
    fn opened(&mut self, c: &Cfg) -> Result<(), End> {
        let Some((dev, secret)) = self.identity else {
            return Err(End::new(Cause::Forced, "no identity"));
        };
        let sess = random::<12>();
        self.sess = Some(Session::new(dev, sess, proto::VERSION, core_now()));
        // Per session, as the core's counters.
        self.config_rate.clear();
        self.watched = false;
        self.status_sent = None;
        lite(|l| {
            l.opened += 1;
            l.refused = false;
            l.too_old = false;
            l.watched = false;
        });
        let conn = self.conn.as_mut().expect("held");
        conn.opened = Some(Instant::now());
        conn.ping.next = Instant::now() + Duration::from_secs(u64::from(c.ping_s.max(1)));
        let full = conn.verify.called;
        let connect_ms = conn.t0.elapsed().as_millis() as u32;
        let free = free_heap();
        let tls_open = tls::tls_in_use() - conn.tls_before;
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
        log::info!("connect: open ({} handshake, {connect_ms} ms, heap {free} B)", if full { "full" } else { "resumed" });
        #[cfg(feature = "connect-measure")]
        if let Job::Handshakes { ref mut done, n, .. } = self.job {
            *done += 1;
            let d = *done;
            tele(|t| t.job_done = d);
            if d >= n {
                log::info!("connect: job done ({d} handshakes)");
                self.job = Job::None;
                tele(|t| t.job = "none");
                self.not_before = Instant::now();
                self.backoff.reset();
            }
        }
        // What the panel can do: sends (unless switched off for this chip:
        // `super::sends_on`), and the credit window the server holds it to.
        let caps = Caps { send: super::sends_on(), window: super::xfer::window() };
        let hello = proto::hello(&secret, crate::state::firmware_version(), &sess, caps);
        let conn = self.conn.as_mut().expect("held");
        conn.text(&hello)?;
        drop(hello);
        self.need_keys = true;
        self.need_info = true;
        // `keys`, `info` and a window's `pair-open`: 3 of the server's 4
        // (its bucket may not be full at a quick reconnect).
        self.meta_burst = 3;
        self.meta_at = Instant::now();
        self.reannounce = true;
        self.sync_out()
    }

    /// One more of `keys`, `info`, `pair-open` within the server's budget
    /// for them? After `hello` the burst; then one a second.
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn meta_ok(&mut self) -> bool {
        // One token a second back, up to the server's burst of 4.
        let now = Instant::now();
        let back = now.saturating_duration_since(self.meta_at).as_secs().min(4) as u8;
        if back > 0 {
            self.meta_burst = (self.meta_burst + back).min(4);
            self.meta_at = if self.meta_burst == 4 { now } else { self.meta_at + Duration::from_secs(u64::from(back)) };
        }
        if self.meta_burst == 0 {
            return false;
        }
        self.meta_burst -= 1;
        true
    }

    /// `keys` and `info` when they are due (after `hello`, after a change).
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn sync_out(&mut self) -> Result<(), End> {
        if !self.conn.as_ref().is_some_and(|c| c.phase == Phase::Open) {
            return Ok(());
        }
        // After `hello` both go at once (the burst); a change waits for the
        // second since the last one (several in a row: one `keys`, the last).
        if self.need_keys && self.meta_ok() {
            self.need_keys = false;
            let m = self.keys.keys_message();
            self.conn.as_mut().expect("held").text(&m)?;
        }
        // Pairings' answers once their keys are at the server.
        if !self.need_keys {
            for p in std::mem::take(&mut self.paired_due) {
                self.conn.as_mut().expect("held").text(&p)?;
            }
        }
        if self.need_info && self.meta_ok() {
            self.need_info = false;
            // The name a controller gave it, or the panel's hostname.
            let name = if self.name.is_empty() { self.link.settings.lock().unwrap().hostname.clone() } else { self.name.clone() };
            self.conn.as_mut().expect("held").text(&proto::info(&name))?;
        }
        Ok(())
    }

    /// An open session's turn: read what came, act on it, ping when due,
    /// status while watched.
    #[optimize(size)]
    #[inline(never)]
    fn service(&mut self, c: &Cfg, started: Instant) -> Result<(), End> {
        loop {
            let conn = self.conn.as_mut().expect("held");
            let r = conn.read();
            if r > 0 {
                let acts = decode(conn, None, r as usize)?;
                self.act(acts)?;
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
                return Err(End::new(Cause::Tls, tls::tls_error(r)));
            }
            break;
        }
        self.sync_out()?;
        let conn = self.conn.as_mut().expect("held");
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
            conn.send(cx::OP_PING, &seq.to_be_bytes())?;
            conn.ping.sent = Some((seq, Instant::now()));
            conn.ping.next = Instant::now() + Duration::from_secs(u64::from(c.ping_s.max(1)));
            tele(|t| t.pings += 1);
            // The pong, right here if it comes soon.
            if conn.poll(PONG_WAIT.as_millis() as u32) {
                let r = conn.read();
                if r > 0 {
                    let acts = decode(conn, None, r as usize)?;
                    self.act(acts)?;
                }
            }
        }
        self.maybe_status()
    }

    /// Act on what the frames asked: answer pings and closes, take pongs,
    /// the server's messages, the commands.
    #[optimize(size)]
    #[inline(never)]
    fn act(&mut self, acts: Acts) -> Result<(), End> {
        let conn = self.conn.as_mut().expect("held");
        if let Some(p) = acts.pong {
            conn.send(cx::OP_PONG, &p)?;
        }
        if let (Some(seq), Some((sent_seq, at))) = (acts.pong_seen, conn.ping.sent) {
            if seq == sent_seq {
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
        for m in acts.texts {
            self.server_message(m)?;
        }
        for f in acts.frames {
            self.frame(f)?;
        }
        if let Some((code, reason)) = acts.close {
            // Answer the close (RFC 6455 5.5.1), then end.
            let conn = self.conn.as_mut().expect("held");
            let _ = conn.send(cx::OP_CLOSE, &code.map(|c| c.to_be_bytes().to_vec()).unwrap_or_default());
            let code = code.unwrap_or(1005);
            return Err(End { cause: Cause::ServerClose, code, reason, why: format!("code {code}") });
        }
        Ok(())
    }

    #[optimize(size)]
    #[inline(never)]
    fn server_message(&mut self, m: ServerMsg) -> Result<(), End> {
        match m {
            ServerMsg::Watch(on) => {
                self.watched = on;
                lite(|l| l.watched = on);
                if on {
                    // A status as soon as the rate allows.
                    self.status_sent = self.status_sent.map(|(at, _)| (at, 0));
                }
                Ok(())
            }
            // Checked at the top level (`pairing`): only queued here.
            ServerMsg::Pair { g, public, label, mac } => {
                if self.pair_reqs.len() < PAIR_QUEUE {
                    let seq = shared(|s| s.pairing.as_ref().map(|p| p.seq)).flatten();
                    self.pair_reqs.push(PairReq { seq, g, public, label, mac });
                    return Ok(());
                }
                let conn = self.conn.as_mut().expect("held");
                conn.text(&proto::paired_err(g, "busy"))
            }
            ServerMsg::Unknown => Ok(()),
        }
    }

    /// One binary message from the server: a signed command, checked
    /// against this panel's own list, then carried out and answered.
    #[optimize(size)]
    #[inline(never)]
    fn frame(&mut self, f: Vec<u8>) -> Result<(), End> {
        // A transfer chunk (`0x02`): checked against the credit and queued
        // for the top level (`xfer`), which hashes and writes it. Any other
        // type is dropped.
        if f.first() == Some(&opendmd_core::connect::xfer::TYPE_CHUNK) {
            self.chunk_in(f);
            return Ok(());
        }
        if f.first() != Some(&proto::TYPE_COMMAND) {
            return Ok(());
        }
        let f = f.as_slice();
        let Some(sess) = self.sess.as_mut() else { return Ok(()) };
        let limits = proto::Limits { max_brightness: self.link.settings.lock().unwrap().max_brightness };
        let t = now_us();
        let checked = sess.check(&self.keys, &limits, f, core_now());
        let us = (now_us() - t) as u32;
        tele(|t| {
            t.commands += 1;
            t.check_max_us = t.check_max_us.max(us);
            if checked.is_ok() {
                t.accepted += 1;
            } else {
                t.refused += 1;
            }
        });
        let ack = match checked {
            Ok(acc) => {
                self.note_use(acc.k);
                if matches!(acc.cmd, proto::Command::Send(_) | proto::Command::SendCancel { .. }) {
                    // Answered from the top level, where the card is.
                    self.xfer_cmd_in(acc)
                } else {
                    Some(self.run(acc))
                }
            }
            Err(r) => {
                log::info!("connect: command refused: {}", r.err.code());
                r.ack()
            }
        };
        // What the command changed (keys, name): taken in now, so the next
        // frame is checked against the list as it is.
        let _ = self.sync_shared();
        if let Some(a) = ack {
            let conn = self.conn.as_mut().expect("held");
            conn.text(&a)?;
        }
        Ok(())
    }

    #[optimize(size)]
    #[inline(never)]
    fn note_use(&mut self, k: KeyId) {
        let now = Instant::now();
        if let Some((_, at)) = self.last_use.iter_mut().find(|(x, _)| *x == k) {
            *at = now;
        } else if self.last_use.len() < proto::MAX_COUNTERS {
            self.last_use.push((k, now));
        }
    }

    /// Status to the server while a controller watches.
    #[optimize(size)]
    #[inline(never)]
    fn maybe_status(&mut self) -> Result<(), End> {
        if !self.watched {
            return Ok(());
        }
        let now = Instant::now();
        if self.status_sent.is_some_and(|(at, _)| now < at + STATUS_MIN) {
            return Ok(());
        }
        let name = if self.name.is_empty() { self.link.settings.lock().unwrap().hostname.clone() } else { self.name.clone() };
        let (json, key) = crate::http::connect_status(&self.link.status, &self.link.settings, &name);
        let due = match self.status_sent {
            None => true,
            Some((at, k)) => k != key || now >= at + STATUS_EVERY,
        };
        if due {
            let conn = self.conn.as_mut().expect("held");
            conn.text(&json)?;
            self.status_sent = Some((now, key));
        }
        Ok(())
    }

    /// A configuration change the player took but had not answered: its
    /// answer, when it comes (then the shared settings follow it).
    #[optimize(size)]
    #[inline(never)]
    fn poll_pending(&mut self) {
        let Some(p) = self.pending.as_mut() else { return };
        match p.rx.try_recv() {
            Ok(answer) => {
                let p = self.pending.take().expect("held");
                if let Some(after) = super::dispatch::settle(&self.link, p, answer) {
                    self.save_card_after(after);
                }
            }
            Err(crate::chan::TryRecvError::Empty) if p.until > Instant::now() => {}
            Err(_) => {
                log::warn!("connect: a remote setting was never answered by the player");
                self.pending = None;
                crate::state::drop_spent();
            }
        }
    }

    /// One config command from `k`, within its 1 a second? (At most
    /// `MAX_COUNTERS` keys tracked, as the core's counters.)
    pub(super) fn config_rate_take(&mut self, k: KeyId) -> bool {
        let now = core_now();
        if let Some((_, b)) = self.config_rate.iter_mut().find(|(x, _)| *x == k) {
            return b.take(now);
        }
        if self.config_rate.len() >= proto::MAX_COUNTERS {
            return false;
        }
        let mut b = proto::Bucket::new(1, now);
        let ok = b.take(now);
        self.config_rate.push((k, b));
        ok
    }

    /// The card's copy due `after` from now (or sooner if already). Made
    /// at a turn's top (`save_card`), never down in a command's path.
    pub(super) fn save_card_after(&mut self, after: Duration) {
        let at = Instant::now() + after;
        self.card_due = Some(self.card_due.map_or(at, |d| d.min(at)));
    }

    /// The settings' card copy, when it is due (`now`: the session ended).
    #[optimize(size)]
    #[inline(never)]
    fn save_card(&mut self, now: bool) {
        if self.card_due.is_some_and(|t| now || Instant::now() >= t) {
            self.card_due = None;
            crate::settings_card::save_shared(&self.link.settings);
        }
    }

    /// An attempt that never opened.
    #[optimize(size)]
    #[inline(never)]
    fn failed(&mut self, e: End) {
        self.reg_secret = None;
        tele(|t| t.causes[e.cause as usize] += 1);
        error(format!("{}: {}", e.cause.as_str(), e.why));
        #[cfg(feature = "connect-measure")]
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
        // A lookup costs the server nothing: DNS failures back off no
        // further than `DNS_BACKOFF_CAP`.
        let wait = self.policy(&e).unwrap_or_else(|| match e.cause {
            Cause::Dns => self.backoff.failed_up_to(rand_u32(), dns::DNS_BACKOFF_CAP),
            _ => self.backoff.failed(rand_u32()),
        });
        self.wait_for(wait);
    }

    /// What the server's answer says about waiting, beyond the backoff:
    /// refused, too old, stay away.
    #[optimize(size)]
    #[inline(never)]
    fn policy(&mut self, e: &End) -> Option<Duration> {
        match (e.cause, e.code) {
            // The id is gone (a registration that never said `hello` is
            // deleted, or the record lost): a new one, on the usual backoff.
            // The id is gone and no controller knows it (a registration that
            // never said hello is deleted): a new one. With controllers it
            // is a refusal like 403, nothing erased (a server that lost its
            // records must not wipe every panel's pairings): the owner
            // chooses Forget on the LAN.
            (Cause::Upgrade, 404) if self.identity.is_some() && self.keys.is_empty() && self.rereg_allowed() => {
                self.reregister = true;
                error("the server no longer knows this id (no controllers yet): registering again".into());
                None
            }
            (Cause::ServerClose, proto::CLOSE_REFUSED) | (Cause::Upgrade, 403 | 404) => {
                lite(|l| l.refused = true);
                error("the server does not know this panel: set Remote control up again (Forget all, in Settings)".into());
                Some(BACKOFF_CAP)
            }
            (Cause::ServerClose, proto::CLOSE_TOO_OLD) | (Cause::Register, 426) => {
                lite(|l| l.too_old = true);
                error("the server needs a newer firmware: update this panel".into());
                Some(TOO_OLD_WAIT)
            }
            (Cause::ServerClose, proto::CLOSE_STAY_AWAY) | (Cause::Register | Cause::Upgrade, 429) => {
                let minutes = proto::stay_away_minutes(&e.reason).unwrap_or(15).clamp(1, 24 * 60);
                // With the 0-30 s spread, so panels told at once come back apart.
                Some(Duration::from_secs(u64::from(minutes) * 60) + Duration::from_millis(u64::from(rand_u32() % 30_001)))
            }
            // Too new (4406): a server behind the panel, for a while.
            _ => None,
        }
    }

    #[optimize(size)]
    #[inline(never)]
    /// One more re-registration within the day's cap?
    fn rereg_allowed(&mut self) -> bool {
        if self.reregs.1.elapsed() >= Duration::from_secs(24 * 3600) {
            self.reregs = (0, Instant::now());
        }
        if self.reregs.0 >= REREGISTER_MAX {
            return false;
        }
        self.reregs.0 += 1;
        true
    }

    fn wait_for(&mut self, wait: Duration) {
        self.not_before = Instant::now() + wait;
        tele(|t| t.backoff_s = wait.as_secs() as u32);
        set_state("backoff");
    }

    /// The session (or attempt) ended for `e`: drop it, count it, wait.
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn end(&mut self, e: End) {
        let Some(conn) = self.conn.take() else { return };
        self.save_card(true);
        self.sess = None;
        // A send waits for its controller to sign it again (a resume).
        self.xfer_session_end();
        // (A pairing's answer cannot follow its guest to another session.)
        // (A key stored meanwhile stays: its device may attach without the
        // answer; one not stored yet is not stored.)
        self.pair_reqs.clear();
        self.paired_due.clear();
        self.pair_store = None;
        self.watched = false;
        lite(|l| l.watched = false);
        let Some(opened) = conn.opened else {
            drop(conn);
            if e.cause == Cause::Forced {
                // Stopped by a setting or a test hook mid-attempt.
                self.reg_secret = None;
                self.not_before = Instant::now();
                set_state("closed");
                return;
            }
            // It never opened: an attempt that failed.
            return self.failed(e);
        };
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
        if e.cause == Cause::Forced {
            self.not_before = Instant::now();
            set_state("closed");
            return;
        }
        lite(|l| l.reconnects += 1);
        error(format!("session ended after {secs} s: {} {}", e.cause.as_str(), e.why));
        let wait = self.policy(&e).unwrap_or_else(|| {
            if secs >= RAN.as_secs() as u32 {
                self.backoff.dropped(rand_u32())
            } else {
                self.backoff.failed(rand_u32())
            }
        });
        self.wait_for(wait);
    }

    /// Close cleanly (a WebSocket close, the server's answer awaited), then
    /// [`Client::end`].
    #[optimize(size)]
    #[inline(never)]
    pub(super) fn close(&mut self, cause: Cause, why: &str) {
        if let Some(conn) = self.conn.as_mut() {
            conn.bio.deadline = Instant::now() + CLOSE_WAIT + Duration::from_secs(2);
            if conn.phase == Phase::Open {
                set_state("closing");
                if conn.send(cx::OP_CLOSE, &cx::close_payload(1000, "")).is_ok() {
                    let until = Instant::now() + CLOSE_WAIT;
                    while Instant::now() < until {
                        let r = conn.read();
                        if r == ERR_WANT_READ {
                            conn.poll(50);
                            continue;
                        }
                        if r <= 0 {
                            break;
                        }
                    }
                    conn.close_notify();
                }
            } else if conn.phase != Phase::Handshake {
                conn.close_notify();
            }
        }
        self.end(End::new(cause, why));
    }
}

/// Feed the decoder `extra` (what followed the upgrade's head) or the first
/// `n` bytes of the connection's buffer; collect what it asks.
#[optimize(size)]
#[inline(never)]
fn decode(conn: &mut Conn, extra: Option<&[u8]>, n: usize) -> Result<Acts, End> {
    let Conn { dec, buf, bin, bin_len, in_bin, .. } = conn;
    let data: &[u8] = match extra {
        Some(d) => d,
        None => &buf[..n],
    };
    let mut acts = Acts::default();
    let r = dec.feed(data, &mut |e| match e {
        // Unknown types and malformed text are ignored (the protocol's
        // rule): the server cannot make the panel do anything with text.
        Event::Text(t) => {
            if let Ok(m) = proto::parse_server_text(t) {
                if m != ServerMsg::Unknown {
                    acts.texts.push(m);
                }
            }
        }
        Event::BinaryStart(_) => {
            if !*in_bin {
                *in_bin = true;
                bin.clear();
                *bin_len = 0;
            }
        }
        Event::Binary(p) => {
            *bin_len += p.len() as u64;
            let room = BIN_KEEP.saturating_sub(bin.len());
            bin.extend_from_slice(&p[..p.len().min(room)]);
        }
        Event::BinaryEnd { fin } => {
            if fin {
                *in_bin = false;
                acts.frames.push(std::mem::take(bin));
            }
        }
        Event::Ping(p) => acts.pong = Some(p.to_vec()),
        Event::Pong(p) => acts.pong_seen = (p.len() == 4).then(|| u32::from_be_bytes([p[0], p[1], p[2], p[3]])),
        Event::Close { code, reason } => acts.close = Some((code, reason.to_vec())),
    });
    r.map_err(|e| End::new(Cause::Ws, e.as_str()))?;
    Ok(acts)
}

/// PSRAM's largest free block, read only when the gate asks about it
/// (`need` > 0: a `psram-tls` S3); 0 on the classic.
#[optimize(size)]
#[inline(never)]
fn psram_largest(need: u32) -> u32 {
    if need == 0 {
        return 0;
    }
    #[cfg(esp32s3)]
    // SAFETY: a read-only query with a capability mask argument.
    return unsafe { esp_idf_sys::heap_caps_get_largest_free_block(esp_idf_sys::MALLOC_CAP_SPIRAM) as u32 };
    #[cfg(not(esp32s3))]
    0
}
