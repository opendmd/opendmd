//! OpenDMD firmware — phase 3a: a networked player.
//!
//! Boot -> HUB75 panel up -> settings from NVS -> the boot task (`boot.rs`,
//! CPU0: the microSD and a seekable index over the manifest, the HTTP server,
//! the web app, then the network worker) while this task plays the start-up
//! show (`show.rs`) -> the handover -> run `opendmd_core::engine::Engine`,
//! pushing every frame to two chained 64x32 HUB75 P4 panels (128x32) over the
//! mrcodetastic DMA driver.
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
//! | main | CPU1 | panel, engine, this loop, the start-up show and the status card. Never touches WiFi or a socket; the card only after the handover. |
//! | `boot` | CPU0 | until the handover: the card's mounts and the playlist index, the httpd, the web app, then the network worker |
//! | `net-worker` | CPU0 | WiFi (connect, reconnect, AP fallback), mDNS, starts the httpd |
//! | `httpd` | CPU0 | every HTTP handler; the only task allowed to block |
//! | (net-worker) | CPU0 | also Follow RePlayOS's polls and context index builds (`follow.rs`) |
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

mod ahead;
mod boot;
mod cache;
mod cache_builder;
mod chan;
mod clock;
mod config;
mod diag;
mod dns_server;
mod files;
mod follow;
mod frametime;
mod http;
mod httpd;
mod improv;
mod logring;
mod net_worker;
mod nvs_config;
mod ota;
mod panel;
mod probe;
#[cfg(feature = "psi-probe")]
mod psi_probe;
mod sched;
mod sd;
mod sdhealth;
mod show;
mod state;
mod stream;
mod wifi;
mod www;

use crate::chan as mpsc;
use anyhow::{Context, Result};
use esp_idf_hal::gpio::{AnyIOPin, AnyOutputPin, Pin};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::http::{Ctx, NetCommand};
use crate::nvs_config::{Config, Settings};
use crate::panel::{Panel, PanelConfig};
use opendmd_core::look::{GlowLevel, Look};
use opendmd_core::panel_test::{self, PanelTest};
use opendmd_core::panel_timing::{self, Effects, PanelOp, PanelTiming, Tuner};
use crate::sd::{SdBus, SdCard, SdSource};
use crate::show::{Screen, CARD_FAILING};
use crate::state::{
    free_heap, largest_free_block, Applied, Command, IndexSource, PlayError, Status,
};

/// True until the network worker is spawned: the heap is still whole, so the
/// in-RAM fallback index may simply be attempted (it always fitted at boot).
/// After that it is only attempted if the block it needs is really there.
static HEAP_FRESH: AtomicBool = AtomicBool::new(true);
/// Set by [`rebuild`] when the only way to get a playlist is a reboot (see
/// [`NeedsFreshHeap`]), and by [`panel_effects`] to undo a panel-timing boot
/// trial; acted on by the main loop *after* it has answered the command that
/// caused it, so the HTTP caller gets its reply first.
static REBOOT_PENDING: AtomicBool = AtomicBool::new(false);
/// A frame has been blitted since boot: an engine built from now on (a
/// playlist change) transitions in from the picture on the panel rather than
/// cutting in. The first engine at boot has nothing to transition from.
static FRAME_SHOWN: AtomicBool = AtomicBool::new(false);
/// The boot task has handed the card over (`boot.rs`): until then the main
/// task does not touch the card at all — every rebuild, set check and
/// manifest install answers [`CARD_NOT_READY`], as they did with no card.
static CARD_READY: AtomicBool = AtomicBool::new(false);
const CARD_NOT_READY: &str = "the card is not ready";
/// How a build that matched nothing ends its error (`finish_build`).
const NO_ENTRIES: &str = "matched no entries";
/// Transitions refused by [`transition_gate`] since boot (heartbeat).
static TRANSITIONS_CUT: AtomicU32 = AtomicU32::new(0);

/// Safe start: from boot until [`config::SAFE_START`] has passed — counted
/// from the later of boot and the network first coming up (`state::NET_UP`)
/// — the panel's ceiling is `min(max_brightness, BRIGHTNESS_SAFE_MAX)`, so a
/// saved brightness that browns the supply out (or freezes the chip) cannot
/// make a device unreachable: after any reset it is up and answering at a
/// safe level for that long, and a `PUT /api/config` can lower it. Then the
/// saved `max_brightness` applies. Times are uptime (`epoch`, like
/// `uptime_secs`); the deadline is published in `state::SAFE_START_UNTIL`.
struct SafeStart {
    /// When the ceiling lifts; `None` once it has.
    until: Option<Duration>,
    /// The network's arrival has been counted.
    net_seen: bool,
}

impl SafeStart {
    fn new(settings: &Settings) -> Self {
        let until = config::SAFE_START;
        state::SAFE_START_UNTIL.store(until.as_secs() as u32, Ordering::Relaxed);
        log::info!(
            "safe start: brightness held at {} until {} s (saved brightness {}, max {})",
            settings
                .brightness
                .min(settings.max_brightness)
                .min(config::BRIGHTNESS_SAFE_MAX),
            until.as_secs(),
            settings.brightness,
            settings.max_brightness,
        );
        SafeStart {
            until: Some(until),
            net_seen: false,
        }
    }

    /// The panel's ceiling for a saved `max_brightness` right now.
    fn ceiling(&self, max: u8) -> u8 {
        if self.until.is_some() {
            max.min(config::BRIGHTNESS_SAFE_MAX)
        } else {
            max
        }
    }

    /// Called every loop iteration with the uptime; true exactly once, when
    /// safe start has just ended (the caller lifts the ceiling).
    fn poll(&mut self, now: Duration) -> bool {
        let Some(until) = self.until else {
            return false;
        };
        if !self.net_seen && state::NET_UP.load(Ordering::Relaxed) {
            self.net_seen = true;
            let later = now + config::SAFE_START;
            if later > until {
                self.until = Some(later);
                let secs = later.as_secs_f32().ceil() as u32;
                state::SAFE_START_UNTIL.store(secs, Ordering::Relaxed);
                log::info!(
                    "safe start: network up at {:.1} s, held until {secs} s",
                    now.as_secs_f32()
                );
                return false;
            }
        }
        if now < until {
            return false;
        }
        self.until = None;
        state::SAFE_START_UNTIL.store(0, Ordering::Relaxed);
        true
    }
}

