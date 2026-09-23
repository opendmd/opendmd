//! OpenDMD firmware — phase 3a: a networked player.
//!
//! Boot -> HUB75 panel up -> mount the microSD -> read settings from NVS ->
//! build a seekable index over the manifest -> start the network worker ->
//! run `opendmd_core::engine::Engine`, pushing every frame to two chained
//! 64x32 HUB75 P4 panels (128x32) over the mrcodetastic DMA driver.
//!
//! Neither the manifest nor its index is held in RAM. One streaming pass over
//! the card's 450 KB / 10,162-line `filelist.txt` writes a `u32` byte offset
//! per matching line to `filelist.txt.idx` beside it (~40 KB unfiltered), and
//! `IndexedCatalog` seeks into both files for one `Entry` at a time, only when
//! the engine is about to open the next GIF. At boot a current `.idx` is simply
//! reopened; see [`open_catalog`].
//!
//! # Threads and cores
//!
//! | task | core | what it does |
//! |---|---|---|
//! | main | CPU1 | panel, SD, engine, this loop. Never touches WiFi or a socket. |
//! | `net-worker` | CPU0 | WiFi (connect, reconnect, AP fallback), mDNS, starts the httpd |
//! | `httpd` | CPU0 | every HTTP handler; the only task allowed to block |
//! | `dns-server` | any | captive-portal DNS, AP mode only |
//!
//! The panel is initialised here, on CPU1, so the I2S-DMA interrupt is
//! installed on CPU1 and never competes with WiFi's on CPU0. The two sides meet
//! only through [`state::Status`] (published here, read there) and
//! [`state::Command`] (sent there, applied here, between frames).
//!
//! # The numbers this phase is judged on
//!
//! The heartbeat and `/api/status` both carry the frame-deadline miss count,
//! the worst lateness seen, and the sampled heap floor. If serving files costs
//! the panel anything, those are where it shows.

mod cache;
mod clock;
mod config;
mod dns_server;
mod files;
mod http;
mod httpd;
mod net_worker;
mod nvs_config;
mod ota;
mod panel;
mod probe;
mod sched;
mod sd;
mod state;
mod wifi;

