//! NVS-backed settings.
//!
//! Shape lifted from `~/projects/stripe-dashboard/src/nvs_config.rs` (one
//! `EspNvs` handle in a private namespace, typed getters that fall back to the
//! compiled-in defaults, small `get_string`/`set_string` helpers). The keys are
//! ours.
//!
//! WiFi credentials live here, not in the WiFi driver's own NVS: the driver is
//! put in `WIFI_STORAGE_RAM` before anything else happens, because letting it
//! write its config to NVS corrupts the application namespace (stripe-dashboard
//! fought that one — `set_str` returns `Ok` and the read afterwards says
//! not-found).
//!
//! **Dev convenience, `dev-credentials` feature only.** With the feature on
//! (`make fw-build`/`fw-flash`), `build.rs` turns `OPENDMD_WIFI_SSID` /
//! `OPENDMD_WIFI_PASS` (from `firmware/.env`, gitignored) into compile-time
//! env vars, and [`Config::seed_credentials_from_build`] copies them into NVS
//! exactly once — only when NVS has no SSID of its own, so anything saved from
//! the setup page always wins. With it off (`make fw-release`) the
//! `option_env!` reads are not compiled at all, so a release image cannot
//! carry a credential whatever the build environment holds.

use esp_idf_svc::nvs::{EspNvs, EspNvsPartition, NvsDefault};
use log::warn;
use opendmd_core::boot::ShowLength;
use opendmd_core::card_speed::CardSpeed;
use opendmd_core::compositor::Transition;
use opendmd_core::look::{GlowLevel, Look};
use opendmd_core::panel_timing::PanelTiming;

use crate::config;

const NAMESPACE: &str = "opendmd";

const KEY_WIFI_SSID: &str = "wifi_ssid";
const KEY_WIFI_PASS: &str = "wifi_pass";
const KEY_BRIGHTNESS: &str = "brightness";
const KEY_MAX_BRIGHTNESS: &str = "max_bright";
const KEY_DWELL_MIN: &str = "dwell_min";
const KEY_DWELL_MAX: &str = "dwell_max";
const KEY_SHUFFLE: &str = "shuffle";
const KEY_PLATFORMS: &str = "platforms";
const KEY_SET: &str = "set";
const KEY_HOSTNAME: &str = "hostname";
const KEY_TZ: &str = "tz";
const KEY_TZ_NAME: &str = "tz_name";
const KEY_SCHEDULE: &str = "schedule";
const KEY_TRANSITION: &str = "transition";
const KEY_AUTO_PREPARE: &str = "auto_prepare";
const KEY_FOLLOW: &str = "follow";
const KEY_FOLLOW_HOST: &str = "follow_host";
const KEY_FOLLOW_CODE: &str = "follow_code";
/// The API key (`opendmd_core::access`), empty = none.
const KEY_API_KEY: &str = "api_key";
const KEY_ATTRACT_SET: &str = "attract_set";
const KEY_LOOK: &str = "look";
const KEY_GLOW: &str = "glow";
/// The Formula look: a formula's `@formula?src=…` path (`""`: none). Its
/// name, blend and amount are the formula's own `name` and `look` lines.
const KEY_LOOK_FORMULA: &str = "look_fx";
/// Keys earlier test builds kept beside it (the look's name, its
/// `blend:amount`): never read, removed at the next save.
const STALE_KEYS: [&str; 2] = ["look_fx_name", "look_mix"];
const KEY_ATTRACT: &str = "attract";
const KEY_BOOT_SHOW: &str = "boot_show";
/// Refresh-synchronised blits (`Panel::set_sync`), 0/1.
const KEY_PANEL_SYNC: &str = "panel_sync";
/// With it, the bottom half a refresh after the top (`Panel::set_sync`), 0/1.
const KEY_PANEL_SPLIT: &str = "panel_split";
const KEY_FORMULA_JIT: &str = "formula_jit";
/// Racing the beam for Formulas (`Panel::set_beam`, the spike), 0/1.
const KEY_FORMULA_BEAM: &str = "formula_beam";
/// The card's clock policy (`opendmd_core::card_speed::CardSpeed`).
const KEY_CARD_SPEED: &str = "card_speed";
/// The longest `attract` value (`text:` and the message).
pub const ATTRACT_MAX: usize = 205;
/// The panel timing (`opendmd_core::panel_timing`): the saved settings and
/// a trial staged for the next boot, both in `PanelTiming::to_nvs` form.
/// Not part of [`Settings`]: the main task's `Tuner` owns them.
const KEY_PANEL: &str = "panel";
const KEY_PANEL_TRY: &str = "panel_try";
/// Internal, not a setting: the CID serial of the card that failed at full
/// speed and mounts at the fallback clock (`sdhealth`). Absent = none.
const KEY_SD_SLOW: &str = "sd_slow";
/// What NVS holds in `attract_set` for "the current playlist" (a `*` can
/// never be a set's name: FAT refuses it in a file name).
const CURRENT_PLAYLIST: &str = "*";

