//! The settings that travel with the card, kept on it (`/opendmd.json`,
//! `opendmd_core::settings_file`): durable the moment they are saved, with
//! no NVS write (which stalls the flash cache on both cores, so it waits
//! for no Formula to play).
//!
//! **What is where.** On the card: [`CARD_KEYS`] (playback, looks,
//! brightness, the schedule, the time zone, ...). In NVS only: the WiFi
//! details and the name (the network starts before the card), the API key
//! and Follow RePlayOS's code (secrets, not for a file a PC can read), the
//! card's clock and its slow-clock record, and the panel timing. NVS also
//! keeps a copy of everything, written as before (deferred while a Formula
//! plays, at the latest before a restart): the settings until a card is
//! mounted, and with no card at all.
//!
//! **When.** The boot task reads the file once the card is mounted, before
//! it builds the playlist ([`adopt_at_mount`]): the card's values win, and
//! the main task applies those that differ at the handover. With no file
//! (a first boot, a new card) the file is written from the settings in
//! force. After that every change is saved here by the CPU0 task that made
//! it (the HTTP handlers, [`save_shared`]); the main task never saves here
//! and never takes [`STORE`] ([`save_soon`] hands its rare change to a
//! CPU0 task).

use crate::http::{apply_patch, ConfigPatch};
use crate::nvs_config::Settings;
use opendmd_core::settings_file::{self, DirStore, Step};
use std::sync::{Arc, Mutex};

/// The settings in the file, by their `/api/config` names. Groups apply
/// together (`brightness` is checked against `max_brightness`, `dwell_max`
/// against `dwell_min`).
const GROUPS: &[&[&str]] = &[
    &["max_brightness", "brightness"],
    &["dwell_min", "dwell_max"],
    &["shuffle"],
    &["platforms"],
    &["set"],
    &["tz"],
    &["tz_name"],
    &["transition"],
    &["transition_ms"],
    &["auto_prepare"],
    &["follow"],
    &["follow_host"],
    &["attract_set"],
    &["attract"],
    &["look"],
    &["glow"],
    &["look_formula"],
    &["boot_show"],
    &["panel_sync"],
    &["panel_split"],
    &["formula_jit"],
    &["formula_arena"],
    &["formula_keep"],
];

/// One save or load at a time; taken only by CPU0 tasks.
static STORE: Mutex<()> = Mutex::new(());

/// Where the settings are kept now (`/api/status` `sd.settings_store`).
static STATE: Mutex<String> = Mutex::new(String::new());

fn set_state(s: impl Into<String>) {
    *STATE.lock().unwrap_or_else(|p| p.into_inner()) = s.into();
}

/// `,"settings_store":"card"` (or `"nvs (no card)"`, `"nvs (card write
/// failed: …)"`), for the status's `sd` part.
pub fn status_json() -> String {
    let s = STATE.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let s = if s.is_empty() { "nvs (no card)".to_string() } else { s };
    format!(r#","settings_store":{}"#, crate::httpd::json_string(&s))
}

/// The file's body for `s`: the card keys, as `/api/config` names them.
pub fn to_json(s: &Settings) -> String {
    use serde_json::{json, Value};
    let v: Value = json!({
        "max_brightness": s.max_brightness,
        "brightness": s.brightness,
        "dwell_min": s.dwell_min,
        "dwell_max": s.dwell_max,
        "shuffle": s.shuffle,
        "platforms": s.platforms,
        "set": s.set,
        "tz": s.tz,
        "tz_name": s.tz_name,
        "schedule": s.schedule,
        "transition": s.transition.kind.name(),
        "transition_ms": s.transition.duration.as_millis() as u64,
        "auto_prepare": s.auto_prepare.as_str(),
        "follow": s.follow,
        "follow_host": s.follow_host,
        "attract_set": s.attract_set,
        "attract": s.attract,
        "look": s.look.name(),
        "glow": s.glow.name(),
        "look_formula": s.look_formula,
        "boot_show": s.boot_show.name(),
        "panel_sync": s.panel_sync,
        "panel_split": s.panel_split,
        "formula_jit": s.formula_jit,
        "formula_arena": s.formula_arena,
        "formula_keep": s.formula_keep.name(),
    });
    v.to_string()
}

/// Whether `text` is a settings object the firmware reads.
fn parses(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text).is_ok_and(|v| v.is_object())
}

/// `base` with the card's values from `text` (a parsed settings object):
/// each group checked as `PUT /api/config` checks it, one refused (an edit
/// on a PC) left as it was in `base`, and said so.
fn apply_text(base: &Settings, text: &str) -> Option<Settings> {
    let serde_json::Value::Object(map) = serde_json::from_str(text).ok()? else {
        return None;
    };
    let mut next = base.clone();
    for group in GROUPS {
        let part: serde_json::Map<String, serde_json::Value> =
            group.iter().filter_map(|k| map.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
        if part.is_empty() {
            continue;
        }
        let patch: ConfigPatch = match serde_json::from_value(serde_json::Value::Object(part)) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("settings on the card: {group:?} not read ({e}); kept as it was");
                continue;
            }
        };
        let mut tried = next.clone();
        match apply_patch(&mut tried, &patch) {
            Ok(()) => next = tried,
            Err(e) => log::warn!("settings on the card: {group:?} refused ({}); kept as it was", e.msg),
        }
    }
    // The schedule, checked as `PUT /api/schedule` checks it.
    if let Some(serde_json::Value::String(sch)) = map.get("schedule") {
        if sch.is_empty() || crate::sched::parse(sch.as_bytes()).is_ok() {
            next.schedule = sch.clone();
        } else {
            log::warn!("settings on the card: the schedule does not parse; kept as it was");
        }
    }
    Some(next)
}

