//! The boot task: everything slow the boot used to do on the display core
//! before the frame loop could start, done on CPU0 while the panel plays
//! the start-up show (`opendmd_core::boot`).
//!
//! **What it does, in order** (each as the main task did it before):
//! 1. The card: `SD_BOOT_ATTEMPTS` mount attempts 1 s apart (none when the
//!    card-failure restart budget is spent), `check_card` (the slow-clock
//!    record, a different card back to full speed), the boot's card checks (a saved
//!    playlist that no longer opens, the retired files), and the playlist
//!    index opened or built (`open_catalog`, on a fresh heap: the in-RAM
//!    fallback's 64 KB block still exists, `HEAP_FRESH`).
//! 2. `http::start` — the httpd and stream tasks, whose stacks are
//!    contiguous allocations taken while the heap is still whole: before
//!    the first GIF, as before.
//! 3. The handover ([`Event::Ready`]: the mounted card and the built
//!    catalog) — the first GIF can open now.
//! 4. `www::init` — map and CRC-check the web app in flash (~170 ms; it
//!    used to come first, but nothing is served before WiFi is up), then
//!    the network worker (WiFi last, as before).
//!
//! So the card is ready ~75 ms after the panel lights, not ~240: with the
//! show `off` the first GIF comes ~0.2 s sooner than before.
//!
//! With no card after the boot attempts it says so ([`Event::NoCard`]),
//! starts the HTTP server and the network anyway, and keeps trying every
//! `RETRY_INTERVAL` (60 s at 20 MHz with the budget spent) — full
//! re-initialisations, as the main loop's retries were — until a mount
//! works, then builds and hands over as above. A card that mounts but whose
//! filesystem cannot be read (`sd::probe`: exFAT) is treated the same way
//! — never handed over, the status card says so — and is unmounted and
//! mounted again every `SD_REMOUNT_INTERVAL` (60 s) to notice a swap — or,
//! asked over HTTP (`POST /api/card/format`, [`request_format`]), unmounted,
//! formatted FAT32 (`sd::SdBus::format`) and mounted again at once. It exits
//! after the handover; its stack goes back to the heap.
//!
//! **Who touches the card when.** Until the handover the display loop does
//! not touch the card at all (`CARD_READY` is false: every rebuild, set
//! check and manifest install answers "not ready"); this task is the only
//! one that mounts it. After the handover the rules are the usual ones
//! (CLAUDE.md, "Sharing the card with the player"). Because every mount is
//! made here, the SPI bus's interrupt is installed on CPU0 (it used to be
//! CPU1's): the card's transfer completions no longer interrupt the display
//! core, and the player's reads are woken from CPU0.

use crate::chan as mpsc;
use anyhow::Result;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use opendmd_core::engine::Catalog;
use opendmd_core::overrides::Overrides;
use std::fs::File;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::http::Ctx;
use crate::nvs_config::{self, Settings};
use crate::sd::{SdBus, SdCard};
use crate::state::{IndexSource, Status};
use crate::{config, net_worker, sdhealth};

/// What the boot task tells the main task.
pub enum Event {
    /// The boot's mount attempts failed: no card for now (the task keeps
    /// trying). The panel shows the status card from here.
    NoCard,
    /// The card is mounted and checked, and its playlist opened or built.
    Ready(Box<Handover>),
    /// The card's `boot_formula`, when it names a Formula on the card
    /// (`/formulas/<stem>.psi`: its compiled bytes, read once the card is
    /// mounted, before the playlist, or why not) or differs from NVS's
    /// (`None`: a bundled or inline one, or `""`): `show::Screen::
    /// card_formula`.
    #[cfg(feature = "psi")]
    Formula(Box<(String, Option<CardPsb>)>),
}

/// A card Formula's compiled bytes, or why they could not be read.
#[cfg(feature = "psi")]
pub type CardPsb = Result<Vec<u8>, opendmd_core::psi::Error>;

/// The playlist's catalog as `open_catalog` returns it.
pub type Built = anyhow::Result<(Box<dyn Catalog + Send>, Overrides, IndexSource)>;

