//! OpenDMD Connect: Remote control through a relay (feature `connect`;
//! the protocol is `docs/connect-protocol.md`, its checks
//! `opendmd_core::connect`).
//!
//! **What it is.** One outbound WebSocket over TLS 1.2 to the Connect
//! server, held open with pings and reconnected with backoff. The panel
//! registers once (`POST /v1/register`, its secret's hash for an id), says
//! `hello` on every connection, tells the server its controllers' public
//! keys (`keys`) and name (`info`), takes signed commands and answers each
//! with an `ack`, and sends its status while a controller watches. The
//! server relays; it can never command the panel: every command is checked
//! here against the panel's own list ([`client`], [`dispatch`]).
//!
//! **Where it runs.** Inside the network worker (CPU0, priority 5, its
//! 12 KB stack with this feature), as a polled client: [`Client::tick`] is
//! called from the worker's loop and does at most a slice of work (~250 ms:
//! a handshake step that is not waiting, a few records read, a command or
//! two) plus one bounded blocking step (≤ 8-9 s at worst: [`tls`]).
//! [`Client::wait`] shortens the loop's sleep while there is something to
//! do. No task of its own. Nothing here runs on CPU1: the display sees
//! only the `Command`s the LAN handlers send too, and the status it
//! publishes. Flash writes (the switch, the identity, the controller list,
//! the name) go through the main task (`Command::ConnectStore`), at the
//! moments settings saves use.
//!
//! **Parts**: [`tls`] (TCP, mbedTLS, the pinned roots, DNS), [`client`]
//! (the session's state, gates, backoff, registration), [`dispatch`]
//! (signed commands to the player), [`pair`] (pairing windows, the QR
//! code and the screens), [`api`] (the LAN routes), and
//! `measure` (only with `connect-measure`: the device tests' telemetry and
//! test hooks, never in a shipping build).
//!
//! **Pairing** (`docs/connect-protocol.md` 6.1): `POST /api/connect/pair`
//! opens a window ([`Pairing`]: a fresh secret, 5 minutes, one use, three
//! tries) and answers the link; with `?qr=1` the panel shows the QR code
//! of `https://opendmd.dev/c#<S>` too. The client announces the window
//! (`pair-open`), checks a guest's `pair` MACs and stores the key from the
//! turn's top level (never under the session's read path: the worker's
//! stack), answers `paired`, and says `pair-close` the moment a window
//! dies. The QR and "Remote control added" screens are drawn by the main
//! task from a [`Screen`] the client sends (`Command::ConnectScreen`).
//!
//! **Sending a GIF** (`docs/connect-protocol.md` 8; [`xfer`]): a signed
//! `send` and its chunks are taken in by the read path and queued; the
//! hashing, the card writes and the `ack`s happen at the turn's top level,
//! inside the credit window `hello` declares (`caps.win`), into
//! `/gifs/Sent/`. Sends are on the S3 and off on the classic until device
//! test D4 passes (`connect-send-classic` turns them on there,
//! `connect-no-send` off anywhere, [`set_sends`] at run time): off,
//! `hello` says `send: false` and a `send` is `not-allowed`.

pub mod api;
mod client;
mod dispatch;
#[cfg_attr(not(feature = "connect-measure"), allow(dead_code))]
mod measure;
mod pair;
mod tls;
mod xfer;

pub use client::Client;
pub use opendmd_core::connect::pair::Window;

/// What the client reaches the player through (the worker's handles).
#[optimize(size)]
#[inline(never)]
pub fn dispatch_link(
    cmd: mpsc::Sender<Command>,
    status: std::sync::Arc<Mutex<crate::state::Status>>,
    settings: std::sync::Arc<Mutex<crate::nvs_config::Settings>>,
) -> dispatch::Link {
    dispatch::Link { cmd, status, settings }
}
#[cfg(feature = "connect-measure")]
pub use measure::routes as measure_routes;
pub(crate) use measure::tele;

use crate::chan as mpsc;
use crate::httpd::json_string;
use crate::state::Command;
use opendmd_core::connect::keys::KeyList;
use opendmd_core::connect::proto::Dev;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;