use anyhow::{Context, Result};
use esp_idf_hal::gpio::{AnyIOPin, AnyOutputPin, Pin};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use opendmd_core::canvas::Canvas;
use opendmd_core::engine::{Catalog, DwellPolicy, Engine, FileCatalog, TickOutcome};
use opendmd_core::filter::Filter;
use opendmd_core::index::{write_index_with_overrides, IndexHeader, IndexedCatalog};
use opendmd_core::overrides::{self, OverrideSet, Overrides, NO_META};
use opendmd_core::schedule::{self, Decision, Schedule};
use opendmd_core::scheduler::{Mode, Scheduler};
use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::http::{Ctx, NetCommand};
use crate::nvs_config::{Config, Settings};
use crate::panel::{Panel, PanelConfig};
use crate::sd::{SdBus, SdSource};
use crate::state::{free_heap, Applied, Command, IndexSource, PlayError, Status};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// True until the network worker is spawned: the heap is still whole, so the
/// in-RAM fallback index may simply be attempted (it always fitted at boot).
/// After that it is only attempted if the block it needs is really there.
static HEAP_FRESH: AtomicBool = AtomicBool::new(true);
/// Set by [`rebuild`] when the only way to get a playlist is a reboot (see
/// [`NeedsFreshHeap`]); acted on by the main loop *after* it has answered the
/// command that caused it, so the HTTP caller gets its reply first.
static REBOOT_PENDING: AtomicBool = AtomicBool::new(false);

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    // Two version strings on purpose: the crate's, and the one the bootloader
    // recorded in `esp_app_desc_t` — the latter is what an OTA image is checked
    // against, so a mismatch is worth seeing at boot.
    log::info!(
        "OpenDMD firmware v{VERSION} ({}, app desc '{}') — 128x32 HUB75 GIF player",
        ota::FLASH_VARIANT,
        ota::running_version()
    );

    // Peripherals are taken exactly once and handed out from here; nothing
    // downstream calls `Peripherals::take()`.
    let peripherals = Peripherals::take().context("peripherals already taken")?;
    let pins = peripherals.pins;
    let modem = peripherals.modem;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs_partition = EspDefaultNvsPartition::take()?;

    // The panel goes up first, before the card is mounted and long before the
    // index is built: its DMA buffers need a large contiguous run of internal
    // DRAM, and a heap already churned by other work is exactly what produces
    // "!KABOOM! HUB75 memory allocation failed". Claim the hard allocation on a
    // virgin heap. Doing it here also puts the I2S interrupt on CPU1.
    let panel_cfg = PanelConfig::default();
    let mut panel = Panel::new(&panel_cfg)?;
    log::info!(
        "panel up: {}x{} ({} x {}x{}), driver={}, double_buff={}, hub75_refresh_rate={} Hz",
        panel_cfg.surface_width(),
        panel_cfg.height,
        panel_cfg.chain,
        panel_cfg.width,
        panel_cfg.height,
        panel_cfg.driver,
        panel_cfg.double_buffer,
        panel.refresh_rate(),
    );

    if probe::ENABLED {
        panel.bench();
    }

    // GPIO 2 (onboard LED) is deliberately untouched — see hardware/PINOUT.md.
    //
    // The GPIO peripherals are typed (`Gpio18` and friends), so the numbers in
    // `config` cannot be used to select them; `expect_pin` asserts the two
    // agree instead, which catches the map drifting out of step.
    let sck = expect_pin(AnyOutputPin::from(pins.gpio18), config::SD_SCK, "SD SCK");
    let mosi = expect_pin(AnyOutputPin::from(pins.gpio23), config::SD_MOSI, "SD MOSI");
    let miso = expect_pin(AnyIOPin::from(pins.gpio19), config::SD_MISO, "SD MISO");
    let cs = expect_pin(AnyOutputPin::from(pins.gpio5), config::SD_CS, "SD CS");
    let mut bus = SdBus::new(peripherals.spi2, sck, mosi, miso, cs)?;

    // A missing or unreadable card must not crash-loop the panel: log, wait,
    // try again, forever. `_card` unmounts on drop, so it lives to the end.
    let _card = loop {
        match bus.mount() {
            Ok(card) => {
                log::info!(
                    "microSD mounted at {} ({:.2} GB), heap {} B free",
                    config::SD_MOUNT,
                    card.capacity_bytes as f64 / 1e9,
                    free_heap()
                );
                break card;
            }
            Err(e) => {
                log::warn!(
                    "microSD mount failed ({e:#}); retrying in {}s",
                    config::RETRY_INTERVAL.as_secs()
                );
                thread::sleep(config::RETRY_INTERVAL);
            }
        }
    };

    // --- Settings ---------------------------------------------------------
    let mut nvs_config = Config::new(nvs_partition.clone())?;
    if nvs_config.seed_credentials_from_build() {
        log::info!("first boot with build-time credentials; they are now in NVS");
    }
    let mut settings = nvs_config.load();
    // A set whose manifest no longer opens (deleted off-device, another card,
    // or saved by a firmware that did not check) would leave the board with no
    // playlist, retrying forever across reboots. Forget it and play the
    // default instead.
    heal_missing_set(&mut settings, &mut nvs_config);
    // Prepared copies are found per GIF now; a playlist of `/odm/...` paths
    // from before that is dead weight. Say so once, and never play it.
    if File::open(format!(
        "{}/{}",
        config::SD_MOUNT,
        config::OBSOLETE_ODM_MANIFEST
    ))
    .is_ok()
    {
        log::warn!(
            "ignoring /{}: prepared copies are found automatically from /filelist.txt; it can be deleted",
            config::OBSOLETE_ODM_MANIFEST
        );
    }
    panel.set_brightness(settings.brightness);
    clock::set_timezone(&settings.tz, &settings.tz_name);
    let mut sched = Scheduling::new(&settings.schedule);
    log::info!(
        "settings: brightness={}, dwell={}..{}s, shuffle={}, platforms='{}', set='{}', tz='{}' ({}), schedule {} rule(s){}, host='{}', wifi={}",
        settings.brightness,
        settings.dwell_min,
        settings.dwell_max,
        settings.shuffle,
        settings.platforms,
        settings.set,
        settings.tz,
        settings.tz_name,
        sched.schedule.rules.len(),
        if sched.schedule.night.is_some() { " + night mode" } else { "" },
        settings.hostname,
        if settings.wifi_ssid.is_empty() {
            "unconfigured".to_string()
        } else {
            format!("'{}'", settings.wifi_ssid)
        }
    );

    let mut canvas = Canvas::new();
    // The engine's clock is a `Duration` since an epoch the caller picks, not
    // an `Instant` — `Instant` has no clock on wasm and the web app shares
    // this core. Boot is our epoch; only differences ever matter.
    let epoch = Instant::now();
    let mut last_heartbeat = Instant::now();
    // Our own heap floor, sampled at every phase boundary of the loop. Do not
    // use esp_get_minimum_free_heap_size() for this: it is the SUM of each
    // heap region's own minimum, which drifts ~30 KB below the true floor as
    // large blocks (the LZW decoder) land in different regions over time.
    let mut sampled_min = free_heap();
    let mut low_water = sampled_min;
    // The display's health, reported in the heartbeat and in /api/status.
    let mut deadline_misses: u64 = 0;
    let mut worst_lateness_ms: u64 = 0;
    let mut started: u64 = 0;
    // Log lines raised while a frame is being produced, printed after its
    // sleep (see step 3): a UART line costs ~87 µs a byte and the console
    // write blocks, so printing inside the frame would count as lateness.
    let mut deferred: Vec<Deferred> = Vec::with_capacity(2);
    // Whether this image has told the bootloader it works (see
    // `ota::confirm_running_image`): done once, after the first frame.
    let mut confirmed = false;

    // --- Shared state, the HTTP server, and the network worker ------------
    let status = Arc::new(Mutex::new(Status::new(
        VERSION,
        settings.hostname.clone(),
        epoch,
    )));
    {
        let mut s = status.lock().unwrap();
        s.sd_mounted = true;
        s.sd_capacity_bytes = _card.capacity_bytes;
        s.www_version = files::read_www_version();
        s.refresh_hz = panel.refresh_rate();
        s.tz_name = tz_label(&settings);
    }
    let shared_settings = Arc::new(Mutex::new(settings.clone()));
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let (net_tx, net_rx) = mpsc::channel::<NetCommand>();
    cache::init(net_tx.clone());

    // The playlist: the manifest and its on-card index, opened (or built) once
    // and kept open; `Exhausted` reseeds the scheduler rather than re-reading
    // either. Done before anything network-shaped exists, so that the
    // in-RAM fallback — the only path that still needs a large block — has a
    // fresh heap to work with. If the card has no readable manifest the loop
    // below keeps retrying.
    let mut engine: Option<Engine> = None;
    let _ = rebuild(&mut engine, &sched.effective(&settings), false, &status);

    // The HTTP server next, still before the first GIF is decoded: its task
    // stack is one contiguous 16 KB internal allocation, and the largest free
    // block while a GIF is playing is smaller than that. It is pinned to CPU0
    // by its own config, so starting it from this task is fine. Held for the
    // life of the program; dropping it would stop the httpd task.
    // lwip has to exist before anything can bind a socket. Normally the WiFi
    // driver's construction does this on the worker thread, but the server is
    // started before that now, and `httpd_start` on a stack that is not up
    // aborts with "assert failed: tcpip_send_msg_wait_sem ... (Invalid mbox)".
    // `esp_netif_init` is idempotent, so the worker's later call is a no-op.
    //
    // SAFETY: an argument-free IDF initialiser, safe to call more than once.
    esp_idf_sys::esp!(unsafe { esp_idf_sys::esp_netif_init() })?;

    let _server = match http::start(Arc::new(Ctx {
        status: status.clone(),
        settings: shared_settings.clone(),
        cmd: cmd_tx.clone(),
        net: net_tx,
    })) {
        Ok(server) => Some(server),
        Err(e) => {
            log::error!("HTTP server failed to start ({e:#}); the player carries on without it");
            None
        }
    };

    // WiFi last and off this core, so it cannot delay first pixels.
    HEAP_FRESH.store(false, Ordering::Relaxed);
    net_worker::spawn(net_worker::NetWorker {
        modem,
        sysloop,
        nvs: nvs_partition,
        ssid: settings.wifi_ssid.clone(),
        password: settings.wifi_pass.clone(),
        hostname: settings.hostname.clone(),
        status: status.clone(),
        rx: net_rx,
    })?;

    loop {
        // 1. Commands from the HTTP handlers. Non-blocking, and everything that
        //    rebuilds the engine or swaps a file on the card happens here — on
        //    this task, between frames, never on the httpd thread.
        while let Ok(command) = cmd_rx.try_recv() {
            match command {
                Command::Skip => {
                    if let Some(e) = engine.as_mut() {
                        e.skip();
                    }
                }
                Command::TogglePause(reply) => {
                    let now = epoch.elapsed();
                    let paused = match engine.as_mut() {
                        Some(e) => {
                            e.toggle_pause(now);
                            e.is_paused()
                        }
                        None => false,
                    };
                    let mut s = status.lock().unwrap();
                    // The frame on screen is frozen at `paused_at`; resuming
                    // shifts its times by the pause, as the engine shifts its
                    // own, so the reported position carries on from there.
                    if paused && !s.paused {
                        s.paused_at = now;
                    } else if !paused && s.paused {
                        let paused_for = now.saturating_sub(s.paused_at);
                        s.frame_shown_at += paused_for;
                        s.frame_due_at += paused_for;
                    }
                    s.paused = paused;
                    drop(s);
                    let _ = reply.send(paused);
                }
                Command::PlayNow { path, reply } => {
                    // One open, here between frames (a directory scan: ~6 ms
                    // in a bucket, ~330 ms in `gifs/Consoles`), then the file
                    // takes the screen at the next tick under the usual dwell.
                    let outcome = match engine.as_mut() {
                        Some(e) => e.play_now(&path, epoch.elapsed()).map_err(|err| {
                            let msg = format!("{path}: {err}");
                            if err.kind() == std::io::ErrorKind::InvalidData {
                                PlayError::Unplayable(msg)
                            } else {
                                PlayError::NotFound(msg)
                            }
                        }),
                        None => Err(PlayError::NoPlaylist),
                    };
                    match &outcome {
                        Ok(()) => log::info!("play now: {path}"),
                        Err(e) => log::warn!("play now: {e:?}"),
                    }
                    let _ = reply.send(outcome);
                }
                Command::ApplyConfig {
                    settings: next,
                    reply,
                } => {
                    let next = *next;
                    // Check the new set's manifest opens *before* anything is
                    // saved: persisting a set that does not exist used to leave
                    // the engine at None and the board retrying forever, across
                    // reboots. `""` is always acceptable (`/filelist.txt`).
                    // The obsolete `filelist.odm.txt` is refused like a set
                    // that is not there.
                    if next.set != settings.set && nvs_config::is_obsolete_set(&next.set) {
                        log::warn!("config: set '{}' refused: obsolete", next.set);
                        let _ = reply.send(Err(state::SET_NOT_FOUND.to_string()));
                        continue;
                    }
                    if next.set != settings.set && !next.set.is_empty() {
                        let path = next.manifest_path();
                        if let Err(e) = File::open(&path) {
                            log::warn!("config: set '{}' rejected, {path}: {e}", next.set);
                            let _ = reply.send(Err(state::SET_NOT_FOUND.to_string()));
                            continue;
                        }
                    }
                    // What plays is the schedule's overlay on the saved
                    // settings: a rule's playlist wins over `set` while it
                    // is in force, so compare what would actually play.
                    let (eff_now, eff_next) = (sched.effective(&settings), sched.effective(&next));
                    let needs_reindex = eff_next.platforms != eff_now.platforms
                        || eff_next.set != eff_now.set
                        || eff_next.shuffle != eff_now.shuffle;
                    let tz_changed = next.tz != settings.tz || next.tz_name != settings.tz_name;

                    match nvs_config.save(&next) {
                        Err(e) => {
                            let _ = reply.send(Err(format!("saving settings: {e:#}")));
                        }
                        Ok(()) => {
                            settings = next;
                            *shared_settings.lock().unwrap() = settings.clone();
                            // Brightness and dwell apply to what is on screen
                            // right now; a filter, set or order change means a
                            // new catalog — rebuilt in place, never a reboot.
                            if !sched.off() {
                                panel.set_brightness(sched.brightness(&settings));
                            }
                            if let Some(e) = engine.as_mut() {
                                e.set_dwell(dwell_policy(&settings));
                            }
                            if tz_changed {
                                clock::set_timezone(&settings.tz, &settings.tz_name);
                                status.lock().unwrap().tz_name = tz_label(&settings);
                                sched.dirty = true;
                                log::info!(
                                    "time: zone {} ({}), local time {}",
                                    settings.tz_name,
                                    settings.tz,
                                    clock::hhmm_now().unwrap_or_else(|| "not set yet".into())
                                );
                            }
                            log::info!(
                                "config applied (reindex={needs_reindex}): brightness={}, dwell={}..{}s, shuffle={}, platforms='{}', set='{}'",
                                settings.brightness,
                                settings.dwell_min,
                                settings.dwell_max,
                                settings.shuffle,
                                settings.platforms,
                                settings.set
                            );
                            let outcome = if needs_reindex {
                                // A current `.idx` for the new set/filter is
                                // reused; anything else is rebuilt.
                                rebuild(&mut engine, &sched.effective(&settings), false, &status)
                            } else {
                                Ok(Applied::unchanged(
                                    engine.as_ref().map_or(0, |e| e.catalog_len()),
                                ))
                            };
                            let _ = reply.send(outcome);
                        }
                    }
                }
                Command::Reindex(reply) => {
                    // Forced: the caller asked, perhaps because the manifest
                    // was edited off-device with its length unchanged.
                    let outcome = rebuild(&mut engine, &sched.effective(&settings), true, &status);
                    log::info!("reindex requested over HTTP: {outcome:?}");
                    let _ = reply.send(outcome);
                }
                Command::InstallManifest { temp, dest, reply } => {
                    // The engine may hold `dest` and `dest.idx` open, and FAT
                    // must not have an open file replaced under it — so drop it
                    // first. Then the sidecar rule: the stale `.idx` is deleted
                    // *before* the new manifest lands (see
                    // `files::remove_index_beside` for why that order), and the
                    // index is rebuilt if this is the manifest in play.
                    // Compared with the manifest actually in play.
                    let active = files::same_card_path(&status.lock().unwrap().manifest, &dest);
                    if active {
                        engine = None;
                    }
                    let outcome = files::remove_index_beside(&dest)
                        .and_then(|()| files::replace(&temp, &dest))
                        .map_err(|e| {
                            let _ = std::fs::remove_file(&temp);
                            format!("{e:#}")
                        })
                        .and_then(|()| {
                            if active {
                                rebuild(&mut engine, &sched.effective(&settings), true, &status)
                            } else {
                                Ok(Applied::unchanged(
                                    engine.as_ref().map_or(0, |e| e.catalog_len()),
                                ))
                            }
                        });
                    log::info!("manifest installed at {dest} (active={active}): {outcome:?}");
                    let _ = reply.send(outcome);
                }
                Command::InstallMeta { temp, dest, reply } => {
                    // The same sequence as a manifest install: the index
                    // carries the resolved overrides and this file's
                    // fingerprint, so it goes first, and the playlist in
                    // play is rebuilt (the engine, which has the index open,
                    // dropped before that).
                    let Some(manifest) = files::meta_manifest(&dest) else {
                        if let Some(t) = &temp {
                            let _ = std::fs::remove_file(t);
                        }
                        let _ =
                            reply.send(Err(format!("{dest} is not a playlist's overrides file")));
                        continue;
                    };
                    let active = files::same_card_path(&status.lock().unwrap().manifest, &manifest);
                    if active {
                        engine = None;
                    }
                    let outcome = files::remove_index_beside(&manifest)
                        .and_then(|()| match &temp {
                            Some(t) => files::replace(t, &dest),
                            None => match std::fs::remove_file(&dest) {
                                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                                    Err(anyhow::anyhow!("removing {dest}: {e}"))
                                }
                                _ => Ok(()),
                            },
                        })
                        .map_err(|e| {
                            if let Some(t) = &temp {
                                let _ = std::fs::remove_file(t);
                            }
                            format!("{e:#}")
                        })
                        .and_then(|()| {
                            if active {
                                rebuild(&mut engine, &sched.effective(&settings), false, &status)
                            } else {
                                Ok(Applied::unchanged(
                                    engine.as_ref().map_or(0, |e| e.catalog_len()),
                                ))
                            }
                        });
                    log::info!(
                        "overrides {} at {dest} (playlist {manifest}, active={active}): {outcome:?}",
                        if temp.is_some() { "installed" } else { "removed" }
                    );
                    let _ = reply.send(outcome);
                }
                Command::SetSchedule { json, reply } => {
                    let parsed = match sched::parse(json.as_bytes()) {
                        Ok(p) => p,
                        Err(e) => {
                            let _ = reply.send(Err(e));
                            continue;
                        }
                    };
                    let mut next = settings.clone();
                    next.schedule = json;
                    match nvs_config.save(&next) {
                        Err(e) => {
                            let _ = reply.send(Err(format!("saving the schedule: {e:#}")));
                        }
                        Ok(()) => {
                            settings = next;
                            *shared_settings.lock().unwrap() = settings.clone();
                            log::info!(
                                "schedule saved: {} rule(s){}",
                                parsed.rules.len(),
                                if parsed.night.is_some() {
                                    " + night mode"
                                } else {
                                    ""
                                }
                            );
                            sched.schedule = parsed;
                            sched.dirty = true;
                            let _ = reply.send(Ok(Applied::unchanged(
                                engine.as_ref().map_or(0, |e| e.catalog_len()),
                            )));
                        }
                    }
                }
                Command::Wake { minutes, reply } => {
                    let minutes = minutes.min(config::WAKE_MAX_MINUTES);
                    if minutes == 0 {
                        sched.wake_until = None;
                        sched.wake_label = None;
                        log::info!("schedule: wake cancelled, schedule back in force");
                    } else {
                        sched.wake_until =
                            Some(Instant::now() + Duration::from_secs(u64::from(minutes) * 60));
                        sched.wake_label = clock::hhmm_in(minutes);
                        log::info!(
                            "schedule: woken for {minutes} min (until {})",
                            sched.wake_label.as_deref().unwrap_or("?")
                        );
                    }
                    sched.dirty = true;
                    status.lock().unwrap().wake_until = sched.wake_label.clone();
                    let _ = reply.send(Ok(Applied::unchanged(
                        engine.as_ref().map_or(0, |e| e.catalog_len()),
                    )));
                }
            }
        }

        // The schedule: at most once a second look at the clock, and act only
        // when the local minute (or the schedule, zone or a wake) changed.
        if let Some(next) = sched.poll() {
            apply_schedule(
                next,
                &mut sched,
                &settings,
                &mut engine,
                &mut panel,
                &mut canvas,
                &status,
                epoch.elapsed(),
            );
        }

        // Only after every reply above has been sent.
        if REBOOT_PENDING.load(Ordering::Relaxed) {
            reboot_to_apply();
        }

        // 2. No engine (boot, or a reindex that failed): try to build one.
        let Some(eng) = engine.as_mut() else {
            let eff = sched.effective(&settings);
            if rebuild(&mut engine, &eff, false, &status).is_err() {
                // A schedule rule's playlist that will not build is set
                // aside (until the rule changes) and the saved one plays.
                if eff.set != settings.set {
                    log::warn!(
                        "schedule: playlist '{}' will not open; playing the saved one",
                        eff.set
                    );
                    sched.broken_set = Some(eff.set.clone());
                    sched.publish(&status);
                    continue;
                }
                // The set may have gone missing under us (deleted off-device,
                // card swapped): fall back to the default at once rather than
                // retrying a manifest that is not there.
                if heal_missing_set(&mut settings, &mut nvs_config) {
                    *shared_settings.lock().unwrap() = settings.clone();
                    continue;
                }
                thread::sleep(config::RETRY_INTERVAL);
            }
            continue;
        };

        // 3. One frame. First whatever the previous frame deferred: we are
        //    past its sleep, and this frame's clock has not started yet.
        for line in deferred.drain(..) {
            line.log();
        }
        probe::flush();
        let entries = eng.catalog_len();
        probe::tick_begin();
        let tick_now = epoch.elapsed();
        // Night mode "off": the panel is dark and nothing is decoded.
        let off = sched.off();
        let outcome = if off {
            TickOutcome::Idle
        } else {
            eng.tick(tick_now, &mut canvas)?
        };
        let tick_us = if probe::ENABLED {
            epoch.elapsed().saturating_sub(tick_now).as_micros() as u32
        } else {
            0
        };
        sampled_min = sampled_min.min(free_heap());
        // What goes to /api/status from this frame: the file's position
        // (content time, see `NowPlaying::elapsed`) and frame number, when the
        // frame went up and when the next is due — all values the loop holds
        // anyway, published with the fields below in the lock step 4 already
        // takes. The browser's preview syncs its clock to these.
        let mut shown = None;
        let mut new_file = None;
        let mut exhausted = false;
        let sleep_for = match outcome {
            TickOutcome::Frame {
                playing,
                started: is_new,
                next_frame_at,
            } => {
                let blit_start = probe::stamp();
                let blit_px = panel.blit(&mut canvas);
                let blit_us = probe::elapsed_us(blit_start);
                sampled_min = sampled_min.min(free_heap());
                let log_start = probe::stamp();
                if is_new {
                    started = playing.ordinal;
                    deferred.push(Deferred::Started {
                        ordinal: playing.ordinal,
                        path: playing.path.clone(),
                        served: sd::take_last(),
                    });
                }
                // Attribute every new low-water mark to the GIF on screen, so
                // a heap regression names the file that caused it.
                if sampled_min + 2048 < low_water {
                    deferred.push(Deferred::LowWater {
                        from: low_water,
                        to: sampled_min,
                        ordinal: playing.ordinal,
                        path: playing.path.clone(),
                    });
                    low_water = sampled_min;
                }
                // Hold the frame for its GIF delay. Read the clock again,
                // because the blit itself took time.
                let now = epoch.elapsed();
                if probe::ENABLED {
                    let times = probe::FrameTimes {
                        tick_us,
                        blit_us,
                        blit_px,
                        log_us: probe::elapsed_us(log_start),
                        work_us: now.saturating_sub(tick_now).as_micros() as u32,
                        delay_ms: next_frame_at.saturating_sub(tick_now).as_millis() as u64,
                    };
                    let late_ms = (now > next_frame_at)
                        .then(|| now.saturating_sub(next_frame_at).as_millis() as u64);
                    probe::frame(&times, late_ms, is_new, &playing.path);
                }
                shown = Some((playing.elapsed, playing.frame, now, next_frame_at));
                if is_new {
                    new_file = Some(playing);
                }
                match next_frame_at.checked_sub(now) {
                    Some(remaining) => remaining,
                    None => {
                        // Already late. This is the acceptance metric: count it
                        // and remember the worst, then yield a tick so the
                        // watchdog and every other task get their turn.
                        deadline_misses += 1;
                        let late = now.saturating_sub(next_frame_at).as_millis() as u64;
                        worst_lateness_ms = worst_lateness_ms.max(late);
                        config::CATCHUP_SLEEP
                    }
                }
            }
            TickOutcome::Idle if off => config::OFF_SLEEP,
            TickOutcome::Idle => config::IDLE_SLEEP,
            TickOutcome::Exhausted => {
                log::error!(
                    "playlist exhausted — nothing opened. Reshuffling in {}s",
                    config::RETRY_INTERVAL.as_secs()
                );
                exhausted = true;
                config::RETRY_INTERVAL
            }
        };

        // 4. Publish, before the sleep, so a status read during the frame's
        //    hold sees the frame that is on the glass. Cheap fields only: the
        //    strings move when a GIF starts.
        if let Ok(mut s) = status.lock() {
            s.uptime_secs = epoch.elapsed().as_secs();
            s.entries = entries;
            s.heap_free = free_heap();
            s.heap_floor = sampled_min;
            s.deadline_misses = deadline_misses;
            s.worst_lateness_ms = worst_lateness_ms;
            if let Some(playing) = new_file {
                s.ordinal = playing.ordinal;
                s.path = playing.path;
                s.title = playing.title;
                s.gifs_started = playing.ordinal;
            }
            if let Some((elapsed, frame, at, due)) = shown {
                s.frame_elapsed = elapsed;
                s.frame = frame;
                s.frame_shown_at = at;
                s.frame_due_at = due;
            }
        }

        thread::sleep(sleep_for);
        if exhausted {
            // The index is still good: reseed and rewind, don't re-read.
            eng.reset(random_seed());
        } else if shown.is_some() {
            // The panel is up, the card is mounted and a frame is on the
            // glass: this image works. Tell the bootloader, so that a freshly
            // OTA'd image is kept rather than rolled back at the next reset.
            // Once per boot, after the frame's sleep.
            if !confirmed {
                confirmed = true;
                ota::confirm_running_image();
            }
            sampled_min = sampled_min.min(free_heap());
        }

        if last_heartbeat.elapsed() >= config::HEARTBEAT_INTERVAL {
            last_heartbeat = Instant::now();
            heartbeat(
                &panel,
                &status,
                started,
                sampled_min,
                deadline_misses,
                worst_lateness_ms,
                &sched.summary(),
            );
        }
    }
}

