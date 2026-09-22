//! OpenDMD firmware — phase 3a: a networked player.
//!
//! Boot -> HUB75 panel up -> mount the microSD -> read settings from NVS ->
//! build a seekable index over the manifest -> start the network worker ->
//! run `opendmd_core::engine::Engine`, pushing every frame to two chained
//! 64x32 HUB75 P4 panels (128x32) over the mrcodetastic DMA driver.
//!
//! The manifest is never held in RAM. `FileCatalog` makes one streaming pass
//! over the card's 450 KB / 10,162-line `filelist.txt` and keeps a `u32` byte
//! offset per matching line (~40 KB unfiltered); the file handle stays open
//! and the engine seeks back to it for one `Entry` at a time, only when it is
//! about to open the next GIF.
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

mod config;
mod dns_server;
mod files;
mod http;
mod httpd;
mod net_worker;
mod nvs_config;
mod ota;
mod panel;
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
use opendmd_core::scheduler::{Mode, Scheduler};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::http::{Ctx, NetCommand};
use crate::nvs_config::{Config, Settings};
use crate::panel::{Panel, PanelConfig};
use crate::sd::{SdBus, SdSource};
use crate::state::{free_heap, Applied, Command, Status};

const VERSION: &str = env!("CARGO_PKG_VERSION");

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
    panel.set_brightness(settings.brightness);
    log::info!(
        "settings: brightness={}, dwell={}..{}s, shuffle={}, platforms='{}', set='{}', host='{}', wifi={}",
        settings.brightness,
        settings.dwell_min,
        settings.dwell_max,
        settings.shuffle,
        settings.platforms,
        settings.set,
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

    // --- Shared state, the HTTP server, and the network worker ------------
    let status = Arc::new(Mutex::new(Status::new(VERSION, settings.hostname.clone())));
    {
        let mut s = status.lock().unwrap();
        s.sd_mounted = true;
        s.sd_capacity_bytes = _card.capacity_bytes;
        s.manifest = settings.manifest_path();
        s.refresh_hz = panel.refresh_rate();
    }
    let shared_settings = Arc::new(Mutex::new(settings.clone()));
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let (net_tx, net_rx) = mpsc::channel::<NetCommand>();

    // One file handle and one offset index for the life of the run: the only
    // pass over the card, and `Exhausted` reseeds the scheduler rather than
    // re-reading it.
    //
    // It is built here, before anything network-shaped exists, because
    // it is the largest single allocation after the panel's and it must not
    // fail. If the card has no readable manifest the loop below keeps retrying.
    let mut engine: Option<Engine> = None;
    let _ = rebuild(&mut engine, &settings);

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
                    let paused = match engine.as_mut() {
                        Some(e) => {
                            e.toggle_pause(epoch.elapsed());
                            e.is_paused()
                        }
                        None => false,
                    };
                    status.lock().unwrap().paused = paused;
                    let _ = reply.send(paused);
                }
                Command::ApplyConfig {
                    settings: next,
                    reply,
                } => {
                    let next = *next;
                    let needs_reindex = next.platforms != settings.platforms
                        || next.set != settings.set
                        || next.shuffle != settings.shuffle;

                    match nvs_config.save(&next) {
                        Err(e) => {
                            let _ = reply.send(Err(format!("saving settings: {e:#}")));
                        }
                        Ok(()) => {
                            settings = next;
                            *shared_settings.lock().unwrap() = settings.clone();
                            status.lock().unwrap().manifest = settings.manifest_path();
                            // Brightness and dwell apply to what is on screen
                            // right now; a filter, set or order change means a
                            // new catalog, which may cost a reboot.
                            panel.set_brightness(settings.brightness);
                            if let Some(e) = engine.as_mut() {
                                e.set_dwell(dwell_policy(&settings));
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
                                reindex(&mut engine, &settings)
                            } else {
                                Ok(Applied {
                                    entries: engine.as_ref().map_or(0, |e| e.catalog_len()),
                                    rebooting: false,
                                })
                            };
                            let rebooting = matches!(
                                outcome,
                                Ok(Applied {
                                    rebooting: true,
                                    ..
                                })
                            );
                            let _ = reply.send(outcome);
                            if rebooting {
                                reboot_to_apply();
                            }
                        }
                    }
                }
                Command::Reindex(reply) => {
                    let outcome = reindex(&mut engine, &settings);
                    log::info!("reindex requested over HTTP: {outcome:?}");
                    let rebooting = matches!(
                        outcome,
                        Ok(Applied {
                            rebooting: true,
                            ..
                        })
                    );
                    let _ = reply.send(outcome);
                    if rebooting {
                        reboot_to_apply();
                    }
                }
                Command::InstallManifest { temp, dest, reply } => {
                    // The engine may hold `dest` open, and FAT will not replace
                    // an open file — so drop it first, swap, then reindex if
                    // the file we just replaced is the one in play.
                    let active = settings.manifest_path() == dest;
                    if active {
                        engine = None;
                    }
                    let outcome = files::replace(&temp, &dest)
                        .map_err(|e| format!("{e:#}"))
                        .and_then(|()| {
                            if active {
                                reindex(&mut engine, &settings)
                            } else {
                                Ok(Applied {
                                    entries: engine.as_ref().map_or(0, |e| e.catalog_len()),
                                    rebooting: false,
                                })
                            }
                        });
                    log::info!("manifest installed at {dest} (active={active}): {outcome:?}");
                    let rebooting = matches!(
                        outcome,
                        Ok(Applied {
                            rebooting: true,
                            ..
                        })
                    );
                    let _ = reply.send(outcome);
                    if rebooting {
                        reboot_to_apply();
                    }
                }
            }
        }

        // 2. No engine (boot, or a reindex that failed): try to build one.
        let Some(eng) = engine.as_mut() else {
            if rebuild(&mut engine, &settings).is_err() {
                thread::sleep(config::RETRY_INTERVAL);
            }
            continue;
        };

        // 3. One frame.
        let entries = eng.catalog_len();
        let outcome = eng.tick(epoch.elapsed(), &mut canvas)?;
        sampled_min = sampled_min.min(free_heap());
        match outcome {
            TickOutcome::Frame {
                playing,
                started: is_new,
                next_frame_at,
            } => {
                panel.blit(&canvas);
                sampled_min = sampled_min.min(free_heap());
                if is_new {
                    started = playing.ordinal;
                    log::info!(
                        "[{}] {} (heap {} B free, largest block {} B)",
                        playing.ordinal,
                        playing.path,
                        free_heap(),
                        largest_free_block()
                    );
                    let mut s = status.lock().unwrap();
                    s.ordinal = playing.ordinal;
                    s.path = playing.path.clone();
                    s.title = playing.title.clone();
                    s.gifs_started = playing.ordinal;
                }
                // Attribute every new low-water mark to the GIF on screen, so
                // a heap regression names the file that caused it.
                if sampled_min + 2048 < low_water {
                    log::warn!(
                        "heap low-water {} B -> {} B during [{}] {}",
                        low_water,
                        sampled_min,
                        playing.ordinal,
                        playing.path
                    );
                    low_water = sampled_min;
                }
                // Hold the frame for its GIF delay. Read the clock again,
                // because the blit itself took time.
                let now = epoch.elapsed();
                match next_frame_at.checked_sub(now) {
                    Some(remaining) => thread::sleep(remaining),
                    None => {
                        // Already late. This is the acceptance metric: count it
                        // and remember the worst, then yield a tick so the
                        // watchdog and every other task get their turn.
                        deadline_misses += 1;
                        let late = now.saturating_sub(next_frame_at).as_millis() as u64;
                        worst_lateness_ms = worst_lateness_ms.max(late);
                        thread::sleep(config::CATCHUP_SLEEP);
                    }
                }
                sampled_min = sampled_min.min(free_heap());
            }
            TickOutcome::Idle => thread::sleep(config::IDLE_SLEEP),
            TickOutcome::Exhausted => {
                log::error!(
                    "playlist exhausted — nothing opened. Reshuffling in {}s",
                    config::RETRY_INTERVAL.as_secs()
                );
                thread::sleep(config::RETRY_INTERVAL);
                // The index is still good: reseed and rewind, don't re-read.
                eng.reset(random_seed());
            }
        }

        // 4. Publish. Cheap fields only: the strings move when a GIF starts.
        if let Ok(mut s) = status.lock() {
            s.uptime_secs = epoch.elapsed().as_secs();
            s.entries = entries;
            s.heap_free = free_heap();
            s.heap_floor = sampled_min;
            s.deadline_misses = deadline_misses;
            s.worst_lateness_ms = worst_lateness_ms;
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
            );
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