/// Everything the main task takes over.
pub struct Handover {
    pub card: SdCard,
    /// The catalog for [`Handover::manifest`] under [`Handover::platforms`],
    /// or why not.
    pub built: Built,
    pub manifest: String,
    pub platforms: String,
    /// How long opening or building it took.
    pub index_ms: u64,
    /// The saved playlist did not open (or is the retired one): the main
    /// task clears it from NVS (it owns NVS's settings); the default was
    /// built instead.
    pub healed_set: Option<String>,
    /// The card's settings (`settings_card`), when they differ from what
    /// the main task has: it applies them (the playlist above was built
    /// from them already).
    pub settings: Option<Box<Settings>>,
}

/// What the task needs, moved to it.
pub struct BootTask {
    pub bus: SdBus,
    pub health: sdhealth::Boot,
    pub nvs: EspDefaultNvsPartition,
    pub ctx: Arc<Ctx>,
    pub net: net_worker::NetWorker,
    pub tx: mpsc::Sender<Event>,
}

/// Start the task on CPU0 and return at once.
pub fn spawn(task: BootTask) -> Result<()> {
    // Priority 1, as the main task: it shares CPU0 round-robin with the
    // timer service task (the log drain) rather than starving it, and the
    // network and the httpd (5) come first once they exist.
    crate::task::spawn_core0(b"boot\0", config::BOOT_STACK, 1, Box::new(move || run(task)))?;
    Ok(())
}

/// A format asked for over HTTP, which ends the remount loop's wait at once.
static FORMAT_ASKED: Mutex<bool> = Mutex::new(false);
static FORMAT_WAKE: Condvar = Condvar::new();

/// Ask for the unreadable card to be formatted (`POST /api/card/format`).
/// The caller has already set `Status::sd_formatting`, under the status
/// lock and with the check that this card may be formatted
/// (`opendmd_core::card::may_format`); the remount loop below does it
/// and clears the flag. Only that loop holds an unreadable card, so a
/// request can only be accepted while it runs.
pub fn request_format() {
    *FORMAT_ASKED.lock().unwrap() = true;
    FORMAT_WAKE.notify_one();
}

/// The remount loop's wait: `interval`, or less if a format is asked for
/// (true).
fn wait_for_format(interval: Duration) -> bool {
    let asked = FORMAT_ASKED.lock().unwrap();
    let (mut asked, _) = FORMAT_WAKE
        .wait_timeout_while(asked, interval, |a| !*a)
        .unwrap();
    std::mem::take(&mut *asked)
}

/// A format asked for is over (done, failed, or nothing left to format):
/// status says so, with `error` why it failed. Nothing changes when none
/// was asked for.
fn format_over(status: &Mutex<Status>, error: Option<String>) {
    let mut s = status.lock().unwrap();
    if s.sd_formatting {
        s.sd_formatting = false;
        s.sd_format_error = error;
    }
}

/// The card's fields in status, after a mount attempt: `card` mounted,
/// `readable` its filesystem answered [`crate::sd::probe`].
fn publish(
    status: &Mutex<Status>,
    card: Option<&SdCard>,
    readable: bool,
    error: Option<String>,
    attempts: u32,
) {
    // Which card this is (`sd.card_id`): its CID serial, its volume's
    // serial and label (one sector), its size. Read here, once per mount,
    // before the status lock.
    let card_id = card.filter(|_| readable).map(|c| {
        let vol = crate::fatidx::volume_id().unwrap_or_else(|e| {
            log::warn!("microSD: reading the boot sector: {e}");
            None
        });
        let id = opendmd_core::card::identity(c.serial, vol.as_ref(), c.capacity_bytes);
        match vol {
            Some(v) => log::info!("microSD: card id {id} (volume {:08X} '{}')", v.serial, v.label()),
            None => log::info!("microSD: card id {id} (no volume serial)"),
        }
        id
    });
    let mut s = status.lock().unwrap();
    s.sd_mounted = card.is_some();
    s.sd_readable = card.is_some() && readable;
    s.sd_capacity_bytes = card.map_or(0, |c| c.capacity_bytes);
    s.sd_card_id = card_id;
    s.sd_error = error;
    s.sd_attempts = attempts;
}

/// What a mount attempt found.
enum Mount {
    /// Mounted, and its filesystem reads: the player's.
    Ready(SdCard),
    /// Mounted at the SD level, but its filesystem cannot be read
    /// (`sd::probe`: exFAT, no filesystem). Never handed over.
    Unreadable(SdCard),
    /// Not mounted: the error as status shows it.
    Failed(String),
}

