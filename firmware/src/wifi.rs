//! WiFi: STA with a scan-first connect, AP fallback for the setup portal.
//!
//! Lifted from `~/projects/stripe-dashboard/src/wifi.rs` and cut down to one
//! saved network. The parts that are there because they were fought for, not
//! because they are pretty:
//!
//! - **`WIFI_STORAGE_RAM` before anything else.** The default
//!   (`WIFI_STORAGE_FLASH`) makes the WiFi driver write its own config into
//!   NVS, which corrupts the application namespace: `set_str` returns `Ok` and
//!   the read afterwards says not-found.
//! - **Scan first, then connect with the auth method the router actually
//!   advertises.** Hardcoding `WPA2Personal` against a WPA2/WPA3 transition
//!   router fails silently.
//! - **`pmf_cfg` is patched through raw FFI** because esp-idf-svc 0.51
//!   hardcodes `{capable: false, required: false}` regardless of the
//!   `ClientConfiguration`, and transition-mode routers require PMF capability.
//! - **The AP's DHCP server must advertise the AP's own IP as DNS**, or phones
//!   use cellular DNS and skip the captive portal entirely. The Rust
//!   `EspNetif.handle` field is private, hence
//!   `esp_netif_get_handle_from_ifkey`.
//! - **The setup AP runs as AP+STA**, so the setup page's Scan works: an
//!   AP-only radio can't scan.
//! - **A work or school network (802.1X, WPA-Enterprise)** is a saved
//!   username ([`Eap`]): ESP-IDF's EAP client gets the username, the outer
//!   identity and the password, and answers PEAP or TTLS, whichever the
//!   network asks for (`CONFIG_ESP_WIFI_ENTERPRISE_SUPPORT`, its TLS the
//!   supplicant's own: opendmd-notes docs/notes/firmware.md "WPA-Enterprise"). The
//!   server's certificate is not checked (no CA certificate is kept), and
//!   there is no EAP-TLS (no client certificate).
//!
//! - **Scans keep nothing of ours on the heap.** esp-idf-svc's `scan()`
//!   copies every answering AP into a `Vec` sized by their count, and its
//!   record conversion panics on a secondary channel it does not know: the
//!   scans here read the driver's records one at a time onto the stack
//!   (`esp_wifi_scan_get_ap_record`) and free its list. The connect's scan
//!   is directed at the network's name ([`WifiManager::connect_sta`]); the
//!   setup page's lists at most [`SCAN_LIST_MAX`] names.
//! - **A reconnect goes straight back first** ([`WifiManager::connect_direct`]):
//!   pinned to the access point, channel and security of the last good
//!   connection ([`Learned`]), no scan; `opendmd_core::wifi_reconnect` says
//!   when to scan instead, when to pause and when the heap is too low.
//!
//! Everything in here runs on the network worker thread (CPU0). The display
//! loop never calls into this module, but for [`act`] and the counters
//! (atomics: the restart record's activity word and `/api/status`).

use esp_idf_hal::modem::Modem;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use esp_idf_svc::wifi::{
    AccessPointConfiguration, AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi,
};
use log::{info, warn};
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering::Relaxed};
use std::thread;
use std::time::{Duration, Instant};

use crate::config;
use crate::nvs_config::Settings;
use opendmd_core::restarts::WifiAct;
use opendmd_core::wifi_login::{self, Phase2};
use opendmd_core::wifi_reconnect::Heap;

/// What the WiFi is doing now ([`WifiAct`], as `u8`).
static ACT: AtomicU8 = AtomicU8::new(WifiAct::Down as u8);
/// Drops seen (the association lost while connected), reconnect attempts
/// (each a pinned connect or a scan and its ladder), reconnects made.
static DROPS: AtomicU32 = AtomicU32::new(0);
static TRIES: AtomicU32 = AtomicU32::new(0);
static RECONNECTS: AtomicU32 = AtomicU32::new(0);
/// The last drop's uptime, seconds (`u32::MAX`: none yet).
static LAST_DROP_S: AtomicU32 = AtomicU32::new(u32::MAX);

/// What the WiFi is doing (the restart record's activity word).
pub fn act() -> WifiAct {
    WifiAct::from_u8(ACT.load(Relaxed))
}

pub fn set_act(a: WifiAct) {
    ACT.store(a as u8, Relaxed);
}

/// Reconnect attempts this run.
pub fn reconnect_tries() -> u32 {
    TRIES.load(Relaxed)
}

/// A drop seen at `uptime_s`.
pub fn note_drop(uptime_s: u32) {
    DROPS.fetch_add(1, Relaxed);
    LAST_DROP_S.store(uptime_s, Relaxed);
}