/// A fresh engine over a streaming index of the configured manifest, shuffled
/// with a hardware-random seed so two boots do not play the same order.
///
/// The `File` is handed to `FileCatalog` and stays open: the engine seeks back
/// into it for one `Entry` at a time. Nothing proportional to the card's size
/// is ever resident — the index is 4 bytes per playable entry.
fn build_engine(settings: &Settings) -> Result<Engine> {
    let path = settings.manifest_path();
    let file = std::fs::File::open(&path).with_context(|| format!("opening {path}"))?;
    let catalog = FileCatalog::build(file, &build_filter(settings))
        .with_context(|| format!("indexing {path}"))?;
    let mode = if settings.shuffle {
        Mode::Shuffle
    } else {
        Mode::Sequential
    };
    let scheduler =
        Scheduler::new(catalog.len(), mode, random_seed()).with_no_repeat(config::NO_REPEAT_WINDOW);
    Ok(Engine::new(
        Box::new(catalog),
        scheduler,
        dwell_policy(settings),
        Box::new(SdSource),
    ))
}

/// Rebuild `engine` in place, reporting the entry count or a message for the
/// HTTP caller. Only ever called from the main task.
fn rebuild(engine: &mut Option<Engine>, settings: &Settings) -> Result<usize, String> {
    let path = settings.manifest_path();
    // Drop first: the old engine holds `path` open, and on FAT a stale handle
    // is the difference between a reindex and a corrupt read.
    *engine = None;
    match build_engine(settings) {
        Ok(built) => {
            let entries = built.catalog_len();
            if entries == 0 {
                log::error!("{path} matched no entries under the configured filter");
                return Err(format!("{path} matched no entries"));
            }
            log::info!(
                "manifest indexed: {path}, {entries} entries, {} index bytes, heap {} B free",
                entries * 4,
                free_heap()
            );
            *engine = Some(built);
            Ok(entries)
        }
        Err(e) => {
            log::error!("{path} unreadable ({e:#})");
            Err(format!("{path}: {e:#}"))
        }
    }
}