/// One mount attempt: the mount, the probe, `check_card`, each logged.
/// `quiet_for`: the serial of a card that could not be read and was just
/// unmounted to be tried again — the same card, still unreadable, is not
/// logged again (once a minute, for as long as it stays in).
#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
#[inline(never)]
fn mount(
    bus: &mut SdBus,
    health: &sdhealth::Boot,
    nvs: &EspDefaultNvsPartition,
    quiet_for: Option<u32>,
) -> Mount {
    let khz = health.speed_khz;
    let card = match bus.mount(khz, health.capped) {
        Ok(c) => c,
        Err(e) => {
            let e = format!("{e:#}");
            log::warn!("microSD mount attempt {} failed ({e})", bus.attempts());
            return Mount::Failed(e);
        }
    };
    let probed = crate::sd::probe();
    let same = probed.is_err() && quiet_for == Some(card.serial);
    if !same {
        crate::log_mounted(&card, bus.attempts(), khz);
    }
    crate::check_card(&card, health, nvs);
    match probed {
        Ok(()) => Mount::Ready(card),
        Err(e) => {
            if !same {
                log::warn!(
                    "microSD: card {:#010x} mounted, but its filesystem cannot be read ({e}): {}. \
                     exFAT (a 64 GB card as sold) is not supported; POST /api/card/format \
                     makes it FAT32. Not playing from it; trying it again every {} s",
                    card.serial,
                    config::SD_UNREADABLE,
                    config::SD_REMOUNT_INTERVAL.as_secs()
                );
            }
            Mount::Unreadable(card)
        }
    }
}

/// The format asked for over HTTP: the unreadable `card` unmounted,
/// formatted ([`SdBus::format`]) and mounted again as any card is. A card
/// that then mounts gets what a fresh card needs. Returns the mount, and
/// why the format failed if it did.
fn format(
    bus: &mut SdBus,
    health: &sdhealth::Boot,
    nvs: &EspDefaultNvsPartition,
    card: SdCard,
) -> (Mount, Option<String>) {
    log::warn!(
        "microSD: formatting card {:#010x} ({:.2} GB) FAT32, as asked over HTTP; everything on it is erased",
        card.serial,
        card.capacity_bytes as f64 / 1e9
    );
    // Dropping it unmounts: `/sdcard` leaves the VFS, so no other task can
    // reach the card while it is formatted.
    drop(card);
    // (Not `heap_caps_monitor_local_minimum_free_size_*` for the low-water:
    // its restore takes each heap region's minimum separately, and their
    // sum, from different moments, left `heap.idf_min` far below anything
    // the heap really reached.)
    let free = crate::state::free_heap();
    let done = bus.format(health.speed_khz);
    let error = match done {
        Ok(f) => {
            log::info!(
                "microSD: formatted {:.2} GB in {} ms; free heap {free} B before, {} B at the lowest since boot",
                f.capacity_bytes as f64 / 1e9,
                f.ms,
                crate::state::min_free_heap()
            );
            None
        }
        Err(e) => {
            log::error!("microSD: the format failed: {e:#}");
            Some(format!("{e:#}"))
        }
    };
    let found = mount(bus, health, nvs, None);
    if let (Mount::Ready(_), None) = (&found, &error) {
        fresh_card();
    }
    (found, error)
}

/// What a freshly formatted card is given: the stock layout's `gifs/`
/// folder and an empty "All files" playlist, so the panel says "The
/// playlist is empty" rather than "No playlist on the card" until GIFs are
/// added. Everything else (`cache/`, `sets/`, `update/`, …) is made by
/// whatever first writes there.
fn fresh_card() {
    let gifs = format!("{}/gifs", config::SD_MOUNT);
    if let Err(e) = std::fs::create_dir_all(&gifs) {
        log::warn!("microSD: making {gifs} on the new card: {e}");
    }
    if let Err(e) = File::create(config::MANIFEST_PATH) {
        log::warn!(
            "microSD: making {} on the new card: {e}",
            config::MANIFEST_PATH
        );
    }
}