/// The server, by build-time setting: `OPENDMD_CONNECT_HOST` (the
/// certificate's name too), and `OPENDMD_CONNECT_ADDR` (`ip:port`, the
/// emulator lane's local server) to connect there instead of looking the
/// host up.
/// Only an emulator image (feature `connect-test`) may name another;
/// `build.rs` refuses the variables in any other build.
#[cfg(feature = "connect-test")]
pub const HOST: &str = match option_env!("OPENDMD_CONNECT_HOST") {
    Some(h) => h,
    None => "connect.opendmd.dev",
};
#[cfg(not(feature = "connect-test"))]
pub const HOST: &str = "connect.opendmd.dev";
#[cfg(feature = "connect-test")]
pub const ADDR: &str = match option_env!("OPENDMD_CONNECT_ADDR") {
    Some(a) => a,
    None => "",
};
#[cfg(not(feature = "connect-test"))]
pub const ADDR: &str = "";
pub const PANEL_PATH: &str = "/v1/panel";
pub const REGISTER_PATH: &str = "/v1/register";

// --- Why a session ended ----------------------------------------------------

/// Why a session ended or an attempt failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Cause {
    /// Closed on purpose (switched off, a setting, forget, an update, a
    /// test hook).
    Forced,
    /// The server sent a WebSocket close.
    ServerClose,
    /// TLS close_notify or TCP end without a WebSocket close.
    Eof,
    /// Two pings in a row unanswered.
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
    /// `POST /v1/register` failed.
    Register,
}

pub const CAUSES: [&str; 13] = [
    "forced", "server_close", "eof", "pong_timeout", "tls", "ws", "wifi", "io", "dns", "tcp", "handshake", "upgrade", "register",
];

impl Cause {
    #[optimize(size)]
    #[inline(never)]
    pub fn as_str(self) -> &'static str {
        CAUSES[self as usize]
    }
}

/// How a session ended: the cause, a WebSocket close code or an HTTP
/// status (0 for none), the close's reason (a "stay away"'s minutes), and
/// the words for `last_error`.
pub struct End {
    pub cause: Cause,
    pub code: u16,
    pub reason: Vec<u8>,
    pub why: String,
}

impl End {
    #[optimize(size)]
    #[inline(never)]
    pub fn new(cause: Cause, why: impl Into<String>) -> End {
        End { cause, code: 0, reason: Vec::new(), why: why.into() }
    }
}

// --- What is stored, and the state the LAN routes share -----------------------

/// What NVS holds for Connect (namespace `opendmd`: `cx_on`, `cx_dev`,
/// `cx_sec`, `cx_keys`, `cx_name`), read by the main task at boot
/// (`nvs_config::Config::connect_load`).
#[derive(Default)]
pub struct Stored {
    pub on: bool,
    /// The Connect id and the panel secret: both or neither.
    pub identity: Option<([u8; 16], [u8; 32])>,
    /// The controller list's blob (`KeyList::encode`), if any.
    pub keys: Option<Vec<u8>>,
    pub name: Option<String>,
}

/// One write to NVS, made by the main task (`Command::ConnectStore`).
pub enum Store {
    On(bool),
    /// A new identity, with the controller list it starts with.
    Identity { dev: [u8; 16], secret: [u8; 32], keys: Vec<u8> },
    Keys(Vec<u8>),
    Name(String),
    /// The identity and the controllers go (the switch and the name stay).
    Forget,
}

/// An open pairing window (`POST /api/connect/pair`): the core's window
/// (the secret, its time, its tries), whether the panel shows its QR code,
/// and its number (each new window a new one: the client's announcement
/// and screen follow it).
pub struct Pairing {
    pub win: Window,
    pub qr: bool,
    pub seq: u32,
}

/// What the panel shows for Remote control instead of playing
/// (`Command::ConnectScreen`; `None` takes it down).
pub enum Screen {
    /// The pairing QR code (computed on CPU0), until the window dies.
    Pair(Box<opendmd_core::connect::qr::Qr>),
    /// "Remote control added", with the new device's name, for
    /// [`ADDED_FOR`].
    Added(String),
}

