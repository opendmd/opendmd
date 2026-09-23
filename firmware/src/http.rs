//! The HTTP surface: the web app off the card, the card's files, and a small
//! JSON API.
//!
//! Route shapes and the captive-portal probe list come from
//! `~/projects/stripe-dashboard/src/web_portal.rs`; the file server and the
//! command plumbing are ours. Everything here runs on the httpd task, which is
//! pinned to CPU0 (see [`crate::httpd`]) and is the *only* task allowed to
//! block: anything that would touch the engine, the panel or a file the player
//! has open is sent to the main task as a [`Command`] and waited on.
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
use crate::httpd::{json_string, query_param, Method, Req, Server};
use crate::nvs_config::Settings;
use crate::ota;
use crate::state::{Applied, Command, PlayError, Status, SET_NOT_FOUND};
use crate::wifi::ScannedAp;

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
}

/// Start the server and register every route.
///
/// Called from the main task before the display loop, *not* from the network
/// worker, and deliberately so: the httpd task's 16 KB stack is one contiguous
/// internal allocation, and by the time WiFi has associated the first GIF is
/// already decoding and the largest free block has fallen below it
/// (`httpd_start` then fails with `ESP_ERR_HTTPD_TASK`, 0xb008 — measured on
/// this board). Starting here, right after the index is built, takes the
/// allocation while the heap is still whole. `core_id` is in the config, so the
/// server still runs on CPU0 however it was started.
///
/// That means the mode is not known yet, so the captive-portal routes are
/// always registered and read the live WiFi state out of [`Status`] when a
/// request arrives.
pub fn start(ctx: Arc<Ctx>) -> Result<Server> {
    let mut server = Server::start(0, config::HTTP_STACK, config::HTTP_MAX_SOCKETS, 32)?;

    // --- The web app, and the setup page as its fallback -------------------
    server.route("/", Method::Get, |req| {
        let index = format!("{}/index.html", config::WWW_ROOT);
        if files::serve(req, &index)? {
            return Ok(());
        }
        send_setup(req)
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

    server.route("/api/ota", Method::Post, ota::handle_upload)?;

    server.route("/api/reboot", Method::Post, |req| {
        info!("reboot requested over HTTP");
        req.json(r#"{"ok":true,"rebooting":true}"#)?;
        ota::reboot_after_response()
    })?;

    // --- Uploads that replace a manifest -----------------------------------
    // Both go through the main task: it is the only place the engine's own
    // file handle can be dropped before the file underneath it is replaced.
    let c = ctx.clone();
    server.route("/api/manifest", Method::Put, move |req| {
        install_manifest(req, &c, config::MANIFEST_PATH.to_string())
    })?;

    let c = ctx.clone();
    server.route("/api/sets/*", Method::Put, move |req| {
        let name = req.path().trim_start_matches("/api/sets/");
        let name = crate::httpd::percent_decode(name);
        if name.is_empty() || name.contains('/') || name.contains("..") {
            return req.bad_request("set name must be a single path segment");
        }
        let dest = format!("{}/{}", config::SETS_DIR, name);
        let dest = if dest.ends_with(".txt") {
            dest
        } else {
            format!("{dest}.txt")
        };
        install_manifest(req, &c, dest)
    })?;

    // --- Arbitrary file upload and delete ----------------------------------
    let c = ctx.clone();
    server.route("/api/files/*", Method::Put, move |req| {
        let rel = crate::httpd::percent_decode(req.path().trim_start_matches("/api/files/"));
        let Some(dest) = files::card_path(config::SD_MOUNT, &rel) else {
            return req.bad_request("path escapes the card");
        };
        // A manifest (`/filelist.*`, `/sets/*`) may be the one the engine has
        // open, and it has an `.idx` sidecar that must go first. Both are the
        // main task's business, exactly as for `PUT /api/manifest`.
        if files::is_manifest_path(&dest) {
            return install_manifest(req, &c, dest);
        }
        // If the player has this very file open, ask it to move on first: FAT
        // cannot replace a file that is being read.
        let playing = c.status.lock().unwrap().path.clone();
        if !playing.is_empty()
            && files::same_card_path(&dest, &format!("{}{}", config::SD_MOUNT, playing))
        {
            let _ = c.cmd.send(Command::Skip);
            std::thread::sleep(Duration::from_millis(200));
        }
        let written = files::receive_upload(req, &dest)?;
        info!("PUT /{rel}: {written} B");
        if files::same_card_path(&dest, config::WWW_VERSION_PATH) {
            c.status.lock().unwrap().www_version = files::read_www_version();
        }
        req.json(&format!(
            r#"{{"ok":true,"path":{},"bytes":{written}}}"#,
            json_string(&format!("/{}", rel.trim_start_matches('/')))
        ))
    })?;

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
        // A manifest's `.idx` goes first, for the same crash-ordering reason
        // as on upload (see `files::remove_index_beside`): an orphaned index
        // with no manifest is harmless, the reverse is not.
        if files::is_manifest_path(&path) {
            if let Err(e) = files::remove_index_beside(&path) {
                return req.error(500, "500 Internal Server Error", &format!("{e:#}"));
            }
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                if files::same_card_path(&path, config::WWW_VERSION_PATH) {
                    c.status.lock().unwrap().www_version = None;
                }
                req.json(r#"{"ok":true}"#)
            }
            Err(e) => req.not_found(&format!("{rel}: {e}")),
        }
    })?;

    // --- Card files, read-only --------------------------------------------
    // Every root manifest — `filelist.txt`, `filelist.odm.txt`,
    // `filelist.hall`, `filelist.xmas` — served as text. Without this they
    // fell through to the `/*` www catch-all and 404ed.
    server.route("/filelist*", Method::Get, |req| {
        if !files::affordable(req)? {
            return Ok(());
        }
        let rel = crate::httpd::percent_decode(req.path());
        let name = rel.trim_start_matches('/');
        if name.contains('/') {
            return req.not_found(&rel);
        }
        let Some(path) = files::card_path(config::SD_MOUNT, name) else {
            return req.bad_request("path escapes the card");
        };
        if files::serve(req, &path)? {
            Ok(())
        } else {
            req.not_found(&rel)
        }
    })?;

    server.route("/gifs/*", Method::Get, |req| {
        serve_under(req, config::SD_MOUNT)
    })?;
    server.route("/sets/*", Method::Get, |req| {
        serve_under(req, config::SD_MOUNT)
    })?;
    // The bucketed `.odm` tree (`opendmd-pack convert --layout bucketed`).
    server.route("/odm/*", Method::Get, |req| {
        serve_under(req, config::SD_MOUNT)
    })?;

    server.route("/www/*", Method::Get, |req| {
        if !files::affordable(req)? {
            return Ok(());
        }
        let rel = crate::httpd::percent_decode(req.path().trim_start_matches("/www/"));
        let Some(path) = files::card_path(config::WWW_ROOT, &rel) else {
            return req.bad_request("path escapes the card");
        };
        if files::serve(req, &path)? {
            Ok(())
        } else {
            req.not_found(&rel)
        }
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
        if let Some(path) = files::card_path(config::WWW_ROOT, &rel) {
            if files::serve(req, &path)? {
                return Ok(());
            }
        }
        match portal_target(&c) {
            Some(target) => req.redirect(&target),
            None => req.not_found(&rel),
        }
    })?;

    Ok(server)
}

/// `Some(url)` while the device is running the setup AP: where a captive-portal
/// probe (or any unknown path) should be sent.
fn portal_target(ctx: &Ctx) -> Option<String> {
    let s = ctx.status.lock().ok()?;
    (s.wifi == "ap" && !s.ip.is_empty()).then(|| format!("http://{}/", s.ip))
}

/// Serve a card-absolute path below `root`, 404ing if it is not a file.
fn serve_under(req: &mut Req, root: &str) -> Result<()> {
    if !files::affordable(req)? {
        return Ok(());
    }
    let rel = crate::httpd::percent_decode(req.path());
    let Some(path) = files::card_path(root, &rel) else {
        return req.bad_request("path escapes the card");
    };
    if files::serve(req, &path)? {
        Ok(())
    } else {
        req.not_found(&rel)
    }
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

/// Stream a manifest upload to a temp file, then hand the swap to the main
/// task, which drops the engine before the file underneath it changes.
fn install_manifest(req: &mut Req, ctx: &Ctx, dest: String) -> Result<()> {
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

/// The playlist fields of a reply, without braces: `"entries":N,"index":…`.
/// `index` is `"loaded"`, `"built"`, `"in-ram"`, or `null` when the command did
/// not touch the playlist.
fn applied_json(a: &Applied) -> String {
    format!(
        r#""entries":{},"index":{},"index_ms":{}"#,
        a.entries,
        a.index
            .map_or_else(|| "null".to_string(), |i| json_string(i.as_str())),
        a.index_ms
    )
}

fn status_json(s: &Status) -> String {
    format!(
        concat!(
            r#"{{"version":{},"variant":{},"uptime_secs":{},"ip":{},"wifi":{},"hostname":{},"#,
            r#""now_playing":{{"ordinal":{},"path":{},"title":{}}},"paused":{},"#,
            r#""entries":{},"manifest":{},"index_state":{},"www_version":{},"#,
            r#""heap":{{"free":{},"floor":{},"idf_min":{}}},"#,
            r#""refresh_hz":{},"deadline_misses":{},"worst_lateness_ms":{},"gifs_started":{},"#,
            r#""sd":{{"mounted":{},"capacity_bytes":{}}}}}"#
        ),
        json_string(s.version),
        json_string(ota::FLASH_VARIANT),
        s.uptime_secs,
        json_string(&s.ip),
        json_string(s.wifi),
        json_string(&s.hostname),
        s.ordinal,
        json_string(&s.path),
        json_string(&s.title),
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
        s.sd_mounted,
        s.sd_capacity_bytes,
    )
}

/// The saved configuration. The WiFi password is never returned — only whether
/// one is set.
fn settings_json(s: &Settings) -> String {
    format!(
        concat!(
            r#"{{"wifi_ssid":{},"wifi_pass_set":{},"brightness":{},"#,
            r#""dwell_min":{},"dwell_max":{},"shuffle":{},"platforms":{},"set":{},"hostname":{}}}"#
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
