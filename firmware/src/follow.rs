//! "Follow RePlayOS": the panel reacts to what the RePlayOS box (a Pi 5
//! running a libretro frontend) is doing — attract playlist while it sits in
//! a menu; when a game starts, a power-on effect, the console's logo, a title
//! card, then GIFs of that console weighted towards that game.
//!
//! Two halves, on the two sides of the seam (see `state.rs`):
//!
//! - **[`Poller`], on the network worker (CPU0).** Polls
//!   `GET http://<host>:55356/api/v1/get_status` (header `X-RePlay-Token`)
//!   every 2 s while enabled — 15 s while the Pi does not answer — each
//!   on a TCP connection of its own (`opendmd_core::http_client`, 3 s
//!   timeouts), feeds the answers to the
//!   pure state machine in `opendmd_core::follow`, and does the slow work the
//!   display task must never do: it builds the game's **context index** (the
//!   playlist's entries for that console, weighted by
//!   `opendmd_core::follow::title_score`) with one streaming pass over the
//!   manifest, cached as `/sets/.ctx/<playlist>.<system>.<hash>.idx` so a
//!   second launch is instant, and keeps `/sets/.ctx/<playlist>.logos` (every
//!   followed console's logo entries, one pass) to choose the logo from.
//!   What it decides goes to the main task as [`Event`]s over the command
//!   channel.
//! - **[`Runner`], on the main task (CPU1).** Owns the overlay: which
//!   playlist plays (the attract playlist, or the context index), and the
//!   sequence — each step a file the engine plays with `loops=1` while
//!   `Engine::set_hold` keeps the playlist from cutting in between them; the
//!   compositor's transition joins them. The power-on effect and the cards
//!   are virtual files (`opendmd_core::synth`), so the browser preview shows
//!   them too.
//!
//! Like the schedule it is an overlay: the saved settings are never touched,
//! and night mode still wins (its brightness applies; while it has the panel
//! dark nothing starts, and a game in progress is picked up — without the
//! sequence — when it lights again).

use crate::chan as mpsc;
use opendmd_core::engine::Engine;
use opendmd_core::filter::Filter;
use opendmd_core::follow::{self as rules, Action, Follow, Game, Observed, State, StatusScan};
use opendmd_core::http_client;
use opendmd_core::index::{write_index_resolving_then, IndexHeader};
use opendmd_core::manifest::{self, Entry};
use opendmd_core::overrides::Override;
use opendmd_core::synth;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::config;
use crate::httpd::json_string;
use crate::nvs_config::{self, Settings};
use crate::state::{free_heap, Command, Status};

// ---------------------------------------------------------------------------
// What status reports (`/api/status` `context`), written by both halves.

struct Shared {
    enabled: bool,
    state: State,
    system: Option<String>,
    platform: Option<&'static str>,
    /// The console's name (`Mega Drive`).
    console: Option<String>,
    /// The game's name as a card shows it.
    game: Option<String>,
    /// Entries weighted towards the game in its context playlist.
    matches: Option<usize>,
    since: Option<Instant>,
    /// Why the Pi is unreachable (`401 (wrong code)`, `no answer`, …).
    error: Option<String>,
    /// The sequence step on the panel (main task).
    step: Option<&'static str>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    enabled: false,
    state: State::Off,
    system: None,
    platform: None,
    console: None,
    game: None,
    matches: None,
    since: None,
    error: None,
    step: None,
});

/// `/api/status`'s `context` object.
pub fn status_json() -> String {
    let s = SHARED.lock().unwrap();
    let opt = |v: Option<&str>| v.map_or_else(|| "null".to_string(), json_string);
    format!(
        concat!(
            r#"{{"source":"replayos","enabled":{},"state":{},"system":{},"platform":{},"#,
            r#""console":{},"game":{},"matches":{},"since":{},"step":{},"error":{}}}"#
        ),
        s.enabled,
        json_string(rules::state_name(s.state, s.step.is_some())),
        opt(s.system.as_deref()),
        opt(s.platform),
        opt(s.console.as_deref()),
        opt(s.game.as_deref()),
        s.matches
            .map_or_else(|| "null".to_string(), |n| n.to_string()),
        s.since
            .map_or_else(|| "null".to_string(), |t| t.elapsed().as_secs().to_string()),
        opt(s.step),
        opt(s.error.as_deref()),
    )
}

/// For the heartbeat.
pub fn summary() -> String {
    let s = SHARED.lock().unwrap();
    if !s.enabled {
        return String::new();
    }
    let mut out = format!(", follow {}", rules::state_name(s.state, s.step.is_some()));
    if let Some(g) = &s.game {
        out += &format!(" '{g}'");
    }
    if let Some(e) = &s.error {
        out += &format!(" ({e})");
    }
    out
}

fn set_step(step: Option<&'static str>) {
    SHARED.lock().unwrap().step = step;
}

// ---------------------------------------------------------------------------
// The events the poller sends the main task.

/// What the network side tells the main task (`Command::Follow`). `key`
/// numbers the games, so a late answer about an earlier one is ignored.
pub enum Event {
    /// The normal playlist (the Pi is gone, or following was turned off).
    Normal,
    /// The attract playlist (the Pi is in a menu).
    Attract,
    /// A game started: run the sequence.
    Game {
        key: u32,
        /// The game's name as a card shows it.
        title: String,
        /// The console's name.
        console: String,
        /// The console's logo on the card, if the playlist has one.
        logo: Option<String>,
    },
    /// The game's context index is ready (`Ok`) or cannot be had (`Err`:
    /// no platform for the system, nothing of it in the playlist, a card
    /// error) — then the sequence ends on the attract playlist.
    Context {
        key: u32,
        ready: Result<Context, String>,
    },
}