/// The zone as status shows it: its name, or the POSIX string if unnamed.
fn tz_label(settings: &Settings) -> String {
    if settings.tz_name.is_empty() {
        settings.tz.clone()
    } else {
        settings.tz_name.clone()
    }
}

/// The schedule at run time: the parsed rules, what was last decided (and
/// applied), and a manual wake. The saved settings are never changed by it —
/// a rule's playlist or brightness is an overlay ([`Scheduling::effective`])
/// that goes away when the rule ends.
struct Scheduling {
    schedule: Schedule,
    decision: Decision,
    wake_until: Option<Instant>,
    wake_label: Option<String>,
    last_check: Instant,
    last_seen: Option<schedule::LocalTime>,
    /// Evaluate at the next poll, whatever the minute.
    dirty: bool,
    /// A rule's playlist that would not build: ignored until the decision
    /// changes, so the loop does not retry it every iteration.
    broken_set: Option<String>,
}

impl Scheduling {
    fn new(json: &str) -> Self {
        let schedule = sched::parse(json.as_bytes()).unwrap_or_else(|e| {
            log::warn!("schedule in NVS unusable ({e}); ignoring it");
            Schedule::default()
        });
        Scheduling {
            schedule,
            decision: Decision::default(),
            wake_until: None,
            wake_label: None,
            last_check: Instant::now(),
            last_seen: None,
            dirty: true,
            broken_set: None,
        }
    }