/// When the device prepares its own playback cache (`cache_builder.rs`):
/// `opendmd_core::auto_prepare`, where the pace and the guard are tested.
pub use opendmd_core::auto_prepare::AutoPrepare;

/// The largest schedule JSON NVS will hold for us (`PUT /api/schedule`).
pub const SCHEDULE_MAX: usize = 2048;

/// Credentials seeded at build time from `firmware/.env`, if it was present.
/// `None` in a dev build that did not set them; absent altogether without the
/// `dev-credentials` feature.
#[cfg(feature = "dev-credentials")]
const BUILD_SSID: Option<&str> = option_env!("OPENDMD_WIFI_SSID");
#[cfg(feature = "dev-credentials")]
const BUILD_PASS: Option<&str> = option_env!("OPENDMD_WIFI_PASS");

/// Everything the device remembers across reboots.
///
/// This is a plain snapshot, read once at boot and re-read after a write, so
/// the HTTP handlers can answer `GET /api/config` from a mutex-guarded copy
/// instead of touching NVS on the httpd task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub wifi_ssid: String,
    pub wifi_pass: String,
    pub brightness: u8,
    /// Ceiling for `brightness` (16..=255), default
    /// [`config::BRIGHTNESS_SAFE_MAX`]. `brightness <= max_brightness` always.
    pub max_brightness: u8,
    pub dwell_min: u64,
    pub dwell_max: u64,
    pub shuffle: bool,
    /// Comma-separated platform prefixes, e.g. `MD,SNES`. Empty = everything.
    pub platforms: String,
    /// Active playlist: `""` is the card's default, `/filelist.txt`; a root
    /// manifest name (`filelist.hall`, `filelist.xmas`, `filelist.txt`)
    /// selects that file; anything else is `/sets/<set>.txt`. Whatever it
    /// names lists GIF paths — prepared copies are found by `SdSource`, never
    /// named by a playlist. `filelist.odm.txt` is not accepted (see
    /// [`is_obsolete_set`]).
    pub set: String,
    pub hostname: String,
    /// POSIX TZ string (`GMT0IST,M3.5.0/1,M10.5.0`), what `localtime_r` uses.
    pub tz: String,
    /// The name the UI chose the zone by (`Europe/Dublin`); display only.
    pub tz_name: String,
    /// The schedule, as the JSON `PUT /api/schedule` stored (validated
    /// there, at most [`SCHEDULE_MAX`] bytes); empty = no schedule.
    pub schedule: String,
    /// How a new file comes in. Stored as `kind:ms` (`dissolve:400`), the
    /// form `Transition::parse` reads; `/api/config` has it as `transition`
    /// + `transition_ms`.
    pub transition: Transition,
    /// `"idle"` (default), `"always"` or `"off"`: see [`AutoPrepare`].
    pub auto_prepare: AutoPrepare,
    /// Follow RePlayOS (`follow.rs`): on/off, the Pi's host (`192.168.1.20`
    /// or `host:port`), and its NET CONTROL code — stored like the WiFi
    /// password, never returned by the API (only `follow_code_set`).
    pub follow: bool,
    pub follow_host: String,
    pub follow_code: String,
    /// The playlist attract mode plays (a `set` value); `None` = whatever
    /// would play anyway (the saved playlist, or the schedule's).
    pub attract_set: Option<String>,
    /// What attract mode shows instead of a playlist: `""` (the playlist,
    /// as before), `"clock"` (the live clock), `"text:<message>"` (the
    /// scrolling text). See [`attract_path`]; a stored `"fx:…"` (a built-in
    /// effect, gone) reads as `""`, with a log line.
    pub attract: String,
    /// The DMD tint (`opendmd_core::look`), applied at the blit.
    pub look: Look,
    /// Phosphor glow; off by default (it holds 12 KB while lit).
    pub glow: GlowLevel,
    /// The Formula look (`formula_look.rs`): a formula drawn over
    /// everything that plays: a bundled Formula's name (`@formula:<name>`),
    /// the user's on the card (`/formulas/<stem>.psi`) or inline
    /// (`@formula?src=…`, at most `synth::MAX_LOOK_PATH`); `""` = none.
    pub look_formula: String,
    /// The start-up show: `off`, `short` (the default) or `full`
    /// (`opendmd_core::boot`, `show.rs`).
    pub boot_show: ShowLength,
    /// Refresh-synchronised blits (`panel.rs`, `opendmd_core::scan`): wait
    /// for a scan position from which a frame shows no seam. On by
    /// default since the rig eye test (2026-10-26).
    pub panel_sync: bool,
    /// With `panel_sync`: each frame's bottom half (rows 16-31) a refresh
    /// after its top half, so the time each row is lit rises smoothly down
    /// the panel instead of jumping back at row 16 (`scan::Pacer::next`).
    pub panel_split: bool,
    /// The narrow JIT for Formulas (`opendmd_core::fx_psi::set_jit`): on by
    /// default; off, every Formula plays on the VM. The kill switch.
    pub formula_jit: bool,
    /// Racing the beam for Formulas (`Panel::set_beam`; console-lessons
    /// §2.6, the spike): a Formula's frames drawn in the panel's pair
    /// order and written behind the scan, on its refresh grid. Off by
    /// default.
    pub formula_beam: bool,
    /// The API key every API and card request must carry when set
    /// (`httpd::gate`, docs/api.md "Security"); empty = none. Never
    /// returned by the API (only `api_key_set`).
    pub api_key: String,
    /// The card's SPI clock: `safe` (the default: always 20 MHz), `fast`
    /// (40 MHz, slowed down in place after an error) or `full` (always
    /// 40 MHz, never slowed: API and bench only). See `cardspeed.rs`.
    pub card_speed: CardSpeed,
}