/// A reconnect attempt begun.
pub fn note_try() {
    TRIES.fetch_add(1, Relaxed);
}

/// A reconnect made.
pub fn note_reconnected() {
    RECONNECTS.fetch_add(1, Relaxed);
}

/// `"wifi_drops":N,"wifi_reconnects":N,"wifi_reconnect_tries":N,
/// "wifi_last_drop_s":N|null,"wifi_activity":"…"` (`/api/status` `net`).
pub fn status_fields() -> String {
    let last = LAST_DROP_S.load(Relaxed);
    format!(
        r#""wifi_drops":{},"wifi_reconnects":{},"wifi_reconnect_tries":{},"wifi_last_drop_s":{},"wifi_activity":"{}""#,
        DROPS.load(Relaxed),
        RECONNECTS.load(Relaxed),
        TRIES.load(Relaxed),
        if last == u32::MAX { "null".to_string() } else { last.to_string() },
        act().name()
    )
}

/// The heap now, as a scan or a reconnect weighs it (one heap walk for
/// the largest block).
pub fn heap_now() -> Heap {
    Heap {
        free: crate::state::free_heap(),
        largest: crate::state::largest_free_block() as u32,
    }
}

/// The most networks the setup page's scan lists (the strongest of each
/// name; the driver's other records are read and dropped).
pub const SCAN_LIST_MAX: usize = 24;

/// The last good connection: the network, its access point, channel and
/// the security that joined it. Fixed-size (no heap): it is written as a
/// connect succeeds and read by the reconnect.
#[derive(Clone, Copy)]
pub struct Learned {
    ssid: [u8; 32],
    ssid_len: u8,
    bssid: [u8; 6],
    channel: u8,
    auth: AuthMethod,
}

impl Learned {
    fn is_for(&self, ssid: &str) -> bool {
        &self.ssid[..self.ssid_len as usize] == ssid.as_bytes()
    }
}

/// One AP a directed scan found.
#[derive(Clone, Copy)]
struct Found {
    bssid: [u8; 6],
    channel: u8,
    rssi: i8,
    auth: Option<AuthMethod>,
}

/// The driver's auth mode as esp-idf-svc names it (`None`: a mode it has no
/// name for, WPA3-Enterprise among them).
#[allow(non_upper_case_globals)]
fn auth_of(mode: esp_idf_sys::wifi_auth_mode_t) -> Option<AuthMethod> {
    use esp_idf_sys::*;
    Some(match mode {
        wifi_auth_mode_t_WIFI_AUTH_OPEN => AuthMethod::None,
        wifi_auth_mode_t_WIFI_AUTH_WEP => AuthMethod::WEP,
        wifi_auth_mode_t_WIFI_AUTH_WPA_PSK => AuthMethod::WPA,
        wifi_auth_mode_t_WIFI_AUTH_WPA2_PSK => AuthMethod::WPA2Personal,
        wifi_auth_mode_t_WIFI_AUTH_WPA_WPA2_PSK => AuthMethod::WPAWPA2Personal,
        wifi_auth_mode_t_WIFI_AUTH_WPA2_ENTERPRISE => AuthMethod::WPA2Enterprise,
        wifi_auth_mode_t_WIFI_AUTH_WPA3_PSK => AuthMethod::WPA3Personal,
        wifi_auth_mode_t_WIFI_AUTH_WPA2_WPA3_PSK => AuthMethod::WPA2WPA3Personal,
        wifi_auth_mode_t_WIFI_AUTH_WAPI_PSK => AuthMethod::WAPIPersonal,
        _ => return None,
    })
}

/// A record's SSID (up to its NUL).
fn record_ssid(rec: &esp_idf_sys::wifi_ap_record_t) -> &[u8] {
    let n = rec.ssid.iter().position(|&b| b == 0).unwrap_or(rec.ssid.len()).min(32);
    &rec.ssid[..n]
}

/// One blocking scan (`ssid`: directed at that name only, which also finds
/// it hidden), then `each` with every record the driver kept, one at a
/// time on the stack, and the driver's list freed. The number of records.
fn scan_records(ssid: Option<&str>, each: &mut dyn FnMut(&esp_idf_sys::wifi_ap_record_t)) -> Result<u16, esp_idf_sys::EspError> {
    let mut name = [0u8; 33];
    // SAFETY: a zeroed scan config is IDF's defaults (an active scan of
    // every channel, the default dwell times); `name` is NUL-terminated
    // and outlives the blocking call. The records are read into a local
    // and the list is cleared whatever happened.
    unsafe {
        let mut cfg: esp_idf_sys::wifi_scan_config_t = core::mem::zeroed();
        if let Some(s) = ssid {
            let n = s.len().min(32);
            name[..n].copy_from_slice(&s.as_bytes()[..n]);
            cfg.ssid = name.as_mut_ptr();
            cfg.show_hidden = true;
        }
        esp_idf_sys::esp!(esp_idf_sys::esp_wifi_scan_start(&cfg, true))?;
        let mut n: u16 = 0;
        let counted = esp_idf_sys::esp_wifi_scan_get_ap_num(&mut n);
        let mut rec: esp_idf_sys::wifi_ap_record_t = core::mem::zeroed();
        if counted == esp_idf_sys::ESP_OK {
            for _ in 0..n {
                if esp_idf_sys::esp_wifi_scan_get_ap_record(&mut rec) != esp_idf_sys::ESP_OK {
                    break;
                }
                each(&rec);
            }
        }
        esp_idf_sys::esp_wifi_clear_ap_list();
        esp_idf_sys::esp!(counted)?;
        Ok(n)
    }
}

