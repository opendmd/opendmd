//! The boot task: everything slow the boot used to do on the display core
//! before the frame loop could start, done on CPU0 while the panel plays
//! the start-up show (`opendmd_core::boot`).
//!
//! **What it does, in order** (each as the main task did it before):
//! 1. The card: `SD_BOOT_ATTEMPTS` mount attempts 1 s apart (none when the
//!    card-failure restart budget is spent), `check_card` (the slow-clock
//!    record, a different card's restart), the boot's card checks (a saved
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
//! mounted again every `SD_REMOUNT_INTERVAL` (60 s) to notice a swap. It exits after the
//! handover; its stack goes back to the heap.
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
use esp_idf_hal::cpu::Core;
use esp_idf_hal::task::thread::ThreadSpawnConfiguration;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use opendmd_core::engine::Catalog;
use opendmd_core::overrides::Overrides;
use std::fs::File;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
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
}

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
    let previous = ThreadSpawnConfiguration::get();
    ThreadSpawnConfiguration {
        name: Some(b"boot\0"),
        stack_size: config::BOOT_STACK,
        // As the main task: it shares CPU0 round-robin with the timer
        // service task (the log drain) rather than starving it, and the
        // network and the httpd (5) come first once they exist.
        priority: 1,
        pin_to_core: Some(Core::Core0),
        ..Default::default()
    }
    .set()?;
    let handle = thread::Builder::new()
        .name("boot".into())
        .stack_size(config::BOOT_STACK)
        .spawn(move || run(task));
    if let Some(previous) = previous {
        let _ = previous.set();
    } else {
        let _ = ThreadSpawnConfiguration::default().set();
    }
    handle?;
    Ok(())
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
    let mut s = status.lock().unwrap();
    s.sd_mounted = card.is_some();
    s.sd_readable = card.is_some() && readable;
    s.sd_capacity_bytes = card.map_or(0, |c| c.capacity_bytes);
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
fn mount(
    bus: &mut SdBus,
    health: &sdhealth::Boot,
    nvs: &EspDefaultNvsPartition,
    quiet_for: Option<u32>,
) -> Mount {
    let khz = health.speed_khz;
    let card = match bus.mount(khz) {
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
                     exFAT (a 64 GB card as sold) is not supported. Not playing from it; \
                     trying it again every {} s",
                    card.serial,
                    config::SD_UNREADABLE,
                    config::SD_REMOUNT_INTERVAL.as_secs()
                );
            }
            Mount::Unreadable(card)
        }
    }
}

fn run(task: BootTask) {
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

    if let Some(c) = card {
        let snapshot = settings.lock().unwrap().clone();
        let handover = prepare(c, &snapshot, &status);
        start_http(ctx);
        let _ = tx.send(Event::Ready(Box::new(handover)));
        let ready = t0.elapsed().as_millis();
        crate::www::init();
        start_network(net);
        log::info!(
            "boot: card handed over {ready} ms after the boot task started; boot task done in {} ms (stack {} B unused)",
            t0.elapsed().as_millis(),
            stack_unused()
        );
        return;
    }

    if !health.capped && unreadable.is_none() {
        log::warn!(
            "microSD: no card yet; starting the network and the web server anyway, and retrying every {} s",
            config::RETRY_INTERVAL.as_secs()
        );
    }
    let _ = tx.send(Event::NoCard);
    start_http(ctx);
    crate::www::init();
    start_network(net);

    // No card mounted: try again every so often (a spent restart budget
    // waits longer, see `sdhealth`). A card that cannot be read is
    // unmounted and mounted again once a minute, so a card swapped in
    // (there is no card-detect pin) is noticed; the same card stays quiet.
    let mut remounts: u32 = 0;
    loop {
        let interval = if health.capped || unreadable.is_some() {
            config::SD_REMOUNT_INTERVAL
        } else {
            config::RETRY_INTERVAL
        };
        thread::sleep(interval);
        remounts += 1;
        if health.capped {
            log::warn!(
                "microSD: remount {remounts} at {} kHz (the card failed repeatedly; every {} s)",
                health.speed_khz,
                interval.as_secs()
            );
        }
        // Dropping the card unmounts it (and frees the bus) before the
        // next attempt builds it all again.
        let quiet_for = unreadable.take().map(|c: SdCard| c.serial);
        match mount(&mut bus, &health, &nvs, quiet_for) {
            Mount::Ready(c) => {
                publish(&status, Some(&c), true, None, bus.attempts());
                let now = settings.lock().unwrap().clone();
                let _ = tx.send(Event::Ready(Box::new(prepare(c, &now, &status))));
                log::info!(
                    "boot: card handed over after {remounts} retries; boot task done (stack {} B unused)",
                    stack_unused()
                );
                return;
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
            }
            Mount::Failed(e) if health.capped => publish(
                &status,
                None,
                false,
                Some(sdhealth::capped_error(remounts, Some(&e))),
                bus.attempts(),
            ),
            Mount::Failed(e) => {
                if quiet_for.is_some() {
                    log::warn!(
                        "microSD: the card that could not be read no longer mounts (taken out?); retrying every {} s",
                        config::RETRY_INTERVAL.as_secs()
                    );
                }
                publish(&status, None, false, Some(e), bus.attempts())
            }
        }
    }
}

/// The HTTP server (and the stream task): see the module docs.
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

/// The network worker, and with it WiFi: after the card, as before.
fn start_network(net: net_worker::NetWorker) {
    crate::HEAP_FRESH.store(false, Ordering::Relaxed);
    if let Err(e) = net_worker::spawn(net) {
        log::error!("network worker failed to start ({e:#}); no WiFi this boot");
    }
}

/// A card just mounted: the boot's checks, then the playlist `settings`
/// name opened or built — everything the main task used to do before its
/// first frame, here.
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
    }
}

/// This task's stack left unused at its deepest, in bytes.
fn stack_unused() -> u32 {
    // SAFETY: a read of the calling task's own high-water mark.
    unsafe { esp_idf_sys::uxTaskGetStackHighWaterMark(std::ptr::null_mut()) }
}