/// Is `v` an `attract` value `PUT /api/config` may store?
pub fn valid_attract(v: &str) -> bool {
    match v.strip_prefix("text:") {
        _ if v.is_empty() || v == "clock" => true,
        Some(msg) => {
            !msg.trim().is_empty() && v.len() <= ATTRACT_MAX && !msg.chars().any(char::is_control)
        }
        None => false,
    }
}

/// The virtual file an `attract` value plays (`@clock`, `@text?msg=…`),
/// `None` for the playlist.
pub fn attract_path(v: &str) -> Option<String> {
    use opendmd_core::synth;
    match v.strip_prefix("text:") {
        _ if v == "clock" => Some(synth::CLOCK.to_string()),
        Some(msg) if !msg.trim().is_empty() => Some(synth::text_path(msg)),
        _ => None,
    }
}

impl Default for Settings {
    /// `config.rs` stays the single source of truth for the fallbacks.
    fn default() -> Self {
        Settings {
            wifi_ssid: String::new(),
            wifi_pass: String::new(),
            brightness: config::BRIGHTNESS,
            max_brightness: config::BRIGHTNESS_SAFE_MAX,
            dwell_min: config::DWELL_MIN_SECS,
            dwell_max: config::DWELL_MAX_SECS,
            shuffle: config::SHUFFLE,
            platforms: config::PLATFORMS.join(","),
            set: String::new(),
            hostname: config::HOSTNAME.to_string(),
            tz: config::TZ.to_string(),
            tz_name: config::TZ_NAME.to_string(),
            schedule: String::new(),
            transition: config::TRANSITION,
            auto_prepare: AutoPrepare::default(),
            follow: false,
            follow_host: String::new(),
            follow_code: String::new(),
            attract_set: None,
            attract: String::new(),
            look: Look::Colour,
            glow: GlowLevel::Off,
            look_formula: String::new(),
            boot_show: ShowLength::Short,
            panel_sync: true,
            panel_split: true,
            formula_jit: true,
            formula_beam: false,
            api_key: String::new(),
            card_speed: CardSpeed::Safe,
        }
    }
}