fn store() -> DirStore {
    DirStore(std::path::PathBuf::from(crate::config::SD_MOUNT))
}

/// Save `s` to the card (a CPU0 task, the card mounted). With `stop`, the
/// save stops after that step (the bench's power-cut test).
fn save_locked(s: &Settings, stop: Option<Step>) {
    let _g = STORE.lock().unwrap_or_else(|p| p.into_inner());
    match settings_file::save(&mut store(), &to_json(s), stop) {
        Ok(()) => set_state("card"),
        Err(e) => {
            log::warn!("settings: saving to the card failed ({e}); NVS keeps them");
            set_state(format!("nvs (card write failed: {e})"));
        }
    }
}

/// The boot task, the card just mounted and not yet handed over: the
/// settings in force with the card's values over them (or the file written
/// from them, the first time). The shared copy is updated; the result, when
/// it differs from what the main task has, goes in the handover.
pub fn adopt_at_mount(shared: &Arc<Mutex<Settings>>) -> (Settings, Option<Box<Settings>>) {
    let base = shared.lock().unwrap().clone();
    let loaded = {
        let _g = STORE.lock().unwrap_or_else(|p| p.into_inner());
        settings_file::load(&mut store(), parses)
    };
    match loaded.and_then(|t| apply_text(&base, &t)) {
        Some(merged) => {
            set_state("card");
            if merged == base {
                return (base, None);
            }
            log::info!("settings: the card's {} applied", settings_file::NAME);
            *shared.lock().unwrap() = merged.clone();
            let changed = Box::new(merged.clone());
            (merged, Some(changed))
        }
        None => {
            // No file (or none usable): written from the settings in force.
            log::info!("settings: {} written from the device's settings", settings_file::NAME);
            save_locked(&base, None);
            (base, None)
        }
    }
}

/// An HTTP handler (CPU0), after the main task applied a change: save the
/// settings in force (the shared copy) to the card. Nothing without a card
/// (NVS keeps them, as the main task saved them).
pub fn save_shared(shared: &Mutex<Settings>) {
    if !crate::CARD_READY.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let s = shared.lock().unwrap().clone();
    save_locked(&s, None);
}

/// The main task's own change (the shared copy already updated): saved by
/// a short-lived CPU0 task, so the display core never waits on the card or
/// on [`STORE`].
#[inline(never)]
pub fn save_soon(shared: Arc<Mutex<Settings>>) {
    let r = crate::task::spawn_core0(b"settings\0", 6 * 1024, 2, Box::new(move || save_shared(&shared)));
    if let Err(e) = r {
        log::warn!("settings: no task to save to the card ({e}); NVS keeps them");
    }
}

/// `bench` builds: save the shared copy and stop after its old file was
/// moved aside, before the new one is renamed into place: what a power cut
/// there leaves. The next boot must load the new settings.
#[cfg(feature = "bench")]
pub fn save_cut(shared: &Mutex<Settings>) {
    let s = shared.lock().unwrap().clone();
    save_locked(&s, Some(Step::MovedToBak));
}

/// psi-probe (the emulator lane): a save to the card, then one cut between
/// its renames (a power cut there), then a load as the next boot's: the
/// second save's values must come back, and the card's own file is put
/// back after. (passed, what came back.)
#[cfg(feature = "psi-probe")]
#[inline(never)]
pub fn self_check() -> (bool, String) {
    let _g = STORE.lock().unwrap_or_else(|p| p.into_inner());
    let mut st = store();
    use opendmd_core::settings_file::Store;
    let before = st.read(settings_file::NAME);
    let mut a = Settings::default();
    a.brightness = 77;
    a.max_brightness = 200;
    a.look = opendmd_core::look::Look::Amber;
    let mut b = a.clone();
    b.brightness = 99;
    b.set = "filelist.xmas".into();
    let saved = settings_file::save(&mut st, &to_json(&a), None).and_then(|()| settings_file::save(&mut st, &to_json(&b), Some(Step::MovedToBak)));
    let cut_left_name = st.exists(settings_file::NAME);
    let got = settings_file::load(&mut st, parses).and_then(|t| apply_text(&Settings::default(), &t));
    let ok = saved.is_ok() && !cut_left_name && got.as_ref().is_some_and(|g| g.brightness == 99 && g.set == "filelist.xmas" && g.look == a.look);
    let what = got.map_or("nothing".into(), |g| format!("brightness {}, set '{}', look {}", g.brightness, g.set, g.look.name()));
    // The card's own settings back as they were.
    let _ = st.remove(settings_file::BAK);
    match before {
        Some(t) => {
            let _ = st.remove(settings_file::NAME);
            let _ = st.write_synced(settings_file::NAME, &t);
        }
        None => {
            let _ = st.remove(settings_file::NAME);
        }
    }
    (ok, what)
}
