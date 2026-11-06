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
//!   supplicant's own: docs/notes/firmware.md "WPA-Enterprise"). The
//!   server's certificate is not checked (no CA certificate is kept), and
//!   there is no EAP-TLS (no client certificate).
//!
//! Everything in here runs on the network worker thread (CPU0). The display
//! loop never calls into this module.

use esp_idf_hal::modem::Modem;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use esp_idf_svc::wifi::{
    AccessPointConfiguration, AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi,
};
use log::{info, warn};
use std::thread;
use std::time::{Duration, Instant};

use crate::config;
use crate::nvs_config::Settings;
use opendmd_core::wifi_login::{self, Phase2};

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
        })
    }

    pub fn state(&self) -> WifiState {
        self.state
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

    /// On-demand scan for the setup page. Unsupported in AP mode: a STA scan
    /// would tear the AP down and disconnect the browser that asked.
    pub fn scan_networks(&mut self) -> anyhow::Result<Vec<ScannedAp>> {
        let aps = self
            .wifi
            .wifi_mut()
            .scan()
            .map_err(|e| anyhow::anyhow!("scan failed: {e}"))?;

        let mut best: Vec<ScannedAp> = Vec::new();
        for ap in aps {
            if ap.channel > 14 {
                continue; // the ESP32 has no 5 GHz radio
            }
            let ssid = ap.ssid.as_str().to_string();
            if ssid.is_empty() {
                continue; // hidden
            }
            // A mode esp-idf-svc 0.51 has no name for (WPA3-Enterprise,
            // WPA2/WPA3-Enterprise, OWE, ...) is not an open network.
            let (label, secure) = ap.auth_method.map_or(("Other", true), auth_label);
            let candidate = ScannedAp {
                ssid,
                rssi: ap.signal_strength,
                channel: ap.channel,
                auth: label,
                secure,
            };
            match best.iter_mut().find(|e| e.ssid == candidate.ssid) {
                Some(existing) if candidate.rssi > existing.rssi => *existing = candidate,
                Some(_) => {}
                None => best.push(candidate),
            }
        }
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
        // A scan right after an association was abandoned can fail while
        // the driver settles: once more, half a second later.
        let scan = match self.wifi.wifi_mut().scan() {
            Ok(s) => s,
            Err(e) => {
                warn!("scan failed ({e}); retrying");
                thread::sleep(Duration::from_millis(500));
                self.wifi.wifi_mut().scan().unwrap_or_default()
            }
        };
        let found = scan
            .iter()
            .filter(|ap| ap.channel <= 14 && ap.ssid.as_str() == ssid)
            .max_by_key(|ap| ap.signal_strength);

        let Some(ap) = found else {
            warn!("'{ssid}' not found in a scan of {} APs", scan.len());
            self.state = WifiState::Disconnected;
            return Ok(false);
        };
        // (A mode esp-idf-svc has no name for, WPA3-Enterprise among them,
        // is `None`: the driver then takes whatever the network offers.)
        let detected = ap.auth_method.unwrap_or(AuthMethod::None);
        info!(
            "'{ssid}' found: rssi={} ch={} auth={detected:?}{}",
            ap.signal_strength,
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
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(()) => {
                    info!("connect to '{ssid}' paused: the browser installer is waiting");
                    self.state = WifiState::Disconnected;
                    return Ok(false);
                }
            }
        }

        warn!("every connect stage failed for '{ssid}'");
        self.state = WifiState::Disconnected;
        Ok(false)
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
        Ok(())
    }
}
