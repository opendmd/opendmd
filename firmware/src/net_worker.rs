//! The network worker: one thread, pinned to CPU0, that owns everything with a
//! radio or a socket in it.
//!
//! Shape lifted from `~/projects/stripe-dashboard/src/net_worker.rs` — a named
//! thread with a 16 KB stack, an `mpsc` inbox drained non-blockingly, and a
//! `recv_timeout` for the idle wait so a command is never left sitting.
//!
//! The reason it exists here is the panel. The HUB75 driver's I2S-DMA
//! interrupt is installed by whichever core calls `hub75_init`, which is the
//! main task on CPU1; WiFi and lwip interrupts belong to CPU0. Pinning this
//! thread (and the httpd task it starts) to CPU0 keeps the two apart. The
//! display loop never calls into WiFi, the httpd, or any socket.
//!
//! It also owns the boot decision: try STA, and on repeated failure raise the
//! open `OpenDMD-Setup` AP with the captive-portal DNS server, on a timeout
//! that reboots rather than leaving an open network up forever.
//!
//! WiFi details saved through the API take effect at the next boot; a new
//! display name is answered to at once ([`rename`]). WiFi details
//! sent over USB by a browser installer (Improv Serial, `improv.rs`) are
//! joined live instead, because the installer waits for the answer: the
//! worker tries them (within [`config::IMPROV_ANSWER_WITHIN`]), saves them
//! only once they work, and brings up what a boot onto the network would
//! (mDNS, SNTP; the setup network and its DNS go). If they fail, the setup
//! network comes back, or the worker's reconnect goes back to the old one.

use crate::chan as mpsc;
use esp_idf_hal::modem::Modem;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::mdns::EspMdns;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use log::{error, info, warn};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::config;
use crate::dns_server::DnsServer;
use crate::http::NetCommand;
use crate::improv::Job;
use crate::nvs_config::Settings;
use crate::state::{Command, Status};
use crate::wifi::{WifiManager, WifiState};

/// Everything the worker needs to take ownership of.
pub struct NetWorker {
    pub modem: Modem,
    pub sysloop: EspSystemEventLoop,
    pub nvs: EspNvsPartition<NvsDefault>,
    pub ssid: String,
    pub password: String,
    pub hostname: String,
    pub status: Arc<Mutex<Status>>,
    pub rx: mpsc::Receiver<NetCommand>,
    /// The live settings (Follow RePlayOS reads its own from them) and the
    /// main task's command channel (where it sends what it decided).
    pub settings: Arc<Mutex<Settings>>,
    pub cmd: mpsc::Sender<Command>,
}

/// Spawn the worker on CPU0 (`task::spawn_core0`) and return immediately.
/// Called by the boot task (`boot.rs`).
pub fn spawn(worker: NetWorker) -> anyhow::Result<()> {
    crate::task::spawn_core0(b"net-worker\0", config::NET_WORKER_STACK, 5, Box::new(move || run(worker)))?;
    Ok(())
}