#[optimize(size)] // (CPU0, boot: no frame runs it)
#[inline(never)]
fn run(task: BootTask) {
    // The updater in ota_0, read once here (a flash read, ~0.3 KB of stack)
    // so neither the httpd task (GET /api/firmware) nor the stream task
    // (PUT, the Installing screen) has to on its tight stack.
    log::info!(
        "updater in ota_0: {} (lights the panel while it installs: {})",
        crate::ota::updater_version().unwrap_or("unreadable"),
        crate::ota::updater_lights_panel()
    );
    let BootTask {
        mut bus,
        health,
        nvs,
        ctx,
        net,
        tx,
    } = task;
    let t0 = Instant::now();
    let status = ctx.status.clone();
    let settings = ctx.settings.clone();

    // The card: a couple of attempts, then the rest of the boot goes ahead
    // without it (CLAUDE.md, "When the card will not mount"). A card that
    // mounts but cannot be read is not tried again at once: it will not
    // read on the second attempt either.
    let mut card = None;
    let mut unreadable = None;
    let mut error = None;
    if health.capped {
        error = Some(sdhealth::capped_error(0, None));
    } else {
        for attempt in 1..=config::SD_BOOT_ATTEMPTS {
            match mount(&mut bus, &health, &nvs, None) {
                Mount::Ready(c) => {
                    card = Some(c);
                    error = None;
                    break;
                }
                Mount::Unreadable(c) => {
                    unreadable = Some(c);
                    error = Some(config::SD_UNREADABLE.to_string());
                    break;
                }
                Mount::Failed(e) => {
                    publish(&status, None, false, Some(e.clone()), bus.attempts());
                    error = Some(e);
                    if attempt < config::SD_BOOT_ATTEMPTS {
                        thread::sleep(Duration::from_secs(1));
                    }
                }
            }
        }
    }
    publish(
        &status,
        card.as_ref().or(unreadable.as_ref()),
        card.is_some(),
        error.clone(),
        bus.attempts(),
    );
    crate::heapmap::cp("card mounted (sdspi + fatfs)");

    if let Some(c) = card {
        // What NVS named as the start-up show's Formula (the show started with it).
        #[cfg(feature = "psi")]
        let nvs_boot = settings.lock().map(|s| s.boot_formula.clone()).unwrap_or_default();
        // The card's settings over the device's (or written from them).
        let (snapshot, card_settings) = crate::settings_card::adopt_at_mount(&settings);
        crate::heapmap::cp("card settings adopted");
        // WiFi or not, as the card says (`network`): decided here, before
        // the httpd and the network would start.
        let online = snapshot.network;
        network_decided(&status, &snapshot);
        // The start-up show's Formula as the card's settings name it: one
        // of the card's read here, where the card is this task's, while the
        // show runs; any other told when it is not NVS's. Then room for the
        // index beside it (`show::make_room_for_index`).
        #[cfg(feature = "psi")]
        {
            let mut sent = false;
            if snapshot.boot_show != opendmd_core::boot::ShowLength::Off {
                let file = opendmd_core::synth::formula_file(&snapshot.boot_formula);
                if file.is_some() || snapshot.boot_formula != nvs_boot {
                    let read = file.map(|(f, _)| crate::formulas::read_card(f));
                    sent = tx.send(Event::Formula(Box::new((snapshot.boot_formula.clone(), read)))).is_ok();
                }
            }
            crate::show::make_room_for_index(sent);
        }
        let mut handover = prepare(c, &snapshot, &status);
        crate::heapmap::cp("playlist index (prepare)");
        handover.settings = card_settings;
        if online {
            start_http(ctx);
            crate::heapmap::cp("httpd + stream task");
        }
        let _ = tx.send(Event::Ready(Box::new(handover)));
        let ready = t0.elapsed().as_millis();
        if online {
            crate::www::init();
            // A pack an update left on the card, when flash has none (`www.rs`).
            crate::www::restore_from_card();
            crate::heapmap::cp("www pack");
        }
        start_network(net, online);
        crate::heapmap::cp("net-worker spawned");
        log::info!(
            "boot: card handed over {ready} ms after the boot task started; boot task done in {} ms (stack {} B unused)",
            t0.elapsed().as_millis(),
            stack_unused()
        );
        // The card's Formulas, compiled where their compiled files are
        // missing or stale: in the background, the card the player's
        // (`card_formulas`).
        #[cfg(feature = "psi")]
        {
            crate::card_formulas::prepare_after_handover(&settings);
            log::info!("boot: card Formulas checked (stack {} B unused)", stack_unused());
        }
        return;
    }

    // No card: WiFi or not as NVS says (what the card last said, or the
    // installer's settings).
    let nvs_settings = settings.lock().unwrap().clone();
    let online = nvs_settings.network;
    network_decided(&status, &nvs_settings);
    drop(nvs_settings);
    if !health.capped && unreadable.is_none() {
        log::warn!(
            "microSD: no card yet; {}retrying every {} s",
            if online { "starting the network and the web server anyway, and " } else { "" },
            config::RETRY_INTERVAL.as_secs()
        );
    }
    let _ = tx.send(Event::NoCard);
    if online {
        start_http(ctx);
        crate::www::init();
    }
    start_network(net, online);

    // No card mounted: try again every so often (a spent restart budget
    // waits longer, see `sdhealth`). A card that cannot be read is
    // unmounted and mounted again once a minute, so a card swapped in
    // (there is no card-detect pin) is noticed; the same card stays quiet.
    // A format asked for over HTTP ends the wait and is done instead.
    let mut remounts: u32 = 0;
    loop {
        let interval = if health.capped || unreadable.is_some() {
            config::SD_REMOUNT_INTERVAL
        } else {
            config::RETRY_INTERVAL
        };
        let asked = wait_for_format(interval);
        remounts += 1;
        // Asked for while the card was being remounted (no unreadable card
        // held then): still asked for, `sd_formatting`, and done now.
        let format_now = unreadable.is_some() && (asked || status.lock().unwrap().sd_formatting);
        if health.capped && !format_now {
            log::warn!(
                "microSD: remount {remounts} at {} kHz (the card failed repeatedly; every {} s)",
                health.speed_khz,
                interval.as_secs()
            );
        }
        // Dropping the card unmounts it (and frees the bus) before the
        // next attempt builds it all again.
        let mut quiet_for = None;
        let mut format_error = None;
        let found = match unreadable.take() {
            Some(c) if format_now => {
                let (found, error) = format(&mut bus, &health, &nvs, c);
                format_error = error;
                found
            }
            c => {
                quiet_for = c.map(|c: SdCard| c.serial);
                mount(&mut bus, &health, &nvs, quiet_for)
            }
        };
        let gone = matches!(found, Mount::Failed(_));
        let ready = match found {
            Mount::Ready(c) => {
                publish(&status, Some(&c), true, None, bus.attempts());
                Some(c)
            }
            Mount::Unreadable(c) => {
                publish(
                    &status,
                    Some(&c),
                    false,
                    Some(config::SD_UNREADABLE.to_string()),
                    bus.attempts(),
                );
                unreadable = Some(c);
                None
            }
            Mount::Failed(e) if health.capped => {
                publish(
                    &status,
                    None,
                    false,
                    Some(sdhealth::capped_error(remounts, Some(&e))),
                    bus.attempts(),
                );
                None
            }
            Mount::Failed(e) => {
                if quiet_for.is_some() {
                    log::warn!(
                        "microSD: the card that could not be read no longer mounts (taken out?); retrying every {} s",
                        config::RETRY_INTERVAL.as_secs()
                    );
                }
                publish(&status, None, false, Some(e), bus.attempts());
                None
            }
        };
        // A format asked for is over once it ran, or once there is no
        // unreadable card left to run it on (taken out, or readable after
        // all). After the card's fields, so a poll never sees it over
        // with the old card state.
        if format_now {
            format_over(&status, format_error);
        } else if unreadable.is_none() {
            format_over(&status, gone.then(|| "the card was taken out".to_string()));
        }
        if let Some(c) = ready {
            let (now, card_settings) = crate::settings_card::adopt_at_mount(&settings);
            if now.network != online {
                log::warn!(
                    "settings: the card turns WiFi {}; in effect from the next start",
                    crate::nvs_config::network_name(now.network)
                );
            }

            let mut handover = prepare(c, &now, &status);
            handover.settings = card_settings;
            let _ = tx.send(Event::Ready(Box::new(handover)));
            log::info!(
                "boot: card handed over after {remounts} retries; boot task done (stack {} B unused)",
                stack_unused()
            );
            #[cfg(feature = "psi")]
            {
                crate::card_formulas::prepare_after_handover(&settings);
                log::info!("boot: card Formulas checked (stack {} B unused)", stack_unused());
            }
            return;
        }
    }
}