    /// The settings that apply now: the saved ones, with the rule's playlist
    /// in place of `set` while a rule chooses one.
    fn effective(&self, settings: &Settings) -> Settings {
        let mut eff = settings.clone();
        if let Some(set) = &self.decision.set {
            if self.broken_set.as_ref() != Some(set) {
                eff.set = set.clone();
            }
        }
        eff
    }

    fn brightness(&self, settings: &Settings) -> u8 {
        self.decision.brightness.unwrap_or(settings.brightness)
    }

    fn off(&self) -> bool {
        self.decision.mode == schedule::Mode::Off
    }

    /// A fresh decision, if the minute (or anything else) changed and it
    /// differs from what is applied. Cheap when it is not time to look.
    fn poll(&mut self) -> Option<Decision> {
        if !self.dirty && self.last_check.elapsed() < config::SCHEDULE_CHECK {
            return None;
        }
        self.last_check = Instant::now();
        if self.wake_until.is_some_and(|t| Instant::now() >= t) {
            log::info!("schedule: wake over, schedule back in force");
            self.wake_until = None;
            self.wake_label = None;
            self.dirty = true;
        }
        let now = clock::local_now();
        if !self.dirty && now == self.last_seen {
            return None;
        }
        self.last_seen = now;
        let forced = std::mem::take(&mut self.dirty);
        let next = match now {
            Some(t) if self.wake_until.is_none() => self.schedule.evaluate(t),
            _ => Decision::default(),
        };
        (forced || next != self.decision).then_some(next)
    }