/// Rebuild the catalog if the heap can still take it, and say so if it cannot.
///
/// The check is the whole point — see [`config::REINDEX_MIN_BLOCK`]. With no
/// 64 KB block left, the caller reboots instead: the new manifest is already on
/// the card and the index is built on a fresh heap at boot. Trying anyway
/// aborts the board, which is what this replaced.
fn reindex(engine: &mut Option<Engine>, settings: &Settings) -> Result<Applied, String> {
    // Drop the old catalog first either way: it holds the manifest open.
    *engine = None;
    let block = largest_free_block();
    if block < config::REINDEX_MIN_BLOCK {
        log::info!(
            "in-place reindex wants {} B contiguous, largest block is {block} B — rebooting to apply",
            config::REINDEX_MIN_BLOCK
        );
        return Ok(Applied {
            entries: 0,
            rebooting: true,
        });
    }
    rebuild(engine, settings).map(|entries| Applied {
        entries,
        rebooting: false,
    })
}

/// Reboot so a playlist change takes effect on a fresh heap.
///
/// The sleep is for the HTTP response: the handler is still writing it on the
/// other core when this runs.
fn reboot_to_apply() -> ! {
    thread::sleep(Duration::from_secs(2));
    // SAFETY: an argument-free IDF call, declared `-> !` by the bindings.
    unsafe { esp_idf_sys::esp_restart() }
}

fn heartbeat(
    panel: &Panel,
    status: &Arc<Mutex<Status>>,
    started: u64,
    sampled_min: u32,
    deadline_misses: u64,
    worst_lateness_ms: u64,
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
         wifi {wifi} {ip}"
    );
}
