//! The HTTP surface: the web app out of flash (`www.rs`), the card's files,
//! and a small JSON API.
//!
//! Route shapes and the captive-portal probe list come from
//! `~/projects/stripe-dashboard/src/web_portal.rs`; the file server and the
//! command plumbing are ours. Everything here runs on the httpd task, which is
//! pinned to CPU0 (see [`crate::httpd`]) and may block — except the card-file
//! `GET`s and the uploads, which are produced and received on the streaming
//! task ([`crate::stream`]) so that one transfer cannot hold up the API. Anything
//! that would touch the engine, the panel or a file the player has open is
//! sent to the main task as a [`Command`] and waited on.
//!
//! Registration order is load-bearing. ESP-IDF prefers an exact match, but
//! among wildcards the *earliest* registration wins, so `/*` goes last.

use crate::chan as mpsc;
use anyhow::Result;
use log::info;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config;
use crate::files;
use crate::httpd::{json_string, query_param, Method, Req, Respond, Server, Upload};
use crate::nvs_config::Settings;
use crate::state::{Applied, Command, PlayError, Status, SET_NOT_FOUND};
use crate::stream;
use crate::wifi::ScannedAp;
use crate::www;
use opendmd_core::compositor::{self, Kind as TransitionKind};
use opendmd_core::panel_timing::{self, PanelOp, PanelPatch};

/// The embedded setup page: rendered and gzipped by `build.rs`, served at
/// `/setup` always, and at `/` in AP mode when there is no good web app in
/// flash (so the captive portal still configures WiFi).
const SETUP_HTML_GZ: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/setup.html.gz"));

/// How long a handler waits for the main task to answer. The display loop
/// drains its channel every frame, so this only ever expires if the player is
/// wedged — in which case saying so beats hanging the browser.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a handler waits for a command that may rebuild the playlist index:
/// a full pass over the stock card's 450 KB manifest plus writing the 40 KB
/// `.idx` takes several seconds over SPI, longer while files are being served.
const REINDEX_TIMEOUT: Duration = Duration::from_secs(60);
/// A WiFi scan takes a couple of seconds and blocks the worker.
const SCAN_TIMEOUT: Duration = Duration::from_secs(12);
/// Two requests with 3 s timeouts, possibly behind a context index build
/// (~5 s) on the network worker.
const FOLLOW_TEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Cap on JSON request bodies. Uploads are streamed and are not subject to it.
const MAX_JSON_BODY: usize = 8192;

/// Wire format for `POST /api/play`.
#[derive(serde::Deserialize)]
struct PlayRequest {
    path: String,
}

/// Requests the network worker answers on behalf of a handler.
pub enum NetCommand {
    Scan(mpsc::Sender<Vec<ScannedAp>>),
    /// Count the playback cache (`cache::run_count`); no reply, the result
    /// lands in `cache`'s state for `GET /api/cache`.
    CountCache,
    /// `POST /api/follow/test`: ask the RePlayOS box at `host` (the saved one
    /// if empty) for its version and, with `code` (the saved one if `None`),
    /// its status. The reply is the JSON object to send.
    FollowTest {
        host: String,
        code: Option<String>,
        reply: mpsc::Sender<String>,
    },
    /// `PUT /api/firmware`: check a bundle header's Ed25519 signature
    /// (`ota::verify_signature`). Here because the network worker's 12 KB
    /// stack has room for it (~8.5 KB at most, measured from the ELF's
    /// frames) and the stream task's 6 KB does not.
    VerifyBundle {
        header: Box<[u8; opendmd_core::ota::HEADER_LEN]>,
        reply: mpsc::Sender<Result<opendmd_core::ota::Meta, opendmd_core::ota::OtaError>>,
    },
}

/// Wire format for `POST /api/follow/test`.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct FollowTestRequest {
    host: String,
    code: Option<String>,
}

/// `null` and absent told apart: absent is `None`, `null` is `Some(None)`.
fn double_option<'de, D>(d: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    <Option<String> as serde::Deserialize>::deserialize(d).map(Some)
}

/// `panel`: absent, `null` (the defaults) or an object.
fn double_option_panel<'de, D>(d: D) -> Result<Option<Option<PanelWire>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    <Option<PanelWire> as serde::Deserialize>::deserialize(d).map(Some)
}

/// `PUT /api/config`'s `panel` object (`opendmd_core::panel_timing`): any
/// subset; an unknown key is a 400, so a typo never passes for a trial.
/// Numbers are read wide so a value out of range gets the range message.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PanelWire {
    clk_phase: Option<bool>,
    latch_blanking: Option<i64>,
    driver: Option<String>,
    i2s_speed: Option<i64>,
    min_refresh: Option<i64>,
}

/// Wire format for `POST /api/files/move`: rename a GIF (and what follows
/// it by name).
#[derive(serde::Deserialize)]
struct MoveRequest {
    from: String,
    to: String,
}

/// Wire format for `POST /api/cache/forget`.
#[derive(serde::Deserialize)]
struct ForgetRequest {
    path: String,
}

/// Wire format for `POST /api/cache/failed`: a GIF the browser could not
/// prepare either, and why.
#[derive(serde::Deserialize)]
struct FailedRequest {
    path: String,
    #[serde(default)]
    reason: String,
}

/// Everything the handlers share. One `Arc` per closure.
pub struct Ctx {
    pub status: Arc<Mutex<Status>>,
    pub settings: Arc<Mutex<Settings>>,
    pub cmd: mpsc::Sender<Command>,
    pub net: mpsc::Sender<NetCommand>,
}

/// Wire format for `PUT /api/config`. Every field is optional: what is absent
/// is left as it was.
#[derive(serde::Deserialize)]
struct ConfigPatch {
    wifi_ssid: Option<String>,
    wifi_pass: Option<String>,
    brightness: Option<u8>,
    /// Read wider than `u8` so an out-of-range value gets the range message.
    max_brightness: Option<u32>,
    dwell_min: Option<u64>,
    dwell_max: Option<u64>,
    shuffle: Option<bool>,
    platforms: Option<String>,
    set: Option<String>,
    hostname: Option<String>,
    tz: Option<String>,
    tz_name: Option<String>,
    transition: Option<String>,
    transition_ms: Option<u32>,
    auto_prepare: Option<String>,
    follow: Option<bool>,
    follow_host: Option<String>,
    /// Never echoed back; `""` clears it.
    follow_code: Option<String>,
    /// `null` = the current playlist, else a `set` value.
    #[serde(default, deserialize_with = "double_option")]
    attract_set: Option<Option<String>>,
    /// `""`, `"clock"`, `"text:<message>"` or `"fx:<effect>[?options]"`.
    attract: Option<String>,
    look: Option<String>,
    glow: Option<String>,
    boot_show: Option<String>,
    /// The API key (`opendmd_core::access`); `""` turns it off. Never
    /// echoed back (only `api_key_set`).
    api_key: Option<String>,
    /// Panel timing: an object is tried (reverts unless confirmed), `null`
    /// saves the defaults (`PanelOp`).
    #[serde(default, deserialize_with = "double_option_panel")]
    panel: Option<Option<PanelWire>>,
    /// `true` keeps the trial (with `panel`: saved without one), `false`
    /// undoes it.
    panel_confirm: Option<bool>,
}

/// Wire format for `POST /api/card/format`: `{"confirm": true}`, so an
/// empty or stray POST never erases a card.
#[derive(serde::Deserialize)]
struct FormatRequest {
    #[serde(default)]
    confirm: bool,
}

/// Wire format for `POST /api/wake`.
#[derive(serde::Deserialize)]
struct WakeRequest {
    minutes: u32,
}