/// A context playlist: `manifest` indexed by `index` (a header-valid
/// `.idx` for the console's filter, weights resolved in it).
#[derive(Clone, Debug)]
pub struct Context {
    pub manifest: String,
    pub index: String,
    pub entries: usize,
    pub matches: usize,
}

// ---------------------------------------------------------------------------
// The network half.

/// The follow settings, as the poller last saw them.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct Cfg {
    enabled: bool,
    host: String,
    code: String,
    /// The playlist the context index is built from: the saved one, or the
    /// schedule rule's.
    base_set: String,
}

impl Cfg {
    fn read(settings: &Mutex<Settings>, status: &Mutex<Status>) -> Cfg {
        let rule_set = status.lock().unwrap().rule_set.clone();
        let s = settings.lock().unwrap();
        Cfg {
            enabled: s.follow && !s.follow_host.trim().is_empty(),
            host: s.follow_host.trim().to_string(),
            code: s.follow_code.clone(),
            base_set: rule_set.unwrap_or_else(|| s.set.clone()),
        }
    }

    fn manifest(&self) -> String {
        nvs_config::manifest_path_for(&self.base_set)
    }

    /// The playlist's part of the cached files' names.
    fn label(&self) -> String {
        if self.base_set.is_empty() {
            "filelist".to_string()
        } else {
            self.base_set.replace(['.', ' '], "_")
        }
    }
}

/// The saved host (`name` or `name:port`) as a name and a port, RePlayOS's
/// unless it names one.
/// `None` for a host `PUT /api/config` would refuse (an empty name, a port
/// that is not a number).
fn host_port(host: &str) -> Option<(&str, u16)> {
    http_client::split_host_port(host)
        .map(|(name, port)| (name, port.unwrap_or(config::FOLLOW_PORT)))
}

/// For the log: what is polled.
fn base_url(host: &str) -> String {
    match host_port(host) {
        Some((name, port)) => format!("http://{name}:{port}/api/v1/"),
        None => format!("{host:?} (not a host)"),
    }
}

/// The poll's socket, read with a whole-exchange deadline: each read's
/// timeout is the time left (at most `FOLLOW_TIMEOUT`), so a server that
/// trickles a byte at a time cannot hold the network worker past it.
struct Deadlined {
    sock: TcpStream,
    deadline: Instant,
}

impl Read for Deadlined {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        // A zero timeout would mean none at all: at least 1 ms.
        let t = left
            .min(config::FOLLOW_TIMEOUT)
            .max(Duration::from_millis(1));
        self.sock.set_read_timeout(Some(t))?;
        self.sock.read(buf)
    }
}

impl Write for Deadlined {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sock.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.sock.flush()
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct VersionReply {
    version: String,
}

/// Why a poll did not produce a status, in the words the status shows
/// (the same as with `esp_http_client` before: `no answer`, `timed out`,
/// `no reply`, `connection dropped`).
enum PollError {
    /// The saved host is not `name[:port]` (cannot be saved any more).
    BadHost,
    /// The name did not resolve, or the connection was refused or timed out.
    Connect,
    Http(http_client::Error),
}

impl From<http_client::Error> for PollError {
    fn from(e: http_client::Error) -> Self {
        PollError::Http(e)
    }
}

impl std::fmt::Display for PollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PollError::BadHost => f.write_str("not a host name or address"),
            PollError::Connect => f.write_str("no answer"),
            PollError::Http(e) => e.fmt(f),
        }
    }
}

/// Reads through `inner`, sleeping a tick after every 4 KB: a manifest pass
/// on the network worker (priority 5) must leave CPU0's idle task and the
/// low-priority tasks (the stream task, the cache builder) their turn, or
/// the task watchdog fires.
struct Yielding<R> {
    inner: R,
    since: usize,
}

impl<R: Read> Read for Yielding<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.since += n;
        if self.since >= 4096 {
            self.since = 0;
            thread::sleep(Duration::from_millis(1));
        }
        Ok(n)
    }
}

fn yielding(file: File) -> BufReader<Yielding<File>> {
    BufReader::with_capacity(
        1024,
        Yielding {
            inner: file,
            since: 0,
        },
    )
}

/// The poller. Lives on the network worker; [`Poller::tick`] once per
/// worker loop (every second or so).
pub struct Poller {
    follow: Follow,
    epoch: Instant,
    cfg: Cfg,
    next_poll: Instant,
    key: u32,
    last_error: Option<String>,
    /// The logos file was found current for this `(manifest, length)`.
    logos_ok: Option<(String, u64)>,
    settings: Arc<Mutex<Settings>>,
    status: Arc<Mutex<Status>>,
    cmd: mpsc::Sender<Command>,
    // Heap and time accounting, for the heartbeat-style line every 5
    // minutes (since the last one).
    polls: u32,
    /// The deepest heap dip of a poll during which nothing else on the
    /// board is known to have allocated much (no GIF started, the cache
    /// builder down), and of the others. The dip is global free heap, so
    /// a GIF's decoder or the builder's encoder landing during a poll is
    /// counted too: that was the "43 KB per poll" in the night log.
    poll_dip_max: u32,
    poll_dip_busy_max: u32,
    poll_ms_sum: u32,
    poll_ms_max: u32,
    last_report: Instant,
}