/// How long a pairing window stays open: the protocol's 5 minutes (an
/// emulator image: `OPENDMD_CONNECT_PAIR_TTL_S`, its lane's shorter one).
#[cfg(feature = "connect-test")]
pub const PAIR_TTL: Duration = match option_env!("OPENDMD_CONNECT_PAIR_TTL_S") {
    Some(v) => Duration::from_secs(client::parse_u64(v)),
    None => opendmd_core::connect::pair::PAIR_TTL,
};
#[cfg(not(feature = "connect-test"))]
pub const PAIR_TTL: Duration = opendmd_core::connect::pair::PAIR_TTL;

/// How long "Remote control added" stays up.
pub const ADDED_FOR: Duration = Duration::from_secs(10);

/// What the client and the LAN routes share. Changes bump [`Shared::gen`];
/// the client acts on them at its next turn (sends `keys` or `info`, or
/// says `forget` and drops the identity).
pub struct Shared {
    pub on: bool,
    pub identity: Option<(Dev, [u8; 32])>,
    pub keys: KeyList,
    pub name: String,
    /// Bumped by every change.
    pub gen: u32,
    /// Which parts changed since the client last looked.
    pub keys_changed: bool,
    pub name_changed: bool,
    /// `POST /api/connect/forget`: the open session says `forget` and ends.
    pub forget: bool,
    /// Switched on from the LAN: no hold, at once.
    pub start_now: bool,
    /// A store is being made (one at a time).
    pub storing: bool,
    /// The pairing window, if one was opened (it may have died since: the
    /// client drops it at its next turn and says `pair-close`).
    pub pairing: Option<Pairing>,
    /// The last window's number.
    pub pair_seq: u32,
}

static SHARED: Mutex<Option<Shared>> = Mutex::new(None);

/// Run `f` on the shared state (`None` before [`init`]).
#[optimize(size)]
#[inline(never)]
pub fn shared<R>(f: impl FnOnce(&mut Shared) -> R) -> Option<R> {
    SHARED.lock().unwrap_or_else(|p| p.into_inner()).as_mut().map(f)
}

/// The test key an emulator image starts with (`OPENDMD_CONNECT_TEST_KEY`,
/// hex, only beside its test root): the lane's controller, paired as if on
/// the panel. Never in any other build.
#[cfg(feature = "connect-test")]
const TEST_KEY: Option<&str> = option_env!("OPENDMD_CONNECT_TEST_KEY");

/// The list a new identity starts with: empty (an emulator image: its
/// test key).
#[optimize(size)]
#[inline(never)]
pub fn first_keys() -> KeyList {
    #[allow(unused_mut)]
    let mut keys = KeyList::new();
    #[cfg(feature = "connect-test")]
    if let Some(public) = TEST_KEY.and_then(opendmd_core::connect::codec::unhex::<32>) {
        let _ = keys.add(opendmd_core::connect::keys::Entry {
            public,
            flags: opendmd_core::connect::keys::FLAG_CAN_ADD,
            label: "emulator".into(),
            added: 0,
            added_by: None,
        });
    }
    keys
}

/// At boot, from what NVS holds (the main task, before the network worker
/// starts). A damaged blob is an empty list, logged.
#[optimize(size)]
#[inline(never)]
pub fn init(s: Stored) {
    let keys = match s.keys.as_deref().map(KeyList::decode) {
        Some(Ok(k)) => k,
        Some(Err(e)) => {
            log::warn!("connect: the stored controller list does not read ({e:?}): none");
            KeyList::new()
        }
        None if s.identity.is_some() => KeyList::new(),
        None => first_keys(),
    };
    let identity = s.identity.map(|(d, sec)| (Dev(d), sec));
    log::info!(
        "connect: Remote control {}, {}, {} controller(s)",
        if s.on { "on" } else { "off" },
        if identity.is_some() { "registered" } else { "not registered yet" },
        keys.len()
    );
    *SHARED.lock().unwrap_or_else(|p| p.into_inner()) = Some(Shared {
        on: s.on,
        identity,
        keys,
        name: s.name.unwrap_or_default(),
        gen: 0,
        keys_changed: false,
        name_changed: false,
        forget: false,
        start_now: false,
        storing: false,
        pairing: None,
        pair_seq: 0,
    });
}