    /// The schedule's fields in status.
    fn publish(&self, status: &Mutex<Status>) {
        let d = &self.decision;
        let mut s = status.lock().unwrap();
        s.mode = d.mode.as_str();
        s.mode_until = d.mode_until.map(schedule::format_hhmm);
        s.rule = d.rule;
        s.rule_set = d
            .set
            .clone()
            .filter(|set| self.broken_set.as_ref() != Some(set));
        s.rule_until = d.rule.and(d.rule_until).map(schedule::format_hhmm);
        s.wake_until = self.wake_label.clone();
    }

    /// For the heartbeat.
    fn summary(&self) -> String {
        let d = &self.decision;
        let mut out = format!("mode {}", d.mode.as_str());
        if let Some(until) = d.mode_until {
            out += &format!(" until {}", schedule::format_hhmm(until));
        }
        if d.mode == schedule::Mode::Off {
            out += " (panel dark, engine paused)";
        }
        if let Some(r) = d.rule {
            out += &format!(
                ", rule {r} -> '{}' until {}",
                d.set.as_deref().unwrap_or("(saved playlist)"),
                d.rule_until.map(schedule::format_hhmm).unwrap_or_default()
            );
        }
        if let Some(w) = &self.wake_label {
            out += &format!(", woken until {w}");
        }
        out
    }
}