impl Poller {
    pub fn new(
        settings: Arc<Mutex<Settings>>,
        status: Arc<Mutex<Status>>,
        cmd: mpsc::Sender<Command>,
    ) -> Self {
        Poller {
            follow: Follow::new(),
            epoch: Instant::now(),
            cfg: Cfg::default(),
            next_poll: Instant::now(),
            key: 0,
            last_error: None,
            logos_ok: None,
            settings,
            status,
            cmd,
            polls: 0,
            poll_dip_max: 0,
            poll_dip_busy_max: 0,
            poll_ms_sum: 0,
            poll_ms_max: 0,
            last_report: Instant::now(),
        }
    }

    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }

    /// How long the worker may block before the next poll is due.
    pub fn wait(&self) -> Duration {
        if !self.cfg.enabled {
            return Duration::from_secs(1);
        }
        self.next_poll
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(50))
    }

    /// Poll if it is time; act on what changed. `online`: the station is
    /// associated (nothing is tried in AP mode or while WiFi is down).
    // Not inlined into the network worker's loop (see `cache::run_count`).
    #[inline(never)]
    #[optimize(size)] // (CPU0, not on a frame's path: net-stall's flash)
    pub fn tick(&mut self, online: bool) {
        let cfg = Cfg::read(&self.settings, &self.status);
        if cfg != self.cfg {
            self.reconfigure(cfg);
        }
        if !self.cfg.enabled || !online {
            return;
        }
        let now = self.now();
        if let Some(a) = self.follow.tick(now) {
            log::info!(
                "follow: Cooldown -> {:?} ({} s without a game)",
                self.follow.state(),
                rules::GRACE.as_secs()
            );
            self.publish();
            self.act(a);
        }
        if Instant::now() < self.next_poll {
            return;
        }
        let before = self.follow.state();
        let polled = self.poll_status();
        let error = polled.as_ref().err().cloned();
        let action = self.follow.observe(self.now(), polled.as_ref().ok());
        self.next_poll = Instant::now() + self.follow.poll_interval();
        if error != self.last_error {
            match &error {
                Some(e) => log::warn!("follow: {} {e}", self.cfg.host),
                None if self.last_error.is_some() => {
                    log::info!("follow: {} answering again", self.cfg.host)
                }
                None => {}
            }
            self.last_error = error;
        }
        if self.follow.state() != before {
            log::info!(
                "follow: {:?} -> {:?}{}",
                before,
                self.follow.state(),
                self.follow
                    .game()
                    .map(|g| format!(" ({} '{}')", g.system, g.name))
                    .unwrap_or_default()
            );
        }
        self.publish();
        if let Some(a) = action {
            self.act(a);
        }
        if self.last_report.elapsed() >= config::FOLLOW_REPORT {
            self.last_report = Instant::now();
            log::info!(
                "follow: {} polls, per-poll heap dip max {} B ({} B while a GIF started or the cache builder ran), time avg {} ms max {} ms, heap {} B free",
                self.polls,
                self.poll_dip_max,
                self.poll_dip_busy_max,
                self.poll_ms_sum / self.polls.max(1),
                self.poll_ms_max,
                free_heap()
            );
            self.polls = 0;
            self.poll_dip_max = 0;
            self.poll_dip_busy_max = 0;
            self.poll_ms_sum = 0;
            self.poll_ms_max = 0;
        }
    }

    fn reconfigure(&mut self, cfg: Cfg) {
        let target_changed = cfg.host != self.cfg.host || cfg.code != self.cfg.code;
        let was = self.cfg.enabled;
        let base_changed = cfg.base_set != self.cfg.base_set;
        self.cfg = cfg;
        if target_changed || !self.cfg.enabled {
            self.last_error = None;
        }
        if base_changed {
            self.logos_ok = None;
        }
        let now = self.now();
        if self.cfg.enabled {
            if !was {
                log::info!("follow: on, polling {}", base_url(&self.cfg.host));
            } else if target_changed {
                log::info!("follow: now polling {}", base_url(&self.cfg.host));
                // Start again from "not reached".
                if let Some(a) = self.follow.set_enabled(false, now) {
                    self.act(a);
                }
            }
            self.follow.set_enabled(true, now);
            self.next_poll = Instant::now();
        } else if was {
            log::info!("follow: off");
            if let Some(a) = self.follow.set_enabled(false, now) {
                self.act(a);
            }
        }
        let mut s = SHARED.lock().unwrap();
        s.enabled = self.cfg.enabled;
        drop(s);
        self.publish();
    }

    fn publish(&self) {
        let mut s = SHARED.lock().unwrap();
        let state = self.follow.state();
        if s.state != state || s.since.is_none() {
            s.since = Some(Instant::now());
        }
        s.state = state;
        s.error = match state {
            State::Unreachable => self.last_error.clone(),
            _ => None,
        };
        // The `replay.*` inputs for Formulas (`inputs.rs`).
        crate::inputs::replay(state, self.follow.game());
        match self.follow.game() {
            Some(g) => {
                s.system = Some(g.system.clone());
                s.platform = g.platform;
                s.console = Some(rules::console_name_for_system(&g.system));
                s.game = Some(rules::clean_game_name(&g.name));
            }
            None => {
                s.system = None;
                s.platform = None;
                s.console = None;
                s.game = None;
                s.matches = None;
            }
        }
    }

    fn send(&self, event: Event) {
        let _ = self.cmd.send(Command::Follow(event));
    }

    #[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
    #[inline(never)]
    fn act(&mut self, action: Action) {
        match action {
            Action::Normal => self.send(Event::Normal),
            Action::Attract => {
                self.send(Event::Attract);
                self.ensure_logos();
            }
            Action::Game(game) => {
                self.key = self.key.wrapping_add(1);
                let key = self.key;
                let title = rules::clean_game_name(&game.name);
                let console = rules::console_name_for_system(&game.system);
                let logo = self.logo_for(&game);
                log::info!(
                    "follow: game #{key} '{title}' on {console} ({}), logo {}",
                    game.platform.unwrap_or("no platform"),
                    logo.as_deref().unwrap_or("none: console card")
                );
                self.send(Event::Game {
                    key,
                    title,
                    console,
                    logo,
                });
                let ready = self.build_context(&game);
                if let Ok(c) = &ready {
                    SHARED.lock().unwrap().matches = Some(c.matches);
                }
                self.send(Event::Context { key, ready });
                self.ensure_logos();
            }
        }
    }

    /// One GET on a connection of its own, closed after: the HTTP status;
    /// the body goes to `body` as it is read, 256 B at a time (nothing is
    /// buffered here). Nothing is kept between polls (a kept-alive
    /// `esp_http_client` held 3.35 KB of heap for a request every 2 s; this
    /// is a plain socket now, `opendmd_core::http_client`). Connecting,
    /// each write and each read time out after `FOLLOW_TIMEOUT`.
    #[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
    #[inline(never)]
    fn get(
        &mut self,
        host: &str,
        path: &str,
        token: Option<&str>,
        body: &mut dyn FnMut(&[u8]),
    ) -> Result<u16, String> {
        let started = Instant::now();
        let gifs_before = self.status.lock().map_or(0, |s| s.gifs_started);
        let builder_before = crate::cache_builder::task_up();
        let before = free_heap();
        let mut low = before;
        let (name, port) = host_port(host).unwrap_or(("", 0));
        let host_header = format!("{name}:{port}");
        let path = format!("/api/v1/{path}");
        let headers: &[(&str, &str)] = match token {
            Some(t) => &[("X-RePlay-Token", t)],
            None => &[],
        };
        crate::diag::check("follow: before a poll");
        let result = (|| {
            if name.is_empty() {
                return Err(PollError::BadHost);
            }
            let addr = (name, port)
                .to_socket_addrs()
                .ok()
                .and_then(|mut a| a.next())
                .ok_or(PollError::Connect)?;
            let sock = TcpStream::connect_timeout(&addr, config::FOLLOW_TIMEOUT)
                .map_err(|_| PollError::Connect)?;
            low = low.min(free_heap());
            sock.set_write_timeout(Some(config::FOLLOW_TIMEOUT))
                .map_err(|e| PollError::Http(e.into()))?;
            let deadline = Instant::now() + config::FOLLOW_DEADLINE;
            let mut sock = Deadlined { sock, deadline };
            let mut buf = [0u8; 256];
            let req = http_client::Get {
                host: &host_header,
                path: &path,
                headers,
                // A status reply is ~300 B; a body this long is not one,
                // and would only keep the worker (and the socket) busy.
                max_body: config::FOLLOW_MAX_BODY,
                deadline: Some(deadline),
            };
            let status = http_client::get(&mut sock, &req, &mut buf, body, &mut || {
                low = low.min(free_heap())
            })?;
            // `Connection: close`: give the server a moment to close first,
            // so the TIME_WAIT (2 x lwip's TCP_MSL) is its pcb, not one of
            // ours per poll. What this read returns does not matter.
            let _ = sock.sock.set_read_timeout(Some(Duration::from_millis(100)));
            let _ = sock.sock.read(&mut buf);
            Ok::<_, PollError>(status)
        })();
        crate::diag::check("follow: after a poll");
        let ms = started.elapsed().as_millis() as u32;
        self.polls += 1;
        self.poll_ms_sum = self.poll_ms_sum.saturating_add(ms);
        self.poll_ms_max = self.poll_ms_max.max(ms);
        // What the request took at its deepest (connection, buffers, socket,
        // body), all of it given back by now — or, if a GIF started or the
        // cache builder ran meanwhile, whatever they took too.
        let busy = builder_before
            || crate::cache_builder::task_up()
            || self.status.lock().map_or(0, |s| s.gifs_started) != gifs_before;
        let dip = before.saturating_sub(low);
        if busy {
            self.poll_dip_busy_max = self.poll_dip_busy_max.max(dip);
        } else {
            self.poll_dip_max = self.poll_dip_max.max(dip);
        }
        result.map_err(|e| e.to_string())
    }

    /// `get_status`, read through [`StatusScan`]: the reply is never held
    /// whole, whatever its size.
    fn get_status(
        &mut self,
        host: &str,
        code: &str,
    ) -> Result<(u16, Result<Observed, String>), String> {
        let mut scan = StatusScan::new();
        let status = self.get(host, "get_status", Some(code), &mut |b| scan.feed(b))?;
        Ok((
            status,
            scan.finish().map_err(|e| format!("bad reply ({e})")),
        ))
    }

    fn poll_status(&mut self) -> Result<Observed, String> {
        let host = self.cfg.host.clone();
        let code = self.cfg.code.clone();
        let (status, observed) = self.get_status(&host, &code)?;
        match status {
            200 => observed,
            401 => Err("401 (wrong code)".into()),
            n => Err(format!("HTTP {n}")),
        }
    }

    /// `POST /api/follow/test`: `get_version` (no code needed), then
    /// `get_status` with the code, against `host` (the saved one if empty)
    /// — nothing is saved or changed. A JSON object for the reply.
    // Not inlined into the network worker's loop (see `cache::run_count`).
    #[inline(never)]
    #[optimize(size)] // (CPU0, not on a frame's path: net-stall's flash)
    pub fn test(&mut self, host: &str, code: Option<&str>) -> String {
        let host = if host.trim().is_empty() {
            self.cfg.host.clone()
        } else {
            host.trim().to_string()
        };
        let code = code.map_or_else(|| self.cfg.code.clone(), str::to_string);
        if host.is_empty() {
            return r#"{"ok":false,"reachable":false,"error":"no host"}"#.to_string();
        }
        // `get_version` is a few dozen bytes: kept (at most 1 KB) and parsed.
        let mut body = Vec::new();
        let version = match self.get(&host, "get_version", None, &mut |b| {
            if body.len() + b.len() <= 1024 {
                body.extend_from_slice(b);
            }
        }) {
            Ok(200) => serde_json::from_slice::<VersionReply>(&body)
                .map(|v| v.version)
                .unwrap_or_default(),
            Ok(n) => {
                return format!(
                    r#"{{"ok":false,"reachable":true,"error":{}}}"#,
                    json_string(&format!("get_version: HTTP {n}"))
                )
            }
            Err(e) => {
                return format!(
                    r#"{{"ok":false,"reachable":false,"error":{}}}"#,
                    json_string(&e)
                )
            }
        };
        let (status, observed) = match self.get_status(&host, &code) {
            Ok(r) => r,
            Err(e) => {
                return format!(
                    r#"{{"ok":false,"reachable":true,"version":{},"error":{}}}"#,
                    json_string(&version),
                    json_string(&e)
                )
            }
        };
        let (r, error) = match (status, observed) {
            (200, Ok(o)) => (o, None),
            (200, Err(e)) => (Observed::default(), Some(e)),
            (401, _) => (Observed::default(), Some("401 (wrong code)".to_string())),
            (n, _) => (Observed::default(), Some(format!("HTTP {n}"))),
        };
        format!(
            r#"{{"ok":{},"reachable":true,"version":{},"status":{},"view":{},"system":{},"game":{},"error":{}}}"#,
            error.is_none(),
            json_string(&version),
            status,
            json_string(&r.view),
            json_string(&r.system),
            json_string(&rules::clean_game_name(&r.game_name)),
            error.map_or_else(|| "null".to_string(), |e| json_string(&e)),
        )
    }

    // --- The context index ---------------------------------------------

    fn build_context(&mut self, game: &Game) -> Result<Context, String> {
        let Some(platform) = game.platform else {
            return Err(format!("no platform for system {}", game.system));
        };
        let manifest = self.cfg.manifest();
        let len = fs::metadata(&manifest)
            .map_err(|e| format!("{manifest}: {e}"))?
            .len() as u32;
        fs::create_dir_all(config::FOLLOW_CTX_DIR)
            .map_err(|e| format!("{}: {e}", config::FOLLOW_CTX_DIR))?;
        let tokens = rules::tokens(&game.name);
        let key = opendmd_core::layout::fnv1a32(tokens.join(" ").as_bytes());
        let index = format!(
            "{}/{}.{}.{key:08x}.idx",
            config::FOLLOW_CTX_DIR,
            self.cfg.label(),
            game.system.replace(['.', '/', ' '], "_")
        );
        let filter = Filter::new().platforms([platform]);
        let fp = rules::context_fingerprint(platform, &game.name);
        let started = Instant::now();

        // A current index: instant.
        if let Ok(mut f) = File::open(&index) {
            if let Ok(h) = IndexHeader::read(&mut f) {
                if h.is_valid_for(len, &filter, fp) && h.count > 0 {
                    drop(f);
                    touch(&index);
                    log::info!(
                        "follow: context for '{}' loaded from {index} in {} ms: {} entries, {} weighted",
                        game.name,
                        started.elapsed().as_millis(),
                        h.count,
                        h.override_count
                    );
                    return Ok(Context {
                        manifest,
                        index,
                        entries: h.count as usize,
                        matches: usize::from(h.override_count),
                    });
                }
            }
        }

        // One pass over the manifest, here on CPU0, while the panel plays.
        let misses_before = self.status.lock().unwrap().deadline_misses;
        let heap_before = free_heap();
        let mut heap_low = heap_before;
        let mut weights = None;
        let mut seen = 0u32;
        let tmp = format!("{index}.tmp");
        let built = File::open(&manifest)
            .and_then(|m| Ok((m, File::create(&tmp)?)))
            .and_then(|(m, out)| {
                write_index_resolving_then(
                    yielding(m),
                    len,
                    &filter,
                    fp,
                    |e: &Entry| {
                        seen += 1;
                        if seen.is_multiple_of(32) {
                            heap_low = heap_low.min(free_heap());
                        }
                        rules::classify(rules::score_tokens(&rules::tokens(&e.title), &tokens))
                            .map(rules::Match::mark)
                    },
                    |count, ov| weights = Some(rules::weigh_context(count, ov)),
                    out,
                )
            });
        heap_low = heap_low.min(free_heap());
        let (header, ov) = match built {
            Ok(b) => b,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(format!("building {index}: {e}"));
            }
        };
        crate::files::replace(&tmp, &index).map_err(|e| format!("{e:#}"))?;
        let ms = started.elapsed().as_millis();
        let misses = self
            .status
            .lock()
            .unwrap()
            .deadline_misses
            .saturating_sub(misses_before);
        let w = weights.unwrap_or_else(|| rules::ContextWeights::for_counts(header.count, 0, 0));
        let [ss, rs, os] = w.shares_ppm();
        log::info!(
            "follow: context for '{}' built in {ms} ms on CPU0: {} of the playlist's entries are {platform}, \
             {} weighted ({} strong x{}, {} related x{}{}, no-repeat window {}: {}/{}/{}% of draws); \
             heap {heap_before} B free before, lowest {heap_low} B during ({} B below), \
             panel deadline misses +{misses} meanwhile; {index}",
            game.name,
            header.count,
            ov.len(),
            w.n_strong,
            w.strong,
            w.n_related,
            w.related,
            if w.capped { ", capped" } else { "" },
            w.window,
            (ss + 5_000) / 10_000,
            (rs + 5_000) / 10_000,
            (os + 5_000) / 10_000,
            heap_before.saturating_sub(heap_low),
        );
        cap_dir();
        if header.count == 0 {
            return Err(format!("no {platform} entries in {manifest}"));
        }
        Ok(Context {
            manifest,
            index,
            entries: header.count as usize,
            matches: ov.len(),
        })
    }

    // --- Logos -----------------------------------------------------------

    fn logos_path(&self) -> String {
        format!("{}/{}.logos", config::FOLLOW_CTX_DIR, self.cfg.label())
    }

    /// Make sure the logos file is current for the playlist: every followed
    /// console's logo entries, `PLATFORM<TAB>path`, after a `#<length>` line.
    /// One pass over the manifest when it is missing or stale.
    #[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
    #[inline(never)]
    fn ensure_logos(&mut self) {
        let manifest = self.cfg.manifest();
        let Ok(len) = fs::metadata(&manifest).map(|m| m.len()) else {
            return;
        };
        if self.logos_ok.as_ref() == Some(&(manifest.clone(), len)) {
            return;
        }
        let path = self.logos_path();
        let current = File::open(&path).ok().and_then(|f| {
            let mut line = String::new();
            BufReader::with_capacity(64, f).read_line(&mut line).ok()?;
            (line.trim() == format!("#{len}")).then_some(())
        });
        if current.is_none() {
            let started = Instant::now();
            let tmp = format!("{path}.tmp");
            let mut count = 0usize;
            let written = fs::create_dir_all(config::FOLLOW_CTX_DIR)
                .and_then(|()| File::open(&manifest))
                .and_then(|m| {
                    let mut out = BufWriter::with_capacity(512, File::create(&tmp)?);
                    writeln!(out, "#{len}")?;
                    let mut failed = None;
                    manifest::scan(yielding(m), |_, e| {
                        let Some(p) = e.platform.as_deref() else {
                            return;
                        };
                        if failed.is_none() && followed_platform(p) && rules::is_logo(e) {
                            count += 1;
                            if let Err(err) = writeln!(out, "{p}\t{}", e.path) {
                                failed = Some(err);
                            }
                        }
                    })?;
                    if let Some(e) = failed {
                        return Err(e);
                    }
                    out.flush()
                });
            if let Err(e) = written
                .map_err(anyhow::Error::from)
                .and_then(|()| crate::files::replace(&tmp, &path))
            {
                let _ = fs::remove_file(&tmp);
                log::warn!("follow: cannot write {path}: {e:#}");
                return;
            }
            log::info!(
                "follow: {count} console logos indexed from {manifest} in {} ms",
                started.elapsed().as_millis()
            );
        }
        self.logos_ok = Some((manifest, len));
    }

    /// The logo to show before `game` (see `choose_logo`), if the logos file
    /// is current.
    fn logo_for(&mut self, game: &Game) -> Option<String> {
        let platform = game.platform?;
        let manifest = self.cfg.manifest();
        let len = fs::metadata(&manifest).ok()?.len();
        let f = File::open(self.logos_path()).ok()?;
        let mut lines = BufReader::with_capacity(512, f).lines();
        if lines.next()?.ok()?.trim() != format!("#{len}") {
            return None;
        }
        let candidates: Vec<(String, String)> = lines
            .map_while(Result::ok)
            .filter_map(|l| {
                let (p, path) = l.split_once('\t')?;
                (p == platform).then(|| (Entry::from_path(path).title, path.to_string()))
            })
            .collect();
        rules::choose_logo(&candidates, &game.name).map(str::to_string)
    }
}