/// How long a store waits for the main task to take it, then to answer.
const STORE_TAKE: Duration = Duration::from_millis(800);
const STORE_ANSWER: Duration = Duration::from_secs(3);
/// ... and, once taken, how much longer its answer is waited for.
const STORE_LATE: Duration = Duration::from_secs(5);
/// A store taken but never answered.
pub(crate) const NOT_IN_TIME: &str = "not answered in time";

/// What a change does to the shared state once it is written.
pub type Apply = Box<dyn FnOnce(&mut Shared) + Send>;
type Make = Box<dyn FnOnce(&Shared) -> Result<(Store, Apply), String> + Send>;

/// Change the shared state through NVS, one change at a time (a second
/// meanwhile is refused `busy`): `make` sees the state and says what to
/// store (or why not); the main task writes it (`Command::ConnectStore`),
/// waited for on the caller's task (the httpd task or the network worker,
/// never CPU1); once written, `apply` changes the state and bumps its
/// generation. (A thin shim over one copy, [`change_dyn`].)
pub fn change<T: Send + 'static>(
    cmd: &mpsc::Sender<Command>,
    make: impl FnOnce(&Shared) -> Result<(Store, T), String> + Send + 'static,
    apply: impl FnOnce(&mut Shared, T) + Send + 'static,
) -> Result<(), String> {
    change_dyn(
        cmd,
        Box::new(move |s| {
            let (what, t) = make(s)?;
            Ok((what, Box::new(move |s: &mut Shared| apply(s, t)) as Apply))
        }),
    )
}

#[optimize(size)]
#[inline(never)]
fn change_dyn(cmd: &mpsc::Sender<Command>, make: Make) -> Result<(), String> {
    let (what, apply) = shared(|s| {
        if s.storing {
            return Err("busy".to_string());
        }
        let r = make(s)?;
        s.storing = true;
        Ok(r)
    })
    .unwrap_or_else(|| Err("Remote control is not running".into()))?;
    let r = write(cmd, what);
    shared(|s| {
        s.storing = false;
        if r.is_ok() {
            apply(s);
            s.gen = s.gen.wrapping_add(1);
        }
    });
    r
}

/// The store itself, through the main task (one copy, whatever the change).
#[optimize(size)]
#[inline(never)]
fn write(cmd: &mpsc::Sender<Command>, what: Store) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    cmd.send(Command::ConnectStore { what: Box::new(what), reply: tx }).map_err(|_| "playback not running".to_string())?;
    crate::wake::kick();
    match rx.answer_within(STORE_TAKE, STORE_ANSWER) {
        mpsc::Handoff::Answered(Ok(_)) => Ok(()),
        mpsc::Handoff::Answered(Err(e)) => Err(e),
        // Taken, not answered yet: the write is under way on the main task
        // (a long frame or an index step ahead of it). Its answer, a while
        // longer, so the shared state follows what NVS now holds; past that
        // the change is not applied here (NVS wins at the next boot).
        mpsc::Handoff::Taken => match rx.recv_timeout(STORE_LATE) {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                log::warn!("connect: a store taken by the main task was not answered in {} s", (STORE_ANSWER + STORE_LATE).as_secs());
                Err(NOT_IN_TIME.into())
            }
        },
        mpsc::Handoff::Withdrawn => Err("busy".into()),
    }
}

// --- Sends ---------------------------------------------------------------------

/// Whether this panel takes GIF sends (`hello`'s `caps.send`): on the S3;
/// **off on the classic** until device test D4 shows its card writes
/// sit beside play (P1-PLAN 10, open question 1), except in a build with
/// `connect-send-classic` (D4's own builds, the emulator lane's image);
/// never with `connect-no-send`. Changeable at run time ([`set_sends`]:
/// from the next session's `hello`, and at once for a new `send`).
static SENDS: AtomicBool = AtomicBool::new(SENDS_BUILT);
const SENDS_BUILT: bool = (cfg!(esp32s3) || cfg!(feature = "connect-send-classic")) && cfg!(not(feature = "connect-no-send"));