/// `filelist.odm.txt`: a playlist of `/odm/...` paths from before prepared
/// copies were resolved automatically. It is ignored — never the default, and
/// refused as a `set` (a saved one is cleared at boot) — because the same
/// copies now play from `filelist.txt` without it.
pub fn is_obsolete_set(set: &str) -> bool {
    set.eq_ignore_ascii_case(config::OBSOLETE_MANIFEST)
}

/// Filesystem path of the manifest a `set` value names: `""` is
/// `/filelist.txt`, `filelist.*` that root manifest, anything else
/// `/sets/<set>.txt`. Pure string work.
pub fn manifest_path_for(set: &str) -> String {
    if set.is_empty() {
        config::MANIFEST_PATH.to_string()
    } else if set.starts_with("filelist.") {
        format!("{}/{}", config::SD_MOUNT, set)
    } else {
        format!("{}/{}.txt", config::SETS_DIR, set)
    }
}

/// Is `set` a name `PUT /api/config` may store? One path segment, no `..`.
/// Whether the file exists is checked separately, by opening it.
pub fn valid_set_name(set: &str) -> bool {
    !set.contains(['/', '\\', '\0']) && !set.contains("..")
}

impl Settings {
    /// Filesystem path of the manifest this configuration selects. Pure
    /// string work: `""` is always `/filelist.txt`.
    pub fn manifest_path(&self) -> String {
        manifest_path_for(&self.set)
    }

    /// Platform prefixes as a list, with blanks dropped.
    pub fn platform_list(&self) -> Vec<String> {
        self.platforms
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
}

/// The NVS handle. Owned by the main task; the HTTP handlers never touch it.
pub struct Config {
    nvs: EspNvs<NvsDefault>,
}

impl Config {
    pub fn new(partition: EspNvsPartition<NvsDefault>) -> anyhow::Result<Self> {
        let nvs = EspNvs::new(partition, NAMESPACE, true)?;
        Ok(Config { nvs })
    }

