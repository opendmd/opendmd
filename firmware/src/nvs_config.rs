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

use crate::config;

const NAMESPACE: &str = "opendmd";

const KEY_WIFI_SSID: &str = "wifi_ssid";
const KEY_WIFI_PASS: &str = "wifi_pass";
const KEY_BRIGHTNESS: &str = "brightness";
const KEY_DWELL_MIN: &str = "dwell_min";
const KEY_DWELL_MAX: &str = "dwell_max";
const KEY_SHUFFLE: &str = "shuffle";
const KEY_PLATFORMS: &str = "platforms";
const KEY_SET: &str = "set";
const KEY_HOSTNAME: &str = "hostname";

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
    pub dwell_min: u64,
    pub dwell_max: u64,
    pub shuffle: bool,
    /// Comma-separated platform prefixes, e.g. `MD,SNES`. Empty = everything.
    pub platforms: String,
    /// Active set name; empty means the card's own `filelist.txt`.
    pub set: String,
    pub hostname: String,
}

impl Default for Settings {
    /// `config.rs` stays the single source of truth for the fallbacks.
    fn default() -> Self {
        Settings {
            wifi_ssid: String::new(),
            wifi_pass: String::new(),
            brightness: config::BRIGHTNESS,
            dwell_min: config::DWELL_MIN_SECS,
            dwell_max: config::DWELL_MAX_SECS,
            shuffle: config::SHUFFLE,
            platforms: config::PLATFORMS.join(","),
            set: String::new(),
            hostname: config::HOSTNAME.to_string(),
        }
    }
}

impl Settings {
    /// Card-absolute path of the manifest this configuration selects.
    pub fn manifest_path(&self) -> String {
        if self.set.is_empty() {
            config::MANIFEST_PATH.to_string()
        } else {
            format!("{}/{}.txt", config::SETS_DIR, self.set)
        }
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
        Settings {
            wifi_ssid: self.get_string(KEY_WIFI_SSID).unwrap_or(d.wifi_ssid),
            wifi_pass: self.get_string(KEY_WIFI_PASS).unwrap_or(d.wifi_pass),
            brightness: self
                .nvs
                .get_u8(KEY_BRIGHTNESS)
                .ok()
                .flatten()
                .unwrap_or(d.brightness),
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
        }
    }

    /// Write every field back. Cheap enough (NVS only rewrites what changed)
    /// and it keeps the "PUT replaces the config" semantics honest.
    pub fn save(&mut self, s: &Settings) -> anyhow::Result<()> {
        self.nvs.set_str(KEY_WIFI_SSID, &s.wifi_ssid)?;
        self.nvs.set_str(KEY_WIFI_PASS, &s.wifi_pass)?;
        self.nvs.set_u8(KEY_BRIGHTNESS, s.brightness)?;
        self.nvs.set_str(KEY_DWELL_MIN, &s.dwell_min.to_string())?;
        self.nvs.set_str(KEY_DWELL_MAX, &s.dwell_max.to_string())?;
        self.nvs.set_u8(KEY_SHUFFLE, u8::from(s.shuffle))?;
        self.nvs.set_str(KEY_PLATFORMS, &s.platforms)?;
        self.nvs.set_str(KEY_SET, &s.set)?;
        self.nvs.set_str(KEY_HOSTNAME, &s.hostname)?;
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