#[optimize(size)]
#[inline(never)]
pub fn sends_on() -> bool {
    SENDS.load(Relaxed)
}

#[allow(dead_code)]
#[optimize(size)]
#[inline(never)]
pub fn set_sends(on: bool) {
    SENDS.store(on && SENDS_BUILT, Relaxed);
}

/// A remote send holds a transfer (`/api/status` `transfer`, as a LAN
/// upload does).
pub(crate) static SENDING: AtomicBool = AtomicBool::new(false);

#[optimize(size)]
#[inline(never)]
pub fn sending() -> bool {
    SENDING.load(Relaxed)
}

/// An emulator image's stand-in for a card taken out, for sends only
/// (`PUT /api/connect` `{"test_no_card":bool}`, `connect-test` builds).
#[cfg(feature = "connect-test")]
pub(crate) static TEST_NO_CARD: AtomicBool = AtomicBool::new(false);
/// ... and for the next send's card write failing (`{"test_fail_write":true}`).
#[cfg(feature = "connect-test")]
pub(crate) static TEST_FAIL_WRITE: AtomicBool = AtomicBool::new(false);

// --- Signals from the rest of the firmware --------------------------------------

/// A firmware or web app update is being received (`ota::receive`,
/// `www::receive`): the session closes and none starts again until the
/// restart (or, if the update failed and no restart came, for
/// [`UPDATE_HOLD`]).
static UPDATING_AT: AtomicU32 = AtomicU32::new(0);
pub const UPDATE_HOLD: Duration = Duration::from_secs(10 * 60);
static UPDATING: AtomicBool = AtomicBool::new(false);

#[optimize(size)]
#[inline(never)]
pub fn hold_for_update() {
    UPDATING_AT.store(tls::now_us().div_ceil(1_000_000) as u32, Relaxed);
    UPDATING.store(true, Relaxed);
}

/// An update is under way (or was, without a restart, within the hold).
#[optimize(size)]
#[inline(never)]
pub fn updating() -> bool {
    UPDATING.load(Relaxed) && (tls::now_us() / 1_000_000) as u32 <= UPDATING_AT.load(Relaxed).saturating_add(UPDATE_HOLD.as_secs() as u32)
}

/// A heavy LAN request was admitted (`http::admitted`): the uptime second.
static HEAVY_AT: AtomicU32 = AtomicU32::new(0);

#[optimize(size)]
#[inline(never)]
pub fn note_heavy() {
    HEAVY_AT.store((tls::now_us() / 1_000_000) as u32 + 1, Relaxed);
}

/// The LAN is busy now: an upload or a stream on the stream task, or a
/// heavy request admitted within the last second.
#[optimize(size)]
#[inline(never)]
pub fn lan_busy() -> bool {
    let now = (tls::now_us() / 1_000_000) as u32;
    let heavy = HEAVY_AT.load(Relaxed);
    crate::stream::transfers() > 0 || crate::stream::uploads() > 0 || (heavy != 0 && now <= heavy)
}

// --- What /api/status and /api/connect say ---------------------------------------

/// The always-on summary (`/api/status` `connect` in a shipping build,
/// `GET /api/connect`).
pub struct Lite {
    pub state: &'static str,
    pub since_s: u32,
    pub opened: u32,
    pub reconnects: u32,
    pub resumed: u32,
    pub handshake_ms: Option<u32>,
    pub last_error: Option<String>,
    pub last_error_s: u32,
    /// While "waiting for memory": the condition unmet ("internal heap
    /// 77 KB < 100 KB"); `None` in any other state.
    pub waiting: Option<String>,
    /// The server refused this panel's identity (4403, or an unknown id):
    /// Settings says to set Remote control up again.
    pub refused: bool,
    /// The server is newer than this firmware can speak to (4426).
    pub too_old: bool,
    pub watched: bool,
    /// The network worker's stack never used, bytes (an emulator image's,
    /// `connect-test`: its lane checks the margin).
    #[cfg(feature = "connect-test")]
    pub stack_unused: u32,
}