/// A work or school network's login (802.1X): the password is the WiFi
/// password. From the saved settings ([`Eap::of`]); never logged.
#[derive(Clone, PartialEq, Eq)]
pub struct Eap {
    pub user: String,
    pub identity: String,
    pub ttls: Phase2,
}

impl Eap {
    /// The saved login, `None` without a username.
    pub fn of(s: &Settings) -> Option<Eap> {
        (!s.wifi_eap_user.is_empty()).then(|| Eap {
            user: s.wifi_eap_user.clone(),
            identity: s.wifi_eap_identity.clone(),
            ttls: s.wifi_eap_ttls,
        })
    }
}

/// One rung of the connect ladder: auth method, optional channel and BSSID
/// pins, and how many attempts to spend on it.
type ConnectStage = (AuthMethod, Option<u8>, Option<[u8; 6]>, u32);

/// An attempt's two waits, each up to 15 s (esp-idf-svc's own limit): the
/// association (a wrong password never associates, so it is the whole
/// wait), then the interface getting an address (DHCP). Then a 2 s pause
/// before the next attempt.
const ASSOC_WAIT: Duration = Duration::from_secs(15);
const NETIF_WAIT: Duration = Duration::from_secs(15);
const RETRY_PAUSE: Duration = Duration::from_secs(2);
/// The waits look at the driver this often, and at the installer's queue
/// (`improv::pending`): a job waits at most this long for a connect to give
/// way.
const WAIT_SLICE: Duration = Duration::from_millis(100);
/// With a deadline, an attempt starts only if this much is left (an
/// association on a good network takes 1-3 s, DHCP about as long); every
/// wait is clipped to the deadline, so none runs past it.
const ATTEMPT_MIN: Duration = Duration::from_secs(8);

/// How a wait ended.
#[derive(PartialEq)]
enum Waited {
    Done,
    TimedOut,
    /// Work for the installer is queued (`improv::pending`) and this connect
    /// is not the installer's own: stop and let the worker take it.
    GaveWay,
}

/// A nearby AP, as returned to the setup page by `GET /api/wifi/scan`.
#[derive(Debug, Clone)]
pub struct ScannedAp {
    pub ssid: String,
    pub rssi: i8,
    pub channel: u8,
    pub auth: &'static str,
    pub secure: bool,
}