fn followed_platform(p: &str) -> bool {
    p == "ARCADE" || rules::SYSTEMS.iter().any(|(_, q)| *q == p)
}

/// Set a file's modification time to now (the cache directory is trimmed
/// oldest-first).
fn touch(path: &str) {
    if let Ok(c) = std::ffi::CString::new(path) {
        // SAFETY: a valid C string; a null `times` means "now".
        unsafe {
            esp_idf_svc::sys::utime(c.as_ptr(), std::ptr::null());
        }
    }
}

/// Keep at most [`config::FOLLOW_CTX_MAX`] context indexes: delete the
/// oldest by modification time.
fn cap_dir() {
    let Ok(dir) = fs::read_dir(config::FOLLOW_CTX_DIR) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = dir
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".idx"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().unwrap_or(std::time::UNIX_EPOCH), e.path()))
        })
        .collect();
    if files.len() <= config::FOLLOW_CTX_MAX {
        return;
    }
    // Unstable (the paths differ, so the order is the same): the stable
    // sort keeps a 4 KB scratch buffer on the network worker's stack.
    files.sort_unstable();
    let extra = files.len() - config::FOLLOW_CTX_MAX;
    for (_, p) in files.into_iter().take(extra) {
        match fs::remove_file(&p) {
            Ok(()) => log::info!("follow: dropped old context {}", p.display()),
            Err(e) => log::warn!("follow: cannot remove {}: {e}", p.display()),
        }
    }
}