pub static LITE: Mutex<Lite> = Mutex::new(Lite {
    state: "starting",
    since_s: 0,
    opened: 0,
    reconnects: 0,
    resumed: 0,
    handshake_ms: None,
    last_error: None,
    last_error_s: 0,
    waiting: None,
    refused: false,
    too_old: false,
    watched: false,
    #[cfg(feature = "connect-test")]
    stack_unused: u32::MAX,
});

#[optimize(size)]
#[inline(never)]
pub fn lite<R>(f: impl FnOnce(&mut Lite) -> R) -> R {
    f(&mut LITE.lock().unwrap_or_else(|p| p.into_inner()))
}

#[optimize(size)]
#[inline(never)]
fn uptime_s() -> u32 {
    (tls::now_us() / 1_000_000) as u32
}

#[optimize(size)]
#[inline(never)]
pub(crate) fn set_state(s: &'static str) {
    lite(|l| {
        if l.state != s {
            if s == WAITING_FOR_MEMORY {
                log::info!("connect: waiting for memory");
            }
            l.state = s;
            l.since_s = uptime_s();
            l.waiting = None;
        }
    });
    tele(|t| {
        if t.state != s {
            t.state = s;
            t.since_s = uptime_s();
        }
    });
}

/// The state a handshake waits in for the heap gate.
pub(crate) const WAITING_FOR_MEMORY: &str = "waiting for memory";

/// What the heap gate found unmet, for `waiting` (replaced only when it
/// reads differently).
#[optimize(size)]
#[inline(never)]
pub(crate) fn set_waiting(unmet: opendmd_core::connect::gate::Unmet) {
    let d = unmet.describe();
    lite(|l| {
        if l.waiting.as_deref() != Some(d.as_str()) {
            l.waiting = Some(d);
        }
    });
}

/// A failure worth a line and `last_error` (never a secret: no message
/// here carries one).
#[optimize(size)]
#[inline(never)]
pub(crate) fn error(e: String) {
    log::warn!("connect: {e}");
    tele(|t| {
        t.last_error = Some(e.clone());
        t.last_error_s = uptime_s();
        t.errors += 1;
    });
    lite(|l| {
        l.last_error = Some(e);
        l.last_error_s = uptime_s();
    });
}

#[optimize(size)]
#[inline(never)]
fn opt_u32(v: Option<u32>) -> String {
    v.map_or_else(|| "null".into(), |v| v.to_string())
}

/// The summary's fields, without braces.
#[optimize(size)]
#[inline(never)]
fn lite_fields() -> String {
    let l = LITE.lock().unwrap_or_else(|p| p.into_inner());
    #[cfg(feature = "connect-test")]
    let stack = format!(r#","stack_unused":{}"#, l.stack_unused);
    #[cfg(not(feature = "connect-test"))]
    let stack = "";
    format!(
        r#""state":{},"since_s":{},"opened":{},"reconnects":{},"resumed":{},"handshake_ms":{},"refused":{},"too_old":{},"watched":{},"last_error":{},"last_error_s":{},"waiting":{}{stack}"#,
        json_string(l.state),
        l.since_s,
        l.opened,
        l.reconnects,
        l.resumed,
        opt_u32(l.handshake_ms),
        l.refused,
        l.too_old,
        l.watched,
        l.last_error.as_deref().map_or_else(|| "null".to_string(), json_string),
        l.last_error_s,
        l.waiting.as_deref().map_or_else(|| "null".to_string(), json_string),
    )
}

/// `/api/status` `connect`: the summary, and whether it is on and
/// registered (a measuring build: S0's whole telemetry instead).
#[optimize(size)]
#[inline(never)]
pub fn status_json() -> String {
    #[cfg(feature = "connect-measure")]
    return measure::json();
    #[cfg(not(feature = "connect-measure"))]
    {
        let (on, registered) = shared(|s| (s.on, s.identity.is_some())).unwrap_or((false, false));
        format!(r#"{{"enabled":{on},"registered":{registered},{}}}"#, lite_fields())
    }
}
