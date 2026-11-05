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
use std::time::{Duration, Instant};

use crate::config;
use crate::files;
use crate::httpd::{json_string, query_param, Method, Req, Respond, Server, Upload};
use crate::nvs_config::Settings;
use crate::state::{Applied, Command, PlayError, Status, SET_NOT_FOUND};
use crate::stream;
use crate::wifi::ScannedAp;
use crate::www;
use opendmd_core::compositor::{self, Kind as TransitionKind};
use opendmd_core::hostname::Check;
use opendmd_core::panel_timing::{self, PanelOp, PanelPatch};
use opendmd_core::restarts::Reason;

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
    /// A live edit of the formula on screen: 409 if there is none any more.
    #[serde(default)]
    live: bool,
    /// Draw the formula over the file on screen (until the playlist moves
    /// on) instead of playing it; `""` takes it off.
    #[serde(default)]
    over: bool,
}

/// `/api/play`'s and Previous's answer to a [`PlayError`] (or a timeout).
fn play_reply<T>(req: &mut Req, r: Result<Result<(), PlayError>, T>, ok: &str) -> Result<()> {
    match r {
        Ok(Ok(())) => req.json(ok),
        Ok(Err(PlayError::NotFound(e))) => req.not_found(&e),
        Ok(Err(PlayError::Unplayable(e))) => req.error(415, "415 Unsupported Media Type", &e),
        Ok(Err(PlayError::BadFormula(e))) => req.bad_request(&e),
        Ok(Err(PlayError::MovedOn)) => req.error(409, "409 Conflict", "no formula is on the panel now"),
        Ok(Err(PlayError::NoPlaylist)) => {
            req.error(503, "503 Service Unavailable", "no playlist is running")
        }
        Err(_) => req.error(504, "504 Gateway Timeout", "player did not answer"),
    }
}

/// Wire format for `POST /api/panel/test`: `{"setting":"clk_phase"}` or
/// `{"stop":true}`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PanelTestRequest {
    setting: Option<String>,
    stop: Option<bool>,
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
    /// (`ota::verify_signature`). Here because the network worker's stack
    /// has room for it (9.4 KB used at most with it, `config::NET_WORKER_STACK`)
    /// and the stream task's 7 KB does not.
    VerifyBundle {
        header: Box<[u8; opendmd_core::ota::HEADER_LEN]>,
        reply: mpsc::Sender<Result<(opendmd_core::ota::Meta, u8), opendmd_core::ota::OtaError>>,
    },
    /// Improv Serial over USB (`improv.rs`): a scan, or WiFi details to
    /// join and save. The worker sends the answer itself.
    Improv(crate::improv::Job),
    /// `PUT /api/config` saved a new `hostname`: answer to it now (mDNS,
    /// the DHCP name, `/api/status`, Improv's device info). No reply.
    Rename(String),
    /// Does another device answer to `<name>.local`? (`net_worker::name_check`.)
    /// The handler stops waiting at `until` ([`config::NAME_CHECK_WAIT`]);
    /// a query that could not end by then is not sent.
    NameCheck {
        name: String,
        until: Instant,
        reply: mpsc::Sender<Check>,
    },
}