// ---------------------------------------------------------------------------
// The main-task half.

/// What the panel is following.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Attract,
    Game,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    PowerOn,
    Logo,
    Title,
    /// The title card is held until the context index is ready.
    Waiting,
}

impl Step {
    fn name(self) -> &'static str {
        match self {
            Step::PowerOn => "power_on",
            Step::Logo => "logo",
            Step::Title => "title",
            Step::Waiting => "waiting",
        }
    }
}

struct GameCard {
    title: String,
    console: String,
    logo: Option<String>,
}

/// What [`Runner`] needs the main loop to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Todo {
    Nothing,
    /// The playlist overlay changed: rebuild with [`Runner::overlay`] (or
    /// the context, [`Runner::context`]).
    Rebuild,
}

/// The main task's side: the overlay and the sequence.
pub struct Runner {
    mode: Mode,
    key: u32,
    game: Option<GameCard>,
    step: Option<Step>,
    step_at: Instant,
    /// The context for `key`: `Some(Ok)` ready, `Some(Err)` will not come.
    ready: Option<Result<Context, String>>,
    /// The engine is playing the context index.
    on_context: bool,
    /// Night mode had the panel dark when the game started: pick it up when
    /// it lights, without the sequence.
    deferred: bool,
    /// The attract playlist would not build: ignored until the mode changes.
    attract_broken: bool,
    timings: Vec<(&'static str, u128)>,
    started: Instant,
}

impl Default for Runner {
    fn default() -> Self {
        Runner {
            mode: Mode::Normal,
            key: 0,
            game: None,
            step: None,
            step_at: Instant::now(),
            ready: None,
            on_context: false,
            deferred: false,
            attract_broken: false,
            timings: Vec::new(),
            started: Instant::now(),
        }
    }
}

impl Runner {
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The settings to build the playlist from: the attract playlist in
    /// place of `set` in attract mode (and in game mode until the context is
    /// on, which is what plays if it never comes).
    /// [`overlay`](Self::overlay)'s playlist alone, borrowed: `set` (the
    /// schedule's) or the attract playlist of `settings`.
    pub fn overlay_set<'a>(&self, settings: &'a Settings, set: &'a str) -> &'a str {
        match &settings.attract_set {
            Some(a) if self.mode != Mode::Normal && !self.attract_broken => a,
            _ => set,
        }
    }