/// Put a new schedule decision into effect: the playlist (rebuilt in place
/// when the rule's differs from what is playing — the panel holds its frame
/// meanwhile, as for any playlist change), the brightness, and night mode's
/// dark panel (cleared, brightness 0; the DMA keeps running) with playback
/// paused. Leaving it repaints the whole canvas at the next frame.
#[allow(clippy::too_many_arguments)]
fn apply_schedule(
    next: Decision,
    sched: &mut Scheduling,
    settings: &Settings,
    engine: &mut Option<Engine>,
    panel: &mut Panel,
    canvas: &mut Canvas,
    status: &Mutex<Status>,
    now: Duration,
) {
    let prev = std::mem::replace(&mut sched.decision, next);
    if prev.set != sched.decision.set {
        sched.broken_set = None;
    }
    let d = sched.decision.clone();
    log::info!(
        "schedule: {} at {}",
        sched.summary(),
        clock::hhmm_now().unwrap_or_else(|| "(clock not set)".into())
    );

    // The playlist.
    let eff = sched.effective(settings);
    let playing = status.lock().unwrap().manifest.clone();
    if engine.is_some() && !files::same_card_path(&eff.manifest_path(), &playing) {
        log::info!("schedule: playlist {} -> {}", playing, eff.manifest_path());
        if rebuild(engine, &eff, false, status).is_err() && eff.set != settings.set {
            log::warn!(
                "schedule: playlist '{}' will not open; playing the saved one",
                eff.set
            );
            sched.broken_set = Some(eff.set.clone());
            let _ = rebuild(engine, &sched.effective(settings), false, status);
        }
    }

    // The panel.
    let was_off = prev.mode == schedule::Mode::Off;
    if d.mode == schedule::Mode::Off {
        if !was_off {
            panel.clear();
            panel.set_brightness(0);
            if let Some(e) = engine.as_mut() {
                e.pause(now);
            }
            log::info!("schedule: night mode, panel dark and playback paused");
        }
    } else {
        if was_off {
            // The DMA buffer was cleared under the canvas: the next frame
            // must repaint all of it, not just what changed.
            canvas.dirty_mut().mark_all();
            let user_paused = status.lock().unwrap().paused;
            if let Some(e) = engine.as_mut() {
                if !user_paused {
                    e.resume(now);
                }
            }
            log::info!("schedule: panel back on");
        }
        panel.set_brightness(sched.brightness(settings));
    }
    sched.publish(status);
}

/// A log line owed by a frame, printed after that frame's sleep.
enum Deferred {
    /// A GIF started. The heap figures are read when the line is printed,
    /// between frames: `heap_caps_get_largest_free_block` walks the heap.
    Started {
        ordinal: u64,
        path: String,
        /// How `SdSource` served it: the prepared copy or the original.
        served: Option<sd::Served>,
    },
    LowWater {
        from: u32,
        to: u32,
        ordinal: u64,
        path: String,
    },
}

impl Deferred {
    fn log(self) {
        match self {
            Deferred::Started {
                ordinal,
                path,
                served,
            } => log::info!(
                "[{ordinal}] {path} <- {} (heap {} B free, largest block {} B)",
                match served {
                    Some(sd::Served::Prepared { try_us }) => {
                        format!("prepared, open {:.1} ms", try_us as f32 / 1000.0)
                    }
                    Some(sd::Served::Original { miss_us }) => {
                        format!("original, miss {:.1} ms", miss_us as f32 / 1000.0)
                    }
                    Some(sd::Served::Direct) | None => "as named".to_string(),
                },
                free_heap(),
                largest_free_block()
            ),
            Deferred::LowWater {
                from,
                to,
                ordinal,
                path,
            } => log::warn!("heap low-water {from} B -> {to} B during [{ordinal}] {path}"),
        }
    }
}

/// 64 bits from the hardware RNG.
fn random_seed() -> u64 {
    // SAFETY: `esp_random()` takes no arguments and has no preconditions once
    // the RF/RNG subsystem is up, which it is by the time `main` runs.
    unsafe { ((esp_idf_sys::esp_random() as u64) << 32) | (esp_idf_sys::esp_random() as u64) }
}

/// Lowest free heap seen since boot, in bytes.
fn min_free_heap() -> u32 {
    // SAFETY: as above.
    unsafe { esp_idf_sys::esp_get_minimum_free_heap_size() }
}

/// Largest single allocation currently possible from byte-addressable heap.
fn largest_free_block() -> usize {
    // SAFETY: a read-only query with a capability mask argument.
    unsafe { esp_idf_sys::heap_caps_get_largest_free_block(esp_idf_sys::MALLOC_CAP_8BIT) }
}

/// Assert that a typed GPIO peripheral really is the pin `config` names.
fn expect_pin<P: Pin>(pin: P, expected: u32, what: &str) -> P {
    assert_eq!(
        pin.pin() as u32,
        expected,
        "{what} is wired to GPIO {expected} (hardware/PINOUT.md) but GPIO {} was handed over",
        pin.pin()
    );
    pin
}

fn build_filter(settings: &Settings) -> Filter {
    let platforms = settings.platform_list();
    let filter = Filter::new();
    if platforms.is_empty() {
        filter
    } else {
        filter.platforms(platforms.iter().map(|s| s.as_str()))
    }
}

fn dwell_policy(settings: &Settings) -> DwellPolicy {
    DwellPolicy {
        min: Duration::from_secs(settings.dwell_min),
        max: Duration::from_secs(settings.dwell_max),
        ..DwellPolicy::default()
    }
}