/// Ask the network worker whether another device answers to `<name>.local`,
/// waiting at most [`config::NAME_CHECK_WAIT`] on the httpd task: a worker
/// that is busy for longer (a connect) or gone makes it `Check::Unknown`,
/// never a hang.
#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
#[inline(never)]
fn name_check(c: &Ctx, name: &str) -> Check {
    let until = Instant::now() + config::NAME_CHECK_WAIT;
    let (tx, rx) = mpsc::channel();
    let cmd = NetCommand::NameCheck {
        name: name.to_string(),
        until,
        reply: tx,
    };
    if c.net.send(cmd).is_err() {
        return Check::Unknown;
    }
    rx.recv_timeout(config::NAME_CHECK_WAIT).unwrap_or(Check::Unknown)
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

/// A member `"key": number|null` of the JSON object `body` (`lat`, `lon`),
/// read by hand rather than through serde, whose float parsing is ~1.5 KB
/// of the app (nothing else reads a float): `None` absent, `Some(None)`
/// null, `Some(Some(v))` the number in hundredths, rounded half away from
/// zero (`i32::MIN` for anything that isn't a plain decimal). serde has
/// already checked that `body` is JSON, and skips these members.
#[optimize(size)] // (CPU0, a config change)
fn hundredths(body: &[u8], key: &[u8]) -> Option<Option<i32>> {
    let mut from = 0;
    while let Some(k) = body[from..].windows(key.len()).position(|w| w == key) {
        let at = from + k;
        from = at + 1;
        // A member's name, not a string's insides: after `{` or `,`.
        let before = body[..at].iter().rev().find(|c| !c.is_ascii_whitespace());
        if !matches!(before, Some(b'{' | b',')) {
            continue;
        }
        let mut i = at + key.len();
        let skip = |i: &mut usize| {
            while body.get(*i).is_some_and(u8::is_ascii_whitespace) {
                *i += 1;
            }
        };
        skip(&mut i);
        if body.get(i) != Some(&b':') {
            continue;
        }
        i += 1;
        skip(&mut i);
        if body[i..].starts_with(b"null") {
            return Some(None);
        }
        let neg = body.get(i) == Some(&b'-');
        i += usize::from(neg);
        // Whole degrees (4 digits at most), then up to three decimals:
        // thousandths, rounded to hundredths.
        let (mut v, mut n, mut dot, mut dec) = (0i64, 0, false, 0);
        while let Some(&c) = body.get(i) {
            match c {
                b'0'..=b'9' if dec < 3 && (dot || n < 4) => {
                    v = v * 10 + i64::from(c - b'0');
                    if dot {
                        dec += 1;
                    } else {
                        n += 1;
                    }
                }
                b'0'..=b'9' if dot => {}
                b'.' if !dot && n > 0 => dot = true,
                b',' | b'}' | b' ' | b'\t' | b'\r' | b'\n' if n > 0 => break,
                _ => return Some(Some(i32::MIN)),
            }
            i += 1;
        }
        if n == 0 {
            return Some(Some(i32::MIN));
        }
        for _ in dec..3 {
            v *= 10;
        }
        let v = ((v + 5) / 10) as i32;
        return Some(Some(if neg { -v } else { v }));
    }
    None
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

/// Why [`apply_patch`] refused a patch: the status and the message.
pub struct PatchErr {
    pub code: u16,
    pub msg: String,
}

impl PatchErr {
    fn bad(msg: &str) -> PatchErr {
        PatchErr { code: 400, msg: msg.to_string() }
    }
    fn too_large(msg: &str) -> PatchErr {
        PatchErr { code: 413, msg: msg.to_string() }
    }
}

/// Apply `patch` to `next`, checked as `PUT /api/config` checks it (and the
/// settings file on the card, `settings_card`, read through the same
/// checks): the first value refused is the error, and `next` is then
/// partly changed (the caller drops it).
pub fn apply_patch(next: &mut Settings, patch: &ConfigPatch) -> Result<(), PatchErr> {
    if let Some(v) = patch.wifi_ssid.clone() {
        next.wifi_ssid = v;
    }
    if let Some(v) = patch.wifi_pass.clone() {
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
        Err(e) => return Err(PatchErr::bad(&e)),
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
    if let Some(v) = patch.platforms.clone() {
        next.platforms = v;
    }
    if let Some(v) = patch.set.clone() {
        if !crate::nvs_config::valid_set_name(&v) {
            return Err(PatchErr::bad("set must be a single path segment"));
        }
        next.set = v;
    }
    if let Some(v) = patch.hostname.clone() {
        match opendmd_core::hostname::parse(&v) {
            Ok(Some(v)) => next.hostname = v,
            Ok(None) => {}
            Err(e) => return Err(PatchErr::bad(&e)),
        }
    }
    if let Some(v) = patch.tz.clone() {
        let v = v.trim().to_string();
        if v.is_empty() || v.len() > 64 || v.contains('\0') {
            return Err(PatchErr::bad("tz must be a POSIX TZ string, e.g. GMT0IST,M3.5.0/1,M10.5.0"));
        }
        next.tz = v;
    }
    if let Some(v) = patch.tz_name.clone() {
        if v.len() > 64 {
            return Err(PatchErr::bad("tz_name is at most 64 characters"));
        }
        next.tz_name = v;
    }
    if let Some(v) = patch.transition.clone() {
        let Some(kind) = TransitionKind::from_name(&v) else {
            return Err(PatchErr::bad(
                "transition must be dissolve, wipe_left, wipe_right, wipe_down, wipe_up, blinds, push, fade or none",
            ));
        };
        next.transition.kind = kind;
    }
    if let Some(ms) = patch.transition_ms {
        let range = compositor::MIN_DURATION.as_millis()..=compositor::MAX_DURATION.as_millis();
        if !range.contains(&u128::from(ms)) {
            return Err(PatchErr::bad(&format!(
                "transition_ms must be {}..={}",
                range.start(),
                range.end()
            )));
        }
        next.transition.duration = Duration::from_millis(u64::from(ms));
    }
    if let Some(v) = patch.auto_prepare.clone() {
        let Some(mode) = crate::nvs_config::AutoPrepare::parse(&v) else {
            return Err(PatchErr::bad("auto_prepare must be idle, always or off"));
        };
        next.auto_prepare = mode;
    }
    if let Some(v) = patch.follow {
        next.follow = v;
    }
    if let Some(v) = patch.follow_host.clone() {
        let v = v.trim().to_string();
        // Empty (no host), or `name[:port]` with a port in digits: a
        // bad port is refused here, never quietly replaced at poll time.
        if v.len() > 64
            || v.contains(['/', ' ', '\0', '?', '#', '@'])
            || (!v.is_empty() && opendmd_core::http_client::split_host_port(&v).is_none())
        {
            return Err(PatchErr::bad("follow_host must be a host name or address, optionally :port"));
        }
        // The saved code is for the saved address: a new address without a
        // code of its own forgets it, so it is never sent anywhere else.
        if !same_follow_host(&v, &next.follow_host) && patch.follow_code.is_none() {
            next.follow_code.clear();
        }
        next.follow_host = v;
    }
    if let Some(v) = patch.follow_code.clone() {
        if v.len() > 64 || v.contains(['\0', '\r', '\n']) {
            return Err(PatchErr::bad("follow_code is at most 64 characters on one line"));
        }
        next.follow_code = v;
    }
    if let Some(v) = patch.attract_set.clone() {
        if let Some(set) = &v {
            if !crate::nvs_config::valid_set_name(set) {
                return Err(PatchErr::bad("attract_set must be a single path segment"));
            }
        }
        next.attract_set = v;
    }
    if let Some(v) = patch.attract.clone() {
        if !crate::nvs_config::valid_attract(&v) {
            return Err(PatchErr::bad(&format!(
                "attract must be \"\" (the playlist), \"clock\" or \"text:<message>\" (at most {} characters, one line); the built-in effects are gone: for a Formula, make a playlist of it and choose it as attract_set",
                crate::nvs_config::ATTRACT_MAX - 5
            )));
        }
        next.attract = v;
    }
    if let Some(v) = patch.look.clone() {
        let Some(look) = opendmd_core::look::Look::from_name(&v) else {
            return Err(PatchErr::bad("look must be colour, amber, red, green, blue or white"));
        };
        next.look = look;
    }
    if let Some(v) = patch.glow.clone() {
        let Some(glow) = opendmd_core::look::GlowLevel::from_name(&v) else {
            return Err(PatchErr::bad("glow must be off, short or long"));
        };
        next.glow = glow;
    }
    if let Some(v) = patch.look_formula.clone() {
        use opendmd_core::synth::{formula_ref, FormulaRef, MAX_LOOK_PATH};
        match formula_ref(&v) {
            _ if v.is_empty() => {}
            // A build without Formulas draws none.
            _ if !crate::state::FORMULAS => return Err(PatchErr::bad(crate::state::NO_FORMULAS)),
            Some(FormulaRef::File { file, .. })
                if files::card_path(config::SD_MOUNT, file).is_none() =>
            {
                return Err(PatchErr::bad("look_formula: the Formula's path must stay on the card"));
            }
            Some(_) => {}
            None => {
                return Err(PatchErr::bad(
                    "look_formula must be \"\" or a Formula (@formula:<name>, /formulas/<name>.psi or @formula?psb=…)",
                ))
            }
        }
        if v.len() > MAX_LOOK_PATH {
            return Err(PatchErr::too_large(&format!(
                "this formula is too long for the panel as a look ({} B as sent; the limit is {MAX_LOOK_PATH} B): save it and set it by name",
                v.len(),
            )));
        }
        next.look_formula = v;
    }
    if let Some(v) = patch.boot_show.clone() {
        let Some(len) = opendmd_core::boot::ShowLength::from_name(&v) else {
            return Err(PatchErr::bad("boot_show must be off, short or full"));
        };
        next.boot_show = len;
    }
    if let Some(v) = patch.panel_sync {
        next.panel_sync = v;
    }
    if let Some(v) = patch.panel_split {
        next.panel_split = v;
    }
    if let Some(v) = patch.formula_jit {
        next.formula_jit = v;
    }
    if let Some(v) = patch.formula_arena {
        next.formula_arena = v;
    }
    if let Some(v) = &patch.formula_keep {
        match opendmd_core::FormulaKeep::from_name(v) {
            Some(k) => next.formula_keep = k,
            None => return Err(PatchErr::bad("formula_keep must be turn, 1m, 5m or hold")),
        }
    }
    if let Some(v) = &patch.api_key {
        if !v.is_empty() && !opendmd_core::access::valid_key(v) {
            return Err(PatchErr::bad(&format!(
                "api_key must be {}..{} of A-Z a-z 0-9 . _ ~ - (or \"\" for none)",
                opendmd_core::access::KEY_MIN,
                opendmd_core::access::KEY_MAX
            )));
        }
        next.api_key = v.clone();
    }
    if let Some(v) = &patch.card_speed {
        let Some(mode) = opendmd_core::card_speed::CardSpeed::parse(v) else {
            return Err(PatchErr::bad(opendmd_core::card_speed::INVALID));
        };
        next.card_speed = mode;
    }
    match (patch.lat, patch.lon) {
        (None, None) => {}
        (Some(None), Some(None)) => next.location = None,
        (Some(Some(lat_c)), Some(Some(lon_c))) => match (lat_c.unsigned_abs() <= 9_000, lon_c.unsigned_abs() <= 18_000) {
            (true, true) => next.location = Some(opendmd_core::sky::Place { lat_c, lon_c }),
            _ => {
                return Err(PatchErr::bad(
                    "lat must be a number from -90 to 90 and lon one from -180 to 180 (degrees)",
                ))
            }
        },
        _ => return Err(PatchErr::bad("lat and lon go together: give both numbers, or both null")),
    }
    if next.dwell_max < next.dwell_min {
        return Err(PatchErr::bad("dwell_max must be >= dwell_min"));
    }
    Ok(())
}

/// Wire format for `PUT /api/config`. Every field is optional: what is absent
/// is left as it was.
#[derive(serde::Deserialize, Default)]
pub struct ConfigPatch {
    pub(crate) wifi_ssid: Option<String>,
    pub(crate) wifi_pass: Option<String>,
    pub(crate) brightness: Option<u8>,
    /// Read wider than `u8` so an out-of-range value gets the range message.
    pub(crate) max_brightness: Option<u32>,
    pub(crate) dwell_min: Option<u64>,
    pub(crate) dwell_max: Option<u64>,
    pub(crate) shuffle: Option<bool>,
    pub(crate) platforms: Option<String>,
    pub(crate) set: Option<String>,
    pub(crate) hostname: Option<String>,
    pub(crate) tz: Option<String>,
    pub(crate) tz_name: Option<String>,
    pub(crate) transition: Option<String>,
    pub(crate) transition_ms: Option<u32>,
    pub(crate) auto_prepare: Option<String>,
    pub(crate) follow: Option<bool>,
    pub(crate) follow_host: Option<String>,
    /// Never echoed back; `""` clears it.
    pub(crate) follow_code: Option<String>,
    /// `null` = the current playlist, else a `set` value.
    #[serde(default, deserialize_with = "double_option")]
    pub(crate) attract_set: Option<Option<String>>,
    /// `""`, `"clock"` or `"text:<message>"`.
    pub(crate) attract: Option<String>,
    pub(crate) look: Option<String>,
    pub(crate) glow: Option<String>,
    /// The Formula look: a formula's `@formula?psb=…` path, `""` = none
    /// (checked by the main task before anything is saved).
    /// Its name, blend and amount are the formula's own `name` and `look`
    /// lines (no fields of their own: unknown fields are ignored).
    pub(crate) look_formula: Option<String>,
    pub(crate) boot_show: Option<String>,
    /// Refresh-synchronised blits, applied at once.
    pub(crate) panel_sync: Option<bool>,
    /// With it, the bottom half a refresh after the top; applied at once.
    pub(crate) panel_split: Option<bool>,
    /// The Formulas' JIT (the kill switch); applied at the next frame.
    pub(crate) formula_jit: Option<bool>,
    pub(crate) formula_arena: Option<bool>,
    /// How long a Formula sent to the panel stays: `turn`, `1m`, `5m` or
    /// `hold` (until something else plays).
    pub(crate) formula_keep: Option<String>,
    /// The API key (`opendmd_core::access`); `""` turns it off. Never
    /// echoed back (only `api_key_set`).
    pub(crate) api_key: Option<String>,
    /// `safe`, `fast` or `full` (`opendmd_core::card_speed`): in effect at
    /// once when a card is mounted (`cardspeed::set_mode`).
    pub(crate) card_speed: Option<String>,
    /// The location in degrees (rounded to 0.01°), both or neither; both
    /// `null` clears it. Kept in NVS only (never the card's file:
    /// `settings_card` has no group for them).
    /// (In hundredths of a degree: read from the body by [`hundredths`].)
    #[serde(skip)]
    pub(crate) lat: Option<Option<i32>>,
    #[serde(skip)]
    pub(crate) lon: Option<Option<i32>>,
    /// Panel timing: an object is tried (reverts unless confirmed), `null`
    /// saves the defaults (`PanelOp`).
    #[serde(default, deserialize_with = "double_option_panel")]
    panel: Option<Option<PanelWire>>,
    /// The schedule's JSON: read from the settings file on the card only
    /// (`settings_card`); `PUT /api/config` ignores it (`PUT /api/schedule`
    /// sets the schedule).
    pub(crate) schedule: Option<String>,
    /// Dev: where upload pacing starts, % of a Formula's frame time
    /// (`pace`); live only, never saved.
    pub(crate) pace_on_pct: Option<u32>,
    /// `true` keeps the trial (with `panel`: saved without one), `false`
    /// undoes it.
    pub(crate) panel_confirm: Option<bool>,
}

/// Wire format for `POST /api/card/format`: `{"confirm": true}`, so an
/// empty or stray POST never erases a card.
#[derive(serde::Deserialize)]
struct FormatRequest {
    #[serde(default)]
    confirm: bool,
}

/// Two RePlayOS addresses are the same box (case and spaces aside).
fn same_follow_host(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
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
    // URI handler slots: one per `route`/`route_json` below (49 now, the
    // captive-portal loop counting six; 50 leaves one spare). Registration
    // past the limit fails the whole server, so count when adding a route.
    let mut server = Server::start(0, config::HTTP_STACK, config::HTTP_MAX_SOCKETS, config::HTTP_MAX_HANDLERS)?;
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
        // The browser's copy is this pack's: 304, no hand-off (www.rs "Caching").
        if www::not_modified(req)? {
            return Ok(());
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
        // Measurement (display-isolation): the heap's use per SRAM pool, or
        // the player's longest card read and open since the last ask.
        if query_param(req.query(), "pools").as_deref() == Some("1") {
            return req.json(&crate::membank::pools_json());
        }
        if query_param(req.query(), "take_read_max").as_deref() == Some("1") {
            return req.json(&crate::membank::take_read_max());
        }
        // The heap map (a `heapmap` build only): the whole of it, fresh,
        // in parts of a few KB each (`heapmap::part`).
        if query_param(req.query(), "heapmap").as_deref() == Some("1") {
            if let Some(first) = crate::heapmap::part(0) {
                req.begin_chunked("200 OK", "application/json")?;
                req.chunk(first.as_bytes())?;
                drop(first);
                let mut k = 1;
                while let Some(p) = crate::heapmap::part(k) {
                    req.chunk(p.as_bytes())?;
                    k += 1;
                }
                return req.finish();
            }
        }
        // The settings' part, made when it is written (last): the status
        // copy holds no lock by then, so never both locks at once. With
        // the heap map's short form, as `"heapmap"`, in every status.
        let looks = || {
            let looks = {
                let set = c.settings.lock().unwrap();
                looks_json(&set)
            };
            match crate::heapmap::status_brief() {
                Some(h) => format!(r#""heapmap":{h},{looks}"#),
                None => looks,
            }
        };
        let recent = query_param(req.query(), "recent").as_deref() == Some("1");
        // A status read costs CPU0 ~7 ms of formatting (code and strings
        // from flash, whose bus CPU1 shares): answered from the last one
        // for `STATUS_CACHE` (a client polling faster than that gets the
        // same JSON within it). The lock is held only to copy the status
        // (µs), never while formatting. The next answer says what it took
        // (`display.status_lock_us`, `status_build_us`, `status_cached`).
        let t0 = std::time::Instant::now();
        // The last answer, if it is fresh: sent from where it is kept (no
        // copy). A stale one is freed here, before a new one is built.
        let fast = {
            let mut last = STATUS_LAST.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(kept) = last.as_ref().filter(|k| k.recent == recent && k.at.elapsed() < STATUS_CACHE) {
                STATUS_CACHED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return req.json(&kept.json);
            }
            *last = None;
            // A client reading faster than `STATUS_CACHE` (the read before
            // this one came within it): this answer is kept for its next.
            let prev = STATUS_READ.lock().unwrap_or_else(|p| p.into_inner()).replace(t0);
            prev.is_some_and(|p| t0.duration_since(p) < STATUS_CACHE)
        };
        // net-wedge2: below this the answer is a 503 at once. Its ~3.7 KB of
        // segments stay in lwIP until the client's ACK, and the radio copies
        // each frame into a buffer of its own; the WiFi driver allocates its
        // receive and transmit buffers from the same heap, and on the spare
        // a heap taken to 8 KB under five uploads at once left device-sent
        // frames undelivered for seconds (every refusal 10.5 s late: the
        // third send of a first segment, lwIP's 1.5 + 3 + 6 s).
        let free = crate::state::free_heap();
        if free < STATUS_MIN_HEAP {
            STATUS_REFUSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            req.header("Retry-After", "1")?;
            return req.error(503, "503 Service Unavailable", "the player needs the heap right now; retry shortly");
        }
        let snap = status_snapshot(&c.status);
        if fast {
            // One block for the answer, the size of the last one and some:
            // no regrowth while it is written, and no abort when the heap
            // has no such block.
            let mut json = String::new();
            let want = STATUS_LEN.load(std::sync::atomic::Ordering::Relaxed) as usize + STATUS_SLACK;
            if json.try_reserve_exact(want).is_err() {
                req.header("Retry-After", "1")?;
                return req.error(503, "503 Service Unavailable", "the player needs the heap right now; retry shortly");
            }
            status_json(&snap, &looks, recent, &mut json);
            drop(snap);
            STATUS_LEN.store(json.len() as u32, std::sync::atomic::Ordering::Relaxed);
            STATUS_BUILD_US.store(t0.elapsed().as_micros() as u32, std::sync::atomic::Ordering::Relaxed);
            let sent = req.json(&json);
            *STATUS_LAST.lock().unwrap_or_else(|p| p.into_inner()) = Some(KeptStatus { at: std::time::Instant::now(), recent, json });
            return sent;
        }
        // net-wedge2: written out as it is made, in pieces of at most
        // `STATUS_PIECE` (chunked), never whole: the whole answer (~3.6 KB)
        // and every part of it at once (the old single `write!`'s
        // arguments) were the emulator's lowest heap under uploads beside
        // status reads (the low watch: the httpd task in a status read, the
        // stream task in an upload, every time).
        let Some(mut out) = Piecewise::new(req) else {
            req.header("Retry-After", "1")?;
            return req.error(503, "503 Service Unavailable", "the player needs the heap right now; retry shortly");
        };
        out.req.begin_chunked("200 OK", "application/json")?;
        status_json(&snap, &looks, recent, &mut out);
        drop(snap);
        let len = out.finish()?;
        STATUS_LEN.store(len as u32, std::sync::atomic::Ordering::Relaxed);
        STATUS_BUILD_US.store(t0.elapsed().as_micros() as u32, std::sync::atomic::Ordering::Relaxed);
        req.finish()
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
        play_reply(req, rx.recv_timeout(REPLY_TIMEOUT), r#"{"ok":true}"#)
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
        let mut patch: ConfigPatch = match serde_json::from_slice(&body) {
            Ok(p) => p,
            Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
        };
        (patch.lat, patch.lon) = (hundredths(&body, b"\"lat\""), hundredths(&body, b"\"lon\""));
        // On the open setup network: the WiFi, the name and the key, and
        // nothing else (`opendmd_core::access::SETUP_CONFIG_KEYS`).
        if crate::httpd::setup_mode() {
            patch = ConfigPatch {
                wifi_ssid: patch.wifi_ssid.take(),
                wifi_pass: patch.wifi_pass.take(),
                hostname: patch.hostname.take(),
                api_key: patch.api_key.take(),
                ..Default::default()
            };
        }
        let mut next = c.settings.lock().unwrap().clone();
        let wifi_before = (next.wifi_ssid.clone(), next.wifi_pass.clone());
        let hostname_before = next.hostname.clone();
        let sky_before = (next.location, next.tz.clone());
        if let Some(p) = patch.pace_on_pct {
            crate::pace::set_on_pct(p);
        }
        if let Err(e) = apply_patch(&mut next, &patch) {
            return match e.code {
                413 => req.error(413, "413 Payload Too Large", &e.msg),
                _ => req.bad_request(&e.msg),
            };
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
        // WiFi details are read once, when the network starts: saved now,
        // in effect after a reboot. A new name is applied live once saved
        // (`NetCommand::Rename`).
        let reboot_required = (next.wifi_ssid.clone(), next.wifi_pass.clone()) != wifi_before;
        let renamed = (next.hostname != hostname_before).then(|| next.hostname.clone());
        // A name another device already answers to is refused: mDNS would
        // hand it to this display and push the other to `<name>-2`. Asked
        // last, once everything else is known good; "could not ask" (no
        // mDNS, a busy worker) lets the save go ahead.
        if let Some(name) = &renamed {
            if let Check::Taken(by) = name_check(&c, name) {
                info!("hostname {name} refused: {name}.local answers from {by}");
                let body = format!(
                    r#"{{"error":{},"name":{},"by":{}}}"#,
                    json_string(&opendmd_core::hostname::taken_message(name, &by)),
                    json_string(name),
                    json_string(&by)
                );
                return req.respond_json("409 Conflict", &body);
            }
        }
        let (saved_brightness, saved_max) = (next.brightness, next.max_brightness);
        let card_speed = next.card_speed;
        // The sky's inputs depend on the location and the zone.
        let sky_changed = (next.location, next.tz.clone()) != sky_before;
        let location = next.location;
        // The shared copy and the log line are this handler's (CPU0), once
        // the main task has applied it: the display core neither clones
        // the settings nor formats a line for a PUT.
        let shared = next.clone();

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
        let answer = rx.recv_timeout(REINDEX_TIMEOUT);
        // The settings the main task replaced (or refused), freed here on
        // CPU0 rather than on the display core.
        crate::state::drop_spent();
        match answer {
            Ok(Ok(applied)) => {
                info!(
                    "config applied: brightness={}, look={}, glow={}, dwell={}..{}s, shuffle={}, platforms='{}', set='{}', transition={}, attract='{}', panel_sync={} (split {})",
                    shared.brightness,
                    shared.look.name(),
                    shared.glow.name(),
                    shared.dwell_min,
                    shared.dwell_max,
                    shared.shuffle,
                    shared.platforms,
                    shared.set,
                    shared.transition,
                    shared.attract,
                    shared.panel_sync,
                    shared.panel_split
                );
                *c.settings.lock().unwrap() = shared;
                // In effect at once: the sky's inputs for the new location
                // (or zone), here on CPU0 (the zone is already set).
                if sky_changed {
                    crate::inputs::set_place(location);
                }
                // Durable at once on the card (CPU0; `settings_card`).
                crate::settings_card::save_shared(&c.settings);
                if let Some(name) = renamed {
                    // Queued, not waited for: the worker may be mid-connect.
                    if c.net.send(NetCommand::Rename(name)).is_err() {
                        log::warn!("hostname saved, but the network worker is gone: in effect after a reboot");
                    }
                }
                if let Some(k) = &patch.api_key {
                    crate::httpd::set_api_key(k);
                    info!("API key {}", if k.is_empty() { "cleared" } else { "set" });
                }
                // Saved: now the clock (here, on CPU0, under the card's
                // I/O lock), and when it took effect; `restart`: it could
                // not be changed in place, so the device restarts (after
                // the reply) and the saved setting applies at that boot.
                let speed_applied = patch.card_speed.map(|_| crate::cardspeed::set_mode(card_speed));
                let speed = match speed_applied {
                    Some(a) => format!(r#","card_speed_applied":"{}""#, a.name()),
                    None => String::new(),
                };
                let (panel, restart) = {
                    let st = c.status.lock().unwrap();
                    (panel_json(&st), st.panel.restart_needed())
                };
                // A restart this request's panel change waits for (not one
                // pending from before: the WiFi banner keys off this).
                let reboot_required = reboot_required || (panel_op.is_some() && restart);
                req.json(&format!(
                    r#"{{"ok":true,{},"brightness":{saved_brightness},"max_brightness":{saved_max},"reboot_required":{reboot_required},{panel}{speed}}}"#,
                    applied_json(&applied)
                ))?;
                if speed_applied == Some(crate::cardspeed::Applied::Restart) {
                    reboot_after_response(Reason::CardClock);
                }
                Ok(())
            }
            Ok(Err(e)) if e == panel_timing::NO_TRIAL => req.error(409, "409 Conflict", &e),
            // Checked by the main task before anything was saved: the
            // configuration and the running playlist are as they were.
            Ok(Err(e)) if e == SET_NOT_FOUND => req.not_found(SET_NOT_FOUND),
            // Likewise refused, the previous playlist rebuilt: nothing saved.
            Ok(Err(e)) if e.starts_with(crate::state::FILTER_EMPTY) => req.bad_request(&e),
            // The formula look does not compile or is too heavy: nothing saved.
            Ok(Err(e)) if e.starts_with(crate::state::LOOK_FORMULA_BAD) => req.bad_request(&e),
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
        let (rules, night) = (parsed.rules.len(), parsed.night.is_some());
        let shared = json.clone();
        let (tx, rx) = mpsc::channel();
        if c.cmd
            .send(Command::SetSchedule {
                json,
                parsed: Box::new(parsed),
                reply: tx,
            })
            .is_err()
        {
            return req.error(503, "503 Service Unavailable", "player not running");
        }
        // Applying may switch the playlist (a rule in force now).
        match rx.recv_timeout(REINDEX_TIMEOUT) {
            Ok(Ok(_)) => {
                // (The shared copy and the line are this handler's, on CPU0.)
                c.settings.lock().unwrap().schedule = shared;
                crate::settings_card::save_shared(&c.settings);
                info!("schedule saved: {rules} rule(s){}", if night { " + night mode" } else { "" });
                req.json(&format!(r#"{{"ok":true,"rules":{rules},"night":{night}}}"#))
            }
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
        // An inline formula (`@formula?psb=…`) may be up to 6 KB; it is
        // compiled on the main task as it opens (a compile error or one too
        // heavy for the panel is a 400 with the compiler's sentence; a
        // bundled name or a card file that isn't there, a 404).
        if opendmd_core::synth::is_formula(&play.path)
            && play.path.len() > opendmd_core::synth::MAX_FORMULA_PATH
        {
            return req.error(
                413,
                "413 Payload Too Large",
                &format!(
                    "this formula is too long for the panel ({} B as sent; the limit is {} B)",
                    play.path.len(),
                    opendmd_core::synth::MAX_FORMULA_PATH
                ),
            );
        }
        // A build without Formulas (`formulas` off): a Formula, inline,
        // bundled or the card's, or one drawn over the GIF, is a 501 that
        // says so (the web app hides Formulas there; a script hears why).
        if !crate::state::FORMULAS
            && (opendmd_core::synth::is_formula(&play.path)
                || opendmd_core::synth::formula_file(&play.path).is_some()
                || play.over
                || play.live)
        {
            return req.error(501, "501 Not Implemented", crate::state::NO_FORMULAS);
        }
        // A built-in effect's path from before they were Formulas: a 415
        // that says so.
        if opendmd_core::synth::is_retired_effect(&play.path) {
            return req.error(
                415,
                "415 Unsupported Media Type",
                "the built-in effects are gone: Formulas replace them (play one as @formula?psb=…)",
            );
        }
        // A virtual file (`@clock`, `@text?msg=…`) is drawn, not read; one
        // this build does not know is a 415 from the engine.
        if play.live && !opendmd_core::synth::is_formula(&play.path) {
            return req.bad_request("\"live\" is only for a formula (@formula?psb=…)");
        }
        let formula_file = opendmd_core::synth::formula_file(&play.path);
        if play.over
            && !play.path.is_empty()
            && !opendmd_core::synth::is_formula(&play.path)
            && formula_file.is_none()
        {
            return req.bad_request(
                "\"over\" is only for a formula (@formula?psb=…), or \"\" to take it off",
            );
        }
        let virtual_file = opendmd_core::synth::is_virtual(&play.path);
        // A Formula's file may carry its controls after `?`.
        let on_card = formula_file.map_or(play.path.as_str(), |(f, _)| f);
        if !(play.over && play.path.is_empty())
            && !virtual_file
            && (!on_card.starts_with('/')
                || files::card_path(config::SD_MOUNT, on_card).is_none())
        {
            return req.bad_request("path must be card-absolute and stay on the card, or a live item (@clock, @text?msg=…)");
        }
        let (tx, rx) = mpsc::channel();
        if c.cmd
            .send(Command::PlayNow {
                path: play.path.clone(),
                live: play.live,
                over: play.over,
                reply: tx,
            })
            .is_err()
        {
            return req.error(503, "503 Service Unavailable", "player not running");
        }
        let ok = format!(
            r#"{{"ok":true,"path":{}{}}}"#,
            json_string(&play.path),
            if play.over { r#","over":true"# } else { "" }
        );
        play_reply(req, rx.recv_timeout(REPLY_TIMEOUT), &ok)
    })?;

    // The panel-timing test (`opendmd_core::panel_test`): the firmware's
    // patterns with one live setting flipped between A and B, ended by
    // `stop`, by any settings change or after two minutes; nothing saved.
    let c = ctx.clone();
    server.route_json("/api/panel/test", Method::Post, move |req| {
        let Some(body) = json_body(req)? else {
            return Ok(());
        };
        let w: PanelTestRequest = match serde_json::from_slice(&body) {
            Ok(w) => w,
            Err(e) => return req.bad_request(&format!("invalid JSON: {e}")),
        };
        use opendmd_core::panel_test::{self as pt, Setting};
        let setting = match (w.setting.as_deref(), w.stop) {
            (None, Some(true)) => None,
            (Some(s), None | Some(false)) => match Setting::from_name(s) {
                Some(s) => Some(s),
                None => return req.bad_request("setting must be clk_phase or latch_blanking"),
            },
            _ => return req.bad_request(r#"send {"setting":"clk_phase"|"latch_blanking"} or {"stop":true}"#),
        };
        let (tx, rx) = mpsc::channel();
        if c.cmd.send(Command::PanelTest { setting, reply: tx }).is_err() {
            return req.error(503, "503 Service Unavailable", "player not running");
        }
        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(Ok(_)) => match setting {
                None => req.json(r#"{"ok":true}"#),
                Some(s) => {
                    let running = c.status.lock().unwrap().panel.running;
                    let (a, b) = pt::variants(s, &running);
                    req.json(&format!(
                        r#"{{"ok":true,"setting":"{}","a":{},"b":{},"left_s":{}}}"#,
                        s.name(),
                        a.json(),
                        b.json(),
                        pt::LIMIT.as_secs()
                    ))
                }
            },
            Ok(Err(e)) => req.error(409, "409 Conflict", &e),
            Err(_) => req.error(504, "504 Gateway Timeout", "player did not answer"),
        }
    })?;

    // Existence, size and kind of one path, with one `stat`. That is one scan
    // of the parent directory under the FATFS lock — cheap in a bucket or
    // `www/`, ~330-550 ms in the 3,781-entry `gifs/Consoles`, where the
    // player's next open waits behind it — but still far cheaper than listing
    // the parent. `size` is null for a directory. (Not `/api/stat`: EasyPrivacy
    // blocks that path in uBlock Origin and Brave; `make blocklist-check`.)
    server.route("/api/exists", Method::Get, |req| {
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
    // On the stream task, like a card file: its bucket walk must not hold
    // the httpd task (and `/api/status`) for the seconds it takes
    // (`listing`: no wait for a place, a 503 + Retry-After: 1 the web app
    // retries; the task holds `cache_builder::busy()` for every job).
    server.route("/api/cache/list", Method::Get, |req| listing(req, crate::cache::stream_list))?;

    // The GIFs the cache builder could not prepare (`/cache/.failed`), and
    // forgetting them so they are tried again.
    server.route("/api/cache/failed", Method::Get, |req| listing(req, crate::cache_builder::stream_failed))?;
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
        if files::card_path_for_write(config::SD_MOUNT, &forget.path).is_none()
            || opendmd_core::layout::cache_path(&forget.path).is_none()
        {
            return req.bad_request("path must be a card-absolute /gifs/….gif");
        }
        // The copy may be the one opened ahead (the GIF's path finds it).
        let _busy = crate::cache_builder::busy();
        let Some(_held) = release_for_write(&c, &format!("{}{}", config::SD_MOUNT, forget.path)) else {
            return req.error(503, "503 Service Unavailable", NOT_RELEASED);
        };
        match crate::cache::forget(&forget.path) {
            Ok(removed) => {
                if let Some(copy) = &removed {
                    crate::fatidx::note_gone(&format!("{}{copy}", config::SD_MOUNT));
                }
                req.json(&format!(
                r#"{{"ok":true,"path":{},"removed":{}}}"#,
                json_string(&forget.path),
                removed.is_some()
            ))
            }
            Err(e) => req.error(500, "500 Internal Server Error", &format!("{e}")),
        }
    })?;

    // A directory listing reads the directory under the FAT lock, ~0.5 s
    // for gifs/Consoles at 40 MHz and 2.3 s on a 20 MHz card, streamed: on
    // the httpd task it held `/api/status` that long (measured on the spare,
    // 2026-10-26). So it goes out from the stream task, queued like a file
    // (a full queue for 2 s: 503 + Retry-After: 1, retried by the web app).
    server.route("/api/files", Method::Get, |req| {
        let dir = query_param(req.query(), "dir").unwrap_or_else(|| "/".to_string());
        let Some(fs_dir) = files::card_path(config::SD_MOUNT, &dir) else {
            return req.bad_request("path escapes the card");
        };
        // Sizes cost a `stat` each, which FAT makes quadratic — see
        // `files::stream_dir_json`. Opt in with `?sizes=1`.
        let sizes = query_param(req.query(), "sizes").is_some_and(|v| v == "1" || v == "true");
        // `?names=1`: a /gifs folder's names from the index's names file,
        // no directory walk (naming a new GIF); listed as usual when that
        // file cannot be trusted.
        if query_param(req.query(), "names").as_deref() == Some("1") && !sizes {
            if let Some(names) = crate::fatidx::names(&dir) {
                return listing(req, move |req| stream_names(req, &dir, names));
            }
        }
        listing(req, move |req| files::stream_dir_json(req, &dir, &fs_dir, sizes))
    })?;

    // Would a new display name clash? (`opendmd_core::hostname::Check`.)
    // `{"name","valid","taken","by"}`: `taken` null when it could not ask.
    let c = ctx.clone();
    server.route("/api/name-check", Method::Get, move |req| {
        let Some(raw) = query_param(req.query(), "name") else {
            return req.bad_request("name= required");
        };
        let name = raw.trim().to_ascii_lowercase();
        let valid = opendmd_core::hostname::valid(&name);
        let check = if !valid {
            Check::Unknown
        } else if name == c.settings.lock().unwrap().hostname {
            // Its own name: whoever else answers to it, this display has it.
            Check::Free
        } else {
            name_check(&c, &name)
        };
        req.json(&format!(
            r#"{{"name":{},"valid":{valid},"taken":{},"by":{}}}"#,
            json_string(&name),
            check.taken_json(),
            check.by().map_or_else(|| "null".to_string(), json_string)
        ))
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
        // The saved code goes only to the saved address: a test of another
        // one brings its own code (M1, docs/api.md).
        if t.code.is_none() {
            let set = c.settings.lock().unwrap();
            if !set.follow_code.is_empty() && !t.host.trim().is_empty() && !same_follow_host(&t.host, &set.follow_host) {
                drop(set);
                return req.bad_request("a new address needs its NET CONTROL code");
            }
        }
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

    // "Try full speed again": forget this card's slow-clock record and go
    // back to full speed, in place when a card is mounted (`cardspeed`),
    // else at the next mount; a restart only if the clock cannot be
    // changed. `?test_errors=N` (test hook): the next N card transfers at
    // full speed fail on purpose, so the in-place slow-down can be watched
    // on a good card.
    server.route("/api/card/full-speed", Method::Post, |req| {
        // `?test_errors=N`: `bench` builds only (ignored in a release).
        #[cfg(feature = "bench")]
        let test = query_param(req.query(), "test_errors")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        #[cfg(not(feature = "bench"))]
        let test = 0;
        info!("try full speed again, asked over HTTP{}", if test > 0 { " (test hook armed)" } else { "" });
        match crate::cardspeed::full_speed_again(test) {
            Err(e) => req.error(409, "409 Conflict", e),
            Ok(crate::cardspeed::Applied::Restart) => {
                req.json(r#"{"ok":true,"applied":"restart","rebooting":true}"#)?;
                reboot_after_response(Reason::FullSpeed)
            }
            Ok(a) => req.json(&format!(
                r#"{{"ok":true,"applied":"{}","clock_khz":{}}}"#,
                a.name(),
                match crate::cardspeed::khz() {
                    0 => "null".to_string(),
                    k => k.to_string(),
                }
            )),
        }
    })?;

    #[cfg(feature = "bench")]
    let c_cut = ctx.clone();
    server.route("/api/reboot", Method::Post, move |req| {
        // Test hook, `bench` builds only (a release has no such code):
        // `?panic=1` panics on purpose, on this (the httpd) task, to check
        // what a crash leaves: the panic message and IDF's backtrace on the
        // console, the log ring flushed first, and `reset_reason: "panic"`
        // after the restart.
        #[cfg(feature = "bench")]
        if query_param(req.query(), "panic").is_some() {
            log::warn!("panicking on purpose (POST /api/reboot?panic=1, a bench test hook)");
            req.json(r#"{"ok":true,"panicking":true}"#)?;
            std::thread::sleep(Duration::from_millis(500));
            panic!("test panic asked for over HTTP (POST /api/reboot?panic=1)");
        }
        // `?overflow=1` (`bench` only): the stack-overflow hook's path, end
        // to end (`restarts::overflow_on_purpose`) — after the restart
        // `restarts[0]` is `stack_overflow`, `task httpd`.
        #[cfg(feature = "bench")]
        if query_param(req.query(), "overflow").is_some() {
            log::warn!("overflowing the stack on purpose (POST /api/reboot?overflow=1, a bench test hook)");
            req.json(r#"{"ok":true,"overflowing":true}"#)?;
            std::thread::sleep(Duration::from_millis(500));
            crate::restarts::overflow_on_purpose();
        }
        // `?settings_cut=1` (`bench` only): the power-cut test of the
        // settings file: a save stopped between its renames (the old file
        // moved aside, the new one not yet in its place), then this reboot.
        // The next boot must come up with the settings in force now.
        #[cfg(feature = "bench")]
        if query_param(req.query(), "settings_cut").is_some() {
            crate::settings_card::save_cut(&c_cut.settings);
            log::warn!("settings: a save stopped between its renames on purpose (bench); restarting");
        }
        info!("reboot requested over HTTP");
        // The card-failure test hooks (`bench` builds only: a release has
        // none, so a LAN caller cannot make the next boots fail on purpose).
        #[cfg(feature = "bench")]
        let hooked = card_test_hooks(req.query());
        #[cfg(not(feature = "bench"))]
        let hooked = false;
        // A plain reboot (the UI's button, no hook) is a person's fresh
        // start: the card-failure restart count goes, the slow clock stays.
        if !hooked {
            crate::sdhealth::request_fresh_start();
        }
        req.json(r#"{"ok":true,"rebooting":true}"#)?;
        reboot_after_response(Reason::ApiReboot)
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
        // `?new=1`: only if there is no such file (409 otherwise): a name
        // picked from a names list that has gone stale never replaces a GIF.
        let create_new = query_param(req.query(), "new").as_deref() == Some("1");
        stream::receive(req, move |up| put_file(up, &c, &rel, create_new))
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
        let Some(path) = files::card_path_for_write(config::SD_MOUNT, &rel) else {
            return req.bad_request(files::BAD_WRITE_PATH);
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
        let Some(_held) = release_for_write(&c, &path) else {
            return req.error(503, "503 Service Unavailable", NOT_RELEASED);
        };
        crate::fatidx::touched(&path);
        // A directory is told apart before anything is removed: FatFs's
        // `f_unlink`, behind `remove_file`, removes an empty directory as
        // readily as a file, so an emptied category deleted through
        // `remove_file` never reached `dir_changed` and the index went on
        // pointing into it (EMPTY_FOLDER, the emulator's API lane).
        if std::fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
            // An empty category (a directory under gifs/) goes: what is left
            // when its last GIF was deleted or moved away. The index stops
            // using its records first (`dir_changed`).
            if files::under(&path, &format!("{}/gifs", config::SD_MOUNT)) {
                crate::fatidx::dir_changed();
            }
            return match std::fs::remove_dir(&path) {
                Ok(()) => {
                    if files::under(&path, config::CACHE_ROOT) {
                        crate::cache::invalidate();
                    }
                    req.json(r#"{"ok":true}"#)
                }
                Err(e) => req.error(409, "409 Conflict", &format!("{rel}: not empty ({e})")),
            };
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                crate::fatidx::note_gone(&path);
                // A GIF takes its prepared copy with it; a copy removed
                // directly changes the count.
                let card = format!("/{}", rel.trim_start_matches('/'));
                match crate::cache::forget(&card) {
                    Ok(Some(copy)) => crate::fatidx::note_gone(&format!("{}{copy}", config::SD_MOUNT)),
                    Ok(None) => {}
                    Err(e) => log::warn!("DELETE {card}: its prepared copy: {e}"),
                }
                if files::under(&path, config::CACHE_ROOT) {
                    crate::cache::invalidate();
                }
                req.json(r#"{"ok":true}"#)
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
        stream_file(req, path, rel, stream::QUEUE_WAIT)
    })?;

    // GIFs and their copies: a full stream queue is a 503 at once (the web
    // app's thumbnails back off, its other reads retry: `queued`), so they
    // never hold the httpd task; a playlist waits for a place as a module does.
    server.route("/gifs/*", Method::Get, |req| {
        serve_under(req, config::SD_MOUNT, Duration::ZERO)
    })?;
    server.route("/sets/*", Method::Get, |req| {
        serve_under(req, config::SD_MOUNT, stream::QUEUE_WAIT)
    })?;
    // The user's Formulas (`/formulas/<stem>.psi`: the web app lists and
    // reads them, and writes them through `PUT /api/files/formulas/…`).
    server.route("/formulas/*", Method::Get, |req| {
        serve_under(req, config::SD_MOUNT, stream::QUEUE_WAIT)
    })?;
    // The playback cache: prepared copies, `cache/<Category>/<xx>/<name>.gif`
    // (the device's own builder, `opendmd-pack convert`, or the web app).
    server.route("/cache/*", Method::Get, |req| {
        serve_under(req, config::SD_MOUNT, Duration::ZERO)
    })?;

    // The web app under `/www/` too (both spellings, as when it was on the
    // card).
    server.route("/www/*", Method::Get, |req| {
        let rel = crate::httpd::percent_decode(req.path().trim_start_matches("/www/"));
        if www::not_modified(req)? {
            return Ok(());
        }
        stream::hand_off(req, move |req| match www::serve(req, &rel)? {
            www::Served::Sent => Ok(()),
            www::Served::NotFound => req.not_found(&format!("/www/{rel}")),
            www::Served::Unavailable(why) => www::unavailable(req, &why),
        })
    })?;

    // --- Captive portal ----------------------------------------------------
    // Registered unconditionally; each one is a redirect in AP mode and a plain
    // 404 otherwise, because on a real network these paths belong to nobody.
    for probe in opendmd_core::access::PORTAL_PROBES {
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
        // Only a file this pack served carries its ETag back.
        if www::not_modified(req)? {
            return Ok(());
        }
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

/// `POST /api/reboot`'s card-failure test hooks (`bench` only; see the
/// route): `?sd_fail=N`, `?sd_io_fail=N[&sd_io_boots=B]`,
/// `?sd_fallback=clear`. Whether any was asked for.
#[cfg(feature = "bench")]
fn card_test_hooks(query: Option<&str>) -> bool {
    // Test hook: `?sd_fail=N` makes the next boot's first N card mounts
    // fail, to exercise the no-card boot without touching the card.
    if let Some(n) = query_param(query, "sd_fail").and_then(|v| v.parse::<u32>().ok()) {
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
    if let Some(n) = query_param(query, "sd_io_fail").and_then(|v| v.parse::<u32>().ok()) {
        let boots = query_param(query, "sd_io_boots")
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
    if query_param(query, "sd_fallback").as_deref() == Some("clear") {
        info!("the next boot forgets the card's slow clock and its card-failure restarts");
        crate::sdhealth::request_clear();
    }
    ["sd_fail", "sd_io_fail", "sd_fallback"]
        .iter()
        .any(|k| query_param(query, k).is_some())
}

/// A JSON listing (`/api/files`, `/api/cache/list`, `/api/cache/failed`)
/// on the stream task: no wait for a place in a full queue (a 503 +
/// `Retry-After: 1` at once, which the web app retries), and the heap
/// checked again when it starts, as `files::serve` does for a file: the
/// queue may have held it while the heap dipped, and a chunked answer
/// started is one the player's reserve must afford to finish.
fn listing<F>(req: &mut Req, work: F) -> Result<()>
where
    F: FnOnce(&mut Req) -> Result<()> + Send + 'static,
{
    stream::hand_off_within(req, Duration::ZERO, move |req| {
        if !files::affordable(req)? {
            return Ok(());
        }
        work(req)
    })
}

/// `Some(url)` while the device is running the setup AP: where a captive-portal
/// probe (or any unknown path) should be sent.
fn portal_target(ctx: &Ctx) -> Option<String> {
    let s = ctx.status.lock().ok()?;
    (s.wifi == "ap" && !s.ip.is_empty()).then(|| format!("http://{}/", s.ip))
}

/// The write intents a writer holds (`fatidx::WriteIntent`) from before it
/// waits for the player until its card work is done: drop it last.
#[must_use]
struct Held(#[allow(dead_code)] Vec<crate::fatidx::WriteIntent>);

/// Before a writer changes, replaces, renames or deletes `fs_path` (and,
/// for a GIF, its prepared copy): register write intents for them, so
/// nothing opens them for raw reads from now on (`fatidx`: the player then
/// reads them through FatFs, which fails cleanly on a freed chain); have
/// the player let go of them if it has them open — the file on screen, the
/// copy it reads for it, the copy opened ahead — answered by the main loop
/// at once (`Command::Release`, the loop woken from its frame's sleep);
/// and wait until neither is held. The caller keeps the returned intents
/// to the end of its card work, and holds `cache_builder::busy()` too, so
/// nothing is opened ahead meanwhile (`state::AHEAD_KEY`). `None` if the
/// player did not let go: the caller refuses and touches nothing.
///
/// A path that is an 8.3 alias (`NES_MA~1.GIF`) may name any file of its
/// folder: the intent and the wait are for the whole folder then.
fn release_for_write(ctx: &Ctx, fs_path: &str) -> Option<Held> {
    let card = fs_path.strip_prefix(config::SD_MOUNT).unwrap_or(fs_path);
    let alias = !opendmd_core::fatread::is_canonical(card);
    let parent = fs_path.rsplit_once('/').map_or(fs_path, |(d, _)| d).to_string();
    let copy = opendmd_core::layout::cache_path(card).map(|p| format!("{}{p}", config::SD_MOUNT));
    // A file at `/cache`'s root (the FAT index, its build's files, the
    // names file, by any name or alias): the build under way and the loaded
    // index go first; the build keeps its files open for writing between
    // slices, and a write after this one's would land in freed clusters.
    if files::same_card_path(&parent, config::CACHE_ROOT) {
        crate::fatidx::dir_changed();
    }
    let mut intents = Vec::with_capacity(2);
    if alias {
        intents.push(crate::fatidx::intend_tree(&parent));
    } else {
        intents.push(crate::fatidx::intend(fs_path));
        if let Some(c) = &copy {
            intents.push(crate::fatidx::intend(c));
        }
    }
    let held = || {
        if alias {
            crate::fatidx::holds_under(&parent)
        } else {
            crate::fatidx::holds(fs_path) || copy.as_deref().is_some_and(crate::fatidx::holds)
        }
    };
    // The file on screen read through FatFs (not raw) is let go too: FAT
    // must not have an open file replaced or unlinked. A Formula's file is
    // read and closed as it opens: nothing holds it (and letting go would
    // drop the Formula being edited).
    let playing = ctx.status.lock().unwrap().path.clone();
    let on_screen = !playing.is_empty() && opendmd_core::synth::formula_file(&playing).is_none() && {
        let gif = format!("{}{playing}", config::SD_MOUNT);
        let pcopy = opendmd_core::layout::cache_path(&playing).map(|p| format!("{}{p}", config::SD_MOUNT));
        files::same_card_path(fs_path, &gif) || pcopy.is_some_and(|c| files::same_card_path(fs_path, &c))
    };
    // The file on screen is let go only if it is the one in the way;
    // else just the copy opened ahead (a transition's outgoing file closes
    // within the wait below).
    if (on_screen || crate::state::is_ahead(fs_path) || held()) && !release_player(ctx, on_screen) {
        return None;
    }
    wait_until_not(held).then_some(Held(intents))
}

/// `Command::Release`: the main loop lets go of the copy opened ahead and,
/// with `screen`, of the file on screen (the picture stays; the next tick
/// opens the next file, and nothing an intent covers opens at all), woken
/// from its frame's sleep for it. False if it did not answer in
/// [`REPLY_TIMEOUT`].
fn release_player(ctx: &Ctx, screen: bool) -> bool {
    let (tx, rx) = mpsc::channel();
    if ctx.cmd.send(Command::Release { screen, reply: tx }).is_err() {
        // No main loop: nothing is open.
        return true;
    }
    crate::wake::kick();
    let answered = rx.recv_timeout(REPLY_TIMEOUT).is_ok();
    if !answered {
        log::warn!("the player did not answer a release");
    }
    answered
}

/// How long a writer waits for a released file to be no longer held (a
/// transition's outgoing file closes when its last step is drawn).
const RELEASE_WAIT: Duration = Duration::from_secs(3);

fn wait_until_not(held: impl Fn() -> bool) -> bool {
    let until = std::time::Instant::now() + RELEASE_WAIT;
    while held() {
        if std::time::Instant::now() >= until {
            log::warn!("a write waited {} s for the player to let go of a file: refused", RELEASE_WAIT.as_secs());
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}


/// The answer when [`release_for_write`] got none: the file is left alone.
const NOT_RELEASED: &str = "the player did not let go of the file; try again";

/// `DELETE /api/files/<dir>?recursive=1`: everything under `path`, which
/// must be `cache/` or the retired `odm/` tree (or a directory in one). The
/// builder is held off and a copy the player is reading is skipped first.
/// `{"ok","path","removed","done"}`; `done` false means the time budget ran
/// out and the client should ask again.
#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
#[inline(never)]
fn delete_tree(req: &mut Req, c: &Ctx, rel: &str, path: &str) -> Result<()> {
    let allowed = [config::CACHE_ROOT, config::LEGACY_ROOT]
        .iter()
        .any(|root| files::same_card_path(path, root) || files::under(path, root));
    if !allowed {
        return req.bad_request("recursive delete is only for cache/ and odm/");
    }
    let _busy = crate::cache_builder::busy();
    // Nothing under it opens for raw reads from here to the end of this
    // round (`fatidx::WriteIntent`, a tree); what the player holds there
    // now (the copy on screen, the one opened ahead) is let go first.
    let _tree = crate::fatidx::intend_tree(path);
    if files::under(path, config::CACHE_ROOT) || files::same_card_path(path, config::CACHE_ROOT) {
        let playing = c.status.lock().unwrap().path.clone();
        let copy_under = opendmd_core::layout::cache_path(&playing)
            .map(|p| format!("{}{p}", config::SD_MOUNT))
            .is_some_and(|copy| files::same_card_path(path, &copy) || files::under(&copy, path));
        let ahead = crate::state::AHEAD_KEY.load(std::sync::atomic::Ordering::SeqCst) != 0;
        if (copy_under || ahead || crate::fatidx::holds_under(path)) && !release_player(c, copy_under) {
            return req.error(503, "503 Service Unavailable", NOT_RELEASED);
        }
    }
    if !wait_until_not(|| crate::fatidx::holds_under(path)) {
        return req.error(503, "503 Service Unavailable", NOT_RELEASED);
    }
    let card = format!("/{}", rel.trim_start_matches('/').trim_end_matches('/'));
    // The FAT index's build keeps its files under `/cache` open for writing
    // between slices, and the index file is open: both go before anything
    // is removed (a write after the remove would land in freed clusters),
    // at every round, and no build starts again until 30 s after.
    let cache_tree = files::under(path, config::CACHE_ROOT) || files::same_card_path(path, config::CACHE_ROOT);
    if cache_tree {
        crate::fatidx::dir_changed();
    }
    match files::remove_tree(path, config::RECURSIVE_DELETE_BUDGET) {
        Ok((removed, done)) => {
            info!("DELETE {card} (recursive): {removed} entries removed, done {done}");
            if cache_tree {
                crate::cache::invalidate();
                // Copies (and perhaps the index file itself) gone: the
                // index is dropped and rebuilt when quiet.
                crate::fatidx::dir_changed();
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

/// Serve a card-absolute path below `root`, 404ing if it is not a file;
/// `wait`: for a place in a full stream queue (`stream::hand_off_within`).
fn serve_under(req: &mut Req, root: &str, wait: Duration) -> Result<()> {
    let rel = crate::httpd::percent_decode(req.path());
    let Some(path) = files::card_path(root, &rel) else {
        return req.bad_request("path escapes the card");
    };
    stream_file(req, path, rel, wait)
}

/// Send the card file at `path` from the streaming task, or a 404 naming
/// `rel` if there is none.
fn stream_file(req: &mut Req, path: String, rel: String, wait: Duration) -> Result<()> {
    stream::hand_off_within(req, wait, move |req| {
        if files::serve(req, &path)? {
            Ok(())
        } else {
            req.not_found(&rel)
        }
    })
}

/// Let the response drain out of the TCP stack, then restart for `why`
/// (`POST /api/reboot`, the card's clock, a firmware update, and the
/// network worker's AP timeout).
pub fn reboot_after_response(why: Reason) -> ! {
    // The handler never returns to the wrapper that records this.
    crate::httpd::note_httpd_stack();
    crate::sdhealth::note_restart();
    // No card write in flight at the reset (the drain wait runs meanwhile).
    let t = std::time::Instant::now();
    crate::cache_builder::quiesce(Duration::from_secs(3));
    std::thread::sleep(Duration::from_secs(1).saturating_sub(t.elapsed()));
    crate::restart(why)
}

fn send_setup(req: &mut Req) -> Result<()> {
    req.header("Content-Encoding", "gzip")?;
    req.header("Cache-Control", "no-cache")?;
    req.app_headers(opendmd_core::access::SETUP_CSP)?;
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
fn put_file(up: &mut Upload, c: &Ctx, rel: &str, create_new: bool) -> Result<()> {
    let Some(dest) = files::card_path_for_write(config::SD_MOUNT, rel) else {
        return up.bad_request(files::BAD_WRITE_PATH);
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
    // Held to the end of the upload and its replace of `dest`.
    let Some(_held) = release_for_write(c, &dest) else {
        return up.error(503, "503 Service Unavailable", NOT_RELEASED);
    };
    if create_new {
        // Taken now, empty (FatFs's create-new: the folder's one walk): a
        // second writer of the same name gets the 409; the body then
        // replaces it as any upload's does.
        if let Some(parent) = std::path::Path::new(&dest).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&dest) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return up.error(409, "409 Conflict", &format!("/{} exists", rel.trim_start_matches('/')));
            }
            Err(e) => return up.error(500, "500 Internal Server Error", &format!("{e}")),
        }
    }
    let written = match files::receive_upload(up, &dest) {
        Ok(w) => w,
        Err(e) => {
            // The name taken for this upload goes back with it.
            if create_new {
                let _ = std::fs::remove_file(&dest);
            }
            return Err(e);
        }
    };
    // Its entry, for the index's lookups (`fatidx` delta).
    crate::fatidx::note_written(&dest);
    info!(
        "PUT /{rel}: {written} B in {:.1} s",
        up.elapsed().as_secs_f32()
    );
    if files::under(&dest, config::CACHE_ROOT) {
        crate::cache::invalidate();
    } else {
        // A GIF written anew is a new file: its failure line (or "plays as
        // it is") was about the old one.
        crate::cache_builder::drop_failed(&format!("/{}", rel.trim_start_matches('/')));
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
            .then(|| files::card_path_for_write(config::SD_MOUNT, p))
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
    // In turn: a refused `from` asks nothing more of the player.
    let Some(_from_held) = release_for_write(c, &from) else {
        return up.error(503, "503 Service Unavailable", NOT_RELEASED);
    };
    let Some(_to_held) = release_for_write(c, &to) else {
        return up.error(503, "503 Service Unavailable", NOT_RELEASED);
    };
    if let Some(dir) = std::path::Path::new(&to).parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return up.error(
                500,
                "500 Internal Server Error",
                &format!("{}: {e}", dir.display()),
            );
        }
    }
    crate::fatidx::touched(&from);
    crate::fatidx::touched(&to);
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
            let moved = files::move_copy(&old, &new);
            crate::fatidx::note_gone(&old);
            if moved == "moved" {
                crate::fatidx::note_written(&new);
            } else {
                crate::fatidx::note_gone(&new);
            }
            moved
        }
        _ => "none",
    };
    crate::fatidx::note_gone(&from);
    crate::fatidx::note_written(&to);
    crate::cache::invalidate();
    let smooth = files::rename_line(config::SMOOTH_LIST, &m.from, Some(&m.to))
        .unwrap_or_else(|e| {
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
#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
#[inline(never)]
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

/// `GET /api/files?names=1`: the listing's JSON from the names file
/// (`{"dir","entries":[{"name","size":null,"dir":false}],"count","sizes":
/// false,"names":true}`), streamed.
fn stream_names(req: &mut Req, dir: &str, names: crate::fatidx::Names) -> Result<()> {
    req.header("Cache-Control", "no-cache")?;
    req.begin_chunked("200 OK", "application/json")?;
    let mut buf = format!(r#"{{"dir":{},"entries":["#, json_string(dir.trim_end_matches('/')));
    let mut count = 0usize;
    let mut sent: Result<()> = Ok(());
    let r = names.each(&mut |n| {
        if count > 0 {
            buf.push(',');
        }
        count += 1;
        buf.push_str(&format!(r#"{{"name":{},"size":null,"dir":false}}"#, json_string(n)));
        if buf.len() >= config::IO_CHUNK {
            sent = req.chunk(buf.as_bytes()).and_then(|()| files::wait_for_heap());
            buf.clear();
            if sent.is_err() {
                return Err(std::io::Error::other("client gone"));
            }
        }
        Ok(())
    });
    sent?;
    if let Err(e) = r {
        // The card failed mid-file: the JSON is cut, and the app lists.
        log::warn!("names {dir}: {e}");
        return req.finish();
    }
    buf.push_str(&format!(r#"],"count":{count},"sizes":false,"names":true}}"#));
    req.chunk(buf.as_bytes())?;
    req.finish()
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
#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
#[inline(never)]
fn status_json(s: &Status, looks: &dyn Fn() -> String, recent: bool, out: &mut dyn std::fmt::Write) {
    // In parts (net-wedge2): each `write!`'s arguments are made, written
    // and dropped before the next part's, so the strings the parts are
    // made of never all exist at once.
    let t = s.epoch.elapsed();
    let _ = write!(
        out,
        r#"{{"version":{},"variant":{},"features":{},"psi_abi":{},"uptime_secs":{},"t_ms":{},"ip":{},"wifi":{},"hostname":{},"mdns":{},"#,
        json_string(s.version),
        json_string(crate::state::FLASH_VARIANT),
        crate::state::features_json(),
        psi_abi(),
        s.uptime_secs,
        t.as_millis(),
        json_string(&s.ip),
        json_string(s.wifi),
        json_string(&s.hostname),
        json_string(&s.mdns),
    );
    let _ = write!(
        out,
        r#""now_playing":{{"ordinal":{},"path":{},"title":{},"elapsed_ms":{},"frame":{}{}{}}},"paused":{},"#,
        s.ordinal,
        json_string(&s.path),
        json_string(&s.title),
        s.position_at(t).as_millis(),
        s.frame,
        formula_json(&s.path),
        kept_json(s),
        s.paused,
    );
    // `?recent=1`: Recently played, newest first, and which is on screen.
    if recent {
        let _ = out.write_str(r#""recent":["#);
        for (k, p) in s.recent.paths().enumerate() {
            let _ = write!(out, "{}{}", if k == 0 { "" } else { "," }, json_string(&p));
        }
        let _ = write!(out, r#"],"recent_at":{},"#, s.recent.cursor());
    }
    {
        let www = crate::www::info();
        let _ = write!(
            out,
            r#""entries":{},"manifest":{},"index_state":{},"www_version":{},"www":{},"#,
            s.entries,
            json_string(&s.manifest),
            json_string(s.index_state),
            opt_json(www.version.as_deref().filter(|_| www.ok)),
            crate::www::info_json(),
        );
    }
    let _ = write!(
        out,
        r#""heap":{{"free":{},"floor":{},"idf_min":{}{}}},"heap_floor_at":{},"#,
        s.heap_free,
        s.heap_floor,
        s.heap_idf_min,
        main_stack_json(),
        s.heap_floor_at
            .as_ref()
            .map_or_else(|| "null".to_string(), |a| a.json()),
    );
    let _ = write!(
        out,
        r#""refresh_hz":{},"deadline_misses":{},"worst_lateness_ms":{},"misses":"#,
        s.refresh_hz,
        s.deadline_misses,
        s.worst_lateness_ms,
    );
    {
        // `misses` with `recent` (the last late frames' phases) inside.
        let mut j = s.misses.json();
        j.pop();
        let _ = out.write_str(&j);
        drop(j);
        let _ = out.write_str(&crate::misslog::json());
        let _ = out.write_str(&crate::misslog::transfer_json());
        let _ = out.write_str("}");
    }
    let _ = write!(
        out,
        r#","gifs_started":{},"streams_aborted":{},"log_drops":{},"stream_stack":{},"net":{},"#,
        s.gifs_started,
        crate::httpd::streams_aborted(),
        crate::logring::drops(),
        stream::stack_low().map_or_else(|| "null".to_string(), |v| v.to_string()),
        net_json(),
    );
    let _ = write!(
        out,
        r#""sd":{{"mounted":{},"readable":{},"capacity_bytes":{},"card_id":{},"error":{},"attempts":{},"formatting":{},"format_error":{}"#,
        s.sd_mounted,
        s.sd_usable(),
        s.sd_capacity_bytes,
        opt_json(s.sd_card_id.as_deref()),
        opt_json(s.sd_error.as_deref()),
        s.sd_attempts,
        s.sd_formatting,
        opt_json(s.sd_format_error.as_deref()),
    );
    // (The card's own part first, then the rest, each with its comma.)
    let _ = out.write_str(",");
    let _ = out.write_str(&crate::sdhealth::status_json());
    let _ = out.write_str(&crate::fatidx::status_json());
    let _ = out.write_str(&crate::settings_card::status_json());
    let _ = out.write_str(&crate::cardspeed::yields_json());
    let _ = out.write_str(&crate::readahead::status_json());
    let _ = write!(
        out,
        r#",{}}},"reset_reason":{},"reset_note":{},{},"#,
        crate::cardspeed::Status,
        json_string(s.reset_reason),
        opt_json(crate::sdhealth::reset_note().or_else(crate::restarts::reset_note)),
        crate::restarts::status_json(),
    );
    let _ = write!(
        out,
        r#""overrides":{},"hidden":{},"mode":{},"mode_until":{},"time":{},"tz":{},"time_synced_s":{},"wake_until":{},"#,
        s.overrides,
        s.hidden,
        json_string(s.mode),
        opt_json(s.mode_until.as_deref()),
        opt_json(crate::clock::hhmm_now().as_deref()),
        json_string(&s.tz_name),
        crate::clock::synced_ago_secs().map_or_else(|| "null".to_string(), |n| n.to_string()),
        opt_json(s.wake_until.as_deref()),
    );
    let _ = write!(
        out,
        r#""schedule":{{"rule":{},"set":{},"until":{}}},"cache":{},"#,
        s.rule.map_or_else(|| "null".to_string(), |r| r.to_string()),
        opt_json(s.rule_set.as_deref()),
        opt_json(s.rule_until.as_deref()),
        crate::cache_builder::status_json(s.entries, crate::cache::peek()),
    );
    let _ = write!(out, r#""context":{},"#, crate::follow::status_json());
    let _ = write!(out, "{}}}", looks());
}

/// `now_playing`'s `,"kept_s":N` while a Formula sent to the panel is kept
/// there (`formula_keep`): for `N` seconds from its start (`elapsed_ms`),
/// `0` until something else plays; empty otherwise.
fn kept_json(s: &Status) -> String {
    match s.kept {
        Some((ordinal, secs)) if ordinal == s.ordinal => format!(r#","kept_s":{secs}"#),
        _ => String::new(),
    }
}

/// The compiled Formulas this player plays (`opendmd_psi::psb::ABI`; null
/// in a build without Formulas): the web app compiles for it.
fn psi_abi() -> String {
    #[cfg(feature = "psi")]
    return opendmd_core::psi::psb::ABI.to_string();
    #[cfg(not(feature = "psi"))]
    return "null".to_string();
}

/// `now_playing`'s `,"formula":{"frame_us","frame_ms","asked_ms",
/// "beam_us","compile_us","still","jit"}` while a formula plays: its mean render time, the
/// frame time its governor holds it to (`fx_psi::Governor`), the one it
/// asked for, its raced frame time (µs on the refresh grid; 0 when its
/// frames are not raced), the last formula compile's time, the heap its still cache
/// holds (B, 0 none), and the JIT: `{"on":true,"bytes":N}` while a kernel
/// draws its pixel stage, else `{"on":false,"why":"…"}`; empty otherwise.
#[cfg(feature = "psi")]
fn formula_json(path: &str) -> String {
    let (us, ms, asked, compile_us) = opendmd_core::fx_psi::stats();
    if !opendmd_core::synth::is_formula(path) || ms == 0 {
        return String::new();
    }
    let still = opendmd_core::fx_psi::still_held();
    let beam = opendmd_core::fx_psi::beam_us();
    let jit = jit_json();
    let (rt, rn) = opendmd_core::fx_psi::render_totals();
    format!(
        r#","formula":{{"frame_us":{us},"frame_ms":{ms},"asked_ms":{asked},"beam_us":{beam},"compile_us":{compile_us},"still":{still},"jit":{jit},"render_us_total":{rt},"renders":{rn}}}"#
    )
}

/// `formula.jit` (see [`formula_json`]); `look`: `formula_look.jit`.
#[cfg(feature = "psi")]
fn jit_json_of(look: bool) -> String {
    #[cfg(feature = "jit")]
    {
        let (on, why, bytes) = if look {
            opendmd_core::fx_psi::look_jit_status()
        } else {
            opendmd_core::fx_psi::jit_status()
        };
        if on {
            return format!(r#"{{"on":true,"bytes":{bytes}}}"#);
        }
        if !opendmd_core::fx_psi::jit_allowed() {
            return r#"{"on":false,"why":"off (setting)"}"#.into();
        }
        format!(r#"{{"on":false,"why":{}}}"#, json_string(why))
    }
    #[cfg(not(feature = "jit"))]
    {
        let _ = look;
        r#"{"on":false,"why":"not in this build"}"#.into()
    }
}

#[cfg(feature = "psi")]
fn jit_json() -> String {
    jit_json_of(false)
}

#[cfg(not(feature = "psi"))]
fn formula_json(_: &str) -> String {
    String::new()
}

/// The end of `heap`: `transitions_cut` (the transition gate's refusals
/// since boot: the heartbeat's figure), and in a probe build `main_stack`
/// (the main task's stack high-water mark, bytes never used, as of the last
/// heartbeat) and `pushed` (pixels pushed to the panel since boot).
fn main_stack_json() -> String {
    use std::sync::atomic::Ordering::Relaxed;
    let cut = crate::TRANSITIONS_CUT.load(Relaxed);
    #[cfg(feature = "probe")]
    return format!(
        r#","transitions_cut":{cut},"main_stack":{},"pushed":{}"#,
        crate::state::MAIN_STACK_UNUSED.load(Relaxed),
        crate::state::PIXELS_PUSHED.load(Relaxed)
    );
    #[cfg(not(feature = "probe"))]
    format!(r#","transitions_cut":{cut}"#)
}

/// `formula_look` in the status: `null`, or what is drawn over the picture
/// (`"setting"`, or `"over_gif"`: the editor's) with the heap its buffers
/// hold (0: no room, the picture goes up as it is), its mean render (µs)
/// and its own frame time (ms; 0: it only follows the picture), and
/// `error` when the setting's formula can no longer be drawn.
fn formula_look_json() -> String {
    use std::sync::atomic::Ordering::Relaxed;
    // (A saved formula that does not build draws nothing, source 0, but
    // the status still says why, as the setting's.)
    let broken = crate::formula_look::ERROR.lock().is_ok_and(|e| !e.is_empty());
    let source = match crate::formula_look::SOURCE.load(Relaxed) {
        1 => "setting",
        2 => "over_gif",
        _ if broken => "setting",
        _ => return "null".to_string(),
    };
    #[cfg(feature = "psi")]
    let (us, ms, held) = opendmd_core::fx_psi::look_stats();
    #[cfg(not(feature = "psi"))]
    let (us, ms, held) = (0, 0, 0);
    #[cfg(feature = "psi")]
    let jit = format!(r#","jit":{}"#, jit_json_of(true));
    #[cfg(not(feature = "psi"))]
    let jit = String::new();
    // The setting's formula no longer builds (a firmware change): why.
    let error = crate::formula_look::ERROR
        .lock()
        .ok()
        .filter(|e| !e.is_empty())
        .map_or(String::new(), |e| format!(r#","error":{}"#, json_string(&e)));
    format!(r#"{{"source":"{source}","held":{held},"frame_us":{us},"frame_ms":{ms}{jit}{error}}}"#)
}

/// The end of the status object: `"look","glow","glow_held","formula_look","attract_live",
/// "max_brightness","brightness_safe_max","safe_start_until","scan"`
/// (see the API table), from the settings and the main loop's flags.
fn looks_json(set: &Settings) -> String {
    use std::sync::atomic::Ordering::Relaxed;
    let live = crate::state::ATTRACT_LIVE
        .load(Relaxed)
        .then(|| crate::nvs_config::attract_path(&set.attract))
        .flatten();
    format!(
        r#""look":"{}","glow":"{}","glow_held":{},"formula_look":{},"attract_live":{},"max_brightness":{},"brightness_safe_max":{},"safe_start_until":{},"scan":{},"display":{}"#,
        set.look.name(),
        set.glow.name(),
        if crate::state::GLOW_HELD.load(Relaxed) {
            opendmd_core::look::GLOW_BYTES
        } else {
            0
        },
        formula_look_json(),
        opt_json(live.as_deref()),
        set.max_brightness,
        crate::config::BRIGHTNESS_SAFE_MAX,
        match crate::state::SAFE_START_UNTIL.load(Relaxed) {
            0 => "null".to_string(),
            n => n.to_string(),
        },
        crate::panel::scan_json(),
        display_json(),
    )
}

/// Below this much free heap a status read is a 503 at once (see the
/// handler): the reply's segments and the radio's copies of them would take
/// what the WiFi driver needs for its own buffers.
const STATUS_MIN_HEAP: u32 = 10 * 1024;
/// Status reads refused for [`STATUS_MIN_HEAP`] (`net.status_refused`).
static STATUS_REFUSED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// The largest piece of a status answer sent at once (its buffer, on the
/// heap: the httpd task's stack has ~1 KB to spare).
const STATUS_PIECE: usize = 768;

/// A status answer written out as it is made: `fmt::Write` into a buffer of
/// [`STATUS_PIECE`], each full one sent as a chunk. The first failed send
/// is kept and ends the writing (the rest is dropped).
struct Piecewise<'a> {
    req: &'a mut Req,
    buf: Vec<u8>,
    sent: usize,
    failed: Option<anyhow::Error>,
}

impl<'a> Piecewise<'a> {
    fn new(req: &'a mut Req) -> Option<Self> {
        let mut buf = Vec::new();
        buf.try_reserve_exact(STATUS_PIECE).ok()?;
        Some(Piecewise { req, buf, sent: 0, failed: None })
    }

    fn flush(&mut self) {
        crate::netstat::sample(crate::netstat::At::Status);
        if self.failed.is_none() && !self.buf.is_empty() {
            if let Err(e) = self.req.chunk(&self.buf) {
                self.failed = Some(e);
            }
            self.sent += self.buf.len();
        }
        self.buf.clear();
    }

    /// The last piece out: the bytes written, or the send that failed.
    fn finish(mut self) -> Result<usize> {
        self.flush();
        match self.failed.take() {
            Some(e) => Err(e),
            None => Ok(self.sent),
        }
    }
}

impl std::fmt::Write for Piecewise<'_> {
    fn write_str(&mut self, mut s: &str) -> std::fmt::Result {
        if self.failed.is_some() {
            return Err(std::fmt::Error);
        }
        while !s.is_empty() {
            let room = STATUS_PIECE - self.buf.len();
            let n = s.len().min(room);
            self.buf.extend_from_slice(&s.as_bytes()[..n]);
            s = &s[n..];
            if self.buf.len() == STATUS_PIECE {
                self.flush();
            }
        }
        Ok(())
    }
}

/// `"net":{...}`: the network side's heap and lwIP's state (`netstat`), and
/// the status reads refused for the heap.
fn net_json() -> String {
    let n = crate::netstat::json();
    format!(
        r#"{},"status_refused":{}}}"#,
        &n[..n.len() - 1],
        STATUS_REFUSED.load(std::sync::atomic::Ordering::Relaxed)
    )
}

/// How long a status answer is reused for another read.
const STATUS_CACHE: std::time::Duration = std::time::Duration::from_millis(100);

/// The status, copied under its lock (µs), on the heap: as a local of the
/// handler the copy (~1.5 KB) sat on the httpd task's stack under the whole
/// of `status_json` (728 B of 6.5 KB left in the emulator).
#[inline(never)]
fn status_snapshot(status: &std::sync::Mutex<Status>) -> Box<Status> {
    let s = status.lock().unwrap();
    let held = std::time::Instant::now();
    let copy = Box::new(s.clone());
    STATUS_LOCK_US.store(held.elapsed().as_micros() as u32, std::sync::atomic::Ordering::Relaxed);
    copy
}

/// A status answer kept for a fast poller: when it was made, with or
/// without `recent`, the JSON.
struct KeptStatus {
    at: std::time::Instant,
    recent: bool,
    json: String,
}

/// The last status answer, kept only while reads come faster than
/// [`STATUS_CACHE`] (httpd's only; freed by the next read when stale).
static STATUS_LAST: std::sync::Mutex<Option<KeptStatus>> = std::sync::Mutex::new(None);

/// When the last status read came (no heap: an `Instant`).
static STATUS_READ: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// The last answer's length: the next one's buffer is reserved at this
/// plus [`STATUS_SLACK`] (the first at 4 KB less the slack).
static STATUS_LEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(3584);
const STATUS_SLACK: usize = 512;

/// Reads answered from the last answer (`display.status_cached`).
static STATUS_CACHED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The last `/api/status`: how long it held `status` (µs) to copy it, and
/// the whole build with the settings' part.
static STATUS_LOCK_US: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static STATUS_BUILD_US: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// `"display":{wakes, wake_late, wake_max_us, spin_us, publish_skipped,
/// status_lock_us, status_build_us}`:
/// how the display loop's wake kept time (`wake.rs`: a wake over 100 µs
/// after its time is late) and the frames whose status publish was skipped
/// for a held lock, since boot.
fn display_json() -> String {
    let (wakes, late, max, spun) = crate::wake::stats();
    format!(
        r#"{{"wakes":{wakes},"wake_late":{late},"wake_max_us":{max},"spin_us":{spun},"publish_skipped":{},"status_lock_us":{},"status_build_us":{},"status_cached":{},"pre_start":{},"commands":{},"buffers":{},"arena":{},"card_read_max_us":{},{}}}"#,
        crate::state::PUBLISH_SKIPPED.load(std::sync::atomic::Ordering::Relaxed),
        STATUS_LOCK_US.load(std::sync::atomic::Ordering::Relaxed),
        STATUS_BUILD_US.load(std::sync::atomic::Ordering::Relaxed),
        STATUS_CACHED.load(std::sync::atomic::Ordering::Relaxed),
        crate::panel::pre_start_json(),
        crate::state::commands_json(),
        crate::membank::buffers_json(),
        arena_json(),
        crate::membank::READ_MAX_US.load(std::sync::atomic::Ordering::Relaxed),
        crate::pace::status_json(),
    )
}

/// `display.arena`: `formula_arena`'s allocations served and wanted while
/// its slot was busy, or `null` when it is off.
fn arena_json() -> String {
    #[cfg(feature = "psi")]
    {
        use std::sync::atomic::Ordering::Relaxed;
        if crate::arena::span().0 != 0 {
            return format!(r#"{{"hits":{},"busy":{}}}"#, crate::arena::HITS.load(Relaxed), crate::arena::BUSY.load(Relaxed));
        }
    }
    "null".to_string()
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
#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
#[inline(never)]
fn settings_json(s: &Settings) -> String {
    format!(
        concat!(
            r#"{{"wifi_ssid":{},"wifi_pass_set":{},"brightness":{},"max_brightness":{},"#,
            r#""dwell_min":{},"dwell_max":{},"shuffle":{},"platforms":{},"set":{},"hostname":{},"#,
            r#""tz":{},"tz_name":{},"transition":"{}","transition_ms":{},"auto_prepare":"{}","#,
            r#""follow":{},"follow_host":{},"follow_code_set":{},"attract_set":{},"attract":{},"#,
            r#""look":"{}","glow":"{}","look_formula":{},"#,
            r#""boot_show":"{}","panel_sync":{},"panel_split":{},"formula_jit":{},"formula_arena":{},"formula_keep":"{}","api_key_set":{},"card_speed":"{}","#,
            r#""lat":{},"lon":{}}}"#
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
        json_string(&s.look_formula),
        s.boot_show.name(),
        s.panel_sync,
        s.panel_split,
        s.formula_jit,
        s.formula_arena,
        s.formula_keep.name(),
        !s.api_key.is_empty(),
        s.card_speed.as_str(),
        s.location.map_or_else(|| "null".to_string(), |p| centi_degrees(p.lat_c)),
        s.location.map_or_else(|| "null".to_string(), |p| centi_degrees(p.lon_c)),
    )
}

/// Hundredths of a degree as a JSON number with two decimals (`-0.13`).
fn centi_degrees(c: i32) -> String {
    let sign = if c < 0 { "-" } else { "" };
    let a = c.unsigned_abs();
    format!("{sign}{}.{:02}", a / 100, a % 100)
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