fn publish(status: &Arc<Mutex<Status>>, state: WifiState, ip: Option<String>) {
    if let Ok(mut s) = status.lock() {
        s.wifi = state.as_str();
        // The setup network asks for no API key (`httpd::gate`).
        crate::httpd::set_setup_mode(s.wifi == "ap");
        s.ip = ip.unwrap_or_default();
        if !s.ip.is_empty() {
            crate::state::NET_UP.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        crate::improv::set_network(s.wifi, &s.ip);
    }
}

/// mDNS as `<hostname>.local`, station mode only. Kept alive by the
/// caller: dropping it unregisters. `named` is set to the name the
/// responder took, or cleared if naming it failed (the loop retries).
fn start_mdns(hostname: &str, named: &mut String) -> Option<EspMdns> {
    match EspMdns::take() {
        Ok(mut mdns) => {
            let ok = name_mdns(&mut mdns, hostname);
            *named = if ok { hostname.to_string() } else { String::new() };
            let ok = ok && mdns.add_service(None, "_http", "_tcp", 80, &[]).is_ok();
            if ok {
                info!("[net] mDNS responding as {hostname}.local");
            } else {
                warn!("[net] mDNS registration incomplete");
            }
            Some(mdns)
        }
        Err(e) => {
            warn!("[net] mDNS unavailable: {e}");
            None
        }
    }
}

/// The responder's host name and its service instance (per display, so two
/// on one network are told apart). On a running responder (espressif/mdns
/// 1.2.5) `mdns_hostname_set` waits for the mDNS task, which sends goodbyes
/// for the old name's records, renames the service host, then probes and
/// announces everything again — so a clash with the new name still takes
/// `<name>-2` (`publish_mdns`); the instance change likewise says goodbye
/// and re-announces the service.
fn name_mdns(mdns: &mut EspMdns, hostname: &str) -> bool {
    mdns.set_hostname(hostname).is_ok()
        && mdns
            .set_instance_name(opendmd_core::hostname::instance_name(hostname))
            .is_ok()
}

/// A new display name saved through the API (`NetCommand::Rename`), in
/// effect now: the station's DHCP name (sent from the next renewal) and
/// the copies `/api/status` and Improv answer with. The running mDNS
/// responder is renamed by the loop right after ([`MdnsName`]), in place:
/// not dropped and taken again, which would free and reallocate its task
/// and buffers on a heap that has little to spare. With no responder
/// (setup network, not connected) the name is used when mDNS starts. The
/// setup network's own name does not change.
fn rename(status: &Arc<Mutex<Status>>, hostname: &str) {
    crate::wifi::set_sta_hostname(hostname);
    crate::improv::set_hostname(hostname);
    if let Ok(mut s) = status.lock() {
        s.hostname = hostname.to_string();
    }
    info!("[net] renamed to {hostname}");
}

/// The name the mDNS responder was last given successfully, and when to
/// try again if that is not the hostname: naming can fail on a short heap
/// (`mdns_hostname_set`'s allocations, a full action queue), and the
/// responder would otherwise answer to the old name until a reboot while
/// a repeated save changes nothing.
#[derive(Default)]
struct MdnsName {
    named: String,
    retry_at: Option<Instant>,
}

impl MdnsName {
    /// Name the responder `hostname` if it is not (now, or when the retry
    /// is due); the first failure of a run of them warns.
    fn sync(&mut self, mdns: &mut EspMdns, hostname: &str) {
        if self.named == hostname || self.retry_at.is_some_and(|t| Instant::now() < t) {
            return;
        }
        if name_mdns(mdns, hostname) {
            info!("[net] mDNS responding as {hostname}.local");
            self.named = hostname.to_string();
            self.retry_at = None;
        } else {
            if self.retry_at.is_none() {
                warn!(
                    "[net] mDNS could not take the name {hostname}; retrying every {} s",
                    config::MDNS_RETRY.as_secs()
                );
            }
            self.retry_at = Some(Instant::now() + config::MDNS_RETRY);
        }
    }
}

/// The setup network's captive-portal DNS (AP mode only). Kept alive by
/// the caller.
fn start_portal_dns(wifi: &WifiManager) -> Option<DnsServer> {
    wifi.ap_ip_bytes()
        .and_then(|ip| match DnsServer::start(ip) {
            Ok(d) => Some(d),
            Err(e) => {
                warn!("[net] captive portal DNS failed: {e:#}");
                None
            }
        })
}

/// Save WiFi details the way `PUT /api/config` does: the whole settings,
/// changed, through the main task (`Command::ApplyConfig`), which writes
/// NVS.
fn save_wifi(
    settings: &Arc<Mutex<Settings>>,
    cmd: &mpsc::Sender<Command>,
    ssid: &str,
    password: &str,
) -> Result<(), String> {
    let mut next = settings.lock().map_err(|_| "settings poisoned")?.clone();
    next.wifi_ssid = ssid.to_string();
    next.wifi_pass = password.to_string();
    let (tx, rx) = mpsc::channel();
    cmd.send(Command::ApplyConfig {
        settings: Box::new(next),
        panel: None,
        reply: tx,
    })
    .map_err(|_| "player not running")?;
    let answer = rx.recv_timeout(config::IMPROV_SAVE_TIMEOUT);
    crate::state::drop_spent();
    match answer {
        Ok(Ok(_)) => {
            // The shared copy as the main task now has it (a later PUT
            // starts from it and would otherwise put the old WiFi back).
            if let Ok(mut s) = settings.lock() {
                s.wifi_ssid = ssid.to_string();
                s.wifi_pass = password.to_string();
            }
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("player did not answer".into()),
    }
}

/// What the worker keeps past start-up: shared by the loop in [`run`] and
/// the long, rare steps it calls ([`Net::improv_provision`],
/// [`Net::reconnect`]), which are kept out of `run`'s own frame. The loop's
/// frame is under everything the worker does, the firmware signature check
/// the deepest of it (~6.5 KB of curve25519 frames: `ota::verify_signature`).
struct Net {
    wifi: WifiManager,
    /// The credentials in use: replaced by WiFi details joined over USB.
    ssid: String,
    password: String,
    hostname: String,
    status: Arc<Mutex<Status>>,
    save_settings: Arc<Mutex<Settings>>,
    save_cmd: mpsc::Sender<Command>,
    /// For the slow-clock record: the WiFi driver takes the partition.
    sd_nvs: EspNvsPartition<NvsDefault>,
    ap_started_at: Option<Instant>,
    /// The boot decision (saved network, or the setup AP after
    /// STA_MAX_FAILURES) when the browser installer interrupted it: the
    /// failures so far, finished by the steady state's reconnects.
    boot_failures: Option<u32>,
    mdns_name: MdnsName,
    /// Kept alive for the life of the worker: dropping it unregisters.
    mdns: Option<EspMdns>,
    _sntp: Option<esp_idf_svc::sntp::EspSntp<'static>>,
    _dns: Option<DnsServer>,
    /// The last scan, for the installer's rescans (`IMPROV_SCAN_FRESH`).
    last_scan: Option<(Instant, Vec<crate::wifi::ScannedAp>)>,
}

#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
#[inline(never)]
fn run(worker: NetWorker) {
    let NetWorker {
        modem,
        sysloop,
        nvs,
        ssid,
        password,
        hostname,
        status,
        rx,
        settings,
        cmd,
    } = worker;
    let (save_settings, save_cmd) = (settings.clone(), cmd.clone());
    let mut follow = crate::follow::Poller::new(settings, status.clone(), cmd);
    let Some(mut net) = start(modem, sysloop, nvs, ssid, password, hostname, status, save_settings, save_cmd) else {
        return;
    };

    // --- Steady state ------------------------------------------------------
    loop {
        // Commands from the HTTP handlers. `recv_timeout` doubles as the poll
        // interval, so a scan is never left waiting for a sleep to finish.
        let got = rx.recv_timeout(follow.wait().min(config::WIFI_POLL_INTERVAL));
        crate::improv::log_events();
        sync_sd_slow(&net.sd_nvs);
        if matches!(got, Ok(NetCommand::Improv(_))) {
            crate::improv::taken();
        }
        match got {
            Ok(NetCommand::Scan(reply)) => {
                let aps = net.wifi.scan_networks().unwrap_or_else(|e| {
                    warn!("[net] scan failed: {e:#}");
                    Vec::new()
                });
                net.last_scan = Some((Instant::now(), aps.clone()));
                let _ = reply.send(aps);
            }
            // The playback-cache count (`cache.rs`): a directory walk and a
            // manifest pass, seconds long, done here so neither the display
            // task nor the httpd task waits on it. A scan request that
            // arrives meanwhile waits too; that is the trade.
            Ok(NetCommand::CountCache) => crate::cache::run_count(),
            Ok(NetCommand::FollowTest { host, code, reply }) => {
                let _ = reply.send(follow.test(&host, code.as_deref()));
            }
            Ok(NetCommand::VerifyBundle { header, reply }) => {
                let _ = reply.send(crate::ota::serve_verify(&header));
            }
            Ok(NetCommand::NameCheck { name, until, reply }) => {
                // Not sent if it would end after the handler stopped
                // waiting (it waited behind a connect or a count).
                let check = if Instant::now() + config::NAME_QUERY > until {
                    opendmd_core::hostname::Check::Unknown
                } else if net.mdns.is_some()
                    && net.wifi.state() == WifiState::Connected
                    && net.wifi.is_associated()
                {
                    name_check(&name, net.wifi.ip_address().as_deref())
                } else {
                    // The setup network, or between networks: no one to ask.
                    opendmd_core::hostname::Check::Unknown
                };
                let _ = reply.send(check);
            }
            Ok(NetCommand::Rename(name)) => {
                if name != net.hostname {
                    rename(&net.status, &name);
                    net.hostname = name;
                    // Named below, now: not after a failure's wait.
                    net.mdns_name.retry_at = None;
                }
            }
            // Improv Serial (`improv.rs`): the answers go straight back to
            // the USB host through the log ring.
            Ok(NetCommand::Improv(Job::Scan { at })) => {
                if at.elapsed() >= config::IMPROV_STALE {
                    // The installer stopped waiting while we were busy.
                    crate::improv::send_scan(&[]);
                } else if let Some((_, aps)) = net.last_scan
                    .as_ref()
                    .filter(|(t, _)| t.elapsed() < config::IMPROV_SCAN_FRESH)
                {
                    crate::improv::send_scan(aps);
                } else {
                    let aps = net.wifi.scan_networks().unwrap_or_else(|e| {
                        warn!("[net] improv scan failed: {e:#}");
                        Vec::new()
                    });
                    crate::improv::send_scan(&aps);
                    net.last_scan = Some((Instant::now(), aps));
                }
            }
            Ok(NetCommand::Improv(Job::Provision {
                ssid: new_ssid,
                password: new_password,
                at,
                stamp,
            })) => net.improv_provision(new_ssid, new_password, at, stamp),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                info!("[net] command channel closed; worker exiting");
                return;
            }
        }

        crate::clock::log_new_syncs();
        crate::diag::check("net-worker: loop");

        // The name mDNS answers to: it takes `<name>-2` if another device
        // has the name, at start-up or later (a conflict re-probes).
        if let Some(m) = net.mdns.as_mut() {
            net.mdns_name.sync(m, &net.hostname);
            publish_mdns(&net.status);
        }

        // Follow RePlayOS: a poll when one is due (station mode only).
        follow.tick(net.wifi.state() == WifiState::Connected && net.wifi.is_associated());

        // A cache count put off for a transfer, once it is over.
        crate::cache::tick();

        // The FAT index (`fatidx`): built or rebuilt when due.
        let before = crate::state::free_heap();
        crate::readahead::tick();
        crate::heapmap::cp_if_moved("read-ahead first took heap", before);
        let before = crate::state::free_heap();
        crate::fatidx::tick();
        crate::heapmap::cp_if_moved("fat index tick first took heap", before);
        crate::netstat::tick();
        #[cfg(feature = "emu-census")]
        census();

        // An open AP does not stay up indefinitely. Rebooting (rather than
        // dropping back to STA in place) re-runs the whole boot decision with
        // whatever credentials were just saved.
        if let Some(started) = net.ap_started_at {
            if started.elapsed() >= config::AP_TIMEOUT {
                info!("[net] AP timeout reached; rebooting");
                crate::http::reboot_after_response(opendmd_core::restarts::Reason::ApTimeout);
            }
        }

        net.reconnect();
    }
}


/// Start-up: the radio, the boot decision (the saved network, or the setup
/// AP), mDNS, SNTP and the portal DNS. Out of `run` so its frames are not
/// under the steady state's. `None`: no WiFi this boot.
#[allow(clippy::too_many_arguments)]
#[optimize(size)]
#[inline(never)]
fn start(
    modem: Modem,
    sysloop: EspSystemEventLoop,
    nvs: EspNvsPartition<NvsDefault>,
    ssid: String,
    password: String,
    hostname: String,
    status: Arc<Mutex<Status>>,
    save_settings: Arc<Mutex<Settings>>,
    save_cmd: mpsc::Sender<Command>,
) -> Option<Net> {
    info!("[net] worker started on CPU0");
    crate::heapmap::cp("net-worker running");

    // For the slow-clock record (below): the WiFi driver takes `nvs`.
    let sd_nvs = nvs.clone();
    let mut wifi = match WifiManager::new(modem, sysloop, nvs, &hostname) {
        Ok(w) => w,
        Err(e) => {
            error!("[net] WiFi init failed ({e:#}); networking is off for this boot");
            return None;
        }
    };
    crate::heapmap::cp("wifi driver init (esp_wifi_init)");

    // --- Associate, or raise the setup AP ---------------------------------
    let mut failures = 0u32;
    let mut ap_started_at: Option<Instant> = None;
    // The boot decision (saved network, or the setup AP after
    // STA_MAX_FAILURES) when the browser installer interrupted it: the
    // failures so far, finished by the steady state's reconnects.
    let mut boot_failures: Option<u32> = None;

    if ssid.is_empty() {
        info!("[net] no saved credentials; starting the setup AP");
    } else {
        while failures < config::STA_MAX_FAILURES {
            match wifi.connect_sta(&ssid, &password, None) {
                Ok(true) => break,
                // Gave way to the installer (`improv::pending`): its job
                // first, then the loop below carries on from here.
                Ok(false) if crate::improv::pending() => {
                    boot_failures = Some(failures);
                    break;
                }
                Ok(false) => failures += 1,
                Err(e) => {
                    warn!("[net] connect error: {e:#}");
                    failures += 1;
                }
            }
        }
    }

    if wifi.state() != WifiState::Connected && boot_failures.is_none() {
        match wifi.start_ap() {
            Ok(()) => ap_started_at = Some(Instant::now()),
            // Do not return: dropping `wifi` here would deinit the radio for
            // the rest of the run. Fall through to the loop, which keeps
            // retrying the association.
            Err(e) => error!("[net] AP mode failed too ({e:#}); staying down"),
        }
    }

    let ip = wifi.ip_address();
    publish(&status, wifi.state(), ip.clone());
    crate::heapmap::cp(match wifi.state() {
        WifiState::Connected => "wifi connected (sta)",
        WifiState::ApMode => "wifi setup ap up",
        _ => "wifi not connected",
    });

    // --- mDNS, STA only ----------------------------------------------------
    // Kept alive for the life of the worker: dropping it unregisters.
    let mut mdns_name = MdnsName::default();
    let mdns = if wifi.state() == WifiState::Connected {
        start_mdns(&hostname, &mut mdns_name.named)
    } else {
        None
    };

    // --- SNTP, STA only -----------------------------------------------------
    // Wall-clock time for the schedule (`clock.rs`); kept alive for the life
    // of the worker. The pool is reached through the station's DNS.
    let mut _sntp = if wifi.state() == WifiState::Connected {
        crate::clock::start_sntp()
    } else {
        None
    };

    // --- Captive portal DNS, AP only --------------------------------------
    let mut _dns = if wifi.state() == WifiState::ApMode {
        start_portal_dns(&wifi)
    } else {
        None
    };
    crate::heapmap::cp("mdns, sntp, portal dns");
    // The last scan, for the installer's rescans (`IMPROV_SCAN_FRESH`).
    let last_scan: Option<(Instant, Vec<crate::wifi::ScannedAp>)> = None;

    // The HTTP server is already up: the boot task starts it before the card,
    // while the heap is still whole enough for its task stack (see
    // `http::start`). All this thread does is tell the world where to find it.
    match wifi.state() {
        WifiState::Connected => info!(
            "[net] ready: http://{}/ and http://{hostname}.local/",
            ip.clone().unwrap_or_default()
        ),
        WifiState::ApMode => info!(
            "[net] ready: AP '{}' (open), http://{}/",
            config::AP_SSID,
            ip.clone().unwrap_or_default()
        ),
        WifiState::Disconnected if boot_failures.is_some() => {
            info!("[net] connecting paused for the browser installer")
        }
        WifiState::Disconnected => warn!("[net] no network; the HTTP server is unreachable"),
    }

    Some(Net {
        wifi,
        ssid,
        password,
        hostname,
        status,
        save_settings,
        save_cmd,
        sd_nvs,
        ap_started_at,
        boot_failures,
        mdns_name,
        mdns,
        _sntp,
        _dns,
        last_scan,
    })
}

impl Net {
    /// WiFi details sent over USB by a browser installer: joined, saved
    /// once they work, and what a boot onto the network brings up.
    #[optimize(size)]
    #[inline(never)]
    fn improv_provision(&mut self, new_ssid: String, new_password: String, at: Instant, stamp: u32) {
        info!("[net] improv: WiFi details for '{new_ssid}' received over USB");
        if at.elapsed() >= config::IMPROV_STALE {
            warn!("[net] improv: WiFi details for '{new_ssid}' waited too long; not tried");
            crate::improv::provision_failed(
                stamp,
                opendmd_core::improv::Error::UnableToConnect,
            );
        } else {
            let was_ap = self.wifi.state() == WifiState::ApMode;
            let deadline = at + config::IMPROV_ANSWER_WITHIN;
            info!("[net] improv: joining '{new_ssid}'");
            let joined = matches!(
                self.wifi.connect_sta(&new_ssid, &new_password, Some(deadline)),
                Ok(true)
            );
            let saved = joined
                && match save_wifi(&self.save_settings, &self.save_cmd, &new_ssid, &new_password) {
                    Ok(()) => true,
                    Err(e) => {
                        error!(
                            "[net] improv: joined '{new_ssid}' but could not save it: {e}"
                        );
                        false
                    }
                };
            if joined {
                // On the network whatever the save did: the old one is
                // gone, so this is the network to stay on until reboot.
                self.ssid = new_ssid;
                self.password = new_password;
                self.ap_started_at = None;
                self.boot_failures = None;
                self._dns = None;
                let ip = self.wifi.ip_address();
                publish(&self.status, WifiState::Connected, ip.clone());
                if self.mdns.is_none() {
                    self.mdns = start_mdns(&self.hostname, &mut self.mdns_name.named);
                }
                if self._sntp.is_none() {
                    self._sntp = crate::clock::start_sntp();
                }
                let ip = ip.unwrap_or_default();
                info!(
                    "[net] improv: on '{}' at http://{ip}/ and http://{}.local/",
                    self.ssid, self.hostname
                );
                match (saved, crate::improv::parse_ipv4(&ip)) {
                    (true, Some(octets)) => crate::improv::provisioned(stamp, octets),
                    _ => crate::improv::provision_failed(
                        stamp,
                        opendmd_core::improv::Error::Unknown,
                    ),
                }
            } else {
                warn!("[net] improv: could not join '{new_ssid}'");
                crate::improv::provision_failed(
                    stamp,
                    opendmd_core::improv::Error::UnableToConnect,
                );
                if was_ap {
                    // Back to the setup network (its DNS server is
                    // still running; the reboot timer carries on).
                    match self.wifi.start_ap() {
                        Ok(()) => publish(&self.status, WifiState::ApMode, self.wifi.ip_address()),
                        Err(e) => {
                            error!("[net] setup network did not come back ({e:#})");
                            publish(&self.status, WifiState::Disconnected, None);
                        }
                    }
                } else {
                    // The reconnect below goes back to the saved network.
                    publish(&self.status, WifiState::Disconnected, None);
                }
            }
        }
    }

    /// Reconnect if the association dropped, or was never made.
    #[optimize(size)]
    #[inline(never)]
    fn reconnect(&mut self) {
        let lost = self.wifi.state() == WifiState::Connected && !self.wifi.is_associated();
        let never = self.wifi.state() == WifiState::Disconnected && !self.ssid.is_empty();
        // Not while installer work is queued: that comes first.
        // Nor while the installer's WiFi form is open (it rescans every
        // 3 s; a connect would hold the radio between its scans).
        let form_open = self.last_scan
            .as_ref()
            .is_some_and(|(t, _)| t.elapsed() < config::IMPROV_SCAN_FRESH);
        if (lost || never) && !crate::improv::pending() && !form_open {
            warn!("[net] WiFi dropped; reconnecting");
            publish(&self.status, WifiState::Disconnected, None);
            match self.wifi.connect_sta(&self.ssid, &self.password, None) {
                Ok(true) => {
                    let ip = self.wifi.ip_address();
                    info!("[net] reconnected, IP {}", ip.clone().unwrap_or_default());
                    publish(&self.status, WifiState::Connected, ip);
                    self.boot_failures = None;
                    if self.mdns.is_none() {
                        self.mdns = start_mdns(&self.hostname, &mut self.mdns_name.named);
                    }
                    if self._sntp.is_none() {
                        self._sntp = crate::clock::start_sntp();
                    }
                }
                // Gave way to the installer: its job is next round.
                Ok(false) if crate::improv::pending() => {}
                Ok(false) | Err(_) if self.boot_failures.is_some() => {
                    // Still the boot decision: the setup AP after
                    // STA_MAX_FAILURES, as at boot.
                    let n = self.boot_failures.unwrap_or(0) + 1;
                    self.boot_failures = Some(n);
                    if n >= config::STA_MAX_FAILURES {
                        self.boot_failures = None;
                        match self.wifi.start_ap() {
                            Ok(()) => {
                                self.ap_started_at = Some(Instant::now());
                                self._dns = start_portal_dns(&self.wifi);
                                publish(&self.status, WifiState::ApMode, self.wifi.ip_address());
                            }
                            Err(e) => error!("[net] AP mode failed ({e:#}); staying down"),
                        }
                    }
                }
                Ok(false) => warn!("[net] reconnect failed; will retry"),
                Err(e) => warn!("[net] reconnect error: {e:#}"),
            }
        }
    }
}

/// Does another device answer to `<name>.local`? One mDNS A query
/// (`mdns_query`, at most [`config::NAME_QUERY`], ending at the first
/// answer). The responder drops packets from our own address, and an
/// answer from `own_ip` would not count anyway. A query that fails
/// (`ESP_ERR_NO_MEM`, …) is `Unknown`, not `Free`. Transient heap only:
/// the search and its result (~0.3 KB).
fn name_check(name: &str, own_ip: Option<&str>) -> opendmd_core::hostname::Check {
    use opendmd_core::hostname::Check;
    use std::ptr;
    let Ok(cname) = std::ffi::CString::new(name) else {
        return Check::Unknown;
    };
    let mut results: *mut esp_idf_sys::mdns_result_t = ptr::null_mut();
    // SAFETY: `cname` outlives the call, which blocks until the search ends;
    // `results` is freed below with the component's own function.
    let err = unsafe {
        esp_idf_sys::mdns_query(
            cname.as_ptr(),
            ptr::null(),
            ptr::null(),
            esp_idf_sys::MDNS_TYPE_A as u16,
            config::NAME_QUERY.as_millis() as u32,
            1,
            &mut results,
        )
    };
    if err != esp_idf_sys::ESP_OK {
        warn!("[net] name check for {name}.local failed ({err})");
        return Check::Unknown;
    }
    let mut answers: Vec<[u8; 4]> = Vec::new();
    let mut r = results;
    // SAFETY: a list `mdns_query` built and nothing else holds; read, then
    // freed once.
    unsafe {
        while !r.is_null() {
            let mut a = (*r).addr;
            while !a.is_null() {
                if u32::from((*a).addr.type_) == esp_idf_sys::ESP_IPADDR_TYPE_V4 {
                    answers.push((*a).addr.u_addr.ip4.addr.to_ne_bytes());
                }
                a = (*a).next;
            }
            r = (*r).next;
        }
        esp_idf_sys::mdns_query_results_free(results);
    }
    let check = Check::from_answers(&answers, own_ip.and_then(crate::improv::parse_ipv4));
    match &check {
        Check::Taken(by) => info!("[net] name check: {name}.local answers from {by}"),
        _ => info!("[net] name check: {name}.local is free"),
    }
    check
}

/// Copy the hostname mDNS is answering to into the status when it changed
/// (`mdns_hostname_get` reads the responder's own copy, which its conflict
/// handling renames; a lock and a copy, once a second).
#[inline(never)]
fn publish_mdns(status: &Arc<Mutex<Status>>) {
    // MDNS_NAME_BUF_LEN is 65, or 69 with IPv6 reverse lookups.
    let mut buf = [0u8; 72];
    // SAFETY: `buf` holds at least MDNS_NAME_BUF_LEN bytes, as the call requires.
    if unsafe { esp_idf_sys::mdns_hostname_get(buf.as_mut_ptr().cast()) } != esp_idf_sys::ESP_OK {
        return;
    }
    let name = std::ffi::CStr::from_bytes_until_nul(&buf)
        .map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Ok(mut s) = status.lock() {
        if s.mdns != name {
            // Not the configured name: another device has it. (The
            // configured name itself is logged by `start_mdns`/`rename`.)
            if name != s.hostname {
                warn!(
                    "[net] mDNS name now {name}.local (configured {})",
                    s.hostname
                );
            }
            s.mdns = name;
        }
    }
}

/// The card's slow-clock record changed this run (`cardspeed` slowed the
/// card down in place, or "try full speed again"): NVS follows, here on
/// CPU0 rather than on the display task (polled at least once a second).
/// RTC memory holds it meanwhile, and until a power cycle if this fails.
#[inline(never)]
fn sync_sd_slow(nvs: &EspNvsPartition<NvsDefault>) {
    let Some(record) = crate::sdhealth::take_nvs_sync() else {
        return;
    };
    if let Err(e) = crate::nvs_config::Config::new(nvs.clone()).and_then(|mut c| c.set_sd_slow(record)) {
        error!("[net] sd_slow: {e:#}");
    }
}

#[cfg(feature = "emu-census")]
extern "C" {
    /// `components/hub75_shim/netdiag.c`: logs `netdiag: sum: … N rx pbufs
    /// held` (and the pcbs and sockets holding them, when any are): the
    /// frames lwIP keeps in the WiFi driver's receive buffers.
    fn opendmd_netdiag();
    /// Sockets with a connection (a count of the socket table, no walk).
    fn opendmd_netdiag_socks() -> i32;
    /// Post a count of what lwIP holds to its thread (`netdiag_snap`).
    fn opendmd_netdiag_snap();
    static netdiag_snap: [i32; 6];
}

/// `emu-census` builds: the low watch, a task on CPU0 that reads the free
/// heap every millisecond and keeps the least of each 5 s window, with the
/// Rust allocator's bytes and the sockets open at that moment; `census`
/// logs it (`lowwatch:`). The API lane reads it beside the bursts.
#[cfg(feature = "emu-census")]
mod lowwatch {
    use std::sync::atomic::{AtomicI32, AtomicU32, Ordering::Relaxed};
    pub static LOW: AtomicU32 = AtomicU32::new(u32::MAX);
    pub static RUST: AtomicI32 = AtomicI32::new(0);
    pub static SOCKS: AtomicI32 = AtomicI32::new(0);
    pub static UPLOADS: AtomicU32 = AtomicU32::new(0);
    pub static PHASE: AtomicU32 = AtomicU32::new(0);
    static SNAP_DUE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static SNAP: [AtomicI32; 6] = [const { AtomicI32::new(0) }; 6];

    fn rust_live() -> i32 {
        #[cfg(feature = "psi")]
        return crate::arena::RUST_LIVE.load(Relaxed) as i32;
        #[cfg(not(feature = "psi"))]
        return 0;
    }

    pub fn start() {
        let _ = esp_idf_hal::task::thread::ThreadSpawnConfiguration {
            name: Some(b"lowwatch\0"),
            stack_size: 3072,
            priority: 6,
            pin_to_core: Some(esp_idf_hal::cpu::Core::Core0),
            ..Default::default()
        }
        .set();
        let _ = std::thread::Builder::new().stack_size(3072).spawn(|| loop {
            let f = crate::state::free_heap();
            if f < LOW.load(Relaxed) {
                LOW.store(f, Relaxed);
                RUST.store(rust_live(), Relaxed);
                // SAFETY: a count of lwIP's socket table.
                SOCKS.store(unsafe { super::opendmd_netdiag_socks() }, Relaxed);
                UPLOADS.store(crate::stream::uploads(), Relaxed);
                PHASE.store(crate::stream::phase(), Relaxed);
                // SAFETY: posts a count to lwIP's thread.
                unsafe { super::opendmd_netdiag_snap() };
                SNAP_DUE.store(true, Relaxed);
            } else if SNAP_DUE.swap(false, Relaxed) {
                // The count posted at the low ran meanwhile (the tcpip thread
                // is above this one): keep it with the low.
                for (k, v) in SNAP.iter().enumerate() {
                    // SAFETY: plain reads of C ints written by the snapshot.
                    v.store(unsafe { std::ptr::read_volatile(&super::netdiag_snap[k]) }, Relaxed);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        });
        let _ = esp_idf_hal::task::thread::ThreadSpawnConfiguration::default().set();
    }

    /// The window's line, and a new window.
    pub fn take() -> String {
        let low = LOW.swap(u32::MAX, Relaxed);
        format!(
            "lowwatch: window low {low} B free (rust {} B, sockets {}, uploads {}); now {} B free, rust {} B; at it: phase {:#x}, lwip rx {} pbufs {} B, tx {} B, pcbs {} tw {} socks {}",
            RUST.load(Relaxed),
            SOCKS.load(Relaxed),
            UPLOADS.load(Relaxed),
            crate::state::free_heap(),
            rust_live(),
            PHASE.load(Relaxed),
            SNAP[0].load(Relaxed),
            SNAP[1].load(Relaxed),
            SNAP[2].load(Relaxed),
            SNAP[3].load(Relaxed),
            SNAP[4].load(Relaxed),
            SNAP[5].load(Relaxed),
        )
    }
}

/// The emulator's leak lane (feature `emu-census`): the heap's blocks in
/// use and bytes, every 20 s (`run.py leak` counts the GIFs between). A walk of the
/// heap (here on CPU0, never the display core). And every 5 s, lwIP's
/// received frames held (`opendmd_netdiag`, for the API lane's bursts).
#[cfg(feature = "emu-census")]
fn census() {
    static NET_LAST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    // SAFETY: a plain IDF call.
    let now = (unsafe { esp_idf_sys::esp_timer_get_time() } / 1000) as u32;
    if now.wrapping_sub(NET_LAST.load(std::sync::atomic::Ordering::Relaxed)) >= 5_000 {
        if NET_LAST.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            lowwatch::start();
        } else {
            log::info!("{}", lowwatch::take());
        }
        NET_LAST.store(now, std::sync::atomic::Ordering::Relaxed);
        // SAFETY: posts a walk to lwIP's own thread; no arguments.
        unsafe { opendmd_netdiag() };
    }
    static LAST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let last = LAST.load(std::sync::atomic::Ordering::Relaxed);
    if now.wrapping_sub(last) < 20_000 {
        return;
    }
    LAST.store(now, std::sync::atomic::Ordering::Relaxed);
    let mut info: esp_idf_sys::multi_heap_info_t = unsafe { std::mem::zeroed() };
    unsafe { esp_idf_sys::heap_caps_get_info(&mut info, esp_idf_sys::MALLOC_CAP_8BIT) };
    log::info!(
        "census: {} blocks in use, {} B allocated, {} B free, largest {} B",
        info.allocated_blocks,
        info.total_allocated_bytes,
        info.total_free_bytes,
        info.largest_free_block
    );
}