/// A fresh engine over the configured manifest's playlist index, shuffled with
/// a hardware-random seed so two boots do not play the same order, with the
/// playlist's overrides (its `.meta`) applied.
fn build_engine(
    settings: &Settings,
    path: &str,
    force: bool,
) -> Result<(Engine, IndexSource, Overrides)> {
    let (catalog, overrides, source) = open_catalog(path, &build_filter(settings), force)?;
    let catalog: Box<dyn Catalog> = if probe::ENABLED {
        Box::new(probe::TimedCatalog(catalog))
    } else {
        catalog
    };
    let mode = if settings.shuffle {
        Mode::Shuffle
    } else {
        Mode::Sequential
    };
    let scheduler =
        Scheduler::new(catalog.len(), mode, random_seed()).with_no_repeat(config::NO_REPEAT_WINDOW);
    let mut engine = Engine::new(
        catalog,
        scheduler,
        dwell_policy(settings),
        Box::new(SdSource),
    );
    engine.set_overrides(&overrides);
    Ok((engine, source, overrides))
}

/// The fingerprint of the playlist's `.meta` as the index records it:
/// [`NO_META`] when there is none. One small file read.
fn meta_fingerprint(meta: &str) -> u64 {
    match File::open(meta) {
        Ok(f) => overrides::fingerprint(f).unwrap_or_else(|e| {
            log::warn!("{meta}: unreadable ({e}); treating it as changed");
            1
        }),
        Err(_) => NO_META,
    }
}

/// The playlist's `.meta`, parsed for an index build (empty if there is
/// none, or it will not read).
fn read_meta(meta: &str) -> OverrideSet {
    match File::open(meta) {
        Ok(f) => OverrideSet::read(BufReader::with_capacity(512, f)).unwrap_or_else(|e| {
            log::warn!("{meta}: unreadable ({e}); no overrides");
            OverrideSet::new()
        }),
        Err(_) => OverrideSet::new(),
    }
}

/// The playlist for `path` under `filter`, in order of preference:
///
/// 1. **Loaded**: `<path>.idx` exists, its header reads, and
///    [`IndexHeader::is_valid_for`] the manifest's current length, this
///    filter's fingerprint and the playlist's `.meta` fingerprint. Nothing
///    is read but the header and the resolved overrides after the offsets.
///    Skipped when `force` is set.
/// 2. **Built**: one streaming pass over the manifest writes
///    `<path>.idx.tmp` — offsets, and the `.meta` overrides resolved against
///    them in the same pass — which is renamed over `<path>.idx` and opened.
///    Needs ~4 KB of heap (a 1 KB read buffer, a 2 KB write buffer, one
///    line) plus 8 B of key per override for the duration.
/// 3. **In RAM**: the card would not take the file (read-only, full), so the
///    offsets are kept in a `Vec` exactly as before this index existed and the
///    panel still plays. That `Vec` passes through a 64 KB block while it
///    grows, which only exists on a fresh heap — see
///    [`config::IN_RAM_INDEX_MIN_BLOCK`] and [`NeedsFreshHeap`].
///
/// Either way the manifest's `File` stays open inside the catalog; with an
/// index file that is two FATFS handles for the life of the playlist.
fn open_catalog(
    path: &str,
    filter: &Filter,
    force: bool,
) -> Result<(Box<dyn Catalog>, Overrides, IndexSource)> {
    let manifest = File::open(path).with_context(|| format!("opening {path}"))?;
    let len = manifest
        .metadata()
        .with_context(|| format!("stat {path}"))?
        .len();
    let len = u32::try_from(len).with_context(|| format!("{path} is over 4 GB"))?;
    let idx = files::index_path(path);
    let meta = overrides::meta_path(path);
    let meta_fp = meta_fingerprint(&meta);

    if force {
        log::info!("{idx}: rebuild requested");
    } else {
        match load_index(&idx, len, filter, meta_fp) {
            Ok(index) => {
                let mut catalog = IndexedCatalog::open(manifest, index)
                    .with_context(|| format!("opening {idx}"))?;
                let ov = catalog
                    .overrides()
                    .with_context(|| format!("reading the overrides in {idx}"))?;
                return Ok((Box::new(catalog), ov, IndexSource::Loaded));
            }
            Err(why) => log::info!("{idx}: {why}; rebuilding"),
        }
    }

    let set = read_meta(&meta);
    if !set.is_empty() {
        log::info!("{meta}: {} overrides to resolve", set.len());
    }
    let mut manifest = manifest;
    match write_index_file(&mut manifest, len, filter, &set, meta_fp, &idx) {
        Ok((header, ov)) => {
            let index = File::open(&idx).with_context(|| format!("reopening {idx}"))?;
            let catalog =
                IndexedCatalog::open(manifest, index).with_context(|| format!("opening {idx}"))?;
            debug_assert_eq!(catalog.len(), header.count as usize);
            Ok((Box::new(catalog), ov, IndexSource::Built))
        }
        Err(e) => {
            log::warn!("cannot write {idx} ({e:#}); falling back to an in-RAM index");
            let block = largest_free_block();
            if !HEAP_FRESH.load(Ordering::Relaxed) && block < config::IN_RAM_INDEX_MIN_BLOCK {
                return Err(anyhow::Error::new(NeedsFreshHeap { block }));
            }
            manifest.seek(SeekFrom::Start(0))?;
            let mut ov = Overrides::new();
            let mut k = 0u32;
            let catalog = FileCatalog::build_with(manifest, filter, |entry, matched| {
                if matched {
                    if let Some(o) = set.get(&entry.path) {
                        ov.push(k, o);
                    }
                    k += 1;
                }
            })
            .with_context(|| format!("indexing {path} in RAM"))?;
            Ok((Box::new(catalog), ov, IndexSource::InRam))
        }
    }
}

/// The index file, if it is current; otherwise why not, for the log.
fn load_index(idx: &str, manifest_len: u32, filter: &Filter, meta_fp: u64) -> Result<File, String> {
    let mut file = match File::open(idx) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err("missing".into()),
        Err(e) => return Err(format!("unreadable ({e})")),
    };
    let header = IndexHeader::read(&mut file).map_err(|e| format!("unusable ({e})"))?;
    if !header.is_valid_for(manifest_len, filter, meta_fp) {
        return Err(format!(
            "stale (built for a {} B manifest, filter {:016x}, overrides {:016x}; now {manifest_len} B, {:016x}, {meta_fp:016x})",
            header.manifest_len,
            header.filter_fingerprint,
            header.meta_fingerprint,
            filter.fingerprint()
        ));
    }
    Ok(file)
}