    /// Read everything, falling back to [`Settings::default`] field by field.
    pub fn load(&self) -> Settings {
        let d = Settings::default();
        // A device without the key gets the default ceiling; a saved
        // brightness above the ceiling (a key out of range, or a device that
        // saved more than the default before the ceiling existed) is read
        // as the ceiling, so the pair is always consistent.
        let max_brightness = self
            .nvs
            .get_u8(KEY_MAX_BRIGHTNESS)
            .ok()
            .flatten()
            .filter(|&m| m >= opendmd_core::brightness::MIN_MAX)
            .unwrap_or(d.max_brightness);
        let brightness = opendmd_core::brightness::effective(
            self.nvs
                .get_u8(KEY_BRIGHTNESS)
                .ok()
                .flatten()
                .unwrap_or(d.brightness),
            max_brightness,
        );
        Settings {
            wifi_ssid: self.get_string(KEY_WIFI_SSID).unwrap_or(d.wifi_ssid),
            wifi_pass: self.get_string(KEY_WIFI_PASS).unwrap_or(d.wifi_pass),
            brightness,
            max_brightness,
            dwell_min: self
                .get_string(KEY_DWELL_MIN)
                .and_then(|s| s.parse().ok())
                .unwrap_or(d.dwell_min),
            dwell_max: self
                .get_string(KEY_DWELL_MAX)
                .and_then(|s| s.parse().ok())
                .unwrap_or(d.dwell_max),
            shuffle: self
                .nvs
                .get_u8(KEY_SHUFFLE)
                .ok()
                .flatten()
                .map(|v| v != 0)
                .unwrap_or(d.shuffle),
            platforms: self.get_string(KEY_PLATFORMS).unwrap_or(d.platforms),
            set: self.get_string(KEY_SET).unwrap_or(d.set),
            hostname: self
                .get_string(KEY_HOSTNAME)
                .filter(|s| !s.is_empty())
                .unwrap_or(d.hostname),
            tz: self
                .get_string(KEY_TZ)
                .filter(|s| !s.is_empty())
                .unwrap_or(d.tz),
            tz_name: self.get_string(KEY_TZ_NAME).unwrap_or(d.tz_name),
            schedule: self
                .get_long_string(KEY_SCHEDULE, SCHEDULE_MAX)
                .unwrap_or(d.schedule),
            transition: self
                .get_string(KEY_TRANSITION)
                .and_then(|s| Transition::parse(&s))
                .unwrap_or(d.transition),
            auto_prepare: self
                .get_string(KEY_AUTO_PREPARE)
                .and_then(|s| AutoPrepare::parse(&s))
                .unwrap_or(d.auto_prepare),
            follow: self
                .nvs
                .get_u8(KEY_FOLLOW)
                .ok()
                .flatten()
                .map(|v| v != 0)
                .unwrap_or(d.follow),
            follow_host: self.get_string(KEY_FOLLOW_HOST).unwrap_or(d.follow_host),
            follow_code: self.get_string(KEY_FOLLOW_CODE).unwrap_or(d.follow_code),
            attract_set: match self.get_string(KEY_ATTRACT_SET) {
                None => d.attract_set,
                Some(s) if s == CURRENT_PLAYLIST => None,
                Some(s) => Some(s),
            },
            attract: self
                .get_string(KEY_ATTRACT)
                .filter(|s| {
                    // The built-in effects' form (`fx:tunnel…`) is gone.
                    if s.starts_with("fx:") {
                        log::warn!(
                            "attract: {:?} is no longer a live item (the built-in effects are gone); showing the playlist",
                            s.chars().take(48).collect::<String>()
                        );
                    }
                    valid_attract(s)
                })
                .unwrap_or(d.attract),
            look: self
                .get_string(KEY_LOOK)
                .and_then(|s| Look::from_name(&s))
                .unwrap_or(d.look),
            glow: self
                .get_string(KEY_GLOW)
                .and_then(|s| GlowLevel::from_name(&s))
                .unwrap_or(d.glow),
            look_formula: self
                .get_long_string(KEY_LOOK_FORMULA, opendmd_core::synth::MAX_LOOK_PATH)
                .unwrap_or(d.look_formula),
            boot_show: self
                .get_string(KEY_BOOT_SHOW)
                .and_then(|s| ShowLength::from_name(&s))
                .unwrap_or(d.boot_show),
            panel_sync: self
                .nvs
                .get_u8(KEY_PANEL_SYNC)
                .ok()
                .flatten()
                .map(|v| v != 0)
                .unwrap_or(d.panel_sync),
            panel_split: self
                .nvs
                .get_u8(KEY_PANEL_SPLIT)
                .ok()
                .flatten()
                .map(|v| v != 0)
                .unwrap_or(d.panel_split),
            formula_jit: self
                .nvs
                .get_u8(KEY_FORMULA_JIT)
                .ok()
                .flatten()
                .map(|v| v != 0)
                .unwrap_or(d.formula_jit),
            formula_beam: self
                .nvs
                .get_u8(KEY_FORMULA_BEAM)
                .ok()
                .flatten()
                .map(|v| v != 0)
                .unwrap_or(d.formula_beam),
            api_key: self
                .get_string(KEY_API_KEY)
                .filter(|k| opendmd_core::access::valid_key(k))
                .unwrap_or(d.api_key),
            card_speed: self.card_speed(),
        }
    }