/// Start the server and register every route.
///
/// Called by the boot task (`boot.rs`) once the card's playlist is open and
/// before it hands the card over, *not* from the network worker, and
/// deliberately so: the httpd task's stack (and the
/// streaming task's, started here too) is one contiguous internal allocation,
/// and by the time WiFi has associated the first GIF is already decoding and
/// the largest free block has fallen below it (`httpd_start` then fails with
/// `ESP_ERR_HTTPD_TASK`, 0xb008 — measured on this board). Starting here,
/// before the first GIF, takes the allocation while the heap is still
/// whole. `core_id` is in the config, so the server runs on CPU0 however it
/// was started.
///
/// That means the mode is not known yet, so the captive-portal routes are
/// always registered and read the live WiFi state out of [`Status`] when a
/// request arrives.
pub fn start(ctx: Arc<Ctx>) -> Result<Server> {
    // URI handler slots: one per `route`/`route_json` below (46 now, the
    // captive-portal loop counting six; 47 leaves one spare). Registration
    // past the limit fails the whole server, so count when adding a route.
    let mut server = Server::start(0, config::HTTP_STACK, config::HTTP_MAX_SOCKETS, 47)?;
    // The API key, if one is set (`api_key`; `httpd::gate` checks it).
    crate::httpd::set_api_key(&ctx.settings.lock().unwrap().api_key);

    // --- The web app, and the setup page ------------------------------------
    // Every file goes out from the streaming task (`stream`), so a big
    // download or a stalled client never holds up the API. The web app comes
    // out of the www pack in flash (`www.rs`), never the card.
    stream::start().unwrap_or_else(|e| {
        log::error!("stream task failed to start ({e:#}); files are served on the httpd task")
    });

    let c = ctx.clone();
    server.route("/", Method::Get, move |req| {
        // In AP mode it's the setup page, so the captive portal (and any
        // address typed while joined to the setup network) lands on getting
        // onto WiFi. The web app is still at /index.html there.
        if portal_target(&c).is_some() {
            return send_setup(req);
        }
        stream::hand_off(req, move |req| match www::serve(req, "index.html")? {
            www::Served::Sent => Ok(()),
            www::Served::NotFound => {
                www::unavailable(req, "the web app in flash has no index.html")
            }
            www::Served::Unavailable(why) => www::unavailable(req, &why),
        })
    })?;
    server.route("/setup", Method::Get, send_setup)?;

    // --- JSON API ----------------------------------------------------------
    let c = ctx.clone();
    server.route("/api/status", Method::Get, move |req| {
        // `diag-heap` only: `?diag_chan=N&impl=std|ours` makes N channels
        // here first (see `diag::channel_torture`).
        #[cfg(feature = "diag-heap")]
        if let Some(n) = query_param(req.query(), "diag_chan").and_then(|n| n.parse().ok()) {
            let which = query_param(req.query(), "impl").unwrap_or_default();
            let line = crate::diag::channel_torture(n, &which);
            return req.ok("text/plain", line.as_bytes());
        }
        // The settings' part first (never both locks at once).
        let looks = {
            let set = c.settings.lock().unwrap();
            looks_json(&set)
        };
        let recent = query_param(req.query(), "recent").as_deref() == Some("1");
        let json = {
            let s = c.status.lock().unwrap();
            status_json(&s, &looks, recent)
        };
        req.json(&json)
    })?;

    let c = ctx.clone();
    server.route("/api/skip", Method::Post, move |req| {
        // `?back=1`: Previous, one step back through Recently played.
        if query_param(req.query(), "back").as_deref() != Some("1") {
            let _ = c.cmd.send(Command::Skip);
            return req.json(r#"{"ok":true}"#);
        }
        let (tx, rx) = mpsc::channel();
        if c.cmd.send(Command::Back(tx)).is_err() {
            return req.error(503, "503 Service Unavailable", "player not running");
        }
        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(Ok(())) => req.json(r#"{"ok":true}"#),
            Ok(Err(PlayError::NotFound(e))) => req.not_found(&e),
            Ok(Err(PlayError::Unplayable(e))) => req.error(415, "415 Unsupported Media Type", &e),
            Ok(Err(PlayError::NoPlaylist)) => {
                req.error(503, "503 Service Unavailable", "no playlist is running")
            }
            Err(_) => req.error(504, "504 Gateway Timeout", "player did not answer"),
        }
    })?;

    let c = ctx.clone();
    server.route("/api/pause", Method::Post, move |req| {
        let (tx, rx) = mpsc::channel();
        if c.cmd.send(Command::TogglePause(tx)).is_err() {
            return req.error(503, "503 Service Unavailable", "player not running");
        }
        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(paused) => req.json(&format!(r#"{{"ok":true,"paused":{paused}}}"#)),
            Err(_) => req.error(504, "504 Gateway Timeout", "player did not answer"),
        }
    })?;

    let c = ctx.clone();
    server.route("/api/config", Method::Get, move |req| {
        let mut json = {
            let s = c.settings.lock().unwrap();
            settings_json(&s)
        };
        json.pop(); // the closing brace: the panel keys go in too
        json.push(',');
        json.push_str(&panel_json(&c.status.lock().unwrap()));
        json.push('}');
        req.json(&json)
    })?;

    let c = ctx.clone();
    server.route_json("/api/config", Method::Put, move |req| {
        let Some(body) = json_body(req)? else {
            return Ok(());
        };
        let patch: ConfigPatch = match serde_json::from_slice(&body) {
            Ok(p) => p,
            Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
        };
        let mut next = c.settings.lock().unwrap().clone();
        let wifi_before = (next.wifi_ssid.clone(), next.wifi_pass.clone());
        let hostname_before = next.hostname.clone();
        if let Some(v) = patch.wifi_ssid {
            next.wifi_ssid = v;
        }
        if let Some(v) = patch.wifi_pass {
            next.wifi_pass = v;
        }
        match opendmd_core::brightness::resolve(
            next.brightness,
            next.max_brightness,
            patch.brightness,
            patch.max_brightness,
        ) {
            Ok((b, m)) => {
                next.brightness = b;
                next.max_brightness = m;
            }
            Err(e) => return req.bad_request(&e),
        }
        if let Some(v) = patch.dwell_min {
            next.dwell_min = v.max(1);
        }
        if let Some(v) = patch.dwell_max {
            next.dwell_max = v.max(1);
        }
        if let Some(v) = patch.shuffle {
            next.shuffle = v;
        }
        if let Some(v) = patch.platforms {
            next.platforms = v;
        }
        if let Some(v) = patch.set {
            if !crate::nvs_config::valid_set_name(&v) {
                return req.bad_request("set must be a single path segment");
            }
            next.set = v;
        }
        if let Some(v) = patch.hostname {
            let v = v.trim().to_ascii_lowercase();
            if !v.is_empty() {
                if !crate::nvs_config::valid_hostname(&v) {
                    return req.bad_request(&format!(
                        "hostname must be 1-{} lowercase letters, digits and hyphens, not starting or ending with a hyphen",
                        crate::nvs_config::HOSTNAME_MAX
                    ));
                }
                next.hostname = v;
            }
        }
        if let Some(v) = patch.tz {
            let v = v.trim().to_string();
            if v.is_empty() || v.len() > 64 || v.contains('\0') {
                return req
                    .bad_request("tz must be a POSIX TZ string, e.g. GMT0IST,M3.5.0/1,M10.5.0");
            }
            next.tz = v;
        }
        if let Some(v) = patch.tz_name {
            if v.len() > 64 {
                return req.bad_request("tz_name is at most 64 characters");
            }
            next.tz_name = v;
        }
        if let Some(v) = patch.transition {
            let Some(kind) = TransitionKind::from_name(&v) else {
                return req.bad_request(
                    "transition must be dissolve, wipe_left, wipe_right, wipe_down, wipe_up, blinds, push, fade or none",
                );
            };
            next.transition.kind = kind;
        }
        if let Some(ms) = patch.transition_ms {
            let range = compositor::MIN_DURATION.as_millis()..=compositor::MAX_DURATION.as_millis();
            if !range.contains(&u128::from(ms)) {
                return req.bad_request(&format!(
                    "transition_ms must be {}..={}",
                    range.start(),
                    range.end()
                ));
            }
            next.transition.duration = Duration::from_millis(u64::from(ms));
        }
        if let Some(v) = patch.auto_prepare {
            let Some(mode) = crate::nvs_config::AutoPrepare::parse(&v) else {
                return req.bad_request("auto_prepare must be idle, always or off");
            };
            next.auto_prepare = mode;
        }
        if let Some(v) = patch.follow {
            next.follow = v;
        }
        if let Some(v) = patch.follow_host {
            let v = v.trim().to_string();
            if v.len() > 64 || v.contains(['/', ' ', '\0', '?', '#', '@']) {
                return req.bad_request("follow_host must be a host name or address, optionally :port");
            }
            next.follow_host = v;
        }
        if let Some(v) = patch.follow_code {
            if v.len() > 64 || v.contains(['\0', '\r', '\n']) {
                return req.bad_request("follow_code is at most 64 characters on one line");
            }
            next.follow_code = v;
        }
        if let Some(v) = patch.attract_set {
            if let Some(set) = &v {
                if !crate::nvs_config::valid_set_name(set) {
                    return req.bad_request("attract_set must be a single path segment");
                }
            }
            next.attract_set = v;
        }
        if let Some(v) = patch.attract {
            if !crate::nvs_config::valid_attract(&v) {
                return req.bad_request(&format!(
                    "attract must be \"\" (the playlist), \"clock\", \"text:<message>\" (at most {} characters, one line) or \"fx:<effect>[?options]\"",
                    crate::nvs_config::ATTRACT_MAX - 5
                ));
            }
            next.attract = v;
        }
        if let Some(v) = patch.look {
            let Some(look) = opendmd_core::look::Look::from_name(&v) else {
                return req.bad_request("look must be colour, amber, red, green, blue or white");
            };
            next.look = look;
        }
        if let Some(v) = patch.glow {
            let Some(glow) = opendmd_core::look::GlowLevel::from_name(&v) else {
                return req.bad_request("glow must be off, short or long");
            };
            next.glow = glow;
        }
        if let Some(v) = patch.boot_show {
            let Some(len) = opendmd_core::boot::ShowLength::from_name(&v) else {
                return req.bad_request("boot_show must be off, short or full");
            };
            next.boot_show = len;
        }
        if let Some(v) = &patch.api_key {
            if !v.is_empty() && !opendmd_core::access::valid_key(v) {
                return req.bad_request(&format!(
                    "api_key must be {}..{} of A-Z a-z 0-9 . _ ~ - (or \"\" for none)",
                    opendmd_core::access::KEY_MIN,
                    opendmd_core::access::KEY_MAX
                ));
            }
            next.api_key = v.clone();
        }
        if next.dwell_max < next.dwell_min {
            return req.bad_request("dwell_max must be >= dwell_min");
        }
        let panel_op = match (patch.panel, patch.panel_confirm) {
            (None, None) => None,
            (None, Some(true)) => Some(PanelOp::Confirm),
            (None, Some(false)) => Some(PanelOp::Revert),
            (Some(None), _) => Some(PanelOp::Save(None)),
            (Some(Some(_)), Some(false)) => {
                return req.bad_request("panel_confirm false undoes a trial: send it without panel");
            }
            (Some(Some(w)), confirm) => {
                let p = match PanelPatch::new(
                    w.clk_phase,
                    w.latch_blanking,
                    w.driver.as_deref(),
                    w.i2s_speed,
                    w.min_refresh,
                ) {
                    Ok(p) => p,
                    Err(e) => return req.bad_request(&e),
                };
                Some(if confirm == Some(true) {
                    PanelOp::Save(Some(p))
                } else {
                    PanelOp::Try(p)
                })
            }
        };
        // WiFi and the hostname are read once, when the network starts
        // (`net_worker`: the STA netif's DHCP name, mDNS): saved now, in
        // effect after a reboot.
        let reboot_required = (next.wifi_ssid.clone(), next.wifi_pass.clone()) != wifi_before
            || next.hostname != hostname_before;
        let (saved_brightness, saved_max) = (next.brightness, next.max_brightness);

        let (tx, rx) = mpsc::channel();
        if c.cmd
            .send(Command::ApplyConfig {
                settings: Box::new(next),
                panel: panel_op,
                reply: tx,
            })
            .is_err()
        {
            return req.error(503, "503 Service Unavailable", "player not running");
        }
        match rx.recv_timeout(REINDEX_TIMEOUT) {
            Ok(Ok(applied)) => {
                if let Some(k) = &patch.api_key {
                    crate::httpd::set_api_key(k);
                    info!("API key {}", if k.is_empty() { "cleared" } else { "set" });
                }
                let (panel, restart) = {
                    let st = c.status.lock().unwrap();
                    (panel_json(&st), st.panel.restart_needed())
                };
                // A restart this request's panel change waits for (not one
                // pending from before: the WiFi banner keys off this).
                let reboot_required = reboot_required || (panel_op.is_some() && restart);
                req.json(&format!(
                    r#"{{"ok":true,{},"brightness":{saved_brightness},"max_brightness":{saved_max},"reboot_required":{reboot_required},{panel}}}"#,
                    applied_json(&applied)
                ))
            }
            Ok(Err(e)) if e == panel_timing::NO_TRIAL => req.error(409, "409 Conflict", &e),
            // Checked by the main task before anything was saved: the
            // configuration and the running playlist are as they were.
            Ok(Err(e)) if e == SET_NOT_FOUND => req.not_found(SET_NOT_FOUND),
            // Likewise refused, the previous playlist rebuilt: nothing saved.
            Ok(Err(e)) if e.starts_with(crate::state::FILTER_EMPTY) => req.bad_request(&e),
            Ok(Err(e)) => req.error(500, "500 Internal Server Error", &e),
            Err(_) => req.error(504, "504 Gateway Timeout", "player did not answer"),
        }
    })?;

    let c = ctx.clone();
    server.route("/api/reindex", Method::Post, move |req| {
        let (tx, rx) = mpsc::channel();
        if c.cmd.send(Command::Reindex(tx)).is_err() {
            return req.error(503, "503 Service Unavailable", "player not running");
        }
        match rx.recv_timeout(REINDEX_TIMEOUT) {
            Ok(Ok(applied)) => req.json(&format!(r#"{{"ok":true,{}}}"#, applied_json(&applied))),
            Ok(Err(e)) => req.error(500, "500 Internal Server Error", &e),
            Err(_) => req.error(504, "504 Gateway Timeout", "player did not answer"),
        }
    })?;

    // A playlist's overrides, parsed: `?set=` takes the config's `set`
    // values (`""` = all files). Streamed a line at a time from its `.meta`.
    server.route("/api/overrides", Method::Get, |req| {
        if !files::affordable(req)? {
            return Ok(());
        }
        let set = query_param(req.query(), "set").unwrap_or_default();
        if !crate::nvs_config::valid_set_name(&set) {
            return req.bad_request("set must be a single path segment");
        }
        let manifest = crate::nvs_config::manifest_path_for(&set);
        let meta = opendmd_core::overrides::meta_path(&manifest);
        stream_overrides(req, &set, &manifest, &meta)
    })?;

    // The schedule: GET is what was stored (or the empty one); PUT replaces
    // it whole, validated here (times, days, rule playlists that open)
    // before the main task saves it to NVS and evaluates it.
    let c = ctx.clone();
    server.route("/api/schedule", Method::Get, move |req| {
        let json = c.settings.lock().unwrap().schedule.clone();
        req.json(if json.trim().is_empty() {
            crate::sched::EMPTY_JSON
        } else {
            &json
        })
    })?;

    let c = ctx.clone();
    server.route_json("/api/schedule", Method::Put, move |req| {
        let declared = req.content_len();
        if declared > crate::nvs_config::SCHEDULE_MAX {
            return req.error(
                413,
                "413 Payload Too Large",
                &format!(
                    "schedule is {declared} B, the limit is {} B",
                    crate::nvs_config::SCHEDULE_MAX
                ),
            );
        }
        let body = req.read_to_vec(crate::nvs_config::SCHEDULE_MAX)?;
        let parsed = match crate::sched::parse(&body) {
            Ok(p) => p,
            Err(e) => return req.bad_request(&e),
        };
        // Parsed as JSON, so it is UTF-8.
        let json = String::from_utf8(body).unwrap_or_default();
        let missing = crate::sched::missing_sets(&parsed);
        if !missing.is_empty() {
            return req.not_found(&format!("set not found: {}", missing.join(", ")));
        }
        let json = if parsed.is_empty() {
            String::new()
        } else {
            json
        };
        let (tx, rx) = mpsc::channel();
        if c.cmd
            .send(Command::SetSchedule { json, reply: tx })
            .is_err()
        {
            return req.error(503, "503 Service Unavailable", "player not running");
        }
        // Applying may switch the playlist (a rule in force now).
        match rx.recv_timeout(REINDEX_TIMEOUT) {
            Ok(Ok(_)) => req.json(&format!(
                r#"{{"ok":true,"rules":{},"night":{}}}"#,
                parsed.rules.len(),
                parsed.night.is_some()
            )),
            Ok(Err(e)) => req.error(500, "500 Internal Server Error", &e),
            Err(_) => req.error(504, "504 Gateway Timeout", "player did not answer"),
        }
    })?;

    // Suspend the schedule: `{"minutes": 60}` (0 resumes it now).
    let c = ctx.clone();
    server.route_json("/api/wake", Method::Post, move |req| {
        let Some(body) = json_body(req)? else {
            return Ok(());
        };
        let wake: WakeRequest = match serde_json::from_slice(&body) {
            Ok(w) => w,
            Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
        };
        if wake.minutes > config::WAKE_MAX_MINUTES {
            return req.bad_request(&format!("minutes is at most {}", config::WAKE_MAX_MINUTES));
        }
        let (tx, rx) = mpsc::channel();
        if c.cmd
            .send(Command::Wake {
                minutes: wake.minutes,
                reply: tx,
            })
            .is_err()
        {
            return req.error(503, "503 Service Unavailable", "player not running");
        }
        match rx.recv_timeout(REINDEX_TIMEOUT) {
            Ok(_) => {
                let until = c.status.lock().unwrap().wake_until.clone();
                req.json(&format!(
                    r#"{{"ok":true,"minutes":{},"wake_until":{}}}"#,
                    wake.minutes,
                    opt_json(until.as_deref())
                ))
            }
            Err(_) => req.error(504, "504 Gateway Timeout", "player did not answer"),
        }
    })?;

    // Play one file now (a Library preview sent to the panel), then carry on
    // with the schedule. `{"path": "/gifs/…"}`, card-absolute.
    let c = ctx.clone();
    server.route_json("/api/play", Method::Post, move |req| {
        let Some(body) = json_body(req)? else {
            return Ok(());
        };
        let play: PlayRequest = match serde_json::from_slice(&body) {
            Ok(p) => p,
            Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
        };
        // A virtual file (`@clock`, `@text?msg=…`) is drawn, not read; one
        // this build does not know is a 415 from the engine.
        let virtual_file = opendmd_core::synth::is_virtual(&play.path);
        if !virtual_file
            && (!play.path.starts_with('/')
                || files::card_path(config::SD_MOUNT, &play.path).is_none())
        {
            return req.bad_request("path must be card-absolute and stay on the card, or a live item (@clock, @text?msg=…)");
        }
        let (tx, rx) = mpsc::channel();
        if c.cmd
            .send(Command::PlayNow {
                path: play.path.clone(),
                reply: tx,
            })
            .is_err()
        {
            return req.error(503, "503 Service Unavailable", "player not running");
        }
        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(Ok(())) => req.json(&format!(
                r#"{{"ok":true,"path":{}}}"#,
                json_string(&play.path)
            )),
            Ok(Err(PlayError::NotFound(e))) => req.not_found(&e),
            Ok(Err(PlayError::Unplayable(e))) => req.error(415, "415 Unsupported Media Type", &e),
            Ok(Err(PlayError::NoPlaylist)) => {
                req.error(503, "503 Service Unavailable", "no playlist is running")
            }
            Err(_) => req.error(504, "504 Gateway Timeout", "player did not answer"),
        }
    })?;

    // Existence, size and kind of one path, with one `stat`. That is one scan
    // of the parent directory under the FATFS lock — cheap in a bucket or
    // `www/`, ~330-550 ms in the 3,781-entry `gifs/Consoles`, where the
    // player's next open waits behind it — but still far cheaper than listing
    // the parent. `size` is null for a directory.
    server.route("/api/stat", Method::Get, |req| {
        let Some(card) = query_param(req.query(), "path") else {
            return req.bad_request("path= required");
        };
        if !card.starts_with('/') {
            return req.bad_request("path must be card-absolute");
        }
        let Some(fs_path) = files::card_path(config::SD_MOUNT, &card) else {
            return req.bad_request("path escapes the card");
        };
        // FATFS cannot `stat` a volume root; it is a directory by definition.
        let meta = if fs_path == config::SD_MOUNT {
            Some((true, 0))
        } else {
            std::fs::metadata(&fs_path)
                .ok()
                .map(|m| (m.is_dir(), m.len()))
        };
        let json = match meta {
            Some((true, _)) => format!(
                r#"{{"path":{},"exists":true,"size":null,"is_dir":true}}"#,
                json_string(&card)
            ),
            Some((false, len)) => format!(
                r#"{{"path":{},"exists":true,"size":{len},"is_dir":false}}"#,
                json_string(&card)
            ),
            None => format!(
                r#"{{"path":{},"exists":false,"size":null,"is_dir":false}}"#,
                json_string(&card)
            ),
        };
        req.json(&json)
    })?;

    // The playback cache. `total` is the playlist length the engine knows;
    // `prepared` is counted on the network worker (see `cache.rs`), so a
    // first request, or one after the card changed, answers "counting" with
    // the last count (or null) and the browser polls.
    let c = ctx.clone();
    server.route("/api/cache", Method::Get, move |req| {
        let snap = crate::cache::snapshot();
        let total = c.status.lock().unwrap().entries;
        req.json(&format!(
            r#"{{"total":{total},"prepared":{},"state":{},"count_ms":{},"auto":{}}}"#,
            snap.prepared
                .map_or_else(|| "null".to_string(), |n| n.to_string()),
            json_string(if snap.counting { "counting" } else { "ready" }),
            snap.count_ms,
            crate::cache_builder::auto_json(total, snap.prepared)
        ))
    })?;

    // Every prepared copy's card path, streamed: what a browser intersects
    // with a playlist to find the files left to prepare.
    server.route("/api/cache/list", Method::Get, |req| {
        if !files::affordable(req)? {
            return Ok(());
        }
        let _busy = crate::cache_builder::busy();
        crate::cache::stream_list(req)
    })?;

    // The GIFs the cache builder could not prepare (`/cache/.failed`), and
    // forgetting them so they are tried again.
    server.route("/api/cache/failed", Method::Get, |req| {
        if !files::affordable(req)? {
            return Ok(());
        }
        crate::cache_builder::stream_failed(req)
    })?;
    // A GIF the browser could not prepare either (`{"path","reason"}`):
    // recorded once per path, so the device's builder skips it too.
    server.route_json("/api/cache/failed", Method::Post, |req| {
        let Some(body) = json_body(req)? else {
            return Ok(());
        };
        let failed: FailedRequest = match serde_json::from_slice(&body) {
            Ok(f) => f,
            Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
        };
        if failed.path.contains(['\t', '\r', '\n'])
            || files::card_path(config::SD_MOUNT, &failed.path).is_none()
            || opendmd_core::layout::cache_path(&failed.path).is_none()
        {
            return req.bad_request("path must be a card-absolute /gifs/….gif");
        }
        match crate::cache_builder::add_failed(&failed.path, &failed.reason) {
            Ok(added) => req.json(&format!(
                r#"{{"ok":true,"path":{},"added":{added}}}"#,
                json_string(&failed.path)
            )),
            Err(e) => req.error(500, "500 Internal Server Error", &format!("{e}")),
        }
    })?;
    server.route("/api/cache/failed", Method::Delete, |req| {
        match crate::cache_builder::clear_failed() {
            Ok(existed) => req.json(&format!(r#"{{"ok":true,"cleared":{existed}}}"#)),
            Err(e) => req.error(500, "500 Internal Server Error", &format!("{e}")),
        }
    })?;

    // Remove the prepared copy of a GIF (it was replaced, or is going), if
    // there is one. `{"path": "/gifs/…"}`.
    let c = ctx.clone();
    server.route_json("/api/cache/forget", Method::Post, move |req| {
        let Some(body) = json_body(req)? else {
            return Ok(());
        };
        let forget: ForgetRequest = match serde_json::from_slice(&body) {
            Ok(p) => p,
            Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
        };
        if files::card_path(config::SD_MOUNT, &forget.path).is_none()
            || opendmd_core::layout::cache_path(&forget.path).is_none()
        {
            return req.bad_request("path must be a card-absolute /gifs/….gif");
        }
        // The copy may be the one opened ahead (the GIF's path finds it).
        let _busy = crate::cache_builder::busy();
        if !skip_if_playing(&c, &format!("{}{}", config::SD_MOUNT, forget.path)) {
            return req.error(503, "503 Service Unavailable", NOT_RELEASED);
        }
        match crate::cache::forget(&forget.path) {
            Ok(removed) => req.json(&format!(
                r#"{{"ok":true,"path":{},"removed":{}}}"#,
                json_string(&forget.path),
                removed.is_some()
            )),
            Err(e) => req.error(500, "500 Internal Server Error", &format!("{e}")),
        }
    })?;

    server.route("/api/files", Method::Get, |req| {
        if !files::affordable(req)? {
            return Ok(());
        }
        let dir = query_param(req.query(), "dir").unwrap_or_else(|| "/".to_string());
        let Some(fs_dir) = files::card_path(config::SD_MOUNT, &dir) else {
            return req.bad_request("path escapes the card");
        };
        // Sizes cost a `stat` each, which FAT makes quadratic — see
        // `files::stream_dir_json`. Opt in with `?sizes=1`.
        let sizes = query_param(req.query(), "sizes").is_some_and(|v| v == "1" || v == "true");
        let _busy = crate::cache_builder::busy();
        files::stream_dir_json(req, &dir, &fs_dir, sizes)
    })?;

    let c = ctx.clone();
    server.route("/api/wifi/scan", Method::Get, move |req| {
        let (tx, rx) = mpsc::channel();
        if c.net.send(NetCommand::Scan(tx)).is_err() {
            return req.error(503, "503 Service Unavailable", "network worker not running");
        }
        match rx.recv_timeout(SCAN_TIMEOUT) {
            Ok(aps) => req.json(&scan_json(&aps)),
            Err(_) => req.error(504, "504 Gateway Timeout", "scan timed out"),
        }
    })?;

    // Follow RePlayOS's "Test" button: the network worker asks the box (it
    // owns the HTTP client); nothing is saved.
    let c = ctx.clone();
    server.route_json("/api/follow/test", Method::Post, move |req| {
        let Some(body) = json_body(req)? else {
            return Ok(());
        };
        let t: FollowTestRequest = if body.is_empty() {
            FollowTestRequest::default()
        } else {
            match serde_json::from_slice(&body) {
                Ok(t) => t,
                Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
            }
        };
        let (tx, rx) = mpsc::channel();
        let cmd = NetCommand::FollowTest {
            host: t.host,
            code: t.code,
            reply: tx,
        };
        if c.net.send(cmd).is_err() {
            return req.error(503, "503 Service Unavailable", "network worker not running");
        }
        match rx.recv_timeout(FOLLOW_TEST_TIMEOUT) {
            Ok(json) => req.json(&json),
            Err(_) => req.error(504, "504 Gateway Timeout", "the test did not finish"),
        }
    })?;

    // The web app in flash: what is there, and a new pack (`www.rs`). The
    // body is received on the stream task like every other upload, which is
    // also where every web app file is sent from: the two never overlap.
    server.route("/api/www", Method::Get, |req| req.json(&www::info_json()))?;
    let c = ctx.clone();
    server.route("/api/www", Method::Put, move |req| {
        // Why staging on the card is not possible, if it is not: the log
        // and the reply say so in words (no card, or one that cannot be read).
        let card = {
            let st = c.status.lock().unwrap();
            if st.sd_usable() { None } else if st.sd_mounted { Some("card not readable") } else { Some("no card") }
        };
        stream::receive(req, move |up| www::receive(up, card))
    })?;

    // Firmware updates over the LAN (`ota.rs`, docs/ota.md): a signed
    // bundle received onto the card on the stream task, then the updater.
    let c = ctx.clone();
    server.route("/api/firmware", Method::Get, move |req| {
        let mounted = c.status.lock().unwrap().sd_usable();
        req.json(&crate::ota::info_json(mounted))
    })?;
    let c = ctx.clone();
    server.route("/api/firmware", Method::Put, move |req| {
        let mounted = c.status.lock().unwrap().sd_usable();
        let net = c.net.clone();
        stream::receive(req, move |up| crate::ota::receive(up, mounted, &net))
    })?;

    // Format the card FAT32 — only one that is mounted but cannot be read
    // (exFAT, as most cards over 32 GB come); never a readable one
    // (`opendmd_core::card::may_format`). The boot task, which holds such
    // a card, does it (`boot::request_format`); `sd.formatting` says when
    // it is over, and `sd.readable` how it went.
    let c = ctx.clone();
    server.route_json("/api/card/format", Method::Post, move |req| {
        let Some(body) = json_body(req)? else {
            return Ok(());
        };
        const UNCONFIRMED: &str = r#"send {"confirm":true}: everything on the card is erased"#;
        match serde_json::from_slice::<FormatRequest>(&body) {
            Ok(f) if f.confirm => {}
            Ok(_) => return req.bad_request(UNCONFIRMED),
            Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
        }
        {
            let mut s = c.status.lock().unwrap();
            if let Err(r) =
                opendmd_core::card::may_format(s.sd_mounted, s.sd_readable, s.sd_formatting)
            {
                drop(s);
                info!("card format refused: {}", r.reason());
                return req.error(409, "409 Conflict", r.reason());
            }
            s.sd_formatting = true;
            s.sd_format_error = None;
        }
        info!("card format requested over HTTP");
        crate::boot::request_format();
        req.respond_json("202 Accepted", r#"{"ok":true,"formatting":true}"#)
    })?;

    server.route("/api/reboot", Method::Post, |req| {
        info!("reboot requested over HTTP");
        // Test hook: `?sd_fail=N` makes the next boot's first N card mounts
        // fail, to exercise the no-card boot without touching the card.
        if let Some(n) = query_param(req.query(), "sd_fail").and_then(|v| v.parse::<u32>().ok()) {
            info!(
                "the next boot's first {} card mount(s) will fail on purpose",
                n.min(20)
            );
            crate::sd::simulate_mount_failures(n.min(20));
        }
        // Test hook: `?sd_io_fail=N[&sd_io_boots=B]` — on each of the next
        // B boots (default 1, at most 5), from 10 s after boot, the player's
        // next N card reads of a file that had read fail with EIO (after
        // them, any `sd_fail` mount failures): N >= 3 trips the card
        // failure (`sdhealth`), so B boots in a row run the restart chain.
        if let Some(n) = query_param(req.query(), "sd_io_fail").and_then(|v| v.parse::<u32>().ok()) {
            let boots = query_param(req.query(), "sd_io_boots")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(1)
                .min(5);
            info!(
                "the player's first {} card read(s) will fail on purpose on each of the next {boots} boot(s)",
                n.min(20)
            );
            crate::sdhealth::simulate_io_failures(n.min(20), boots);
        }
        // `?sd_fallback=clear`: the next boot forgets the slow-clock record
        // (NVS and RTC) and the card-failure restart count — the bench's
        // stand-in for inserting a different card.
        if query_param(req.query(), "sd_fallback").as_deref() == Some("clear") {
            info!("the next boot forgets the card's slow clock and its card-failure restarts");
            crate::sdhealth::request_clear();
        }
        // A plain reboot (the UI's button, no hook) is a person's fresh
        // start: the card-failure restart count goes, the slow clock stays.
        let hooked = ["sd_fail", "sd_io_fail", "sd_fallback"]
            .iter()
            .any(|k| query_param(req.query(), k).is_some());
        if !hooked {
            crate::sdhealth::request_fresh_start();
        }
        req.json(r#"{"ok":true,"rebooting":true}"#)?;
        reboot_after_response()
    })?;

    // --- Uploads ------------------------------------------------------------
    // Every body is received on the stream task (`stream::receive`), never
    // on the httpd task: a client that trickles its upload must not hold up
    // the API. The cache builder stops for each, as for a download.
    //
    // A manifest goes through the main task: it is the only place the
    // engine's own file handle can be dropped before the file underneath it
    // is replaced.
    let c = ctx.clone();
    server.route("/api/manifest", Method::Put, move |req| {
        let c = c.clone();
        stream::receive(req, move |up| {
            install_manifest(up, &c, config::MANIFEST_PATH.to_string())
        })
    })?;

    let c = ctx.clone();
    server.route("/api/sets/*", Method::Put, move |req| {
        let name = req.path().trim_start_matches("/api/sets/");
        let name = crate::httpd::percent_decode(name);
        let c = c.clone();
        stream::receive(req, move |up| {
            if name.is_empty() || name.contains('/') || name.contains("..") {
                return up.bad_request("set name must be a single path segment");
            }
            let dest = format!("{}/{}", config::SETS_DIR, name);
            let dest = if dest.ends_with(".txt") {
                dest
            } else {
                format!("{dest}.txt")
            };
            install_manifest(up, &c, dest)
        })
    })?;

    let c = ctx.clone();
    server.route("/api/files/*", Method::Put, move |req| {
        let rel = crate::httpd::percent_decode(req.path().trim_start_matches("/api/files/"));
        let c = c.clone();
        stream::receive(req, move |up| put_file(up, &c, &rel))
    })?;

    // Rename a GIF (`{"from","to"}`): its prepared copy, its Smooth motion
    // line and its failure line follow. On the stream task, like the other
    // card writes: two FAT directory scans per rename, up to ~0.5 s each in
    // gifs/Consoles, must not hold the API.
    let c = ctx.clone();
    server.route_json("/api/files/move", Method::Post, move |req| {
        let c = c.clone();
        stream::receive(req, move |up| move_file(up, &c))
    })?;

    // --- Delete ---------------------------------------------------------------

    let c = ctx.clone();
    server.route("/api/files/*", Method::Delete, move |req| {
        let rel = crate::httpd::percent_decode(req.path().trim_start_matches("/api/files/"));
        let Some(path) = files::card_path(config::SD_MOUNT, &rel) else {
            return req.bad_request("path escapes the card");
        };
        // The engine holds the active manifest and its `.idx` open, and FAT
        // (no FF_FS_LOCK in this build) would let us unlink them underneath it.
        // "Active" is what the player resolved, so with `set` = "" this
        // protects whichever default the card got.
        let active = c.status.lock().unwrap().manifest.clone();
        if files::same_card_path(&path, &active)
            || files::same_card_path(&path, &files::index_path(&active))
        {
            return req.error(
                409,
                "409 Conflict",
                "that is the active manifest; select another set first",
            );
        }
        // A playlist's overrides: the main task removes it (and the index
        // that carries them) and reindexes if the playlist is in play.
        if files::meta_manifest(&path).is_some() {
            return apply_meta(req, &c, path, None);
        }
        // A whole directory, only under cache/ (prepared copies) and the
        // retired format's tree: in pieces of at most
        // `RECURSIVE_DELETE_BUDGET`, the client asking again until "done".
        if query_param(req.query(), "recursive").as_deref() == Some("1") {
            return delete_tree(req, &c, &rel, &path);
        }
        // A manifest's `.idx` goes first, for the same crash-ordering reason
        // as on upload (see `files::remove_index_beside`): an orphaned index
        // with no manifest is harmless, the reverse is not.
        if files::is_manifest_path(&path) {
            if let Err(e) = files::remove_index_beside(&path) {
                return req.error(500, "500 Internal Server Error", &format!("{e:#}"));
            }
        }
        if files::same_card_path(&path, config::BUILDER_FAILED_PATH) {
            return match crate::cache_builder::clear_failed() {
                Ok(true) => req.json(r#"{"ok":true}"#),
                Ok(false) => req.not_found(&rel),
                Err(e) => req.error(500, "500 Internal Server Error", &format!("{e}")),
            };
        }
        // Held to the end: the GIF's copy goes too, and nothing may open
        // it ahead in between.
        let _busy = crate::cache_builder::busy();
        if !skip_if_playing(&c, &path) {
            return req.error(503, "503 Service Unavailable", NOT_RELEASED);
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                // A GIF takes its prepared copy with it; a copy removed
                // directly changes the count.
                let card = format!("/{}", rel.trim_start_matches('/'));
                if let Err(e) = crate::cache::forget(&card) {
                    log::warn!("DELETE {card}: its prepared copy: {e}");
                }
                if files::under(&path, config::CACHE_ROOT) {
                    crate::cache::invalidate();
                }
                req.json(r#"{"ok":true}"#)
            }
            // An empty category (a directory under gifs/) goes too: what is
            // left when its last GIF was deleted or moved away.
            Err(_)
                if std::fs::metadata(&path).is_ok_and(|m| m.is_dir())
                    && files::under(&path, &format!("{}/gifs", config::SD_MOUNT)) =>
            {
                match std::fs::remove_dir(&path) {
                    Ok(()) => req.json(r#"{"ok":true}"#),
                    Err(e) => req.error(409, "409 Conflict", &format!("{rel}: not empty ({e})")),
                }
            }
            Err(e) => req.not_found(&format!("{rel}: {e}")),
        }
    })?;

    // --- Card files, read-only --------------------------------------------
    // Every root manifest — `filelist.txt`, `filelist.hall`, `filelist.xmas`
    // (and an obsolete one, if it is still there) — served as
    // text. Without this they
    // fell through to the `/*` www catch-all and 404ed.
    server.route("/filelist*", Method::Get, |req| {
        let rel = crate::httpd::percent_decode(req.path());
        let name = rel.trim_start_matches('/');
        if name.contains('/') {
            return req.not_found(&rel);
        }
        let Some(path) = files::card_path(config::SD_MOUNT, name) else {
            return req.bad_request("path escapes the card");
        };
        stream_file(req, path, rel)
    })?;

    server.route("/gifs/*", Method::Get, |req| {
        serve_under(req, config::SD_MOUNT)
    })?;
    server.route("/sets/*", Method::Get, |req| {
        serve_under(req, config::SD_MOUNT)
    })?;
    // The playback cache: prepared copies, `cache/<Category>/<xx>/<name>.gif`
    // (the device's own builder, `opendmd-pack convert`, or the web app).
    server.route("/cache/*", Method::Get, |req| {
        serve_under(req, config::SD_MOUNT)
    })?;

    // The web app under `/www/` too (both spellings, as when it was on the
    // card).
    server.route("/www/*", Method::Get, |req| {
        let rel = crate::httpd::percent_decode(req.path().trim_start_matches("/www/"));
        stream::hand_off(req, move |req| match www::serve(req, &rel)? {
            www::Served::Sent => Ok(()),
            www::Served::NotFound => req.not_found(&format!("/www/{rel}")),
            www::Served::Unavailable(why) => www::unavailable(req, &why),
        })
    })?;

    // --- Captive portal ----------------------------------------------------
    // Registered unconditionally; each one is a redirect in AP mode and a plain
    // 404 otherwise, because on a real network these paths belong to nobody.
    for probe in [
        "/hotspot-detect.html", // Apple
        "/generate_204",        // Android
        "/gen_204",             // Android, older
        "/ncsi.txt",            // Windows
        "/connecttest.txt",     // Windows 10+
        "/canonical.html",      // Firefox
    ] {
        let c = ctx.clone();
        server.route(probe, Method::Get, move |req| match portal_target(&c) {
            Some(target) => req.redirect(&target),
            None => req.not_found("not a captive portal"),
        })?;
    }

    // --- Catch-all, registered last ---------------------------------------
    //
    // The app's own index.html links its assets relatively, so a browser asks
    // for `/app.js` and `/pkg/opendmd_web_bg.wasm`, not `/www/...`. Serving
    // the pack from here is what makes both spellings work.
    let c = ctx.clone();
    server.route("/*", Method::Get, move |req| {
        let rel = crate::httpd::percent_decode(req.path());
        // Decided here, where the context is: what to say if the pack has no
        // such file (in AP mode, the portal).
        let target = portal_target(&c);
        stream::hand_off(req, move |req| {
            match www::serve(req, rel.trim_start_matches('/'))? {
                www::Served::Sent => Ok(()),
                _ if target.is_some() => req.redirect(target.as_deref().unwrap_or("/")),
                www::Served::NotFound => req.not_found(&rel),
                www::Served::Unavailable(why) => www::unavailable(req, &why),
            }
        })
    })?;

    Ok(server)
}

/// `Some(url)` while the device is running the setup AP: where a captive-portal
/// probe (or any unknown path) should be sent.
fn portal_target(ctx: &Ctx) -> Option<String> {
    let s = ctx.status.lock().ok()?;
    (s.wifi == "ap" && !s.ip.is_empty()).then(|| format!("http://{}/", s.ip))
}

/// If the player has `fs_path` open — the GIF on screen, or the prepared copy
/// it is actually reading for it — skip to the next entry and give it a beat
/// to close the file: FAT must not have an open file replaced or unlinked.
/// The copy it opened ahead for the next file — `fs_path` itself, or the
/// copy of the GIF at `fs_path` — is let go first, and waited for (its pick
/// then opens whatever is there when it plays). The caller holds
/// `cache_builder::busy()` from before this to the end of its card work, so
/// nothing is opened ahead meanwhile (`state::AHEAD_KEY`). False if the
/// player did not answer: the caller refuses rather than touch the file.
///
/// The skip is still a beat, not an answer: the file on screen closes at
/// the loop's next tick, which an answer would have to wait across
/// iterations for.
#[must_use]
fn skip_if_playing(ctx: &Ctx, fs_path: &str) -> bool {
    if crate::state::is_ahead(fs_path) && !release_ahead(ctx) {
        return false;
    }
    let playing = ctx.status.lock().unwrap().path.clone();
    if playing.is_empty() {
        return true;
    }
    let gif = format!("{}{playing}", config::SD_MOUNT);
    let copy =
        opendmd_core::layout::cache_path(&playing).map(|p| format!("{}{p}", config::SD_MOUNT));
    if files::same_card_path(fs_path, &gif)
        || copy.is_some_and(|c| files::same_card_path(fs_path, &c))
    {
        let _ = ctx.cmd.send(Command::Skip);
        std::thread::sleep(Duration::from_millis(200));
    }
    true
}

/// `Command::ReleaseAhead`, and its answer: true once the copy opened ahead
/// is closed (or there was none), false if the main loop did not answer in
/// [`REPLY_TIMEOUT`] (a frame held longer than that, or a stuck loop).
fn release_ahead(ctx: &Ctx) -> bool {
    let (tx, rx) = mpsc::channel();
    if ctx.cmd.send(Command::ReleaseAhead(tx)).is_err() {
        // No main loop: nothing is open.
        return true;
    }
    let answered = rx.recv_timeout(REPLY_TIMEOUT).is_ok();
    if !answered {
        log::warn!("the player did not let go of the next file's copy");
    }
    answered
}

/// The answer when [`skip_if_playing`] got none: the file is left alone.
const NOT_RELEASED: &str = "the player did not let go of the file; try again";

/// `DELETE /api/files/<dir>?recursive=1`: everything under `path`, which
/// must be `cache/` or the retired `odm/` tree (or a directory in one). The
/// builder is held off and a copy the player is reading is skipped first.
/// `{"ok","path","removed","done"}`; `done` false means the time budget ran
/// out and the client should ask again.
fn delete_tree(req: &mut Req, c: &Ctx, rel: &str, path: &str) -> Result<()> {
    let allowed = [config::CACHE_ROOT, config::LEGACY_ROOT]
        .iter()
        .any(|root| files::same_card_path(path, root) || files::under(path, root));
    if !allowed {
        return req.bad_request("recursive delete is only for cache/ and odm/");
    }
    let _busy = crate::cache_builder::busy();
    if files::under(path, config::CACHE_ROOT) || files::same_card_path(path, config::CACHE_ROOT) {
        // A copy opened ahead may be anywhere under it.
        if crate::state::AHEAD_KEY.load(std::sync::atomic::Ordering::SeqCst) != 0
            && !release_ahead(c)
        {
            return req.error(503, "503 Service Unavailable", NOT_RELEASED);
        }
        let playing = c.status.lock().unwrap().path.clone();
        if let Some(copy) = opendmd_core::layout::cache_path(&playing) {
            let copy = format!("{}{copy}", config::SD_MOUNT);
            if (files::same_card_path(path, &copy) || files::under(&copy, path))
                && !skip_if_playing(c, &copy)
            {
                return req.error(503, "503 Service Unavailable", NOT_RELEASED);
            }
        }
    }
    let card = format!("/{}", rel.trim_start_matches('/').trim_end_matches('/'));
    match files::remove_tree(path, config::RECURSIVE_DELETE_BUDGET) {
        Ok((removed, done)) => {
            info!("DELETE {card} (recursive): {removed} entries removed, done {done}");
            if files::under(path, config::CACHE_ROOT)
                || files::same_card_path(path, config::CACHE_ROOT)
            {
                crate::cache::invalidate();
            }
            req.json(&format!(
                r#"{{"ok":true,"path":{},"removed":{removed},"done":{done}}}"#,
                json_string(&card)
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => req.not_found(&card),
        Err(e) => req.error(500, "500 Internal Server Error", &format!("{e}")),
    }
}

/// Serve a card-absolute path below `root`, 404ing if it is not a file.
fn serve_under(req: &mut Req, root: &str) -> Result<()> {
    let rel = crate::httpd::percent_decode(req.path());
    let Some(path) = files::card_path(root, &rel) else {
        return req.bad_request("path escapes the card");
    };
    stream_file(req, path, rel)
}

/// Send the card file at `path` from the streaming task, or a 404 naming
/// `rel` if there is none.
fn stream_file(req: &mut Req, path: String, rel: String) -> Result<()> {
    stream::hand_off(req, move |req| {
        if files::serve(req, &path)? {
            Ok(())
        } else {
            req.not_found(&rel)
        }
    })
}

/// Let the response drain out of the TCP stack, then restart
/// (`POST /api/reboot`, and the network worker's AP timeout).
pub fn reboot_after_response() -> ! {
    // The handler never returns to the wrapper that records this.
    crate::httpd::note_httpd_stack();
    crate::sdhealth::note_restart();
    // No card write in flight at the reset (the drain wait runs meanwhile).
    let t = std::time::Instant::now();
    crate::cache_builder::quiesce(Duration::from_secs(3));
    std::thread::sleep(Duration::from_secs(1).saturating_sub(t.elapsed()));
    crate::restart()
}

fn send_setup(req: &mut Req) -> Result<()> {
    req.header("Content-Encoding", "gzip")?;
    req.header("Cache-Control", "no-cache")?;
    req.ok("text/html; charset=utf-8", SETUP_HTML_GZ)
}

/// Read a JSON request body of at most [`MAX_JSON_BODY`] bytes.
///
/// `Ok(None)` means a 413 has already been sent. ESP-IDF's server only takes
/// bodies with a `Content-Length`, so the declared length is the whole check;
/// whatever is left unread is drained by the server after the handler.
fn json_body(req: &mut Req) -> Result<Option<Vec<u8>>> {
    let declared = req.content_len();
    if declared > MAX_JSON_BODY {
        req.error(
            413,
            "413 Payload Too Large",
            &format!("body is {declared} B, the limit is {MAX_JSON_BODY} B"),
        )?;
        return Ok(None);
    }
    req.read_to_vec(MAX_JSON_BODY).map(Some)
}

/// `PUT /api/files/<rel>`, on the stream task: anywhere on the card, a
/// manifest or a `.meta` by way of the main task.
fn put_file(up: &mut Upload, c: &Ctx, rel: &str) -> Result<()> {
    let Some(dest) = files::card_path(config::SD_MOUNT, rel) else {
        return up.bad_request("path escapes the card");
    };
    // A manifest (`/filelist.*`, `/sets/*`) may be the one the engine has
    // open, and it has an `.idx` sidecar that must go first. Both are the
    // main task's business, exactly as for `PUT /api/manifest`.
    if files::is_manifest_path(&dest) {
        return install_manifest(up, c, dest);
    }
    // A playlist's overrides: its index carries them, so the swap and the
    // reindex are the main task's, like a manifest's.
    if files::meta_manifest(&dest).is_some() {
        let temp = format!("{dest}.part");
        files::stream_body_to(up, &temp)?;
        return apply_meta(up, c, dest, Some(temp));
    }
    // If the player has this very file open — the GIF, or its prepared
    // copy — ask it to move on first: FAT cannot replace a file that is
    // being read.
    if !skip_if_playing(c, &dest) {
        return up.error(503, "503 Service Unavailable", NOT_RELEASED);
    }
    let written = files::receive_upload(up, &dest)?;
    info!(
        "PUT /{rel}: {written} B in {:.1} s",
        up.elapsed().as_secs_f32()
    );
    if files::under(&dest, config::CACHE_ROOT) {
        crate::cache::invalidate();
    }
    up.json(&format!(
        r#"{{"ok":true,"path":{},"bytes":{written}}}"#,
        json_string(&format!("/{}", rel.trim_start_matches('/')))
    ))
}

/// `POST /api/files/move`, on the stream task: `{"from":"/gifs/A/x.gif",
/// "to":"/gifs/B/y.gif"}`. Both must be card-absolute `.gif` paths under
/// `/gifs/` with no tab or line break (400); `from` must exist (404) and
/// `to` must not, unless it is `from` in another case (409). Renames the
/// GIF (creating `to`'s directory), then its prepared copy to the new
/// bucket — or deletes it if that fails, and deletes any stale copy at the
/// new place, so no copy is ever left for a name that does not exist or
/// played for the wrong GIF — then renames its line in `/sets/.smooth` and
/// in `/cache/.failed`. Playlists and their overrides are the client's
/// (the web app rewrites them). `{"ok","from","to","copy":"moved"|"removed"
/// |"none","smooth","failed"}`. Everything it holds is transient.
fn move_file(up: &mut Upload, c: &Ctx) -> Result<()> {
    if up.content_len() > MAX_JSON_BODY as u64 {
        return up.error(413, "413 Payload Too Large", "body over 8 KB");
    }
    let mut body = Vec::with_capacity(up.content_len() as usize);
    let mut buf = [0u8; 128];
    loop {
        match up.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&buf[..n]),
            Err(e) => return up.error(408, "408 Request Timeout", &format!("{e:#}")),
        }
    }
    let m: MoveRequest = match serde_json::from_slice(&body) {
        Ok(m) => m,
        Err(e) => return up.bad_request(&format!("invalid JSON: {e}")),
    };
    drop(body);
    let gif = |p: &str| {
        let under_gifs = p.get(..6).is_some_and(|g| g.eq_ignore_ascii_case("/gifs/"));
        (under_gifs && !p.contains(['\t', '\r', '\n']))
            .then(|| files::card_path(config::SD_MOUNT, p))
            .flatten()
            .filter(|_| opendmd_core::layout::cache_path(p).is_some())
    };
    let (Some(from), Some(to)) = (gif(&m.from), gif(&m.to)) else {
        return up.bad_request("from and to must be card-absolute /gifs/….gif paths");
    };
    if m.from == m.to {
        return up.bad_request("from and to are the same");
    }
    if std::fs::metadata(&from).map_or(true, |md| md.is_dir()) {
        return up.not_found(&m.from);
    }
    // FAT compares names case-insensitively: a case-only rename finds
    // itself at `to`, which is not a clash.
    if !files::same_card_path(&from, &to) && std::fs::metadata(&to).is_ok() {
        return up.error(409, "409 Conflict", &format!("{} already exists", m.to));
    }
    // The player must not have the GIF, or its copy, open under a rename;
    // nor a stale copy at `to`'s place, which goes (a playlist line naming
    // `to` would have played it). The stream task holds
    // `cache_builder::busy()` throughout.
    if !skip_if_playing(c, &from) || !skip_if_playing(c, &to) {
        return up.error(503, "503 Service Unavailable", NOT_RELEASED);
    }
    if let Some(dir) = std::path::Path::new(&to).parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return up.error(
                500,
                "500 Internal Server Error",
                &format!("{}: {e}", dir.display()),
            );
        }
    }
    if let Err(e) = std::fs::rename(&from, &to) {
        return up.error(
            500,
            "500 Internal Server Error",
            &format!("rename {} -> {}: {e}", m.from, m.to),
        );
    }
    // The prepared copy follows the name: its bucket is the name's.
    let copy = match (
        opendmd_core::layout::cache_path(&m.from),
        opendmd_core::layout::cache_path(&m.to),
    ) {
        (Some(old), Some(new)) => {
            let (old, new) = (
                format!("{}{old}", config::SD_MOUNT),
                format!("{}{new}", config::SD_MOUNT),
            );
            files::move_copy(&old, &new)
        }
        _ => "none",
    };
    crate::cache::invalidate();
    let smooth = files::rename_line(config::SMOOTH_LIST, &m.from, &m.to).unwrap_or_else(|e| {
        log::warn!("move {}: {}: {e}", m.from, config::SMOOTH_LIST);
        false
    });
    let failed = crate::cache_builder::rename_failed(&m.from, &m.to).unwrap_or_else(|e| {
        log::warn!("move {}: {}: {e}", m.from, config::BUILDER_FAILED_PATH);
        false
    });
    info!(
        "move {} -> {}: copy {copy}, smooth {smooth}, failed {failed} in {:.1} s",
        m.from,
        m.to,
        up.elapsed().as_secs_f32()
    );
    up.json(&format!(
        r#"{{"ok":true,"from":{},"to":{},"copy":"{copy}","smooth":{smooth},"failed":{failed}}}"#,
        json_string(&m.from),
        json_string(&m.to)
    ))
}

/// Stream a manifest upload to a temp file, then hand the swap to the main
/// task, which drops the engine before the file underneath it changes.
fn install_manifest(req: &mut Upload, ctx: &Ctx, dest: String) -> Result<()> {
    let temp = format!("{dest}.part");
    files::stream_body_to(req, &temp)?;

    let (tx, rx) = mpsc::channel();
    if ctx
        .cmd
        .send(Command::InstallManifest {
            temp: temp.clone(),
            dest: dest.clone(),
            reply: tx,
        })
        .is_err()
    {
        let _ = std::fs::remove_file(&temp);
        return req.error(503, "503 Service Unavailable", "player not running");
    }
    // A full reindex of the stock card's 10,162-line manifest takes a few
    // seconds over SPI, so this waits longer than the non-playlist replies.
    match rx.recv_timeout(REINDEX_TIMEOUT) {
        Ok(Ok(applied)) => req.json(&format!(
            r#"{{"ok":true,"path":{},{}}}"#,
            json_string(&dest),
            applied_json(&applied)
        )),
        Ok(Err(e)) => req.error(500, "500 Internal Server Error", &e),
        Err(_) => req.error(504, "504 Gateway Timeout", "player did not answer"),
    }
}

/// A playlist's overrides: hand the swap of the uploaded `temp` (or, for a
/// DELETE, `None`: the removal) — and the reindex, if it is the playlist in
/// play — to the main task.
fn apply_meta<R: Respond>(
    req: &mut R,
    ctx: &Ctx,
    dest: String,
    temp: Option<String>,
) -> Result<()> {
    let upload = temp.is_some();
    let (tx, rx) = mpsc::channel();
    if ctx
        .cmd
        .send(Command::InstallMeta {
            temp: temp.clone(),
            dest: dest.clone(),
            reply: tx,
        })
        .is_err()
    {
        if let Some(t) = &temp {
            let _ = std::fs::remove_file(t);
        }
        return req.error(503, "503 Service Unavailable", "player not running");
    }
    match rx.recv_timeout(REINDEX_TIMEOUT) {
        Ok(Ok(applied)) => {
            let path = if upload {
                format!(
                    r#""path":{},"#,
                    json_string(dest.strip_prefix(config::SD_MOUNT).unwrap_or(&dest))
                )
            } else {
                String::new()
            };
            req.json(&format!(
                r#"{{"ok":true,{path}{}}}"#,
                applied_json(&applied)
            ))
        }
        Ok(Err(e)) => req.error(500, "500 Internal Server Error", &e),
        Err(_) => req.error(504, "504 Gateway Timeout", "player did not answer"),
    }
}

/// `GET /api/overrides`: `{"set","playlist","overrides":[{path, never?, w?,
/// dwell?, dwell_x?, loops?}]}`, one `.meta` line at a time (a missing file
/// is an empty list).
fn stream_overrides(req: &mut Req, set: &str, manifest: &str, meta: &str) -> Result<()> {
    use opendmd_core::overrides::{format_decimal, scan_meta, Dwell};
    let card = |p: &str| p.strip_prefix(config::SD_MOUNT).unwrap_or(p).to_string();
    req.header("Cache-Control", "no-cache")?;
    req.begin_chunked("200 OK", "application/json")?;
    let mut buf = format!(
        r#"{{"set":{},"playlist":{},"overrides":["#,
        json_string(set),
        json_string(&card(manifest))
    );
    let mut first = true;
    let mut sent: Result<()> = Ok(());
    if let Ok(f) = std::fs::File::open(meta) {
        let reader = std::io::BufReader::with_capacity(512, f);
        let scanned = scan_meta(reader, |path, ov| {
            if sent.is_err() {
                return;
            }
            if !first {
                buf.push(',');
            }
            first = false;
            buf.push_str(&format!(r#"{{"path":{}"#, json_string(path)));
            if ov.never {
                buf.push_str(r#","never":true"#);
            }
            if let Some(w) = ov.weight {
                buf.push_str(&format!(r#","w":{}"#, format_decimal(w)));
            }
            match ov.dwell {
                Some(Dwell::Secs(s)) => buf.push_str(&format!(r#","dwell":{s}"#)),
                Some(Dwell::Times(k)) => {
                    buf.push_str(&format!(r#","dwell_x":{}"#, format_decimal(k)))
                }
                None => {}
            }
            if let Some(n) = ov.loops {
                buf.push_str(&format!(r#","loops":{n}"#));
            }
            buf.push('}');
            if buf.len() >= config::IO_CHUNK {
                sent = req
                    .chunk(buf.as_bytes())
                    .and_then(|()| files::wait_for_heap());
                buf.clear();
            }
        });
        if let Err(e) = scanned {
            log::warn!("{meta}: {e}");
        }
    }
    sent?;
    buf.push_str("]}");
    req.chunk(buf.as_bytes())?;
    req.finish()
}

/// The playlist fields of a reply, without braces: `"entries":N,"index":…`.
/// `index` is `"loaded"`, `"built"`, `"in-ram"`, or `null` when the command did
/// not touch the playlist; `overrides` is how many were resolved into it.
fn applied_json(a: &Applied) -> String {
    format!(
        r#""entries":{},"index":{},"index_ms":{},"overrides":{},"hidden":{}"#,
        a.entries,
        a.index
            .map_or_else(|| "null".to_string(), |i| json_string(i.as_str())),
        a.index_ms,
        a.overrides,
        a.hidden
    )
}

/// `t_ms` is the device's clock (ms since the main loop's epoch) at the
/// moment this is built, and `now_playing.elapsed_ms` the file's position at
/// that same moment — the pair a browser needs to put its preview in step
/// with the panel, correcting for its own request latency the NTP way.
fn status_json(s: &Status, looks: &str, recent: bool) -> String {
    let t = s.epoch.elapsed();
    // `?recent=1`: Recently played, newest first, and which is on screen.
    let recent = if recent {
        let paths: Vec<String> = s.recent.paths().map(|p| json_string(&p)).collect();
        format!(r#""recent":[{}],"recent_at":{},"#, paths.join(","), s.recent.cursor())
    } else {
        String::new()
    };
    let www = crate::www::info();
    format!(
        concat!(
            r#"{{"version":{},"variant":{},"uptime_secs":{},"t_ms":{},"ip":{},"wifi":{},"hostname":{},"mdns":{},"#,
            r#""now_playing":{{"ordinal":{},"path":{},"title":{},"elapsed_ms":{},"frame":{}}},"paused":{},{}"#,
            r#""entries":{},"manifest":{},"index_state":{},"www_version":{},"www":{},"#,
            r#""heap":{{"free":{},"floor":{},"idf_min":{}}},"heap_floor_at":{},"#,
            r#""refresh_hz":{},"deadline_misses":{},"worst_lateness_ms":{},"misses":{},"gifs_started":{},"streams_aborted":{},"log_drops":{},"#,
            r#""sd":{{"mounted":{},"readable":{},"capacity_bytes":{},"error":{},"attempts":{},"formatting":{},"format_error":{},{}}},"reset_reason":{},"reset_note":{},"#,
            r#""overrides":{},"hidden":{},"#,
            r#""mode":{},"mode_until":{},"time":{},"tz":{},"time_synced_s":{},"wake_until":{},"#,
            r#""schedule":{{"rule":{},"set":{},"until":{}}},"cache":{},"context":{},"#,
            r#"{}}}"#
        ),
        json_string(s.version),
        json_string(crate::state::FLASH_VARIANT),
        s.uptime_secs,
        t.as_millis(),
        json_string(&s.ip),
        json_string(s.wifi),
        json_string(&s.hostname),
        json_string(&s.mdns),
        s.ordinal,
        json_string(&s.path),
        json_string(&s.title),
        s.position_at(t).as_millis(),
        s.frame,
        s.paused,
        recent,
        s.entries,
        json_string(&s.manifest),
        json_string(s.index_state),
        opt_json(www.version.as_deref().filter(|_| www.ok)),
        crate::www::info_json(),
        s.heap_free,
        s.heap_floor,
        s.heap_idf_min,
        s.heap_floor_at
            .as_ref()
            .map_or_else(|| "null".to_string(), |a| a.json()),
        s.refresh_hz,
        s.deadline_misses,
        s.worst_lateness_ms,
        s.misses.json(),
        s.gifs_started,
        crate::httpd::streams_aborted(),
        crate::logring::drops(),
        s.sd_mounted,
        s.sd_usable(),
        s.sd_capacity_bytes,
        opt_json(s.sd_error.as_deref()),
        s.sd_attempts,
        s.sd_formatting,
        opt_json(s.sd_format_error.as_deref()),
        crate::sdhealth::status_json(),
        json_string(s.reset_reason),
        opt_json(crate::sdhealth::reset_note()),
        s.overrides,
        s.hidden,
        json_string(s.mode),
        opt_json(s.mode_until.as_deref()),
        opt_json(crate::clock::hhmm_now().as_deref()),
        json_string(&s.tz_name),
        crate::clock::synced_ago_secs().map_or_else(|| "null".to_string(), |n| n.to_string()),
        opt_json(s.wake_until.as_deref()),
        s.rule.map_or_else(|| "null".to_string(), |r| r.to_string()),
        opt_json(s.rule_set.as_deref()),
        opt_json(s.rule_until.as_deref()),
        crate::cache_builder::status_json(s.entries, crate::cache::peek()),
        crate::follow::status_json(),
        looks,
    )
}

/// The end of the status object: `"look","glow","glow_held","attract_live",
/// "max_brightness","brightness_safe_max","safe_start_until"`
/// (see the API table), from the settings and the main loop's flags.
fn looks_json(set: &Settings) -> String {
    use std::sync::atomic::Ordering::Relaxed;
    let live = crate::state::ATTRACT_LIVE
        .load(Relaxed)
        .then(|| crate::nvs_config::attract_path(&set.attract))
        .flatten();
    format!(
        r#""look":"{}","glow":"{}","glow_held":{},"attract_live":{},"max_brightness":{},"brightness_safe_max":{},"safe_start_until":{}"#,
        set.look.name(),
        set.glow.name(),
        if crate::state::GLOW_HELD.load(Relaxed) {
            opendmd_core::look::GLOW_BYTES
        } else {
            0
        },
        opt_json(live.as_deref()),
        set.max_brightness,
        crate::config::BRIGHTNESS_SAFE_MAX,
        match crate::state::SAFE_START_UNTIL.load(Relaxed) {
            0 => "null".to_string(),
            n => n.to_string(),
        },
    )
}

/// A JSON string, or `null`.
fn opt_json(s: Option<&str>) -> String {
    s.map_or_else(|| "null".to_string(), json_string)
}

/// `"panel":{saved},"panel_now":{running},"panel_next":{…}|null,"panel_trial_s":N|null`
/// (`opendmd_core::panel_timing::Tuner`, published by the main task).
fn panel_json(st: &Status) -> String {
    let t = &st.panel;
    let null = || "null".to_string();
    format!(
        r#""panel":{},"panel_now":{},"panel_next":{},"panel_trial_s":{}"#,
        t.saved.json(),
        t.running.effective().json(),
        t.next_boot().map_or_else(null, |n| n.json()),
        t.trial_left(st.epoch.elapsed()).map_or_else(null, |n| n.to_string()),
    )
}

/// The saved configuration. The WiFi password and the RePlayOS code are never
/// returned — only whether one is set.
fn settings_json(s: &Settings) -> String {
    format!(
        concat!(
            r#"{{"wifi_ssid":{},"wifi_pass_set":{},"brightness":{},"max_brightness":{},"#,
            r#""dwell_min":{},"dwell_max":{},"shuffle":{},"platforms":{},"set":{},"hostname":{},"#,
            r#""tz":{},"tz_name":{},"transition":"{}","transition_ms":{},"auto_prepare":"{}","#,
            r#""follow":{},"follow_host":{},"follow_code_set":{},"attract_set":{},"attract":{},"#,
            r#""look":"{}","glow":"{}","boot_show":"{}","api_key_set":{}}}"#
        ),
        json_string(&s.wifi_ssid),
        !s.wifi_pass.is_empty(),
        s.brightness,
        s.max_brightness,
        s.dwell_min,
        s.dwell_max,
        s.shuffle,
        json_string(&s.platforms),
        json_string(&s.set),
        json_string(&s.hostname),
        json_string(&s.tz),
        json_string(&s.tz_name),
        s.transition.kind.name(),
        s.transition.duration.as_millis(),
        s.auto_prepare.as_str(),
        s.follow,
        json_string(&s.follow_host),
        !s.follow_code.is_empty(),
        opt_json(s.attract_set.as_deref()),
        json_string(&s.attract),
        s.look.name(),
        s.glow.name(),
        s.boot_show.name(),
        !s.api_key.is_empty(),
    )
}

fn scan_json(aps: &[ScannedAp]) -> String {
    let items: Vec<String> = aps
        .iter()
        .map(|a| {
            format!(
                r#"{{"ssid":{},"rssi":{},"channel":{},"auth":{},"secure":{}}}"#,
                json_string(&a.ssid),
                a.rssi,
                a.channel,
                json_string(a.auth),
                a.secure
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}