    pub fn overlay(&self, mut eff: Settings) -> Settings {
        if self.mode != Mode::Normal && !self.attract_broken {
            if let Some(set) = &eff.attract_set {
                eff.set = set.clone();
            }
        }
        eff
    }

    /// The attract playlist would not build: play without
    /// the overlay.
    pub fn attract_failed(&mut self) {
        self.attract_broken = true;
    }

    /// The context playlist the engine should be on, if any.
    pub fn context(&self) -> Option<&Context> {
        match (&self.ready, self.on_context) {
            (Some(Ok(c)), true) => Some(c),
            _ => None,
        }
    }

    /// The context would not open: back to the overlay's playlist.
    pub fn context_failed(&mut self) {
        self.on_context = false;
        self.ready = None;
    }

    /// A sequence is on the panel.
    pub fn in_sequence(&self) -> bool {
        self.step.is_some()
    }

    /// An event from the network side. `dark`: night mode has the panel
    /// off. Starts the sequence on `engine` for a game.
    pub fn handle(
        &mut self,
        event: Event,
        engine: Option<&mut Engine>,
        dark: bool,
        now: Duration,
    ) -> Todo {
        match event {
            Event::Normal | Event::Attract => {
                let mode = if matches!(event, Event::Normal) {
                    Mode::Normal
                } else {
                    Mode::Attract
                };
                let was_context = self.on_context;
                let changed = mode != self.mode || was_context || self.step.is_some();
                self.end_sequence(engine);
                self.mode = mode;
                self.game = None;
                self.ready = None;
                self.on_context = false;
                self.deferred = false;
                self.attract_broken = false;
                log::info!("follow: panel -> {mode:?}");
                if changed {
                    Todo::Rebuild
                } else {
                    Todo::Nothing
                }
            }
            Event::Game {
                key,
                title,
                console,
                logo,
            } => {
                self.end_sequence(None);
                let was_context = self.on_context;
                self.mode = Mode::Game;
                self.key = key;
                self.ready = None;
                self.on_context = false;
                self.game = Some(GameCard {
                    title,
                    console,
                    logo,
                });
                self.started = Instant::now();
                self.timings.clear();
                match engine {
                    Some(e) if !dark => {
                        e.set_hold(true);
                        let once = once();
                        match e.play_now_with(synth::POWER_ON, now, Some(once)) {
                            Ok(()) => self.enter(Step::PowerOn),
                            Err(err) => log::warn!("follow: power-on effect: {err}"),
                        }
                        self.deferred = false;
                        Todo::Nothing
                    }
                    _ => {
                        // Dark (or no playlist): no sequence; the context
                        // when it comes.
                        self.deferred = true;
                        if was_context {
                            Todo::Rebuild
                        } else {
                            Todo::Nothing
                        }
                    }
                }
            }
            Event::Context { key, ready } => {
                if key != self.key || self.mode != Mode::Game {
                    return Todo::Nothing;
                }
                match &ready {
                    Ok(c) => log::info!(
                        "follow: context #{key} ready ({} entries, {} weighted) at +{} ms",
                        c.entries,
                        c.matches,
                        self.started.elapsed().as_millis()
                    ),
                    Err(e) => log::warn!("follow: no context for game #{key}: {e}"),
                }
                self.ready = Some(ready);
                // Waiting for it (or no sequence at all): swap now.
                if matches!(self.step, Some(Step::Waiting))
                    || (self.step.is_none() && !self.deferred)
                {
                    return self.finish_sequence(engine);
                }
                Todo::Nothing
            }
        }
    }