/// Whether a transition may take its buffers now: `need` is its 24 KB plus
/// what the new file's decoder has still to allocate (a GIF's LZW state);
/// the heap must keep [`config::TRANSITION_HEAP_RESERVE`] after both, and a
/// 16 KB block (the LZW's largest piece) must exist. Called on the main task
/// inside the tick; counts refusals.
fn transition_gate(need: usize) -> bool {
    let ok = free_heap() as usize >= need + config::TRANSITION_HEAP_RESERVE
        && largest_free_block() >= 16 * 1024;
    if !ok {
        TRANSITIONS_CUT.fetch_add(1, Ordering::Relaxed);
    }
    ok
}

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    // Synchronous until `logring::start` below, like EspLogger before it.
    logring::init();

    // The version (`state::firmware_version`: the app descriptor's, which
    // `FW_VERSION` sets) and the descriptor itself (`"<version> 4mb"`).
    log::info!(
        "OpenDMD firmware v{} ({}, app desc '{}') — 128x32 HUB75 GIF player",
        state::firmware_version(),
        state::FLASH_VARIANT,
        state::running_version()
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
    //
    // Its timing comes from NVS (`panel`), or from a trial staged for this
    // one boot (`panel_try`), which is removed before the panel starts: if
    // the trial leaves the panel dark or the chip crashes, the next boot is
    // on the saved settings whatever happens (`opendmd_core::panel_timing`).
    let mut nvs_config = Config::new(nvs_partition.clone())?;
    let mut tuner = {
        let (saved, staged) = nvs_config.panel();
        if staged.is_some() {
            if let Err(e) = nvs_config.set_panel_try(None) {
                log::error!("panel timing: removing the staged trial from NVS failed ({e:#})");
            }
        }
        Tuner::boot(saved.as_deref(), staged.as_deref())
    };
    let panel_cfg = PanelConfig::new(&tuner.running.effective());
    let mut panel = Panel::new(&panel_cfg)?;
    log::info!(
        "panel up: {}x{} ({} x {}x{}), timing {} ({}), double_buff={}, hub75_refresh_rate={} Hz",
        panel_cfg.surface_width(),
        panel_cfg.height,
        panel_cfg.chain,
        panel_cfg.width,
        panel_cfg.height,
        tuner.running.to_nvs(),
        match tuner.trial {
            Some(_) => "on trial: the staged settings, for this boot only",
            None if tuner.running == PanelTiming::default() => "the defaults",
            None => "saved",
        },
        panel_cfg.double_buffer,
        panel.refresh_rate(),
    );

    if probe::ENABLED {
        panel.bench();
    }
    #[cfg(feature = "psi-probe")]
    psi_probe::run();

    // GPIO 2 (onboard LED) is deliberately untouched — see hardware/PINOUT.md.
    //
    // The GPIO peripherals are typed (`Gpio18` and friends), so the numbers in
    // `config` cannot be used to select them; `expect_pin` asserts the two
    // agree instead, which catches the map drifting out of step.
    let sck = expect_pin(AnyOutputPin::from(pins.gpio18), config::SD_SCK, "SD SCK");
    let mosi = expect_pin(AnyOutputPin::from(pins.gpio23), config::SD_MOSI, "SD MOSI");
    let miso = expect_pin(AnyIOPin::from(pins.gpio19), config::SD_MISO, "SD MISO");
    let cs = expect_pin(AnyOutputPin::from(pins.gpio5), config::SD_CS, "SD CS");
    let bus = SdBus::new(peripherals.spi3, sck, mosi, miso, cs);
    let reset_reason = reset_reason();
    log::info!("reset reason: {reset_reason}");

    // The card's records first (`sdhealth`, before any other task exists):
    // a card that failed at full speed mounts at the fallback clock, and a
    // device that has already restarted `SD_REBOOT_CAP` times for card
    // failure does not mount at boot at all but remounts every
    // `SD_REMOUNT_INTERVAL`. The mount itself is the boot task's (`boot.rs`).
    let health = {
        let poweron = matches!(reset_reason, "poweron" | "brownout");
        let health = sdhealth::boot(poweron, nvs_config.sd_slow());
        if health.clear_nvs {
            if let Err(e) = nvs_config.set_sd_slow(None) {
                log::error!("clearing the slow-clock record in NVS failed ({e:#})");
            }
        }
        health
    };

    // --- Settings ---------------------------------------------------------
    // Read before the card now: the show needs them (and the brightness, the
    // look), and nothing here touches the card. A saved playlist that no
    // longer opens is found by the boot task and cleared at the handover.
    if nvs_config.seed_credentials_from_build() {
        log::info!("first boot with build-time credentials; they are now in NVS");
    }
    let mut settings = nvs_config.load();
    // The ceiling first, so nothing below (the show included) can exceed it;
    // until safe start is over it is no higher than `BRIGHTNESS_SAFE_MAX`.
    let mut safe = SafeStart::new(&settings);
    panel.set_max_brightness(safe.ceiling(settings.max_brightness));
    panel.set_brightness(settings.brightness);
    panel.set_display(settings.look, settings.glow);
    clock::set_timezone(&settings.tz, &settings.tz_name);
    // The live clock (`@clock`) reads local time from here.
    opendmd_core::synth::set_wall_clock(Some(clock::wall_now));
    let mut sched = Scheduling::new(&settings.schedule);
    // Follow RePlayOS: the overlay and the sequence (`follow.rs`).
    let mut runner = follow::Runner::default();
    log::info!(
        "settings: brightness={} (max {}), look={}, glow={}, dwell={}..{}s, shuffle={}, platforms='{}', set='{}', transition={}, boot_show={}, tz='{}' ({}), schedule {} rule(s){}, host='{}', wifi={}",
        settings.brightness,
        settings.max_brightness,
        settings.look.name(),
        settings.glow.name(),
        settings.dwell_min,
        settings.dwell_max,
        settings.shuffle,
        settings.platforms,
        settings.set,
        settings.transition,
        settings.boot_show.name(),
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
    // A new low also notes who was running on each core (`frametime::Floor`).
    let mut floor = frametime::Floor::new();
    let mut low_water = floor.min;
    // The display's health, reported in the heartbeat and in /api/status.
    let mut deadline_misses: u64 = 0;
    let mut worst_lateness_ms: u64 = 0;
    let mut misses = frametime::Misses::default();
    // When the file on screen's first frame went up (epoch time): a miss
    // within `frametime::SWITCH_WINDOW` of it is the GIF change's.
    let mut first_shown: Option<Duration> = None;
    let mut started: u64 = 0;
    // Where an iteration's time went before its tick (see `frametime`):
    // when the last sleep ended, how much later than planned, and the
    // heartbeat's time since. `fresh`: the last iteration had no engine, so
    // there is nothing to attribute.
    let mut woke = Instant::now();
    let mut wake_us: u32 = 0;
    let mut log_us: u32 = 0;
    let mut fresh = true;
    // The file frame last published to status, `(ordinal, frame)`: a
    // transition step that decoded nothing moves the picture but not the
    // file's position.
    let mut published: (u64, u64) = (0, 0);
    // The file on screen is a prepared copy (the cache builder may borrow
    // its headroom while it plays).
    let mut current_prepared = false;
    // Glow steps taken between frames since the last frame: counted into
    // the next frame's `blit` phase if it is late (`frametime`).
    let mut glow_us: u32 = 0;

    // --- Shared state, and the boot task ----------------------------------
    let status = Arc::new(Mutex::new(Status::new(
        state::firmware_version(),
        settings.hostname.clone(),
        epoch,
    )));
    {
        let mut s = status.lock().unwrap();
        s.reset_reason = reset_reason;
        s.refresh_hz = panel.refresh_rate();
        s.tz_name = tz_label(&settings);
        s.panel = tuner;
    }
    let shared_settings = Arc::new(Mutex::new(settings.clone()));
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let (net_tx, net_rx) = mpsc::channel::<NetCommand>();
    cache::init(net_tx.clone());
    // Improv Serial (a browser installer over USB), answered from the log
    // ring's drain from here on.
    improv::init(&settings.hostname, net_tx.clone());

    // Logging goes asynchronous first (a ring drained by a CPU0 task,
    // `logring`): from here on no task waits for the UART — the show's
    // frames included.
    logring::start();
    // lwip has to exist before anything can bind a socket. Normally the WiFi
    // driver's construction does this on the worker thread, but the server is
    // started before that (by the boot task), and `httpd_start` on a stack
    // that is not up aborts with "assert failed: tcpip_send_msg_wait_sem ...
    // (Invalid mbox)". `esp_netif_init` is idempotent, so the worker's later
    // call is a no-op.
    // SAFETY: an argument-free IDF initialiser, safe to call more than once.
    esp_idf_sys::esp!(unsafe { esp_idf_sys::esp_netif_init() })?;

    // The boot task (`boot.rs`, CPU0): the card and its playlist index, the
    // HTTP server, the web app in flash, then WiFi — everything slow
    // the boot used to do here before the first frame. It hands the card and
    // the catalog over on `boot_rx`; until then this loop plays the show and
    // never touches the card (`CARD_READY`).
    let (boot_tx, boot_rx) = mpsc::channel::<boot::Event>();
    // Dropped once the card is handed over: the channel's few hundred bytes
    // are not kept for the life of the run.
    let mut boot_rx = Some(boot_rx);
    boot::spawn(boot::BootTask {
        bus,
        health,
        nvs: nvs_partition.clone(),
        ctx: Arc::new(Ctx {
            status: status.clone(),
            settings: shared_settings.clone(),
            cmd: cmd_tx.clone(),
            net: net_tx,
        }),
        net: net_worker::NetWorker {
            modem,
            sysloop,
            nvs: nvs_partition.clone(),
            ssid: settings.wifi_ssid.clone(),
            password: settings.wifi_pass.clone(),
            hostname: settings.hostname.clone(),
            status: status.clone(),
            rx: net_rx,
            settings: shared_settings.clone(),
            cmd: cmd_tx.clone(),
        },
        tx: boot_tx,
    })?;

    // `card` unmounts on drop, so it lives to the end once handed over.
    let mut card: Option<SdCard> = None;
    let mut engine: Option<Engine> = None;
    // The card failed again with no restart left: the player has stopped.
    let mut card_dead = false;
    // The start-up show, then the status card while there is no card (or
    // nothing to play from it); see `Screen`.
    let mut screen = Screen::new(settings.boot_show);
    // A handover waiting for the show's minimum.
    let mut pending: Option<Box<boot::Handover>> = None;
    // With the card but no engine: when the playlist was last tried.
    let mut last_build_try: Option<Instant> = None;
    // The panel-timing test (`POST /api/panel/test`): while it runs it owns
    // the panel and the canvas, and nothing plays.
    let mut ptest: Option<PanelTest> = None;
    // What the panel showed when the test started (a paused file's frame,
    // the status card), put back after it; `None` if the 12 KB was not there.
    let mut ptest_saved: Option<Canvas> = None;

    loop {
        // 0. Safe start: extended once when the network comes up, then the
        //    saved ceiling takes over.
        if safe.poll(epoch.elapsed()) {
            panel.set_max_brightness(settings.max_brightness);
            log::info!("safe start: over, brightness {}", panel.brightness());
        }
        // A panel-timing trial whose time is up goes back to the saved
        // settings (live, or by a restart after the replies below).
        if let Some(fx) = tuner.poll(epoch.elapsed(), state::NET_UP.load(Ordering::Relaxed)) {
            log::warn!("panel timing: trial not kept within {} s, undoing it", panel_timing::TRIAL.as_secs());
            panel_effects(fx, &tuner, &mut panel, &mut nvs_config, &status);
        }

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
                Command::ReleaseAhead(reply) => {
                    if let Some(e) = engine.as_mut() {
                        ahead::release(e);
                    }
                    let _ = reply.send(());
                }
                Command::TogglePause(reply) => {
                    let now = epoch.elapsed();
                    let paused = match engine.as_mut() {
                        Some(e) => {
                            e.toggle_pause(now);
                            if e.is_paused() {
                                // Let the file go — its decoder is a GIF's
                                // ~54 KB, which the cache builder (full
                                // speed while paused) and any download want
                                // more than a frozen frame does. The panel
                                // keeps showing it: the DMA buffer holds the
                                // frame, and the canvas too, so resuming
                                // opens the *next* file with the configured
                                // transition from it rather than carrying on
                                // with this one.
                                e.drop_current(&mut canvas);
                            }
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
                    let outcome = play_now(engine.as_mut(), &path, epoch.elapsed());
                    match &outcome {
                        Ok(()) => log::info!("play now: {path}"),
                        Err(e) => log::warn!("play now: {e:?}"),
                    }
                    let _ = reply.send(outcome);
                }
                Command::Back(reply) => {
                    // The history lives in the status (one copy); its start
                    // is recognised when published below.
                    let target = status.lock().unwrap().recent.back();
                    let outcome = match target {
                        Some(path) => {
                            let r = play_now(engine.as_mut(), &path, epoch.elapsed());
                            match &r {
                                Ok(()) => log::info!("previous: {path}"),
                                Err(e) => log::warn!("previous: {e:?}"),
                            }
                            r
                        }
                        None => Err(PlayError::NotFound("nothing earlier".into())),
                    };
                    if outcome.is_err() {
                        status.lock().unwrap().recent.back_failed();
                    }
                    let _ = reply.send(outcome);
                }
                Command::ApplyConfig {
                    settings: next,
                    panel: panel_op,
                    reply,
                } => {
                    let next = *next;
                    // A settings change ends the panel test first: its live
                    // timing and the look go back before anything applies.
                    end_panel_test(&mut ptest, &mut ptest_saved, &mut panel, &mut canvas, &settings, "a settings change");
                    // A confirm must find a trial running (it may have just
                    // reverted): refused before anything is saved.
                    if panel_op == Some(PanelOp::Confirm) && tuner.trial.is_none() {
                        let _ = reply.send(Err(panel_timing::NO_TRIAL.to_string()));
                        continue;
                    }
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
                        let opens = if CARD_READY.load(Ordering::Relaxed) {
                            File::open(&path).map(drop)
                        } else {
                            Err(std::io::Error::other(CARD_NOT_READY))
                        };
                        if let Err(e) = opens {
                            log::warn!("config: set '{}' rejected, {path}: {e}", next.set);
                            let _ = reply.send(Err(state::SET_NOT_FOUND.to_string()));
                            continue;
                        }
                    }
                    // The attract playlist likewise.
                    if let Some(a) = next.attract_set.as_ref().filter(|a| !a.is_empty()) {
                        if next.attract_set != settings.attract_set {
                            let path = nvs_config::manifest_path_for(a);
                            if nvs_config::is_obsolete_set(a)
                                || !CARD_READY.load(Ordering::Relaxed)
                                || File::open(&path).is_err()
                            {
                                log::warn!("config: attract playlist '{a}' rejected ({path})");
                                let _ = reply.send(Err(state::SET_NOT_FOUND.to_string()));
                                continue;
                            }
                        }
                    }
                    // What plays is the schedule's overlay on the saved
                    // settings: a rule's playlist wins over `set` while it
                    // is in force, so compare what would actually play.
                    // Follow RePlayOS's attract playlist is an overlay too.
                    let (eff_now, eff_next) = (
                        runner.overlay(sched.effective(&settings)),
                        runner.overlay(sched.effective(&next)),
                    );
                    let needs_reindex = eff_next.platforms != eff_now.platforms
                        || eff_next.set != eff_now.set
                        || eff_next.shuffle != eff_now.shuffle
                        // Attract shows the clock or a text instead now.
                        || (runner.mode() == follow::Mode::Attract
                            && next.attract != settings.attract);
                    let tz_changed = next.tz != settings.tz || next.tz_name != settings.tz_name;

                    // A platform filter that would leave the playlist empty
                    // is refused before anything is saved. The build is the
                    // only count there is, so the new playlist is built
                    // first. If that build fails in any way the settings are
                    // not verified and nothing is saved: the previous
                    // playlist is built again (its `.idx` was just replaced:
                    // another ~6 s on the stock card), or, when the build
                    // needs a fresh heap (`NeedsFreshHeap` set
                    // REBOOT_PENDING), the restart after this reply comes
                    // back on the old settings, which are still in NVS.
                    let mut prebuilt = None;
                    if needs_reindex
                        && CARD_READY.load(Ordering::Relaxed)
                        && !eff_next.platform_list().is_empty()
                        && (eff_next.platforms != eff_now.platforms || eff_next.set != eff_now.set)
                    {
                        let tried =
                            replay(&mut engine, &next, &sched, &mut runner, false, &status, &mut canvas);
                        if let Err(e) = &tried {
                            let restart = REBOOT_PENDING.load(Ordering::Relaxed);
                            let back = if restart {
                                Ok(Applied::unchanged(0))
                            } else {
                                replay(&mut engine, &settings, &sched, &mut runner, false, &status, &mut canvas)
                            };
                            log::warn!(
                                "config: platforms '{}' in {} not verified ({e}); refused, nothing saved, previous playlist back: {back:?}",
                                next.platforms,
                                eff_next.manifest_path()
                            );
                            let msg = if e.ends_with(NO_ENTRIES) {
                                format!("{} \"{}\"; nothing was saved", state::FILTER_EMPTY, next.platforms.trim())
                            } else if restart {
                                "the new playlist needs a restart to be built; restarting on the previous settings, nothing was saved".to_string()
                            } else {
                                format!("the new playlist could not be built ({e}); nothing was saved")
                            };
                            let _ = reply.send(Err(msg));
                            continue;
                        }
                        prebuilt = Some(tried);
                    }

                    match nvs_config.save(&next) {
                        Err(e) => {
                            if prebuilt.is_some() {
                                // Playing what was not saved: back to what was.
                                let _ = replay(
                                    &mut engine,
                                    &settings,
                                    &sched,
                                    &mut runner,
                                    false,
                                    &status,
                                    &mut canvas,
                                );
                            }
                            let _ = reply.send(Err(format!("saving settings: {e:#}")));
                        }
                        Ok(()) => {
                            settings = next;
                            *shared_settings.lock().unwrap() = settings.clone();
                            if let Some(op) = panel_op {
                                match tuner.apply(op, epoch.elapsed()) {
                                    Ok(fx) => {
                                        let pending = match (tuner.trial, tuner.staged) {
                                            (Some(t), _) => format!("on trial until {} s", t.until.as_secs()),
                                            (None, Some(st)) => format!("{} staged for the next boot", st.to_nvs()),
                                            (None, None) if tuner.restart_needed() => "a restart applies the rest".into(),
                                            (None, None) => "nothing pending".into(),
                                        };
                                        log::info!(
                                            "panel timing: {op:?} -> running {}, saved {}, {pending}",
                                            tuner.running.to_nvs(),
                                            tuner.saved.to_nvs()
                                        );
                                        panel_effects(fx, &tuner, &mut panel, &mut nvs_config, &status);
                                    }
                                    Err(e) => log::warn!("panel timing: {e}"),
                                }
                            }
                            // Brightness and dwell apply to what is on screen
                            // right now; a filter, set or order change means a
                            // new catalog — rebuilt in place, never a reboot.
                            // (Under safe start's ceiling while it lasts: a
                            // lower brightness applies now, a higher one when
                            // it ends.)
                            panel.set_max_brightness(safe.ceiling(settings.max_brightness));
                            if !sched.off() {
                                panel.set_brightness(sched.brightness(&settings));
                            }
                            // The tint and the glow: the panel repainted
                            // at once if what it shows no longer matches
                            // (a dark panel is repainted when it lights).
                            if panel.set_display(settings.look, settings.glow) && !sched.off() {
                                let shown = match engine.as_mut() {
                                    Some(e) => e.display_mut(&mut canvas),
                                    None => &mut canvas,
                                };
                                shown.dirty_mut().mark_all();
                                panel.blit(shown);
                            }
                            state::GLOW_HELD.store(panel.glow_bytes() > 0, Ordering::Relaxed);
                            // (Attract's clock or text plays for as long as
                            // attract lasts: not the playlist's dwell.)
                            let live = state::ATTRACT_LIVE.load(Ordering::Relaxed);
                            if let Some(e) = engine.as_mut().filter(|_| !live) {
                                e.set_dwell(dwell_policy(&settings));
                                e.set_transition(settings.transition);
                                // Night mode with the builder newly allowed:
                                // it wants the paused file's heap.
                                if sched.off()
                                    && settings.auto_prepare != nvs_config::AutoPrepare::Off
                                {
                                    e.drop_current(&mut canvas);
                                }
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
                                "config applied (reindex={needs_reindex}): brightness={}, look={}, glow={}, dwell={}..{}s, shuffle={}, platforms='{}', set='{}', transition={}, attract='{}'",
                                settings.brightness,
                                settings.look.name(),
                                settings.glow.name(),
                                settings.dwell_min,
                                settings.dwell_max,
                                settings.shuffle,
                                settings.platforms,
                                settings.set,
                                settings.transition,
                                settings.attract
                            );
                            let outcome = if let Some(built) = prebuilt {
                                // Built above, before the save.
                                built
                            } else if needs_reindex && !CARD_READY.load(Ordering::Relaxed) {
                                // No card yet: saved, and the playlist the
                                // card brings is built with it (the
                                // handover compares, the boot task reads
                                // the settings when a late card mounts).
                                Ok(Applied::unchanged(0))
                            } else if needs_reindex {
                                // A current `.idx` for the new set/filter is
                                // reused; anything else is rebuilt.
                                replay(
                                    &mut engine,
                                    &settings,
                                    &sched,
                                    &mut runner,
                                    false,
                                    &status,
                                    &mut canvas,
                                )
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
                    let outcome = replay(
                        &mut engine,
                        &settings,
                        &sched,
                        &mut runner,
                        true,
                        &status,
                        &mut canvas,
                    );
                    log::info!("reindex requested over HTTP: {outcome:?}");
                    let _ = reply.send(outcome);
                }
                Command::InstallManifest { temp, dest, reply } => {
                    // (Before the handover the card is the boot task's.)
                    if !CARD_READY.load(Ordering::Relaxed) {
                        let _ = reply.send(Err(CARD_NOT_READY.to_string()));
                        continue;
                    }
                    // The engine may hold `dest` and `dest.idx` open, and FAT
                    // must not have an open file replaced under it — so drop it
                    // first. Then the sidecar rule: the stale `.idx` is deleted
                    // *before* the new manifest lands (see
                    // `files::remove_index_beside` for why that order), and the
                    // index is rebuilt if this is the manifest in play.
                    // Compared with the manifest actually in play.
                    let active = files::same_card_path(&status.lock().unwrap().manifest, &dest);
                    if active {
                        retire(&mut engine, &mut canvas);
                    }
                    let outcome = files::remove_index_beside(&dest)
                        .and_then(|()| files::replace(&temp, &dest))
                        .map_err(|e| {
                            let _ = std::fs::remove_file(&temp);
                            format!("{e:#}")
                        })
                        .and_then(|()| {
                            if active {
                                replay(
                                    &mut engine,
                                    &settings,
                                    &sched,
                                    &mut runner,
                                    true,
                                    &status,
                                    &mut canvas,
                                )
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
                    if !CARD_READY.load(Ordering::Relaxed) {
                        let _ = reply.send(Err(CARD_NOT_READY.to_string()));
                        continue;
                    }
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
                    let (active, on_screen) = {
                        let s = status.lock().unwrap();
                        (files::same_card_path(&s.manifest, &manifest), s.path.clone())
                    };
                    if active {
                        retire(&mut engine, &mut canvas);
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
                                replay(
                                    &mut engine,
                                    &settings,
                                    &sched,
                                    &mut runner,
                                    false,
                                    &status,
                                    &mut canvas,
                                )
                            } else {
                                Ok(Applied::unchanged(
                                    engine.as_ref().map_or(0, |e| e.catalog_len()),
                                ))
                            }
                        });
                    // The GIF on screen carries on (starting over), so a
                    // favourite tapped on Now doesn't jump the panel to
                    // another one; unless the new overrides hide it.
                    if active && outcome.is_ok() && !on_screen.is_empty() && !on_screen.starts_with('@') {
                        let hidden = read_meta(&dest).get(&on_screen).is_some_and(|o| o.never);
                        if !hidden {
                            if let Err(e) = play_now(engine.as_mut(), &on_screen, epoch.elapsed()) {
                                log::warn!("overrides: {on_screen} did not carry on: {e:?}");
                            }
                        }
                    }
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
                Command::Follow(event) => {
                    let was_context = runner.context().is_some();
                    let todo = runner.handle(event, engine.as_mut(), sched.off(), epoch.elapsed());
                    follow_todo(
                        todo,
                        was_context,
                        &mut engine,
                        &settings,
                        &sched,
                        &mut runner,
                        &status,
                        &mut canvas,
                    );
                }
                Command::PanelTest { setting, reply } => {
                    let refused = setting.and_then(|s| panel_test::refusal(s, &tuner, sched.off()));
                    let outcome = match (setting, refused) {
                        (Some(_), Some(why)) => Err(why.to_string()),
                        (Some(s), None) => {
                            if ptest.is_none() {
                                // The file on screen goes (its decoder's heap
                                // back; the next one opens after the test),
                                // the picture is kept to put back (a paused
                                // file is not redrawn), and the patterns go
                                // out untinted, unglowed.
                                if let Some(e) = engine.as_mut() {
                                    e.drop_current(&mut canvas);
                                }
                                ptest_saved = snapshot(&canvas);
                                panel.set_display(Look::Colour, GlowLevel::Off);
                            }
                            let t = PanelTest::start(s, tuner.running, epoch.elapsed());
                            log::info!(
                                "panel test: {} A {} / B {} for {} s",
                                s.name(),
                                t.a.to_nvs(),
                                t.b.to_nvs(),
                                panel_test::LIMIT.as_secs()
                            );
                            ptest = Some(t);
                            Ok(Applied::unchanged(0))
                        }
                        (None, _) => {
                            end_panel_test(&mut ptest, &mut ptest_saved, &mut panel, &mut canvas, &settings, "stopped");
                            Ok(Applied::unchanged(0))
                        }
                    };
                    let _ = reply.send(outcome);
                }
            }
        }

        // Follow RePlayOS's sequence: the next step once the engine has
        // finished the one on screen (it holds the last frame meanwhile).
        if runner.in_sequence() && !sched.off() && ptest.is_none() {
            if let Some(e) = engine.as_mut() {
                let todo = runner.advance(e, epoch.elapsed());
                follow_todo(
                    todo,
                    false,
                    &mut engine,
                    &settings,
                    &sched,
                    &mut runner,
                    &status,
                    &mut canvas,
                );
            }
        }

        // The schedule: at most once a second look at the clock, and act only
        // when the local minute (or the schedule, zone or a wake) changed.
        if let Some(next) = sched.poll() {
            // A change (night mode's dark panel, a rule's playlist or
            // brightness) ends the panel test first, then applies.
            end_panel_test(&mut ptest, &mut ptest_saved, &mut panel, &mut canvas, &settings, "a schedule change");
            apply_schedule(
                next,
                &mut sched,
                &mut runner,
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

        // The panel-timing test: its pictures and live timing instead of
        // the engine or the status card (neither is ticked, so nothing goes
        // into Recently played), until it is stopped or its time is up.
        // After the schedule and the reboot check: a schedule change ends
        // it (above), and a pending restart is not held up.
        if let Some(t) = ptest.as_mut() {
            match t.step(epoch.elapsed()) {
                panel_test::Step::Draw {
                    pattern,
                    side,
                    timing,
                    ..
                } => {
                    panel.set_live_timing(&timing.effective());
                    panel_test::draw(&mut canvas, pattern, side);
                    panel.blit(&mut canvas);
                }
                panel_test::Step::Wait(_) => {}
                panel_test::Step::End => {
                    end_panel_test(&mut ptest, &mut ptest_saved, &mut panel, &mut canvas, &settings, "time is up")
                }
            }
            if ptest.is_some() {
                fresh = true;
                if let Ok(mut s) = status.lock() {
                    s.uptime_secs = epoch.elapsed().as_secs();
                    s.heap_free = free_heap();
                }
                thread::sleep(config::OFF_SLEEP);
                continue;
            }
        }

        // The card is failing (`sdhealth`): restart, or stop playing.
        if sdhealth::tripped() {
            if let Some(c) = card.as_ref().filter(|_| !card_dead) {
                card_failed(c, &mut nvs_config, &status);
                // Still here: no restart left. Let go of the card's files.
                // (The cache builder backs off from card errors by itself.)
                retire(&mut engine, &mut canvas);
                card_dead = true;
            }
        }
        // A command (a playlist change, `play`) may have built a new engine
        // on the stopped card since: let it go again.
        if card_dead && engine.is_some() {
            retire(&mut engine, &mut canvas);
        }

        // The boot task's news: no card for now, or the card and its
        // playlist — taken over once the show has run its minimum.
        if let Some(rx) = boot_rx.as_ref().filter(|_| pending.is_none()) {
            match rx.try_recv() {
                Ok(boot::Event::NoCard) => screen.no_card(),
                Ok(boot::Event::Ready(h)) => {
                    screen.ready();
                    pending = Some(h);
                    boot_rx = None;
                }
                Err(_) => {}
            }
        }
        if let Some(h) = pending.take_if(|_| screen.may_end()) {
            let boot::Handover {
                card: mounted,
                built,
                manifest,
                platforms,
                index_ms,
                healed_set,
            } = *h;
            card = Some(mounted);
            CARD_READY.store(true, Ordering::Relaxed);
            // The saved playlist would not open: forget it (NVS is ours).
            if healed_set.is_some_and(|set| set == settings.set) {
                settings.set.clear();
                if let Err(e) = nvs_config.save(&settings) {
                    log::error!("clearing the set in NVS failed ({e:#}); the default plays for this boot only");
                }
                *shared_settings.lock().unwrap() = settings.clone();
            }
            // Still what should play? A schedule rule, Follow RePlayOS or
            // a filter change since the boot task read the settings means
            // another playlist: built here, the card being ours now.
            let eff = runner.overlay(sched.effective(&settings));
            let same = runner.context().is_none()
                && attract_live(&runner, &eff).is_none()
                && files::same_card_path(&eff.manifest_path(), &manifest)
                && eff.platforms == platforms;
            let outcome = if same {
                status.lock().unwrap().manifest = manifest.clone();
                finish_build(&mut engine, &eff, &manifest, built, index_ms, &status)
            } else {
                drop(built);
                replay(
                    &mut engine,
                    &settings,
                    &sched,
                    &mut runner,
                    false,
                    &status,
                    &mut canvas,
                )
            };
            if let Err(e) = &outcome {
                screen.playlist_problem(e);
            }
            last_build_try = Some(Instant::now());
            screen.handed_over(engine.is_some());
        }

        // 2. No engine: the start-up show or the status card (booting, no
        //    card, nothing to play on it, a card that failed), and the
        //    playlist tried again every so often.
        let Some(eng) = engine.as_mut() else {
            fresh = true;
            cache_builder::publish(
                cache_builder::Pace::Stop(cache_builder::Reason::NoPlaylist),
                settings.auto_prepare,
            );
            // The card failed with no restart left: nothing plays. One
            // restart once the budget's window has run out.
            if card_dead && sdhealth::window_left_ms() == 0 {
                log::error!("microSD: the card-failure window has run out; restarting to try the card again");
                sdhealth::restart_after_window();
                restart_for_card();
            }
            // The card, and nothing playing from it: the playlist again
            // every `RETRY_INTERVAL` (the status card says why meanwhile).
            if card.is_some()
                && !card_dead
                && last_build_try.is_none_or(|t| t.elapsed() >= config::RETRY_INTERVAL)
            {
                last_build_try = Some(Instant::now());
                let eff = sched.effective(&settings);
                match replay(
                    &mut engine,
                    &settings,
                    &sched,
                    &mut runner,
                    false,
                    &status,
                    &mut canvas,
                ) {
                    Ok(_) => {
                        screen.handed_over(true);
                        continue;
                    }
                    Err(e) => {
                        screen.playlist_problem(&e);
                        // A schedule rule's playlist that will not build is
                        // set aside (until the rule changes) and the saved
                        // one plays.
                        if eff.set != settings.set {
                            log::warn!(
                                "schedule: playlist '{}' will not open; playing the saved one",
                                eff.set
                            );
                            sched.broken_set = Some(eff.set.clone());
                            sched.publish(&status);
                            last_build_try = None;
                            continue;
                        }
                        // The set may have gone missing under us (deleted
                        // off-device, card swapped): fall back to the default
                        // at once rather than retrying a manifest that is not
                        // there.
                        if heal_missing_set(&mut settings, &mut nvs_config) {
                            *shared_settings.lock().unwrap() = settings.clone();
                            last_build_try = None;
                            continue;
                        }
                    }
                }
            }
            let wait = if sched.off() {
                // Night mode "off": the panel stays dark.
                panel.glow_settle();
                config::OFF_SLEEP
            } else {
                let problem = if card_dead {
                    Some(CARD_FAILING)
                } else {
                    card.is_some().then_some(screen.playlist_note)
                };
                let wait = screen.frame(&mut panel, &mut canvas, &status, problem);
                if screen.drawn() {
                    FRAME_SHOWN.store(true, Ordering::Relaxed);
                }
                wait
            };
            if let Ok(mut s) = status.lock() {
                s.uptime_secs = epoch.elapsed().as_secs();
                s.heap_free = free_heap();
            }
            thread::sleep(wait);
            continue;
        };

        screen.playing();

        // 3. One frame. What the loop did since the last sleep ended is
        //    the frame's `other` (less the heartbeat's `log`, and any card
        //    work a command did, which is `open`/`read`): see `frametime`.
        let entries = eng.catalog_len();
        probe::tick_begin();
        let (pre_open, pre_read) = frametime::take();
        let tick_start = Instant::now();
        if fresh {
            fresh = false;
            (woke, wake_us, log_us) = (tick_start, 0, 0);
        }
        let pre_us = us(tick_start.duration_since(woke));
        let tick_now = epoch.elapsed();
        let due_in_us = if probe::ENABLED {
            eng.frame_due()
                .map(|d| d.as_micros() as i64 - tick_now.as_micros() as i64)
        } else {
            None
        };
        probe::tick_due(due_in_us);
        // Night mode "off": the panel is dark and nothing is decoded.
        let off = sched.off();
        let outcome = if off {
            TickOutcome::Idle
        } else {
            eng.tick(tick_now, &mut canvas)?
        };
        let tick_us = us(tick_start.elapsed());
        let (open_us, read_us) = frametime::take();
        floor.sample();
        // What goes to /api/status from this frame: the file's position
        // (content time, see `NowPlaying::elapsed`) and frame number, when the
        // frame went up and when the next is due — all values the loop holds
        // anyway, published with the fields below in the lock step 4 already
        // takes. The browser's preview syncs its clock to these.
        let mut shown = None;
        let mut new_file = None;
        let mut exhausted = false;
        let mut judged = None;
        // The next frame's due time, when this iteration may step the next
        // file ahead after it (a file frame on time, no transition).
        let mut slot = None;
        let sleep_for = match outcome {
            TickOutcome::Frame {
                playing,
                started: is_new,
                next_frame_at,
                transitioning,
            } => {
                // The composited picture while a transition runs.
                if is_new {
                    // The glow's copy may fit now if it did not before.
                    panel.glow_retry();
                }
                let blit_start = Instant::now();
                let blit_px = panel.blit(eng.display_mut(&mut canvas));
                let blit_us = us(blit_start.elapsed());
                let free = free_heap();
                floor.note(free);
                FRAME_SHOWN.store(true, Ordering::Relaxed);
                let decoded = (playing.ordinal, playing.frame) != published;
                // The loop's own lines (a GIF started, a heap low-water
                // mark) are logged here: formatted into the log ring, not
                // written to the UART (`logring`).
                let log_start = Instant::now();
                if is_new {
                    started = playing.ordinal;
                    // (A virtual file — Follow RePlayOS's cards — opened
                    // nothing: the last open was the previous file's.)
                    let served = if opendmd_core::synth::is_virtual(&playing.path) {
                        None
                    } else {
                        sd::take_last()
                    };
                    current_prepared = match served {
                        Some(sd::Served::Prepared { .. } | sd::Served::Ahead { .. }) => true,
                        // A playlist line naming a copy under `cache/`.
                        Some(sd::Served::Direct) => matches!(
                            opendmd_core::layout::classify(&playing.path),
                            opendmd_core::layout::Prepared::Direct
                        ),
                        _ => false,
                    };
                    log_started(playing.ordinal, &playing.path, served);
                    probe::switch_frame(tick_us, eng.started_ahead());
                }
                // Attribute every new low-water mark to the GIF on screen, so
                // a heap regression names the file that caused it.
                if floor.min + 2048 < low_water {
                    log::warn!(
                        "heap low-water {low_water} B -> {} B during [{}] {}",
                        floor.min,
                        playing.ordinal,
                        playing.path
                    );
                    low_water = floor.min;
                }
                let post_log_us = us(log_start.elapsed());
                // Hold the frame for its GIF delay. Read the clock again,
                // because the blit itself took time.
                let now = epoch.elapsed();
                judged = Some(Instant::now());
                if is_new {
                    first_shown = Some(now);
                }
                // The deadline is the file's next frame. A transition step
                // in between (`next_frame_at` is then earlier) is not one:
                // a step a few ms late shows nothing different, and if its
                // work makes the file's frame late, that frame counts.
                let deadline = eng.frame_due().unwrap_or(next_frame_at).max(next_frame_at);
                if probe::ENABLED {
                    let times = probe::FrameTimes {
                        tick_us,
                        blit_us,
                        blit_px,
                        log_us: post_log_us,
                        work_us: now.saturating_sub(tick_now).as_micros() as u32,
                        delay_ms: next_frame_at.saturating_sub(tick_now).as_millis() as u64,
                        due_in_us: due_in_us.unwrap_or(0),
                    };
                    let late_ms =
                        (now > deadline).then(|| now.saturating_sub(deadline).as_millis() as u64);
                    probe::frame(&times, late_ms, is_new, &playing.path);
                    probe::transition_frame(transitioning, decoded, &times, free);
                }
                // The file's position moves only with its own frames, and
                // its hold is its own delay, not a transition step's.
                if decoded {
                    published = (playing.ordinal, playing.frame);
                    let due = eng.frame_due().unwrap_or(next_frame_at);
                    shown = Some((playing.elapsed, playing.frame, now, due));
                }
                if is_new {
                    new_file = Some(playing);
                }
                if decoded && !transitioning && now < deadline {
                    slot = Some(deadline);
                }
                let glow_between = std::mem::take(&mut glow_us);
                // A dot fading (glow): wake for its next step.
                let glow_wait = panel.glow_wait().map(|w| w.max(Duration::from_millis(1)));
                match next_frame_at.checked_sub(now) {
                    Some(remaining) => glow_wait.map_or(remaining, |w| remaining.min(w)),
                    None if now <= deadline => config::CATCHUP_SLEEP,
                    None => {
                        // Already late. This is the acceptance metric: count it
                        // and remember the worst, then yield a tick so the
                        // watchdog and every other task get their turn.
                        deadline_misses += 1;
                        let late = now.saturating_sub(deadline).as_millis() as u64;
                        worst_lateness_ms = worst_lateness_ms.max(late);
                        let phases = frametime::Phases {
                            wake: wake_us,
                            log: log_us + post_log_us,
                            other: pre_us.saturating_sub(log_us + pre_open + pre_read),
                            open: pre_open + open_us,
                            read: pre_read + read_us,
                            decode: tick_us.saturating_sub(open_us + read_us),
                            // Glow steps between frames count as blit.
                            blit: blit_us + glow_between,
                            transitioning,
                        };
                        let when = frametime::when(
                            is_new,
                            transitioning,
                            first_shown.map(|t| now.saturating_sub(t)),
                        );
                        misses.record(phases.cause(), late, when);
                        config::CATCHUP_SLEEP
                    }
                }
            }
            TickOutcome::Idle if off => config::OFF_SLEEP,
            TickOutcome::Idle => {
                // Between frames: the glow's next step, if one is due.
                glow_us += panel.glow_tick(eng.display(&canvas));
                // A wake before the file's frame (a glow step's) sleeps no
                // further than the frame: a flat 5 ms made it up to 5 ms
                // late, which nothing counted.
                let due = eng.frame_due().filter(|_| !eng.is_paused());
                match due.and_then(|d| d.checked_sub(epoch.elapsed())) {
                    Some(left) if !left.is_zero() => left.min(config::IDLE_SLEEP),
                    _ => config::IDLE_SLEEP,
                }
            }
            TickOutcome::Exhausted => {
                log::error!(
                    "playlist exhausted — nothing opened. Reshuffling in {}s",
                    config::RETRY_INTERVAL.as_secs()
                );
                exhausted = true;
                config::RETRY_INTERVAL
            }
        };

        let judged = judged.unwrap_or_else(Instant::now);

        // The next file, ahead: a held copy let go on a card error or low
        // heap; then the steps that fit before the next frame (`ahead.rs`).
        ahead::guard(eng);
        let stepped = match slot {
            Some(deadline) if !off => ahead::between_frames(eng, deadline, &epoch),
            _ => false,
        };

        // Nothing moving (paused, or dark): once the fade is over the
        // glow's 12 KB goes back (the builder works at full speed now).
        if off || eng.is_paused() {
            panel.glow_settle();
        }
        // And whenever the heap runs low: the network side comes first.
        if panel.glow_yield(eng.display(&canvas)) {
            log::info!(
                "glow: heap low ({} B free), its copy given back",
                free_heap()
            );
        }
        state::GLOW_HELD.store(panel.glow_bytes() > 0, Ordering::Relaxed);

        // The cache builder's pace for what the panel is doing now.
        cache_builder::publish(
            builder_pace(&settings, off, eng, current_prepared),
            settings.auto_prepare,
        );

        // 4. Publish, before the sleep, so a status read during the frame's
        //    hold sees the frame that is on the glass. Cheap fields only: the
        //    strings move when a GIF starts.
        if let Ok(mut s) = status.lock() {
            s.uptime_secs = epoch.elapsed().as_secs();
            s.entries = entries;
            s.heap_free = free_heap();
            s.heap_floor = floor.min;
            s.heap_floor_at = floor.at;
            s.deadline_misses = deadline_misses;
            s.worst_lateness_ms = worst_lateness_ms;
            s.misses = misses;
            if let Some(playing) = new_file {
                s.recent.started(&playing.path);
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

        // The sleep should end at `judged + sleep_for`; anything after that
        // (publishing above, preemption on CPU1) is the next frame's `wake`.
        let planned = judged + sleep_for;
        thread::sleep(if stepped {
            planned.saturating_duration_since(Instant::now())
        } else {
            sleep_for
        });
        woke = Instant::now();
        wake_us = us(woke.saturating_duration_since(planned));
        log_us = 0;
        if exhausted {
            // The index is still good: reseed and rewind, don't re-read.
            eng.reset(random_seed());
        } else if shown.is_some() {
            floor.sample();
        }

        if last_heartbeat.elapsed() >= config::HEARTBEAT_INTERVAL {
            last_heartbeat = Instant::now();
            heartbeat(
                &mut panel,
                &status,
                started,
                &floor,
                deadline_misses,
                worst_lateness_ms,
                &misses,
                &(sched.summary() + &follow::summary() + &ahead::summary(Some(eng))),
            );
            log_us = us(last_heartbeat.elapsed());
        }
    }
}

/// Act on what Follow RePlayOS's runner asked for: a rebuild onto the
/// context index or the overlay's playlist — skipped when that is the
/// playlist already playing (a mode change that does not change it).
#[allow(clippy::too_many_arguments)]
fn follow_todo(
    todo: follow::Todo,
    was_context: bool,
    engine: &mut Option<Engine>,
    settings: &Settings,
    sched: &Scheduling,
    runner: &mut follow::Runner,
    status: &Mutex<Status>,
    canvas: &mut Canvas,
) {
    if todo != follow::Todo::Rebuild {
        return;
    }
    if runner.context().is_none() && !was_context {
        let base = sched.effective(settings);
        let playing = status.lock().unwrap().manifest.clone();
        // (The live item shown is always the one `attract` names: a
        // change to it is a config change, which rebuilds.)
        let live_now = state::ATTRACT_LIVE.load(Ordering::Relaxed);
        let same = match attract_live(runner, &base) {
            Some(_) => live_now,
            None => {
                !live_now && files::same_card_path(&runner.overlay(base).manifest_path(), &playing)
            }
        };
        if engine.is_some() && same {
            return;
        }
    }
    let outcome = replay(engine, settings, sched, runner, false, status, canvas);
    if let Err(e) = outcome {
        log::warn!("follow: rebuild failed: {e}");
    }
}

/// Rebuild what should be playing now: Follow RePlayOS's context index when
/// the runner is on one, else the saved settings under the schedule's and
/// then Follow's overlays (the attract playlist). A context that will not
/// open, or an attract playlist that will not build, is set aside and the
/// next thing down plays.
#[allow(clippy::too_many_arguments)]
fn replay(
    engine: &mut Option<Engine>,
    settings: &Settings,
    sched: &Scheduling,
    runner: &mut follow::Runner,
    force: bool,
    status: &Mutex<Status>,
    canvas: &mut Canvas,
) -> Result<Applied, String> {
    let base = sched.effective(settings);
    if let Some(ctx) = runner.context().cloned() {
        match rebuild_context(engine, &base, &ctx, status, canvas) {
            Ok(a) => return Ok(a),
            Err(e) => {
                log::warn!(
                    "follow: context {} will not play ({e}); attract instead",
                    ctx.index
                );
                runner.context_failed();
            }
        }
    }
    if let Some(live) = attract_live(runner, &base) {
        match rebuild_live(engine, &live, &base, status, canvas) {
            Ok(a) => return Ok(a),
            Err(e) => {
                log::warn!("follow: attract {live} will not play ({e}); the playlist instead");
                runner.attract_failed();
            }
        }
    }
    let eff = runner.overlay(base.clone());
    let outcome = rebuild(engine, &eff, force, status, canvas);
    if outcome.is_err() && eff.set != base.set {
        log::warn!(
            "follow: attract playlist '{}' will not open; playing the usual one",
            eff.set
        );
        runner.attract_failed();
        return rebuild(engine, &base, force, status, canvas);
    }
    outcome
}

/// What Follow RePlayOS's attract mode shows instead of a playlist, if the
/// `attract` setting names a live item (`clock`, `text:<message>`) and
/// attract is what the runner is in.
fn attract_live(runner: &follow::Runner, settings: &Settings) -> Option<String> {
    (runner.mode() == follow::Mode::Attract && !runner.attract_is_broken())
        .then(|| nvs_config::attract_path(&settings.attract))
        .flatten()
}

/// A playlist of one path, held as the path: the attract mode's live item.
struct OneEntry(String);

impl Catalog for OneEntry {
    fn len(&self) -> usize {
        1
    }

    fn entry(&mut self, _i: usize) -> std::io::Result<opendmd_core::manifest::Entry> {
        Ok(opendmd_core::manifest::Entry::from_path(&self.0))
    }
}

/// An engine over one virtual file (`@clock`, `@text?msg=…`), played for as
/// long as attract lasts: a playlist of that one path, no card involved, no
/// dwell.
fn rebuild_live(
    engine: &mut Option<Engine>,
    path: &str,
    settings: &Settings,
    status: &Mutex<Status>,
    canvas: &mut Canvas,
) -> Result<Applied, String> {
    cache_builder::publish(
        cache_builder::Pace::Stop(cache_builder::Reason::Reindex),
        settings.auto_prepare,
    );
    retire(engine, canvas);
    let catalog = OneEntry(path.to_string());
    let mut e = engine_over(Box::new(catalog), &Overrides::default(), settings, 1);
    e.set_dwell(DwellPolicy {
        min: Duration::MAX,
        max: Duration::MAX,
        min_loops: u32::MAX,
    });
    *engine = Some(e);
    log::info!("follow: attract shows {path}, heap {} B free", free_heap());
    let mut s = status.lock().unwrap();
    // `manifest` stays the playlist attract would otherwise play (what the
    // web app offers new GIFs to); the live item is noted beside it.
    s.manifest = settings.manifest_path();
    state::ATTRACT_LIVE.store(true, Ordering::Relaxed);
    s.entries = 1;
    s.overrides = 0;
    s.hidden = 0;
    s.index_state = "ready";
    Ok(Applied::unchanged(1))
}

/// Rebuild the engine over a Follow RePlayOS context index — built (and
/// checked) by the network worker, so this only opens two files and reads
/// the weights back: milliseconds, between frames.
fn rebuild_context(
    engine: &mut Option<Engine>,
    settings: &Settings,
    ctx: &follow::Context,
    status: &Mutex<Status>,
    canvas: &mut Canvas,
) -> Result<Applied, String> {
    if !CARD_READY.load(Ordering::Relaxed) {
        return Err(CARD_NOT_READY.to_string());
    }
    cache_builder::publish(
        cache_builder::Pace::Stop(cache_builder::Reason::Reindex),
        settings.auto_prepare,
    );
    retire(engine, canvas);
    {
        let mut s = status.lock().unwrap();
        s.manifest = ctx.manifest.clone();
        state::ATTRACT_LIVE.store(false, Ordering::Relaxed);
        s.index_state = "building";
    }
    let started = Instant::now();
    let built = (|| -> Result<(Engine, Overrides)> {
        let manifest =
            File::open(&ctx.manifest).with_context(|| format!("opening {}", ctx.manifest))?;
        let index = File::open(&ctx.index).with_context(|| format!("opening {}", ctx.index))?;
        let mut catalog = IndexedCatalog::open(manifest, index)?;
        let ov = catalog.overrides()?;
        // Its own no-repeat window, so a small set of matches can cycle.
        let window = opendmd_core::follow::context_window(ov.len());
        Ok((engine_over(Box::new(catalog), &ov, settings, window), ov))
    })();
    let outcome = match built {
        Ok((e, ov)) if e.catalog_len() > 0 => {
            let entries = e.catalog_len();
            log::info!(
                "follow: playing context {} ({entries} entries, {} weighted, no-repeat window {}) in {} ms, heap {} B free",
                ctx.index,
                ov.len(),
                opendmd_core::follow::context_window(ov.len()),
                started.elapsed().as_millis(),
                free_heap()
            );
            *engine = Some(e);
            Ok(Applied {
                entries,
                index: Some(IndexSource::Loaded),
                index_ms: started.elapsed().as_millis() as u64,
                overrides: ov.len(),
                hidden: 0,
            })
        }
        Ok(_) => Err(format!("{} is empty", ctx.index)),
        Err(e) => Err(format!("{e:#}")),
    };
    let mut s = status.lock().unwrap();
    s.entries = outcome.as_ref().map_or(0, |a| a.entries);
    s.overrides = outcome.as_ref().map_or(0, |a| a.overrides);
    s.hidden = 0;
    s.index_state = if outcome.is_ok() {
        "ready"
    } else {
        "unavailable"
    };
    outcome
}

/// How fast the cache builder may work while the panel does what it is doing:
/// full speed while nothing animates (night mode "off", or paused), a slow
/// drip while a prepared file plays (`auto_prepare` = `always`, and no
/// transition running — its 24 KB and the builder's ~63 KB do not both fit),
/// otherwise not at all. The builder adds its own conditions (heap, transfers).
fn builder_pace(
    settings: &Settings,
    off: bool,
    eng: &Engine,
    current_prepared: bool,
) -> cache_builder::Pace {
    use cache_builder::{Pace, Reason};
    use nvs_config::AutoPrepare;
    match settings.auto_prepare {
        AutoPrepare::Off => Pace::Stop(Reason::Off),
        _ if off || eng.is_paused() => Pace::Full,
        AutoPrepare::Idle => Pace::Stop(Reason::Playing),
        AutoPrepare::Always if eng.is_transitioning() => Pace::Stop(Reason::Transition),
        AutoPrepare::Always if !current_prepared => Pace::Stop(Reason::GifOnScreen),
        AutoPrepare::Always => Pace::Drip,
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

    /// The level a rule or night mode asks for, under the ceiling.
    fn brightness(&self, settings: &Settings) -> u8 {
        opendmd_core::brightness::effective(
            self.decision.brightness.unwrap_or(settings.brightness),
            settings.max_brightness,
        )
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
    runner: &mut follow::Runner,
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

    // The playlist — unless Follow RePlayOS has a game on the panel (its
    // overlay wins over a rule's playlist; the attract playlist does too).
    let eff = runner.overlay(sched.effective(settings));
    let playing = status.lock().unwrap().manifest.clone();
    if engine.is_some()
        && runner.mode() != follow::Mode::Game
        && attract_live(runner, &eff).is_none()
        && !files::same_card_path(&eff.manifest_path(), &playing)
    {
        log::info!("schedule: playlist {} -> {}", playing, eff.manifest_path());
        if rebuild(engine, &eff, false, status, canvas).is_err() && eff.set != settings.set {
            log::warn!(
                "schedule: playlist '{}' will not open; playing the saved one",
                eff.set
            );
            sched.broken_set = Some(sched.effective(settings).set);
            let _ = replay(engine, settings, sched, runner, false, status, canvas);
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
                // The cache builder works at full speed while the panel is
                // dark, and wants the playing file's heap (a GIF's is ~54
                // KB): let the file go. The next one opens when the panel
                // comes back.
                if settings.auto_prepare != nvs_config::AutoPrepare::Off {
                    e.drop_current(canvas);
                }
            }
            log::info!("schedule: night mode, panel dark and playback paused");
        }
    } else {
        if was_off {
            // The DMA buffer was cleared under the canvas: the next frame
            // must repaint all of it, not just what changed.
            canvas.dirty_mut().mark_all();
            if let Some(e) = engine.as_mut() {
                // A transition's output, if one is on display.
                e.display_mut(canvas).dirty_mut().mark_all();
            }
            let user_paused = status.lock().unwrap().paused;
            if let Some(e) = engine.as_mut() {
                if !user_paused {
                    e.resume(now);
                }
            }
            log::info!("schedule: panel back on");
            // A game that started in the dark: straight to its playlist.
            if runner.resume(engine.as_mut()) == follow::Todo::Rebuild {
                let _ = replay(engine, settings, sched, runner, false, status, canvas);
            }
        }
        panel.set_brightness(sched.brightness(settings));
    }
    sched.publish(status);
}

/// The GIF-start line: how `SdSource` served it (the prepared copy or the
/// original) and the heap as it starts. Logged from inside the frame, which
/// only costs the formatting (`logring`), and the heap walk behind
/// `largest_free_block`.
/// `Engine::play_now`, its error as `/api/play` reports it (404 / 415 / 503).
fn play_now(engine: Option<&mut Engine>, path: &str, now: Duration) -> Result<(), PlayError> {
    let Some(e) = engine else {
        return Err(PlayError::NoPlaylist);
    };
    e.play_now(path, now).map_err(|err| {
        let msg = format!("{path}: {err}");
        if err.kind() == std::io::ErrorKind::InvalidData {
            PlayError::Unplayable(msg)
        } else {
            PlayError::NotFound(msg)
        }
    })
}

fn log_started(ordinal: u64, path: &str, served: Option<sd::Served>) {
    log::info!(
        "[{ordinal}] {path} <- {} (heap {} B free, largest block {} B)",
        match served {
            Some(sd::Served::Prepared { try_us }) => {
                format!("prepared, open {:.1} ms", try_us as f32 / 1000.0)
            }
            Some(sd::Served::Original { miss_us }) => {
                format!("original, miss {:.1} ms", miss_us as f32 / 1000.0)
            }
            Some(sd::Served::Ahead { try_us }) => {
                format!("prepared, opened ahead in {:.1} ms", try_us as f32 / 1000.0)
            }
            None if path.starts_with('@') => "drawn".to_string(),
            Some(sd::Served::Direct) | None => "as named".to_string(),
        },
        free_heap(),
        largest_free_block()
    );
}

/// A duration in whole µs, saturated.
fn us(d: Duration) -> u32 {
    d.as_micros().min(u32::MAX as u128) as u32
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

/// An engine over `catalog` with the settings' order, dwell and transition,
/// `overrides` applied, carrying over the picture on the panel if there is
/// one.
fn engine_over(
    catalog: Box<dyn Catalog + Send>,
    overrides: &Overrides,
    settings: &Settings,
    no_repeat: usize,
) -> Engine {
    let catalog: Box<dyn Catalog> = if probe::ENABLED {
        Box::new(probe::TimedCatalog(catalog))
    } else {
        catalog
    };
    // The lookup is the card work of choosing the next file: `open` time.
    let catalog: Box<dyn Catalog> = Box::new(frametime::CountedCatalog(catalog, false));
    let mode = if settings.shuffle {
        Mode::Shuffle
    } else {
        Mode::Sequential
    };
    let scheduler = Scheduler::new(catalog.len(), mode, random_seed()).with_no_repeat(no_repeat);
    let mut engine = Engine::new(
        catalog,
        scheduler,
        dwell_policy(settings),
        Box::new(SdSource),
    );
    engine.set_overrides(overrides);
    engine.set_transition(settings.transition);
    engine.set_transition_gate(Some(transition_gate));
    if FRAME_SHOWN.load(Ordering::Relaxed) {
        // A playlist change: the panel shows the old playlist's last frame.
        engine.carry_over();
    }
    engine
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
) -> Result<(Box<dyn Catalog + Send>, Overrides, IndexSource)> {
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
    canvas: &mut Canvas,
) -> Result<Applied, String> {
    if !CARD_READY.load(Ordering::Relaxed) {
        return Err(CARD_NOT_READY.to_string());
    }
    // The builder stops for a playlist change (it may be walking this very
    // manifest); the main loop publishes its pace again afterwards.
    cache_builder::publish(
        cache_builder::Pace::Stop(cache_builder::Reason::Reindex),
        settings.auto_prepare,
    );
    retire(engine, canvas);
    let path = settings.manifest_path();
    {
        let mut s = status.lock().unwrap();
        s.manifest = path.clone();
        state::ATTRACT_LIVE.store(false, Ordering::Relaxed);
        s.index_state = "building";
    }
    let started = Instant::now();
    let built = open_catalog(&path, &build_filter(settings), force);
    finish_build(
        engine,
        settings,
        &path,
        built,
        started.elapsed().as_millis() as u64,
        status,
    )
}

/// An engine over the catalog `open_catalog` gave for `path` (here, or on
/// the boot task), reported — the log line, status, the cache's playlist —
/// or why there is none, for the HTTP caller.
fn finish_build(
    engine: &mut Option<Engine>,
    settings: &Settings,
    path: &str,
    built: boot::Built,
    ms: u64,
    status: &Mutex<Status>,
) -> Result<Applied, String> {
    let outcome = match built {
        Ok((catalog, ov, source)) => {
            let built = engine_over(catalog, &ov, settings, config::NO_REPEAT_WINDOW);
            let entries = built.catalog_len();
            if entries == 0 {
                log::error!("{path} matched no entries under the configured filter");
                Err(format!("{path} {NO_ENTRIES}"))
            } else {
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
        }
        Err(e) => {
            if e.downcast_ref::<NeedsFreshHeap>().is_some() {
                log::error!("{path}: {e:#}");
                REBOOT_PENDING.store(true, Ordering::Relaxed);
            } else {
                log::error!("{path} unreadable ({e:#})");
            }
            Err(format!("{path}: {e:#}"))
        }
    };
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
        cache::set_playlist(path, &settings.platforms);
    }
    outcome
}

/// Drop the engine, first ending any transition it is running, so the
/// canvas (which outlives it) holds what is on the panel and owes the panel
/// exactly the difference — the next engine carries over from it.
fn retire(engine: &mut Option<Engine>, canvas: &mut Canvas) {
    if let Some(e) = engine.as_mut() {
        e.finish_transition(canvas);
    }
    *engine = None;
}

/// A card just mounted (`boot::mount`), logged: where, how big, which
/// card, on which attempt, at what clock.
fn log_mounted(card: &SdCard, attempt: u32, khz: u32) {
    log::info!(
        "microSD mounted at {} ({:.2} GB, card {:#010x}) on attempt {attempt}, SPI clock {} kHz (asked {khz}), heap {} B free",
        config::SD_MOUNT,
        card.capacity_bytes as f64 / 1e9,
        card.serial,
        card.real_freq_khz,
        free_heap()
    );
}

/// A card just mounted: if it was mounted slow for a card that failed at
/// full speed and it is a different card, forget that record and restart
/// at full speed (a clean boot rather than a remount on a live bus);
/// otherwise note its clock for status.
fn check_card(card: &SdCard, health: &sdhealth::Boot, nvs: &EspDefaultNvsPartition) {
    let full = card.real_freq_khz as u32 >= config::SD_SPEED_KHZ;
    match health.slow_serial {
        Some(slow) if slow != card.serial => {
            log::warn!(
                "microSD: card {:#010x} is not the card that failed ({slow:#010x}); forgetting the slow clock and restarting at {} kHz",
                card.serial,
                config::SD_SPEED_KHZ
            );
            match Config::new(nvs.clone()).and_then(|mut c| c.set_sd_slow(None)) {
                Ok(()) => {}
                Err(e) => log::error!("clearing the slow-clock record in NVS failed ({e:#})"),
            }
            sdhealth::forget_for_new_card();
            restart_for_card();
        }
        Some(slow) => {
            // The same card. NVS may not have it (a failed write at the
            // trip, RTC's copy used): make sure it does.
            if let Ok(mut c) = Config::new(nvs.clone()) {
                if c.sd_slow() != Some(slow) {
                    if let Err(e) = c.set_sd_slow(Some(slow)) {
                        log::error!("recording the slow clock in NVS failed ({e:#})");
                    }
                }
            }
        }
        None => {}
    }
    sdhealth::mounted(card.real_freq_khz as u32, !full);
}

/// The card has failed (`sdhealth`): the error line (with the counts),
/// status, the slow-clock record in NVS when it failed at full speed, then
/// a restart once the log has reached the console. Returns only when the
/// restart budget is spent.
fn card_failed(card: &SdCard, nvs: &mut Config, status: &Mutex<Status>) {
    let trip = sdhealth::trip(card.serial, card.real_freq_khz as u32);
    log::error!("microSD: {}", trip.summary);
    if trip.slow_down {
        if let Err(e) = nvs.set_sd_slow(Some(card.serial)) {
            log::error!(
                "recording the slow clock in NVS failed ({e:#}); RTC memory has it until a power cycle"
            );
        }
    }
    status.lock().unwrap().sd_error = Some(trip.summary);
    if trip.restart {
        restart_for_card();
    }
    // Nothing plays from here on.
    let mut s = status.lock().unwrap();
    s.path.clear();
    s.title.clear();
}

/// A restart for the card's sake: the cache builder out of its card
/// write, then [`restart`].
fn restart_for_card() -> ! {
    cache_builder::quiesce(Duration::from_secs(3));
    restart()
}

/// Every planned restart ends here: the panel off, the log ring out to the
/// console, then `esp_restart`.
///
/// The panel first because `esp_restart` on the ESP32 resets only the CPUs
/// (`esp_restart_noos`: WiFi, timers, SPI and UART are reset, I2S and the
/// GPIO matrix are not). Left alone, I2S1's DMA keeps clocking the last
/// frame's buffers, then whatever the bootloader and the next app's start-up
/// write over them, onto the panel through the ~0.8 s to `Panel::new`, OE
/// included; two `POST /api/reboot`s on the boot-show build were followed by
/// a brownout reset right after the next app's `cpu_start` (where the
/// brownout detector is armed). `panel::blank` drives OE high as a GPIO,
/// which survives that reset, and stops the DMA. Callable from any task.
pub fn restart() -> ! {
    panel::blank();
    log::info!("restarting (panel off)");
    if !logring::flush(config::RESTART_LOG_FLUSH) {
        log::warn!("log ring not drained before the restart");
    }
    // SAFETY: an argument-free IDF call, declared `-> !` by the bindings.
    unsafe { esp_idf_sys::esp_restart() }
}

/// What the boot checks on a card once it is mounted, besides the saved
/// playlist (`boot::prepare`): the retired format's files, pointed out once
/// and never played.
fn retired_files_checks() {
    // Prepared copies are found per GIF now; a playlist of paths into the
    // retired format's tree from before that is dead weight. Say so once,
    // and never play it.
    if File::open(format!(
        "{}/{}",
        config::SD_MOUNT,
        config::OBSOLETE_MANIFEST
    ))
    .is_ok()
    {
        log::warn!(
            "ignoring /{}: prepared copies are found automatically from /filelist.txt; it can be deleted",
            config::OBSOLETE_MANIFEST
        );
    }
    // The retired pre-rendered format's copies: never read (prepared copies
    // are GIFs under cache/ now). One `stat` of the card root's entry.
    if std::fs::metadata(config::LEGACY_ROOT).is_ok() {
        log::warn!(
            "ignoring {}/: the old prepared-copy tree is not used any more; \
             DELETE /api/files/odm?recursive=1 removes it",
            config::LEGACY_ROOT.trim_start_matches(config::SD_MOUNT)
        );
    }
}

/// `esp_reset_reason`, as a word.
fn reset_reason() -> &'static str {
    use esp_idf_sys as sys;
    // SAFETY: an argument-free query of the reset cause.
    #[allow(non_upper_case_globals)]
    match unsafe { sys::esp_reset_reason() } {
        sys::esp_reset_reason_t_ESP_RST_POWERON => "poweron",
        sys::esp_reset_reason_t_ESP_RST_EXT => "external",
        sys::esp_reset_reason_t_ESP_RST_SW => "software",
        sys::esp_reset_reason_t_ESP_RST_PANIC => "panic",
        sys::esp_reset_reason_t_ESP_RST_INT_WDT => "interrupt watchdog",
        sys::esp_reset_reason_t_ESP_RST_TASK_WDT => "task watchdog",
        sys::esp_reset_reason_t_ESP_RST_WDT => "watchdog",
        sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP => "deep sleep",
        sys::esp_reset_reason_t_ESP_RST_BROWNOUT => "brownout",
        sys::esp_reset_reason_t_ESP_RST_SDIO => "sdio",
        _ => "unknown",
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

/// Carry out what a panel-timing change asks (`Tuner::apply`/`poll`): the
/// live values to the driver, NVS, the published state; a boot trial undone
/// is a restart, after every pending reply (`REBOOT_PENDING`).
fn panel_effects(
    fx: Effects,
    tuner: &Tuner,
    panel: &mut Panel,
    nvs: &mut Config,
    status: &Arc<Mutex<Status>>,
) {
    if let Some(t) = fx.apply_live {
        panel.set_live_timing(&t.effective());
    }
    if let Some(t) = fx.save {
        if let Err(e) = nvs.set_panel(&t) {
            log::error!("panel timing: saving {} failed ({e:#})", t.to_nvs());
        }
    }
    if let Some(staged) = fx.stage {
        if let Err(e) = nvs.set_panel_try(staged.as_ref()) {
            log::error!("panel timing: staging the trial failed ({e:#})");
        }
    }
    status.lock().unwrap().panel = *tuner;
    if fx.restart {
        log::info!("panel timing: restarting on the saved settings {}", tuner.saved.to_nvs());
        REBOOT_PENDING.store(true, Ordering::Relaxed);
    }
}

/// A copy of the picture, allocated fallibly (12 KB): `None` if it is not
/// there.
fn snapshot(canvas: &Canvas) -> Option<Canvas> {
    let mut px = Vec::new();
    px.try_reserve_exact(opendmd_core::canvas::PIXELS).ok()?;
    px.extend_from_slice(canvas.as_slice());
    Some(Canvas::from_parts(px, Default::default()))
}

/// End the panel-timing test, if one runs: the timing that ran before it
/// (the saved settings: a test never starts during a trial or with a change
/// staged, and a settings change ends it before it applies), the look, and
/// the picture from before it (a paused file's frame stays on screen; the
/// next file arrives from it) — black if there was no room to keep it.
fn end_panel_test(
    test: &mut Option<PanelTest>,
    saved: &mut Option<Canvas>,
    panel: &mut Panel,
    canvas: &mut Canvas,
    settings: &Settings,
    why: &str,
) {
    let Some(t) = test.take() else {
        return;
    };
    panel.set_live_timing(&t.restore.effective());
    panel.set_display(settings.look, settings.glow);
    match saved.take() {
        Some(pic) => {
            canvas.as_mut_slice().copy_from_slice(pic.as_slice());
            canvas.dirty_mut().mark_all();
        }
        None => canvas.clear(),
    }
    panel.blit(canvas);
    log::info!("panel test: ended ({why}), timing back to {}", t.restore.to_nvs());
}

/// Reboot after the pending replies: so the in-RAM fallback index can be
/// built on a fresh heap, or to undo a panel-timing boot trial.
///
/// Reached through [`NeedsFreshHeap`], i.e. a card that refuses the `.idx`
/// file, and through a panel-timing boot trial undone (`panel_effects`).
/// The reply has already been queued; the sleep is for the
/// handler on the other core, which is still writing it.
fn reboot_to_apply() -> ! {
    sdhealth::note_restart();
    cache_builder::quiesce(Duration::from_secs(3));
    thread::sleep(Duration::from_secs(2));
    restart()
}

#[allow(clippy::too_many_arguments)]
fn heartbeat(
    panel: &mut Panel,
    status: &Arc<Mutex<Status>>,
    started: u64,
    floor: &frametime::Floor,
    deadline_misses: u64,
    worst_lateness_ms: u64,
    misses: &frametime::Misses,
    schedule: &str,
) {
    diag::check("heartbeat");
    let sampled_min = floor.min;
    let free = free_heap();
    let min_free = min_free_heap();
    let refresh = panel.refresh_rate();
    let (ip, wifi, entries) = {
        let mut s = status.lock().unwrap();
        s.refresh_hz = refresh;
        s.heap_idf_min = min_free;
        (s.ip.clone(), s.wifi, s.entries)
    };
    log::info!(
        "heartbeat: heap {free} B free (floor {sampled_min} B{}, idf-min {min_free} B), refresh {refresh} Hz, \
         {started} GIFs started, {deadline_misses} deadline misses (worst {worst_lateness_ms} ms; {}), \
         transitions cut {}, streams aborted {}, main stack {} B unused, wifi {wifi} {ip}, {schedule}{}{}{}{}{}{}{}",
        floor.heartbeat(),
        misses.heartbeat(),
        TRANSITIONS_CUT.load(Ordering::Relaxed),
        httpd::streams_aborted(),
        frametime::stack_unused(),
        sd::period_summary(),
        cache_builder::heartbeat(entries, cache::peek()),
        sdhealth::heartbeat(),
        logring::summary(),
        probe::period_summary(),
        panel.glow_summary(),
        diag::summary()
    );
}