fn auth_label(auth: AuthMethod) -> (&'static str, bool) {
    match auth {
        AuthMethod::None => ("Open", false),
        AuthMethod::WEP => ("WEP", true),
        AuthMethod::WPA => ("WPA", true),
        AuthMethod::WPA2Personal => ("WPA2", true),
        AuthMethod::WPAWPA2Personal => ("WPA/WPA2", true),
        AuthMethod::WPA2Enterprise => ("WPA2-Ent", true),
        AuthMethod::WPA3Personal => ("WPA3", true),
        AuthMethod::WPA2WPA3Personal => ("WPA2/WPA3", true),
        AuthMethod::WAPIPersonal => ("WAPI", true),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiState {
    Disconnected,
    Connected,
    ApMode,
}

impl WifiState {
    pub fn as_str(self) -> &'static str {
        match self {
            WifiState::Disconnected => "disconnected",
            WifiState::Connected => "sta",
            WifiState::ApMode => "ap",
        }
    }
}

pub struct WifiManager {
    wifi: BlockingWifi<EspWifi<'static>>,
    state: WifiState,
    /// The EAP client is on (`esp_wifi_sta_enterprise_enable`).
    eap_on: bool,
    /// The last good connection, for [`WifiManager::connect_direct`].
    learned: Option<Learned>,
}

/// The station's DHCP client name (`esp_netif_set_hostname`). Before the
/// driver starts, the first DHCP request carries it; on a running netif
/// (a rename, `net_worker`) lwip sends it from the next lease renewal or
/// reconnect. esp-netif copies the string and refuses more than 32 bytes
/// (`opendmd_core::hostname::MAX`).
pub fn set_sta_hostname(hostname: &str) {
    let Ok(name) = std::ffi::CString::new(hostname) else {
        return;
    };
    // SAFETY: the netif handle is checked for null; `name` outlives the call.
    let err = unsafe {
        let netif =
            esp_idf_sys::esp_netif_get_handle_from_ifkey(c"WIFI_STA_DEF".as_ptr() as *const _);
        if netif.is_null() {
            return;
        }
        esp_idf_sys::esp_netif_set_hostname(netif, name.as_ptr())
    };
    if err != esp_idf_sys::ESP_OK {
        warn!("[wifi] DHCP name {hostname} not set: error {err}");
    }
}

impl WifiManager {
    pub fn new(
        modem: Modem,
        sysloop: EspSystemEventLoop,
        nvs: EspNvsPartition<NvsDefault>,
        hostname: &str,
    ) -> anyhow::Result<Self> {
        let wifi = EspWifi::new(modem, sysloop.clone(), Some(nvs))?;
        let wifi = BlockingWifi::wrap(wifi, sysloop)?;
        crate::heapmap::cp("wifi: EspWifi::new");

        // Must happen before any set_configuration/start, AP mode included.
        // SAFETY: an argument-only driver setting, called before the driver is
        // configured or started.
        unsafe {
            esp_idf_sys::esp_wifi_set_storage(esp_idf_sys::wifi_storage_t_WIFI_STORAGE_RAM);
        }

        // The STA hostname is what the router's device list and DHCP show;
        // set before start() it goes out with the first DHCP request.
        set_sta_hostname(hostname);

        Ok(WifiManager {
            wifi,
            state: WifiState::Disconnected,
            eap_on: false,
            learned: None,
        })
    }

    pub fn state(&self) -> WifiState {
        self.state
    }

    /// The association was lost (the worker saw it): disconnected from
    /// here on, so the next look is a reconnect, not another drop.
    pub fn mark_dropped(&mut self) {
        if self.state == WifiState::Connected {
            self.state = WifiState::Disconnected;
        }
        set_act(WifiAct::Down);
    }

    /// The emulator's low-heap lane (`oom_test`): the learned access point
    /// and channel made wrong, so the pinned connect fails.
    #[cfg(feature = "emu-oom-test")]
    pub fn spoil_learned(&mut self) {
        if let Some(l) = self.learned.as_mut() {
            l.bssid[5] ^= 0x55;
            l.channel = l.channel % 13 + 1;
        }
    }

    /// The last good connection is to `ssid` (a pinned connect can be tried).
    pub fn knows(&self, ssid: &str) -> bool {
        self.learned.as_ref().is_some_and(|l| l.is_for(ssid))
    }

    /// Note the connection just made to `ssid` with `auth`: its access
    /// point and channel from the driver.
    fn learn(&mut self, ssid: &str, auth: AuthMethod) {
        // SAFETY: the driver fills a zeroed record of its own type.
        let mut rec: esp_idf_sys::wifi_ap_record_t = unsafe { core::mem::zeroed() };
        if unsafe { esp_idf_sys::esp_wifi_sta_get_ap_info(&mut rec) } != esp_idf_sys::ESP_OK || ssid.len() > 32 {
            self.learned = None;
            return;
        }
        let mut name = [0u8; 32];
        name[..ssid.len()].copy_from_slice(ssid.as_bytes());
        self.learned = Some(Learned {
            ssid: name,
            ssid_len: ssid.len() as u8,
            bssid: rec.bssid,
            channel: rec.primary,
            auth,
        });
    }

    /// Ask the driver itself, bypassing our state machine — this is how the
    /// worker notices a drop.
    pub fn is_associated(&self) -> bool {
        self.wifi.is_connected().unwrap_or(false)
    }

    pub fn ip_address(&self) -> Option<String> {
        let netif = match self.state {
            WifiState::Connected => self.wifi.wifi().sta_netif(),
            WifiState::ApMode => self.wifi.wifi().ap_netif(),
            WifiState::Disconnected => return None,
        };
        netif.get_ip_info().ok().map(|info| info.ip.to_string())
    }

    pub fn ap_ip_bytes(&self) -> Option<[u8; 4]> {
        self.wifi
            .wifi()
            .ap_netif()
            .get_ip_info()
            .ok()
            .map(|info| info.ip.octets())
    }

    /// On-demand scan for the setup page and `GET /api/wifi/scan`. It works
    /// in setup mode too: the setup network runs as AP+STA, so the station
    /// half scans without taking the AP down. Refused when the heap is low
    /// (`Heap::scan_ok`): the driver holds every answering AP on it until
    /// read. At most [`SCAN_LIST_MAX`] names, the list's room taken
    /// fallibly first.
    pub fn scan_networks(&mut self) -> anyhow::Result<Vec<ScannedAp>> {
        let heap = heap_now();
        if !heap.scan_ok() {
            anyhow::bail!(
                "not scanning: {} B free, largest block {} B (a scan needs {} B and {} B)",
                heap.free,
                heap.largest,
                opendmd_core::wifi_reconnect::SCAN_MIN_FREE,
                opendmd_core::wifi_reconnect::SCAN_MIN_BLOCK
            );
        }
        let mut best: Vec<ScannedAp> = Vec::new();
        best.try_reserve_exact(SCAN_LIST_MAX)
            .map_err(|_| anyhow::anyhow!("not scanning: no room for the list"))?;
        let was = act();
        set_act(WifiAct::Scanning);
        let scanned = scan_records(None, &mut |rec| {
            if rec.primary > 14 {
                return; // the ESP32 has no 5 GHz radio
            }
            let Ok(ssid) = core::str::from_utf8(record_ssid(rec)) else { return };
            if ssid.is_empty() {
                return; // hidden
            }
            // A mode esp-idf-svc 0.51 has no name for (WPA3-Enterprise,
            // WPA2/WPA3-Enterprise, OWE, ...) is not an open network.
            let (label, secure) = auth_of(rec.authmode).map_or(("Other", true), auth_label);
            let room = best.len() < SCAN_LIST_MAX;
            match best.iter_mut().find(|e| e.ssid == ssid) {
                Some(existing) if rec.rssi > existing.rssi => {
                    existing.rssi = rec.rssi;
                    existing.channel = rec.primary;
                    existing.auth = label;
                    existing.secure = secure;
                }
                Some(_) => {}
                None if room => best.push(ScannedAp {
                    ssid: ssid.to_string(),
                    rssi: rec.rssi,
                    channel: rec.primary,
                    auth: label,
                    secure,
                }),
                None => {}
            }
        });
        set_act(was);
        scanned.map_err(|e| anyhow::anyhow!("scan failed: {e}"))?;
        // Unstable: the stable sort keeps a 4 KB scratch buffer on the
        // stack (the network worker's; ties in RSSI may come in any order).
        best.sort_unstable_by_key(|ap| std::cmp::Reverse(ap.rssi));
        Ok(best)
    }

    /// The EAP client set up for `eap` (with `password`), or off with
    /// `None`. `false`: it refused the login (logged; the connect is not
    /// tried).
    #[optimize(size)]
    #[inline(never)]
    fn set_eap(&mut self, eap: Option<&Eap>, password: &str) -> bool {
        let Some(eap) = eap else {
            if self.eap_on {
                // SAFETY: argument-free driver calls on an initialised
                // driver (the clears free the EAP client's own copies).
                unsafe {
                    esp_idf_sys::esp_wifi_sta_enterprise_disable();
                    esp_idf_sys::esp_eap_client_clear_identity();
                    esp_idf_sys::esp_eap_client_clear_username();
                    esp_idf_sys::esp_eap_client_clear_password();
                }
                self.eap_on = false;
            }
            return true;
        };
        let outer = wifi_login::outer(&eap.identity, &eap.user);
        // SAFETY: each setter copies its bytes (pointer and length of a
        // live &str) before it returns; the rest are argument-only calls.
        let errs = unsafe {
            [
                esp_idf_sys::esp_eap_client_set_identity(outer.as_ptr(), outer.len() as i32),
                esp_idf_sys::esp_eap_client_set_username(eap.user.as_ptr(), eap.user.len() as i32),
                if password.is_empty() {
                    esp_idf_sys::esp_eap_client_clear_password();
                    esp_idf_sys::ESP_OK
                } else {
                    esp_idf_sys::esp_eap_client_set_password(password.as_ptr(), password.len() as i32)
                },
                esp_idf_sys::esp_eap_client_set_ttls_phase2_method(eap.ttls.esp_type()),
                esp_idf_sys::esp_wifi_sta_enterprise_enable(),
            ]
        };
        if let Some(e) = errs.iter().find(|&&e| e != esp_idf_sys::ESP_OK) {
            warn!("[wifi] the work or school login was refused by the EAP client: error {e}");
            return false;
        }
        self.eap_on = true;
        true
    }

    /// Advertise PMF capability. esp-idf-svc 0.51 hardcodes it off.
    fn fix_pmf(&self) {
        // SAFETY: get/modify/set of the driver's own config struct, zeroed
        // before the get fills it in.
        unsafe {
            let mut cfg: esp_idf_sys::wifi_config_t = core::mem::zeroed();
            if esp_idf_sys::esp_wifi_get_config(esp_idf_sys::wifi_interface_t_WIFI_IF_STA, &mut cfg)
                != 0
            {
                return;
            }
            cfg.sta.pmf_cfg = esp_idf_sys::wifi_pmf_config_t {
                capable: true,
                required: false,
            };
            esp_idf_sys::esp_wifi_set_config(esp_idf_sys::wifi_interface_t_WIFI_IF_STA, &mut cfg);
        }
    }

    /// Wait up to `limit` (and never past `deadline`) for `done`. Without a
    /// deadline (not the installer's own connect), gives way to queued
    /// installer work.
    fn wait(
        &self,
        limit: Duration,
        deadline: Option<Instant>,
        done: impl Fn(&Self) -> bool,
    ) -> Waited {
        let start = Instant::now();
        let end = deadline.map_or(start + limit, |d| d.min(start + limit));
        loop {
            if done(self) {
                return Waited::Done;
            }
            if deadline.is_none() && crate::improv::pending() {
                return Waited::GaveWay;
            }
            // The worker's loop is not running meanwhile.
            crate::improv::log_events();
            let now = Instant::now();
            if now >= end {
                return Waited::TimedOut;
            }
            thread::sleep(WAIT_SLICE.min(end - now));
        }
    }

    /// Connect attempts on the configuration set. `Err(())`: gave way to
    /// the installer (see [`Waited::GaveWay`]).
    fn try_connect(
        &mut self,
        attempts: u32,
        stage: u8,
        deadline: Option<Instant>,
    ) -> Result<bool, ()> {
        for attempt in 1..=attempts {
            if deadline.is_some_and(|d| d.saturating_duration_since(Instant::now()) < ATTEMPT_MIN) {
                info!("  STA stage {stage}: out of time");
                return Ok(false);
            }
            if deadline.is_none() && crate::improv::pending() {
                return Err(());
            }
            info!("  STA stage {stage}, attempt {attempt}/{attempts}");
            set_act(WifiAct::Connecting);
            // The driver's non-blocking connect, then our own waits
            // (`BlockingWifi::connect` and `wait_netif_up` block 15 s each
            // and can be neither interrupted nor clipped).
            let mut waited = Waited::TimedOut;
            if self.wifi.wifi_mut().connect().is_ok() {
                waited = self.wait(ASSOC_WAIT, deadline, |s| {
                    s.wifi.wifi().is_connected().unwrap_or(false)
                });
                if waited == Waited::Done {
                    waited = self.wait(NETIF_WAIT, deadline, |s| s.wifi.is_up().unwrap_or(false));
                }
            }
            if waited == Waited::Done {
                if let Ok(ip) = self.wifi.wifi().sta_netif().get_ip_info() {
                    info!("WiFi connected, IP {}", ip.ip);
                    self.state = WifiState::Connected;
                    set_act(WifiAct::Up);
                    // No power save: the httpd is latency-sensitive and the
                    // board is mains-powered anyway.
                    // SAFETY: an argument-only driver setting.
                    unsafe {
                        esp_idf_sys::esp_wifi_set_ps(esp_idf_sys::wifi_ps_type_t_WIFI_PS_NONE);
                    }
                    return Ok(true);
                }
            }
            let _ = self.wifi.disconnect();
            if waited == Waited::GaveWay {
                return Err(());
            }
            if attempt < attempts && self.wait(RETRY_PAUSE, deadline, |_| false) == Waited::GaveWay
            {
                return Err(());
            }
        }
        Ok(false)
    }

    /// Scan, find the SSID, and connect with the auth method it advertises,
    /// falling back through progressively looser configurations.
    ///
    /// `Ok(true)` on success. `Ok(false)` means "not in range, or every stage
    /// failed" — the caller counts those and falls back to AP mode. Also
    /// `Ok(false)`, without a deadline, when work for the browser installer
    /// was queued meanwhile (`improv::pending`): the connect stops at its
    /// next step (within 100 ms) so the worker can take it; the caller
    /// should not count that. With a `deadline` (WiFi details sent over
    /// USB, which the installer waits 45 s for) nothing runs past it and
    /// nothing gives way.
    #[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
    #[inline(never)]
    pub fn connect_sta(
        &mut self,
        ssid: &str,
        password: &str,
        eap: Option<&Eap>,
        deadline: Option<Instant>,
    ) -> anyhow::Result<bool> {
        if ssid.is_empty() {
            return Ok(false);
        }
        // 802.1X: the password goes to the EAP client, not the driver's
        // PSK (and may be longer than its 64 bytes).
        if !self.set_eap(eap, password) {
            self.state = WifiState::Disconnected;
            return Ok(false);
        }
        let password = if eap.is_some() { "" } else { password };
        if deadline.is_none() && crate::improv::pending() {
            self.state = WifiState::Disconnected;
            return Ok(false);
        }
        // Still on a network (Change Wi-Fi over USB while connected): leave
        // it first, or the wait below sees the old association at once and
        // takes it for the new one.
        if self.wifi.is_connected().unwrap_or(false) {
            let _ = self.wifi.disconnect();
        }
        let fallback_auth = if eap.is_some() {
            AuthMethod::WPA2Enterprise
        } else if password.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        };

        // Bring STA up with a seed config so a scan is possible at all.
        self.wifi
            .set_configuration(&Configuration::Client(ClientConfiguration {
                ssid: ssid.try_into().unwrap_or_default(),
                password: password.try_into().unwrap_or_default(),
                auth_method: fallback_auth,
                ..Default::default()
            }))?;
        if !self.wifi.is_started().unwrap_or(false) {
            self.wifi.start()?;
        }
        crate::heapmap::cp("wifi: started (sta)");

        info!("scanning for '{ssid}'");
        set_act(WifiAct::Scanning);
        // Directed at the name: the driver keeps only its access points.
        let mut found: Option<Found> = None;
        let mut pick = |rec: &esp_idf_sys::wifi_ap_record_t| {
            if rec.primary <= 14
                && record_ssid(rec) == ssid.as_bytes()
                && found.is_none_or(|f| rec.rssi > f.rssi)
            {
                found = Some(Found {
                    bssid: rec.bssid,
                    channel: rec.primary,
                    rssi: rec.rssi,
                    auth: auth_of(rec.authmode),
                });
            }
        };
        // A scan right after an association was abandoned can fail while
        // the driver settles: once more, half a second later.
        let scanned = match scan_records(Some(ssid), &mut pick) {
            Ok(n) => n,
            Err(e) => {
                warn!("scan failed ({e}); retrying");
                thread::sleep(Duration::from_millis(500));
                scan_records(Some(ssid), &mut pick).unwrap_or(0)
            }
        };

        let Some(ap) = found else {
            warn!("'{ssid}' not found (a scan for it answered by {scanned} APs)");
            self.state = WifiState::Disconnected;
            set_act(WifiAct::Down);
            return Ok(false);
        };
        // (A mode esp-idf-svc has no name for, WPA3-Enterprise among them,
        // is `None`: the driver then takes whatever the network offers.)
        let detected = ap.auth.unwrap_or(AuthMethod::None);
        info!(
            "'{ssid}' found: rssi={} ch={} auth={detected:?}{}",
            ap.rssi,
            ap.channel,
            if eap.is_some() { ", joining with a work or school login (802.1X)" } else { "" }
        );

        // Stage 1: pin BSSID and channel, use the detected auth method.
        // Stage 2: same auth, let the driver roam.
        // Stage 3: the fallback auth method, minimal config.
        let stages: [ConnectStage; 3] = [
            (detected, Some(ap.channel), Some(ap.bssid), 3),
            (detected, None, None, 3),
            (fallback_auth, None, None, 4),
        ];
        for (stage, (auth, channel, bssid, attempts)) in stages.into_iter().enumerate() {
            self.wifi
                .set_configuration(&Configuration::Client(ClientConfiguration {
                    ssid: ssid.try_into().unwrap_or_default(),
                    password: password.try_into().unwrap_or_default(),
                    auth_method: auth,
                    channel,
                    bssid,
                    ..Default::default()
                }))?;
            self.fix_pmf();
            match self.try_connect(attempts, stage as u8 + 1, deadline) {
                Ok(true) => {
                    self.learn(ssid, auth);
                    return Ok(true);
                }
                Ok(false) => {}
                Err(()) => {
                    info!("connect to '{ssid}' paused: the browser installer is waiting");
                    self.state = WifiState::Disconnected;
                    set_act(WifiAct::Down);
                    return Ok(false);
                }
            }
        }

        warn!("every connect stage failed for '{ssid}'");
        self.state = WifiState::Disconnected;
        set_act(WifiAct::Down);
        Ok(false)
    }

    /// Back onto `ssid` (a reconnect after a drop) without a scan: one
    /// attempt pinned to the access point, channel and security of the last
    /// good connection ([`Learned`]). `Ok(false)` when it does not join (or
    /// nothing is known for `ssid`): the caller scans next
    /// (`opendmd_core::wifi_reconnect`). The EAP client keeps the login it
    /// was given by the connect that learned it.
    #[optimize(size)]
    #[inline(never)]
    pub fn connect_direct(&mut self, ssid: &str, password: &str, eap: Option<&Eap>) -> anyhow::Result<bool> {
        let Some(l) = self.learned.filter(|l| l.is_for(ssid)) else {
            return Ok(false);
        };
        let password = if eap.is_some() { "" } else { password };
        info!(
            "reconnecting to '{ssid}' at {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} ch {} ({:?}), no scan",
            l.bssid[0], l.bssid[1], l.bssid[2], l.bssid[3], l.bssid[4], l.bssid[5], l.channel, l.auth
        );
        if self.wifi.is_connected().unwrap_or(false) {
            let _ = self.wifi.disconnect();
        }
        self.wifi
            .set_configuration(&Configuration::Client(ClientConfiguration {
                ssid: ssid.try_into().unwrap_or_default(),
                password: password.try_into().unwrap_or_default(),
                auth_method: l.auth,
                channel: Some(l.channel),
                bssid: Some(l.bssid),
                ..Default::default()
            }))?;
        if !self.wifi.is_started().unwrap_or(false) {
            self.wifi.start()?;
        }
        self.fix_pmf();
        match self.try_connect(1, 0, None) {
            Ok(true) => Ok(true),
            Ok(false) | Err(()) => {
                self.state = WifiState::Disconnected;
                set_act(WifiAct::Down);
                Ok(false)
            }
        }
    }

    /// Leave the network joined, if any (an offline display's join that
    /// could not be saved).
    #[optimize(size)]
    #[inline(never)]
    pub fn disconnect(&mut self) -> anyhow::Result<()> {
        if self.wifi.is_connected().unwrap_or(false) {
            self.wifi.disconnect()?;
        }
        self.state = WifiState::Disconnected;
        set_act(WifiAct::Down);
        Ok(())
    }

    /// The station started with no network to join: what a scan needs
    /// (an offline display's radio, started for the browser installer:
    /// `net_worker::Offline`). Nothing is joined and nothing is advertised.
    #[optimize(size)]
    #[inline(never)]
    pub fn start_idle(&mut self) -> anyhow::Result<()> {
        if !self.wifi.is_started().unwrap_or(false) {
            self.wifi
                .set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
            self.wifi.start()?;
        }
        Ok(())
    }

    /// Raise the open setup network and point its DHCP server at ourselves.
    #[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
    #[inline(never)]
    pub fn start_ap(&mut self) -> anyhow::Result<()> {
        info!("starting AP mode: {}", config::AP_SSID);

        // AP+STA, with the STA side idle: only so that Scan works.
        self.wifi.set_configuration(&Configuration::Mixed(
            ClientConfiguration::default(),
            AccessPointConfiguration {
                ssid: config::AP_SSID.try_into().unwrap_or_default(),
                auth_method: AuthMethod::None,
                max_connections: 4,
                ..Default::default()
            },
        ))?;
        if self.wifi.is_started().unwrap_or(false) {
            self.wifi.stop()?;
        }
        self.wifi.start()?;
        crate::heapmap::cp("wifi: started (ap)");

        if let Ok(ip_info) = self.wifi.wifi().ap_netif().get_ip_info() {
            // SAFETY: the netif handle is null-checked; every pointer handed to
            // esp_netif here points at a live local that outlives the call.
            unsafe {
                let netif = esp_idf_sys::esp_netif_get_handle_from_ifkey(
                    c"WIFI_AP_DEF".as_ptr() as *const _
                );
                if !netif.is_null() {
                    esp_idf_sys::esp_netif_dhcps_stop(netif);
                    let mut dns: esp_idf_sys::esp_netif_dns_info_t = core::mem::zeroed();
                    dns.ip.u_addr.ip4.addr = u32::from_ne_bytes(ip_info.ip.octets());
                    esp_idf_sys::esp_netif_set_dns_info(
                        netif,
                        esp_idf_sys::esp_netif_dns_type_t_ESP_NETIF_DNS_MAIN,
                        &mut dns,
                    );
                    let mut offer_dns: u8 = 2; // OFFER_DNS
                    esp_idf_sys::esp_netif_dhcps_option(
                        netif,
                        esp_idf_sys::esp_netif_dhcp_option_mode_t_ESP_NETIF_OP_SET,
                        esp_idf_sys::esp_netif_dhcp_option_id_t_ESP_NETIF_DOMAIN_NAME_SERVER,
                        &mut offer_dns as *mut _ as *mut core::ffi::c_void,
                        core::mem::size_of_val(&offer_dns) as u32,
                    );
                    esp_idf_sys::esp_netif_dhcps_start(netif);
                }
            }
            info!(
                "AP mode active: SSID '{}' at http://{}/",
                config::AP_SSID,
                ip_info.ip
            );
        } else {
            info!("AP mode active: SSID '{}'", config::AP_SSID);
        }

        self.state = WifiState::ApMode;
        set_act(WifiAct::Setup);
        Ok(())
    }
}