    /// Night mode lit the panel again: a game that started in the dark
    /// goes straight to its context.
    pub fn resume(&mut self, engine: Option<&mut Engine>) -> Todo {
        if !self.deferred || self.mode != Mode::Game {
            return Todo::Nothing;
        }
        self.deferred = false;
        if self.ready.is_some() {
            self.finish_sequence(engine)
        } else {
            Todo::Nothing
        }
    }

    /// Between frames: move the sequence on when the engine has finished
    /// the step on screen.
    pub fn advance(&mut self, engine: &mut Engine, now: Duration) -> Todo {
        let Some(step) = self.step else {
            return Todo::Nothing;
        };
        if !engine.is_waiting() {
            return Todo::Nothing;
        }
        let Some(game) = self.game.as_ref() else {
            return self.finish_sequence(Some(engine));
        };
        match step {
            Step::PowerOn => {
                let logo = game.logo.clone();
                let played =
                    logo.as_deref()
                        .is_some_and(|p| match engine.play_now_with(p, now, None) {
                            Ok(()) => true,
                            Err(e) => {
                                log::warn!("follow: logo {p}: {e}");
                                false
                            }
                        });
                if !played {
                    let card = synth::card_path(&game.console, "");
                    let _ = engine.play_now_with(&card, now, Some(once()));
                }
                self.enter(Step::Logo);
                Todo::Nothing
            }
            Step::Logo => {
                let card = synth::card_path(&game.title, &game.console);
                let _ = engine.play_now_with(&card, now, Some(once()));
                self.enter(Step::Title);
                Todo::Nothing
            }
            Step::Title => {
                if self.ready.is_some() {
                    self.finish_sequence(Some(engine))
                } else {
                    self.enter(Step::Waiting);
                    Todo::Nothing
                }
            }
            Step::Waiting => {
                if self.step_at.elapsed() >= config::FOLLOW_CONTEXT_WAIT {
                    log::warn!(
                        "follow: no context after {} s; playing on",
                        config::FOLLOW_CONTEXT_WAIT.as_secs()
                    );
                    engine.set_hold(false);
                    self.close_steps();
                }
                Todo::Nothing
            }
        }
    }