/// Write `idx` via `idx.tmp` and a rename, so a crash or a full card never
/// leaves a half-written index under the real name. The temp file is removed
/// on failure.
fn write_index_file(
    manifest: &mut File,
    manifest_len: u32,
    filter: &Filter,
    set: &OverrideSet,
    meta_fp: u64,
    idx: &str,
) -> Result<(IndexHeader, Overrides)> {
    let tmp = format!("{idx}.tmp");
    let written = File::create(&tmp)
        .with_context(|| format!("creating {tmp}"))
        .and_then(|out| {
            // `out` is dropped (closed, flushed to the card) when this returns.
            write_index_with_overrides(
                BufReader::with_capacity(1024, &mut *manifest),
                manifest_len,
                filter,
                set,
                meta_fp,
                out,
            )
            .with_context(|| format!("writing {tmp}"))
        })
        .and_then(|built| files::replace(&tmp, idx).map(|()| built));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// The card would not take an index file, and the in-RAM fallback needs a
/// contiguous block this heap no longer has. Only reachable after boot (see
/// [`HEAP_FRESH`]); the main loop reboots, and the boot-time build — on a
/// fresh heap — takes the in-RAM path successfully.
#[derive(Debug)]
struct NeedsFreshHeap {
    block: usize,
}

impl std::fmt::Display for NeedsFreshHeap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the card will not take an index file and an in-RAM index needs {} B contiguous \
             (largest block {} B); rebooting to build it on a fresh heap",
            config::IN_RAM_INDEX_MIN_BLOCK,
            self.block
        )
    }
}

impl std::error::Error for NeedsFreshHeap {}

/// Rebuild `engine` in place, reporting what happened or a message for the
/// HTTP caller. Only ever called from the main task. `force` skips reusing a
/// current `.idx` and rebuilds it regardless.
///
/// The old engine is dropped first: it holds the manifest and its `.idx` open
/// (the latter is about to be replaced), and its GIF decoder is the biggest
/// thing on the heap.
fn rebuild(
    engine: &mut Option<Engine>,
    settings: &Settings,
    force: bool,
    status: &Mutex<Status>,
) -> Result<Applied, String> {
    *engine = None;
    let path = settings.manifest_path();
    {
        let mut s = status.lock().unwrap();
        s.manifest = path.clone();
        s.index_state = "building";
    }
    let outcome = build_and_report(engine, settings, &path, force);
    let mut s = status.lock().unwrap();
    // `entries` too, so a status read right after the reply is already right
    // (the frame loop only republishes it after its next frame).
    s.entries = outcome.as_ref().map_or(0, |a| a.entries);
    s.overrides = outcome.as_ref().map_or(0, |a| a.overrides);
    s.hidden = outcome.as_ref().map_or(0, |a| a.hidden);
    s.index_state = if outcome.is_ok() {
        "ready"
    } else {
        "unavailable"
    };
    drop(s);
    if outcome.is_ok() {
        cache::set_playlist(&path, &settings.platforms);
    }
    outcome
}

fn build_and_report(
    engine: &mut Option<Engine>,
    settings: &Settings,
    path: &str,
    force: bool,
) -> Result<Applied, String> {
    let started = Instant::now();
    match build_engine(settings, path, force) {
        Ok((built, source, ov)) => {
            let ms = started.elapsed().as_millis() as u64;
            let entries = built.catalog_len();
            if entries == 0 {
                log::error!("{path} matched no entries under the configured filter");
                return Err(format!("{path} matched no entries"));
            }
            let (hidden, weighted, timed) = ov.summary();
            log::info!(
                "manifest indexed: {path}, {entries} entries, index {} in {ms} ms ({} B {}), \
                 overrides {} ({hidden} hidden, {weighted} more/less often, {timed} timed), heap {} B free",
                source.as_str(),
                entries * 4,
                if source == IndexSource::InRam { "in RAM" } else { "on card" },
                ov.len(),
                free_heap()
            );
            if hidden >= entries {
                log::warn!("{path}: every entry is hidden; nothing will play");
            }
            *engine = Some(built);
            Ok(Applied {
                entries,
                index: Some(source),
                index_ms: ms,
                overrides: ov.len(),
                hidden,
            })
        }
        Err(e) => {
            if e.downcast_ref::<NeedsFreshHeap>().is_some() {
                log::error!("{path}: {e:#}");
                REBOOT_PENDING.store(true, Ordering::Relaxed);
                return Err(format!("{path}: {e:#}"));
            }
            log::error!("{path} unreadable ({e:#})");
            Err(format!("{path}: {e:#}"))
        }
    }
}

/// If the configured set's manifest will not open, fall back to the default
/// manifest and clear `set` in NVS. Returns true if it did.
///
/// Called at boot and whenever the main loop has no engine. `""` (the default)
/// is never "missing" here: if the card has no `filelist.txt` either, the loop
/// keeps retrying exactly as it does for a card with nothing on it.
fn heal_missing_set(settings: &mut Settings, nvs: &mut Config) -> bool {
    if settings.set.is_empty() {
        return false;
    }
    if nvs_config::is_obsolete_set(&settings.set) {
        log::warn!(
            "configured set '{}' is obsolete (prepared copies are found automatically); playing the default manifest and clearing it from NVS",
            settings.set
        );
    } else {
        let path = settings.manifest_path();
        let Err(e) = File::open(&path) else {
            return false;
        };
        log::warn!(
            "configured set '{}' does not open ({path}: {e}); falling back to the default manifest and clearing it from NVS",
            settings.set
        );
    }
    settings.set.clear();
    if let Err(e) = nvs.save(settings) {
        log::error!("clearing the set in NVS failed ({e:#}); the default plays for this boot only");
    }
    true
}

/// Reboot so the in-RAM fallback index can be built on a fresh heap.
///
/// Only reached through [`NeedsFreshHeap`], i.e. a card that refuses the
/// `.idx` file. The reply has already been queued; the sleep is for the
/// handler on the other core, which is still writing it.
fn reboot_to_apply() -> ! {
    thread::sleep(Duration::from_secs(2));
    // SAFETY: an argument-free IDF call, declared `-> !` by the bindings.
    unsafe { esp_idf_sys::esp_restart() }
}

#[allow(clippy::too_many_arguments)]
fn heartbeat(
    panel: &Panel,
    status: &Arc<Mutex<Status>>,
    started: u64,
    sampled_min: u32,
    deadline_misses: u64,
    worst_lateness_ms: u64,
    schedule: &str,
) {
    let free = free_heap();
    let min_free = min_free_heap();
    let refresh = panel.refresh_rate();
    let (ip, wifi) = {
        let mut s = status.lock().unwrap();
        s.refresh_hz = refresh;
        s.heap_idf_min = min_free;
        (s.ip.clone(), s.wifi)
    };
    log::info!(
        "heartbeat: heap {free} B free (floor {sampled_min} B, idf-min {min_free} B), refresh {refresh} Hz, \
         {started} GIFs started, {deadline_misses} deadline misses (worst {worst_lateness_ms} ms), \
         wifi {wifi} {ip}, {schedule}{}{}",
        sd::period_summary(),
        probe::period_summary()
    );
}
