//! The HTTP surface: the web app off the card, the card's files, and a small
//! JSON API.
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

use anyhow::Result;
use log::info;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config;
use crate::files;
use crate::httpd::{json_string, query_param, Method, Req, Respond, Server, Upload};
use crate::nvs_config::Settings;
use crate::ota;
use crate::state::{Applied, Command, PlayError, Status, SET_NOT_FOUND};
use crate::stream;
use crate::wifi::ScannedAp;
use opendmd_core::compositor::{self, Kind as TransitionKind};

/// The embedded setup page: rendered and gzipped by `build.rs`, served when
/// the card has no web app on it yet (and always at `/setup`).
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
}

/// Wire format for `POST /api/wake`.
#[derive(serde::Deserialize)]
struct WakeRequest {
    minutes: u32,
}

/// Start the server and register every route.
///
/// Called from the main task before the display loop, *not* from the network
/// worker, and deliberately so: the httpd task's stack (and the streaming
/// task's, started here too) is one contiguous internal allocation, and by the
/// time WiFi has associated the first GIF is already decoding and the largest
/// free block has fallen below it
/// (`httpd_start` then fails with `ESP_ERR_HTTPD_TASK`, 0xb008 — measured on
/// this board). Starting here, right after the index is built, takes the
/// allocation while the heap is still whole. `core_id` is in the config, so the
/// server still runs on CPU0 however it was started.
///
/// That means the mode is not known yet, so the captive-portal routes are
/// always registered and read the live WiFi state out of [`Status`] when a
/// request arrives.
pub fn start(ctx: Arc<Ctx>) -> Result<Server> {
    // URI handler slots: one per `route` below (41 now, the captive-portal loop counting six). Registration past
    // the limit fails the whole server, so count when adding a route.
    let mut server = Server::start(0, config::HTTP_STACK, config::HTTP_MAX_SOCKETS, 42)?;

    // --- The web app, and the setup page as its fallback -------------------
    // Every card file goes out from the streaming task (`stream`), so a big
    // download or a stalled client never holds up the API.
    stream::start().unwrap_or_else(|e| {
        log::error!("stream task failed to start ({e:#}); files are served on the httpd task")
    });

    server.route("/", Method::Get, |req| {
        stream::hand_off(req, |req| {
            let index = format!("{}/index.html", config::WWW_ROOT);
            if files::serve(req, &index)? {
                return Ok(());
            }
            send_setup(req)
        })
    })?;
    server.route("/setup", Method::Get, send_setup)?;

    // --- JSON API ----------------------------------------------------------
    let c = ctx.clone();
    server.route("/api/status", Method::Get, move |req| {
        let json = {
            let s = c.status.lock().unwrap();
            status_json(&s)
        };
        req.json(&json)
    })?;

    let c = ctx.clone();
    server.route("/api/skip", Method::Post, move |req| {
        let _ = c.cmd.send(Command::Skip);
        req.json(r#"{"ok":true}"#)
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
        let json = {
            let s = c.settings.lock().unwrap();
            settings_json(&s)
        };
        req.json(&json)
    })?;

    let c = ctx.clone();
    server.route("/api/config", Method::Put, move |req| {
        let Some(body) = json_body(req)? else {
            return Ok(());
        };
        let patch: ConfigPatch = match serde_json::from_slice(&body) {
            Ok(p) => p,
            Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
        };
        let mut next = c.settings.lock().unwrap().clone();
        let wifi_before = (next.wifi_ssid.clone(), next.wifi_pass.clone());
        if let Some(v) = patch.wifi_ssid {
            next.wifi_ssid = v;
        }
        if let Some(v) = patch.wifi_pass {
            next.wifi_pass = v;
        }
        if let Some(v) = patch.brightness {
            next.brightness = v;
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
            if !v.is_empty() {
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
        if next.dwell_max < next.dwell_min {
            return req.bad_request("dwell_max must be >= dwell_min");
        }
        let wifi_changed = (next.wifi_ssid.clone(), next.wifi_pass.clone()) != wifi_before;

        let (tx, rx) = mpsc::channel();
        if c.cmd
            .send(Command::ApplyConfig {
                settings: Box::new(next),
                reply: tx,
            })
            .is_err()
        {
            return req.error(503, "503 Service Unavailable", "player not running");
        }
        match rx.recv_timeout(REINDEX_TIMEOUT) {
            Ok(Ok(applied)) => req.json(&format!(
                r#"{{"ok":true,{},"reboot_required":{wifi_changed}}}"#,
                applied_json(&applied)
            )),
            // Checked by the main task before anything was saved: the
            // configuration and the running playlist are as they were.
            Ok(Err(e)) if e == SET_NOT_FOUND => req.not_found(SET_NOT_FOUND),
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
    server.route("/api/schedule", Method::Put, move |req| {
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
    server.route("/api/wake", Method::Post, move |req| {
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
    server.route("/api/play", Method::Post, move |req| {
        let Some(body) = json_body(req)? else {
            return Ok(());
        };
        let play: PlayRequest = match serde_json::from_slice(&body) {
            Ok(p) => p,
            Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
        };
        if !play.path.starts_with('/') || files::card_path(config::SD_MOUNT, &play.path).is_none() {
            return req.bad_request("path must be card-absolute and stay on the card");
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

    // The GIFs the cache builder could not prepare (`/odm/.failed`), and
    // forgetting them so they are tried again.
    server.route("/api/cache/failed", Method::Get, |req| {
        if !files::affordable(req)? {
            return Ok(());
        }
        crate::cache_builder::stream_failed(req)
    })?;
    // A GIF the browser could not prepare either (`{"path","reason"}`):
    // recorded once per path, so the device's builder skips it too.
    server.route("/api/cache/failed", Method::Post, |req| {
        let Some(body) = json_body(req)? else {
            return Ok(());
        };
        let failed: FailedRequest = match serde_json::from_slice(&body) {
            Ok(f) => f,
            Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
        };
        if failed.path.contains(['\t', '\r', '\n'])
            || files::card_path(config::SD_MOUNT, &failed.path).is_none()
            || opendmd_core::layout::bucketed_path(&failed.path).is_none()
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

    // Remove the prepared copy of a GIF (it was replaced, or is going): its
    // bucketed `.odm`, if there is one. `{"path": "/gifs/…"}`.
    let c = ctx.clone();
    server.route("/api/cache/forget", Method::Post, move |req| {
        let Some(body) = json_body(req)? else {
            return Ok(());
        };
        let forget: ForgetRequest = match serde_json::from_slice(&body) {
            Ok(p) => p,
            Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
        };
        if files::card_path(config::SD_MOUNT, &forget.path).is_none()
            || opendmd_core::layout::bucketed_path(&forget.path).is_none()
        {
            return req.bad_request("path must be a card-absolute /gifs/….gif");
        }
        skip_if_playing(&c, &format!("{}{}", config::SD_MOUNT, forget.path));
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
    server.route("/api/follow/test", Method::Post, move |req| {
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

    // On the httpd task, unlike every other upload: it writes flash through
    // `esp_ota_*` and reboots from here, and nothing else may run a transfer
    // or a card write meanwhile. Capped in time like the others (`ota.rs`).
    server.route("/api/ota", Method::Post, |req| {
        let _busy = crate::cache_builder::busy();
        ota::handle_upload(req)
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
        req.json(r#"{"ok":true,"rebooting":true}"#)?;
        ota::reboot_after_response()
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
        skip_if_playing(&c, &path);
        match std::fs::remove_file(&path) {
            Ok(()) => {
                if files::same_card_path(&path, config::WWW_VERSION_PATH) {
                    c.status.lock().unwrap().www_version = None;
                }
                // A GIF takes its prepared copy with it; a copy removed
                // directly changes the count.
                let card = format!("/{}", rel.trim_start_matches('/'));
                if let Err(e) = crate::cache::forget(&card) {
                    log::warn!("DELETE {card}: its prepared copy: {e}");
                }
                if files::under(&path, config::ODM_ROOT) {
                    crate::cache::invalidate();
                }
                req.json(r#"{"ok":true}"#)
            }
            Err(e) => req.not_found(&format!("{rel}: {e}")),
        }
    })?;

    // --- Card files, read-only --------------------------------------------
    // Every root manifest — `filelist.txt`, `filelist.hall`, `filelist.xmas`
    // (and an obsolete `filelist.odm.txt`, if one is still there) — served as
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
    // The playback cache: prepared copies, `odm/<Category>/<bb>/<name>.odm`
    // (`opendmd-pack convert --layout bucketed`, or the web app).
    server.route("/odm/*", Method::Get, |req| {
        serve_under(req, config::SD_MOUNT)
    })?;

    server.route("/www/*", Method::Get, |req| {
        let rel = crate::httpd::percent_decode(req.path().trim_start_matches("/www/"));
        let Some(path) = files::card_path(config::WWW_ROOT, &rel) else {
            return req.bad_request("path escapes the card");
        };
        stream_file(req, path, rel)
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
    // for `/app.js` and `/pkg/opendmd_web_bg.wasm`, not `/www/...`. Serving the
    // card's `www/` from here is what makes both spellings work.
    let c = ctx.clone();
    server.route("/*", Method::Get, move |req| {
        let rel = crate::httpd::percent_decode(req.path());
        // Decided here, where the context is: what to say if the card has no
        // such file.
        let target = portal_target(&c);
        let Some(path) = files::card_path(config::WWW_ROOT, &rel) else {
            return match target {
                Some(target) => req.redirect(&target),
                None => req.not_found(&rel),
            };
        };
        stream::hand_off(req, move |req| {
            if files::serve(req, &path)? {
                return Ok(());
            }
            match target {
                Some(target) => req.redirect(&target),
                None => req.not_found(&rel),
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
fn skip_if_playing(ctx: &Ctx, fs_path: &str) {
    let playing = ctx.status.lock().unwrap().path.clone();
    if playing.is_empty() {
        return;
    }
    let gif = format!("{}{playing}", config::SD_MOUNT);
    let copy =
        opendmd_core::layout::bucketed_path(&playing).map(|p| format!("{}{p}", config::SD_MOUNT));
    if files::same_card_path(fs_path, &gif)
        || copy.is_some_and(|c| files::same_card_path(fs_path, &c))
    {
        let _ = ctx.cmd.send(Command::Skip);
        std::thread::sleep(Duration::from_millis(200));
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
    skip_if_playing(c, &dest);
    let written = files::receive_upload(up, &dest)?;
    info!(
        "PUT /{rel}: {written} B in {:.1} s",
        up.elapsed().as_secs_f32()
    );
    if files::under(&dest, config::ODM_ROOT) {
        crate::cache::invalidate();
    }
    if files::same_card_path(&dest, config::WWW_VERSION_PATH) {
        c.status.lock().unwrap().www_version = files::read_www_version();
    }
    up.json(&format!(
        r#"{{"ok":true,"path":{},"bytes":{written}}}"#,
        json_string(&format!("/{}", rel.trim_start_matches('/')))
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
fn status_json(s: &Status) -> String {
    let t = s.epoch.elapsed();
    format!(
        concat!(
            r#"{{"version":{},"variant":{},"uptime_secs":{},"t_ms":{},"ip":{},"wifi":{},"hostname":{},"#,
            r#""now_playing":{{"ordinal":{},"path":{},"title":{},"elapsed_ms":{},"frame":{}}},"paused":{},"#,
            r#""entries":{},"manifest":{},"index_state":{},"www_version":{},"#,
            r#""heap":{{"free":{},"floor":{},"idf_min":{}}},"#,
            r#""refresh_hz":{},"deadline_misses":{},"worst_lateness_ms":{},"gifs_started":{},"streams_aborted":{},"#,
            r#""sd":{{"mounted":{},"capacity_bytes":{},"error":{},"attempts":{}}},"reset_reason":{},"#,
            r#""overrides":{},"hidden":{},"#,
            r#""mode":{},"mode_until":{},"time":{},"tz":{},"time_synced_s":{},"wake_until":{},"#,
            r#""schedule":{{"rule":{},"set":{},"until":{}}},"cache":{},"context":{}}}"#
        ),
        json_string(s.version),
        json_string(ota::FLASH_VARIANT),
        s.uptime_secs,
        t.as_millis(),
        json_string(&s.ip),
        json_string(s.wifi),
        json_string(&s.hostname),
        s.ordinal,
        json_string(&s.path),
        json_string(&s.title),
        s.position_at(t).as_millis(),
        s.frame,
        s.paused,
        s.entries,
        json_string(&s.manifest),
        json_string(s.index_state),
        s.www_version
            .as_deref()
            .map_or_else(|| "null".to_string(), json_string),
        s.heap_free,
        s.heap_floor,
        s.heap_idf_min,
        s.refresh_hz,
        s.deadline_misses,
        s.worst_lateness_ms,
        s.gifs_started,
        crate::httpd::streams_aborted(),
        s.sd_mounted,
        s.sd_capacity_bytes,
        opt_json(s.sd_error.as_deref()),
        s.sd_attempts,
        json_string(s.reset_reason),
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
    )
}

/// A JSON string, or `null`.
fn opt_json(s: Option<&str>) -> String {
    s.map_or_else(|| "null".to_string(), json_string)
}

/// The saved configuration. The WiFi password and the RePlayOS code are never
/// returned — only whether one is set.
fn settings_json(s: &Settings) -> String {
    format!(
        concat!(
            r#"{{"wifi_ssid":{},"wifi_pass_set":{},"brightness":{},"#,
            r#""dwell_min":{},"dwell_max":{},"shuffle":{},"platforms":{},"set":{},"hostname":{},"#,
            r#""tz":{},"tz_name":{},"transition":"{}","transition_ms":{},"auto_prepare":"{}","#,
            r#""follow":{},"follow_host":{},"follow_code_set":{},"attract_set":{}}}"#
        ),
        json_string(&s.wifi_ssid),
        !s.wifi_pass.is_empty(),
        s.brightness,
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