    fn enter(&mut self, step: Step) {
        if let Some(prev) = self.step {
            self.timings
                .push((prev.name(), self.step_at.elapsed().as_millis()));
        }
        self.step = Some(step);
        self.step_at = Instant::now();
        set_step(Some(step.name()));
    }

    fn close_steps(&mut self) {
        if let Some(prev) = self.step.take() {
            self.timings
                .push((prev.name(), self.step_at.elapsed().as_millis()));
            let parts: Vec<String> = self
                .timings
                .iter()
                .map(|(n, ms)| format!("{n} {ms} ms"))
                .collect();
            log::info!(
                "follow: sequence #{} done in {} ms: {}",
                self.key,
                self.started.elapsed().as_millis(),
                parts.join(", ")
            );
        }
        set_step(None);
    }

    /// The sequence is over: to the context if it is ready, else release
    /// the hold (the overlay's playlist plays on).
    fn finish_sequence(&mut self, engine: Option<&mut Engine>) -> Todo {
        self.close_steps();
        match &self.ready {
            Some(Ok(_)) => {
                self.on_context = true;
                Todo::Rebuild
            }
            _ => {
                if let Some(e) = engine {
                    e.set_hold(false);
                }
                Todo::Nothing
            }
        }
    }

    /// Drop a sequence in progress (another event took over).
    fn end_sequence(&mut self, engine: Option<&mut Engine>) {
        if self.step.is_some() {
            log::info!("follow: sequence #{} interrupted", self.key);
            self.close_steps();
        }
        if let Some(e) = engine {
            e.set_hold(false);
        }
    }
}

fn once() -> Override {
    Override {
        loops: Some(1),
        ..Override::default()
    }
}