/// The HTTP server (and the stream task): see the module docs.
#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
#[inline(never)]
fn start_http(ctx: Arc<Ctx>) {
    match crate::http::start(ctx) {
        // Held for the life of the program: dropping it would stop the
        // httpd task.
        Ok(server) => std::mem::forget(server),
        Err(e) => {
            log::error!("HTTP server failed to start ({e:#}); the player carries on without it")
        }
    }
}

/// The `network` setting as this boot has it (`s.network`, the card's
/// or NVS's): off, the display has no WiFi and no clock this boot, and
/// says so once; status says `wifi: "off"`.
#[optimize(size)]
#[inline(never)]
fn network_decided(status: &Mutex<Status>, s: &Settings) {
    #[cfg(feature = "psi")]
    opendmd_core::fx_psi::set_no_clock(!s.network);
    if s.network {
        return;
    }
    status.lock().unwrap().wifi = "off";
    crate::improv::set_network("off", "");
    log::info!(
        "network off (\"network\": \"off\" in the settings): no WiFi, no web app; the clock is not set, so the schedule and night mode are off and Formulas that show the time are skipped. \
         To turn WiFi on: \"network\": \"on\" in the card's opendmd.json, or Connect to Wi-Fi in the browser installer"
    );
    if s.follow {
        log::info!("follow: Follow RePlayOS is on, but the network is off: nothing to follow");
    }
}

