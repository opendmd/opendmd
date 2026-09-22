//! OpenDMD firmware — phase 1: a standalone player.
//!
//! Boot -> mount the microSD -> build a seekable index over
//! `/sdcard/filelist.txt` -> run `opendmd_core::engine::Engine` -> push every
//! frame to two chained 64x32 HUB75 P4 panels (128x32) over the mrcodetastic
//! DMA driver.
//!
//! The manifest is never held in RAM. `FileCatalog` makes one streaming pass
//! over the card's 450 KB / 10,162-line `filelist.txt` and keeps a `u32` byte
//! offset per matching line (~40 KB unfiltered); the file handle stays open
//! and the engine seeks back to it for one `Entry` at a time, only when it is
//! about to open the next GIF.
//!
//! There is no networking in this phase. The shape here anticipates it: the
//! peripherals are taken once and handed to module constructors, every
//! tunable is a constant in [`config`], and the main loop is a single
//! `engine.tick` -> `panel.blit` -> sleep cycle that a background net worker
//! (lifted from ~/projects/stripe-dashboard in the next phase) can feed
//! through an `mpsc` channel without being restructured.

mod config;
mod panel;
mod sd;

use anyhow::{Context, Result};
use esp_idf_hal::gpio::{AnyIOPin, AnyOutputPin, Pin};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::log::EspLogger;
use opendmd_core::canvas::Canvas;
use opendmd_core::engine::{Catalog, DwellPolicy, Engine, FileCatalog, TickOutcome};
use opendmd_core::filter::Filter;
use opendmd_core::scheduler::{Mode, Scheduler};
use std::thread;
use std::time::Instant;

use crate::panel::{Panel, PanelConfig};
use crate::sd::{SdBus, SdSource};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    log::info!("OpenDMD firmware v{VERSION} — 128x32 HUB75 GIF player");

    // Peripherals are taken exactly once and handed out from here; nothing
    // downstream calls `Peripherals::take()`.
    let peripherals = Peripherals::take().context("peripherals already taken")?;
    let pins = peripherals.pins;

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

    // The panel is brought up before the index is built, which is the reverse
    // of the obvious order: its DMA buffers need a large contiguous run of
    // internal DRAM, and a heap already churned by the indexing pass is
    // exactly what produces "!KABOOM! HUB75 memory allocation failed". Claim
    // the hard allocation first.
    let panel_cfg = PanelConfig::default();
    let mut panel = Panel::new(&panel_cfg)?;
    panel.set_brightness(config::BRIGHTNESS);
    log::info!(
        "panel up: {}x{} ({} x {}x{}), driver={}, double_buff={}, hub75_refresh_rate={} Hz, brightness={}",
        panel_cfg.surface_width(),
        panel_cfg.height,
        panel_cfg.chain,
        panel_cfg.width,
        panel_cfg.height,
        panel_cfg.driver,
        panel_cfg.double_buffer,
        panel.refresh_rate(),
        panel_cfg.brightness,
    );

    let filter = build_filter();
    let mut canvas = Canvas::new();
    let mut started: u64 = 0;
    let mut last_heartbeat = Instant::now();
    // Our own heap floor, sampled at every phase boundary of the loop. Do not
    // use esp_get_minimum_free_heap_size() for this: it is the SUM of each
    // heap region's own minimum, which drifts ~30 KB below the true floor as
    // large blocks (the LZW decoder) land in different regions over time.
    let mut sampled_min = free_heap();
    let mut low_water = sampled_min;

    // One file handle and one offset index for the life of the run. Building
    // it is the only pass over the card; `Exhausted` reseeds the scheduler
    // instead of re-reading, so the 450 KB file is read exactly once.
    let mut engine = loop {
        match build_engine(&filter) {
            Ok(engine) if engine.catalog_len() > 0 => break engine,
            Ok(_) => log::error!(
                "{} matched no entries under the configured filter",
                config::MANIFEST_PATH
            ),
            Err(e) => log::error!("{} unreadable ({e:#})", config::MANIFEST_PATH),
        }
        thread::sleep(config::RETRY_INTERVAL);
    };
    log::info!(
        "manifest indexed: {} entries, {} index bytes, heap {} B free",
        engine.catalog_len(),
        engine.catalog_len() * 4,
        free_heap()
    );

    loop {
        let outcome = engine.tick(Instant::now(), &mut canvas)?;
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
                // Hold the frame for its GIF delay. `Instant::now()` again,
                // because the blit itself took time.
                match next_frame_at.checked_duration_since(Instant::now()) {
                    Some(remaining) => thread::sleep(remaining),
                    // Already late: yield a tick so the watchdog and every
                    // other task get their turn, then catch up next pass.
                    None => thread::sleep(config::CATCHUP_SLEEP),
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
                engine.reset(random_seed());
            }
        }

        if last_heartbeat.elapsed() >= config::HEARTBEAT_INTERVAL {
            last_heartbeat = Instant::now();
            heartbeat(&panel, started, sampled_min);
        }
    }
}

/// 64 bits from the hardware RNG.
fn random_seed() -> u64 {
    // SAFETY: `esp_random()` takes no arguments and has no preconditions once
    // the RF/RNG subsystem is up, which it is by the time `main` runs.
    unsafe { ((esp_idf_sys::esp_random() as u64) << 32) | (esp_idf_sys::esp_random() as u64) }
}

/// Free heap in bytes.
fn free_heap() -> u32 {
    // SAFETY: an argument-free read of ESP-IDF's heap accounting.
    unsafe { esp_idf_sys::esp_get_free_heap_size() }
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

fn build_filter() -> Filter {
    let filter = Filter::new();
    if config::PLATFORMS.is_empty() {
        filter
    } else {
        filter.platforms(config::PLATFORMS.iter().copied())
    }
}

/// A fresh engine over a streaming index of the card's manifest, shuffled with
/// a hardware-random seed so two boots do not play the same order.
///
/// The `File` is handed to `FileCatalog` and stays open: the engine seeks back
/// into it for one `Entry` at a time. Nothing proportional to the card's size
/// is ever resident — the index is 4 bytes per playable entry.
fn build_engine(filter: &Filter) -> Result<Engine> {
    let file = std::fs::File::open(config::MANIFEST_PATH)
        .with_context(|| format!("opening {}", config::MANIFEST_PATH))?;
    let catalog = FileCatalog::build(file, filter)
        .with_context(|| format!("indexing {}", config::MANIFEST_PATH))?;
    let scheduler = Scheduler::new(catalog.len(), Mode::Shuffle, random_seed())
        .with_no_repeat(config::NO_REPEAT_WINDOW);
    Ok(Engine::new(
        Box::new(catalog),
        scheduler,
        DwellPolicy::default(),
        Box::new(SdSource),
    ))
}

fn heartbeat(panel: &Panel, started: u64, sampled_min: u32) {
    let free = free_heap();
    // SAFETY: an argument-free read of ESP-IDF's heap accounting.
    let min_free = min_free_heap();
    log::info!(
        "heartbeat: heap {free} B free (floor {sampled_min} B, idf-min {min_free} B), refresh {} Hz, {started} GIFs started",
        panel.refresh_rate(),
    );
}