    /// Write every field back. Cheap enough (NVS only rewrites what changed)
    /// and it keeps the "PUT replaces the config" semantics honest.
    pub fn save(&mut self, s: &Settings) -> anyhow::Result<()> {
        self.nvs.set_str(KEY_WIFI_SSID, &s.wifi_ssid)?;
        self.nvs.set_str(KEY_WIFI_PASS, &s.wifi_pass)?;
        self.nvs.set_u8(KEY_BRIGHTNESS, s.brightness)?;
        self.nvs.set_u8(KEY_MAX_BRIGHTNESS, s.max_brightness)?;
        self.nvs.set_str(KEY_DWELL_MIN, &s.dwell_min.to_string())?;
        self.nvs.set_str(KEY_DWELL_MAX, &s.dwell_max.to_string())?;
        self.nvs.set_u8(KEY_SHUFFLE, u8::from(s.shuffle))?;
        self.nvs.set_str(KEY_PLATFORMS, &s.platforms)?;
        self.nvs.set_str(KEY_SET, &s.set)?;
        self.nvs.set_str(KEY_HOSTNAME, &s.hostname)?;
        self.nvs.set_str(KEY_TZ, &s.tz)?;
        self.nvs.set_str(KEY_TZ_NAME, &s.tz_name)?;
        self.nvs.set_str(KEY_SCHEDULE, &s.schedule)?;
        self.nvs
            .set_str(KEY_TRANSITION, &s.transition.to_string())?;
        self.nvs
            .set_str(KEY_AUTO_PREPARE, s.auto_prepare.as_str())?;
        self.nvs.set_u8(KEY_FOLLOW, u8::from(s.follow))?;
        self.nvs.set_str(KEY_FOLLOW_HOST, &s.follow_host)?;
        self.nvs.set_str(KEY_FOLLOW_CODE, &s.follow_code)?;
        self.nvs.set_str(
            KEY_ATTRACT_SET,
            s.attract_set.as_deref().unwrap_or(CURRENT_PLAYLIST),
        )?;
        self.nvs.set_str(KEY_ATTRACT, &s.attract)?;
        self.nvs.set_str(KEY_LOOK, s.look.name())?;
        self.nvs.set_str(KEY_GLOW, s.glow.name())?;
        self.nvs.set_str(KEY_LOOK_FORMULA, &s.look_formula)?;
        for k in STALE_KEYS {
            // (Absent is the usual answer; a failure only leaves it unread.)
            let _ = self.nvs.remove(k);
        }
        self.nvs.set_str(KEY_BOOT_SHOW, s.boot_show.name())?;
        self.nvs.set_u8(KEY_PANEL_SYNC, u8::from(s.panel_sync))?;
        self.nvs.set_u8(KEY_PANEL_SPLIT, u8::from(s.panel_split))?;
        self.nvs.set_u8(KEY_FORMULA_JIT, u8::from(s.formula_jit))?;
        self.nvs.set_u8(KEY_FORMULA_BEAM, u8::from(s.formula_beam))?;
        self.nvs.set_str(KEY_API_KEY, &s.api_key)?;
        self.nvs.set_str(KEY_CARD_SPEED, s.card_speed.as_str())?;
        Ok(())
    }

    /// Copy `firmware/.env` credentials into NVS on a device that has none.
    ///
    /// Returns true when something was written. Never overwrites credentials
    /// entered through the setup page: NVS wins as soon as it has an SSID.
    /// Always false without the `dev-credentials` feature.
    #[cfg(not(feature = "dev-credentials"))]
    pub fn seed_credentials_from_build(&mut self) -> bool {
        false
    }