/// The network worker, and with it WiFi (`online`; offline, the worker
/// without the radio: `net_worker::run_offline`): after the card, as
/// before.
fn start_network(mut net: net_worker::NetWorker, online: bool) {
    net.offline = !online;
    crate::HEAP_FRESH.store(false, Ordering::Relaxed);
    if let Err(e) = net_worker::spawn(net) {
        log::error!("network worker failed to start ({e:#}); no WiFi this boot");
    }
}

/// A card just mounted: the boot's checks, then the playlist `settings`
/// name opened or built — everything the main task used to do before its
/// first frame, here.
#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
#[inline(never)]
fn prepare(card: SdCard, settings: &Settings, status: &Mutex<Status>) -> Handover {
    let mut s = settings.clone();
    // A saved playlist that no longer opens (deleted off-device, another
    // card) or the retired one: the default instead, and the main task
    // clears it from NVS.
    let healed_set = if s.set.is_empty() {
        None
    } else if nvs_config::is_obsolete_set(&s.set) {
        log::warn!(
            "configured set '{}' is obsolete (prepared copies are found automatically); playing the default manifest and clearing it from NVS",
            s.set
        );
        Some(std::mem::take(&mut s.set))
    } else {
        match File::open(s.manifest_path()) {
            Ok(_) => None,
            Err(e) => {
                log::warn!(
                    "configured set '{}' does not open ({}: {e}); falling back to the default manifest and clearing it from NVS",
                    s.set,
                    s.manifest_path()
                );
                Some(std::mem::take(&mut s.set))
            }
        }
    };
    crate::retired_files_checks();
    // A firmware update that ran (or did not): log it, tidy /update/.
    crate::ota::after_boot();
    let manifest = s.manifest_path();
    {
        let mut st = status.lock().unwrap();
        st.manifest = manifest.clone();
        st.index_state = "building";
    }
    let started = Instant::now();
    let built = crate::open_catalog(&manifest, &crate::build_filter(&s), false);
    Handover {
        card,
        built,
        manifest,
        platforms: s.platforms.clone(),
        index_ms: started.elapsed().as_millis() as u64,
        healed_set,
        settings: None,
    }
}

/// This task's stack left unused at its deepest, in bytes.
fn stack_unused() -> u32 {
    // SAFETY: a read of the calling task's own high-water mark.
    unsafe { esp_idf_sys::uxTaskGetStackHighWaterMark(std::ptr::null_mut()) }
}
