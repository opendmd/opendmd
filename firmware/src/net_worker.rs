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

use esp_idf_hal::cpu::Core;
use esp_idf_hal::modem::Modem;
use esp_idf_hal::task::thread::ThreadSpawnConfiguration;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::mdns::EspMdns;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use log::{error, info, warn};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::config;
use crate::dns_server::DnsServer;
use crate::http::NetCommand;
use crate::state::Status;
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
}

/// Spawn the worker on CPU0 and return immediately.
///
/// `ThreadSpawnConfiguration` is process-wide state consulted by the next
/// `thread::spawn`, so it is set, used, and put back before anything else can
/// spawn a thread — the display loop has not started yet at this point.
pub fn spawn(worker: NetWorker) -> anyhow::Result<()> {
    let previous = ThreadSpawnConfiguration::get();
    ThreadSpawnConfiguration {
        name: Some(b"net-worker\0"),
        stack_size: config::NET_WORKER_STACK,
        priority: 5,
        pin_to_core: Some(Core::Core0),
        ..Default::default()
    }
    .set()?;

    let handle = std::thread::Builder::new()
        .name("net-worker".into())
        .stack_size(config::NET_WORKER_STACK)
        .spawn(move || run(worker));

    // Restore the inherited configuration whatever happened, so nothing else
    // accidentally lands on CPU0 with these settings.
    if let Some(previous) = previous {
        let _ = previous.set();
    } else {
        let _ = ThreadSpawnConfiguration::default().set();
    }
    handle?;
    Ok(())
}

fn publish(status: &Arc<Mutex<Status>>, state: WifiState, ip: Option<String>) {
    if let Ok(mut s) = status.lock() {
        s.wifi = state.as_str();
        s.ip = ip.unwrap_or_default();
    }
}

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
    } = worker;

    info!("[net] worker started on CPU0");

    let mut wifi = match WifiManager::new(modem, sysloop, nvs, &hostname) {
        Ok(w) => w,
        Err(e) => {
            error!("[net] WiFi init failed ({e:#}); networking is off for this boot");
            return;
        }
    };

    // --- Associate, or raise the setup AP ---------------------------------
    let mut failures = 0u32;
    let mut ap_started_at: Option<Instant> = None;

    if ssid.is_empty() {
        info!("[net] no saved credentials; starting the setup AP");
    } else {
        while failures < config::STA_MAX_FAILURES {
            match wifi.connect_sta(&ssid, &password) {
                Ok(true) => break,
                Ok(false) => failures += 1,
                Err(e) => {
                    warn!("[net] connect error: {e:#}");
                    failures += 1;
                }
            }
        }
    }

    if wifi.state() != WifiState::Connected {
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

    // --- mDNS, STA only ----------------------------------------------------
    // Kept alive for the life of the worker: dropping it unregisters.
    let _mdns = if wifi.state() == WifiState::Connected {
        match EspMdns::take() {
            Ok(mut mdns) => {
                let ok = mdns.set_hostname(&hostname).is_ok()
                    && mdns.set_instance_name("OpenDMD").is_ok()
                    && mdns.add_service(None, "_http", "_tcp", 80, &[]).is_ok();
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
    } else {
        None
    };

    // --- Captive portal DNS, AP only --------------------------------------
    let _dns = if wifi.state() == WifiState::ApMode {
        wifi.ap_ip_bytes()
            .and_then(|ip| match DnsServer::start(ip) {
                Ok(d) => Some(d),
                Err(e) => {
                    warn!("[net] captive portal DNS failed: {e:#}");
                    None
                }
            })
    } else {
        None
    };

    // The HTTP server is already up: `main` starts it before the display loop,
    // while the heap is still whole enough for its 16 KB task stack (see
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
        WifiState::Disconnected => warn!("[net] no network; the HTTP server is unreachable"),
    }

    // --- Steady state ------------------------------------------------------
    loop {
        // Commands from the HTTP handlers. `recv_timeout` doubles as the poll
        // interval, so a scan is never left waiting for a sleep to finish.
        match rx.recv_timeout(config::WIFI_POLL_INTERVAL) {
            Ok(NetCommand::Scan(reply)) => {
                let aps = wifi.scan_networks().unwrap_or_else(|e| {
                    warn!("[net] scan failed: {e:#}");
                    Vec::new()
                });
                let _ = reply.send(aps);
            }
            // The playback-cache count (`cache.rs`): a directory walk and a
            // manifest pass, seconds long, done here so neither the display
            // task nor the httpd task waits on it. A scan request that
            // arrives meanwhile waits too; that is the trade.
            Ok(NetCommand::CountCache) => crate::cache::run_count(),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                info!("[net] command channel closed; worker exiting");
                return;
            }
        }

        // An open AP does not stay up indefinitely. Rebooting (rather than
        // dropping back to STA in place) re-runs the whole boot decision with
        // whatever credentials were just saved.
        if let Some(started) = ap_started_at {
            if started.elapsed() >= config::AP_TIMEOUT {
                info!("[net] AP timeout reached; rebooting");
                crate::ota::reboot_after_response();
            }
        }

        // Reconnect if the association dropped, or was never made.
        let lost = wifi.state() == WifiState::Connected && !wifi.is_associated();
        let never = wifi.state() == WifiState::Disconnected && !ssid.is_empty();
        if lost || never {
            warn!("[net] WiFi dropped; reconnecting");
            publish(&status, WifiState::Disconnected, None);
            match wifi.connect_sta(&ssid, &password) {
                Ok(true) => {
                    let ip = wifi.ip_address();
                    info!("[net] reconnected, IP {}", ip.clone().unwrap_or_default());
                    publish(&status, WifiState::Connected, ip);
                }
                Ok(false) => warn!("[net] reconnect failed; will retry"),
                Err(e) => warn!("[net] reconnect error: {e:#}"),
            }
        }
    }
}
