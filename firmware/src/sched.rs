//! The schedule as stored and served: JSON in NVS (`schedule`, at most
//! [`nvs_config::SCHEDULE_MAX`] bytes), parsed into the core's
//! [`Schedule`], whose evaluator decides the mode, playlist and brightness
//! (`opendmd_core::schedule`, where the rules are documented and tested).
//!
//! ```json
//! {"rules":[{"days":[1,2,3,4,5],"from":"18:00","to":"23:00","set":"consoles","brightness":96}],
//!  "night":{"from":"23:30","to":"07:00","mode":"off","brightness":8}}
//! ```
//!
//! `days` are ISO weekdays (1 = Monday; empty or absent = every day); `set`
//! and `brightness` are optional (absent = as configured); `night` is
//! optional, `mode` is `"off"` or `"dim"` and `brightness` is the dim level.
//! A night with `"sun":true` runs from sunset to sunrise at the saved
//! location instead of `from`–`to` (kept, but not needed), `offset` minutes
//! after sunset and before sunrise (−180..180, default 0).

use opendmd_core::schedule::{self, Night, NightMode, NightWhen, Rule, Schedule, Window};
use serde::Deserialize;

use crate::config;
use crate::nvs_config;

/// What an empty schedule reads back as.
pub const EMPTY_JSON: &str = r#"{"rules":[],"night":null}"#;

#[derive(Deserialize)]
struct RuleJson {
    #[serde(default)]
    days: Vec<u8>,
    from: String,
    to: String,
    #[serde(default)]
    set: Option<String>,
    #[serde(default)]
    brightness: Option<u8>,
}

#[derive(Deserialize)]
struct NightJson {
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    #[serde(default)]
    sun: bool,
    #[serde(default)]
    offset: i16,
    mode: String,
    #[serde(default)]
    brightness: Option<u8>,
}

#[derive(Deserialize)]
struct ScheduleJson {
    #[serde(default)]
    rules: Vec<RuleJson>,
    #[serde(default)]
    night: Option<NightJson>,
}

fn window(from: &str, to: &str, what: &str) -> Result<Window, String> {
    let f = schedule::parse_hhmm(from).ok_or(format!("{what}: from '{from}' is not HH:MM"))?;
    let t = schedule::parse_hhmm(to).ok_or(format!("{what}: to '{to}' is not HH:MM"))?;
    Ok(Window { from: f, to: t })
}

/// Parse and validate a schedule. An empty string is the empty schedule.
/// Whether each rule's playlist exists is not checked here (see
/// [`missing_sets`]).
#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
#[inline(never)]
pub fn parse(json: &[u8]) -> Result<Schedule, String> {
    if json.iter().all(u8::is_ascii_whitespace) {
        return Ok(Schedule::default());
    }
    // `from_slice`, like every other request body: a second reader type
    // would instantiate the whole deserializer again.
    let raw: ScheduleJson =
        serde_json::from_slice(json).map_err(|e| format!("invalid schedule JSON: {e}"))?;
    if raw.rules.len() > config::SCHEDULE_MAX_RULES {
        return Err(format!(
            "at most {} rules (got {})",
            config::SCHEDULE_MAX_RULES,
            raw.rules.len()
        ));
    }
    let mut rules = Vec::with_capacity(raw.rules.len());
    for (i, r) in raw.rules.iter().enumerate() {
        let what = format!("rule {}", i + 1);
        let days =
            schedule::days_mask(&r.days).ok_or(format!("{what}: days are 1 (Mon) to 7 (Sun)"))?;
        if let Some(set) = &r.set {
            if !nvs_config::valid_set_name(set) || nvs_config::is_obsolete_set(set) {
                return Err(format!("{what}: '{set}' is not a playlist name"));
            }
        }
        rules.push(Rule {
            days,
            window: window(&r.from, &r.to, &what)?,
            set: r.set.clone(),
            brightness: r.brightness,
        });
    }
    let night = match &raw.night {
        None => None,
        Some(n) => Some(Night {
            when: if !n.sun {
                NightWhen::Clock(window(&n.from, &n.to, "night")?)
            } else if (-180..=180).contains(&n.offset) {
                NightWhen::Sun { offset: n.offset }
            } else {
                return Err("night: offset is -180 to 180 minutes".into());
            },
            mode: match n.mode.as_str() {
                "off" => NightMode::Off,
                "dim" => NightMode::Dim,
                other => return Err(format!("night: mode '{other}' is not off or dim")),
            },
            brightness: n.brightness.unwrap_or(8),
        }),
    };
    Ok(Schedule { rules, night })
}

/// The rules' playlists whose manifests do not open (one `open` each, of a
/// root file or one in `/sets`).
pub fn missing_sets(s: &Schedule) -> Vec<String> {
    let mut missing: Vec<String> = Vec::new();
    for set in s.rules.iter().filter_map(|r| r.set.as_deref()) {
        if !set.is_empty()
            && !missing.iter().any(|m| m == set)
            && std::fs::File::open(nvs_config::manifest_path_for(set)).is_err()
        {
            missing.push(set.to_string());
        }
    }
    missing
}