    /// See the `dev-credentials`-off twin above.
    #[cfg(feature = "dev-credentials")]
    pub fn seed_credentials_from_build(&mut self) -> bool {
        let (Some(ssid), Some(pass)) = (BUILD_SSID, BUILD_PASS) else {
            return false;
        };
        if ssid.is_empty() {
            return false;
        }
        if self
            .get_string(KEY_WIFI_SSID)
            .is_some_and(|existing| !existing.is_empty())
        {
            return false;
        }
        match (
            self.nvs.set_str(KEY_WIFI_SSID, ssid),
            self.nvs.set_str(KEY_WIFI_PASS, pass),
        ) {
            (Ok(_), Ok(_)) => {
                // The SSID is logged (it is broadcast anyway); the password is
                // never printed, here or in any status payload.
                log::info!("seeded WiFi credentials for '{ssid}' from the build environment");
                true
            }
            _ => {
                warn!("failed to seed WiFi credentials into NVS");
                false
            }
        }
    }

    /// `card_speed` alone, for the boot's card records (`sdhealth::boot`),
    /// which come before the rest of the settings.
    pub fn card_speed(&self) -> CardSpeed {
        // Older firmware's `auto` (and no key) reads as `safe`.
        CardSpeed::from_nvs(self.get_string(KEY_CARD_SPEED).as_deref())
    }

    /// The slow-clock record: the CID serial of the card that failed at
    /// full speed (`sdhealth`). Not part of [`Settings`]: never in
    /// `/api/config`, never written by [`save`](Self::save).
    pub fn sd_slow(&self) -> Option<u32> {
        match self.nvs.get_u32(KEY_SD_SLOW) {
            Ok(v) => v,
            Err(e) => {
                warn!("NVS get '{KEY_SD_SLOW}': {e}");
                None
            }
        }
    }

    /// Record (`Some(serial)`) or forget (`None`) the slow-clock card.
    pub fn set_sd_slow(&mut self, serial: Option<u32>) -> anyhow::Result<()> {
        match serial {
            Some(v) => self.nvs.set_u32(KEY_SD_SLOW, v)?,
            None => {
                self.nvs.remove(KEY_SD_SLOW)?;
            }
        }
        Ok(())
    }

    /// The saved panel timing and the trial staged for the next boot, as
    /// stored (`Tuner::boot` parses them; absent or damaged = none).
    pub fn panel(&self) -> (Option<String>, Option<String>) {
        (self.get_string(KEY_PANEL), self.get_string(KEY_PANEL_TRY))
    }

    pub fn set_panel(&mut self, t: &PanelTiming) -> anyhow::Result<()> {
        self.nvs.set_str(KEY_PANEL, &t.to_nvs())?;
        Ok(())
    }

    /// Stage (`Some`) or drop (`None`) the trial for the next boot.
    pub fn set_panel_try(&mut self, t: Option<&PanelTiming>) -> anyhow::Result<()> {
        match t {
            Some(t) => self.nvs.set_str(KEY_PANEL_TRY, &t.to_nvs())?,
            None => {
                self.nvs.remove(KEY_PANEL_TRY)?;
            }
        }
        Ok(())
    }

    /// [`get_string`](Self::get_string) for a value up to `max` bytes, read
    /// through a heap buffer rather than the stack.
    fn get_long_string(&self, key: &str, max: usize) -> Option<String> {
        let mut buf = vec![0u8; max + 1];
        match self.nvs.get_str(key, &mut buf) {
            Ok(Some(s)) => Some(s.trim_end_matches('\0').to_string()),
            Ok(None) => None,
            Err(e) => {
                warn!("NVS get '{key}': {e}");
                None
            }
        }
    }

    fn get_string(&self, key: &str) -> Option<String> {
        // 512 B is plenty for every key here; the manifest never lives in NVS.
        let mut buf = [0u8; 512];
        match self.nvs.get_str(key, &mut buf) {
            Ok(Some(s)) => Some(s.trim_end_matches('\0').to_string()),
            Ok(None) => None,
            Err(e) => {
                warn!("NVS get '{key}': {e}");
                None
            }
        }
    }
}
