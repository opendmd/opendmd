//! Every tunable in one place: pin map, panel timing, playlist policy.
//!
//! The pin numbers are measured, not guessed — see `hardware/PINOUT.md` in the
//! repository root (continuity walk, 2026-09-22, DMDos board v3).

use esp_idf_sys::hub75;
use std::time::Duration;

// ---------------------------------------------------------------------------
// HUB75 "DMD IN" header (2x8 IDC). hardware/PINOUT.md, "HUB75 DMD IN".
// This is the library's stock ESP32 map with only A/B/C moved (23->33, 19->32,
// 5->22) because the stock three collide with the SD card's SPI bus.
// ---------------------------------------------------------------------------
pub const PIN_R1: i8 = 25;
pub const PIN_G1: i8 = 26;
pub const PIN_B1: i8 = 27;
pub const PIN_R2: i8 = 14;
pub const PIN_G2: i8 = 12;
pub const PIN_B2: i8 = 13;
pub const PIN_A: i8 = 33;
pub const PIN_B: i8 = 32;
pub const PIN_C: i8 = 22;
pub const PIN_D: i8 = 17;
/// E is unused at 1/16 scan: pin 8 of the header is silent. -1 = not wired.
pub const PIN_E: i8 = -1;
pub const PIN_LAT: i8 = 4;
pub const PIN_OE: i8 = 15;
pub const PIN_CLK: i8 = 16;

// ---------------------------------------------------------------------------
// Panel geometry. Two 64x32 modules chained into one 128x32 surface, which is
// exactly `opendmd_core::canvas::{WIDTH, HEIGHT}`.
// ---------------------------------------------------------------------------
pub const PANEL_WIDTH: u16 = 64;
pub const PANEL_HEIGHT: u16 = 32;
pub const PANEL_CHAIN: u16 = 2;

// ---------------------------------------------------------------------------
// Panel timing. These five are the knobs to turn when the panel misbehaves;
// nothing else in the firmware hardcodes them.
//
// DRIVER: the shift-register chip on the panel is unknown, so we default to a
// plain shift register. **If the panel stays completely dark, try FM6126A
// first** — it is by far the most common panel that needs an init sequence,
// and it shows as "no output at all" rather than as corruption.
// ---------------------------------------------------------------------------
pub const DRIVER: i32 = hub75::HUB75_DRIVER_SHIFTREG as i32;
/// Positive-edge clocking; the library's default since June 2021.
pub const CLK_PHASE: bool = true;
pub const I2S_SPEED: i32 = hub75::HUB75_SPEED_8M as i32;
/// Clocks of OE blanking around LAT. >1 causes artefacts on ICS panels.
pub const LATCH_BLANKING: u8 = 1;
/// Asking for more costs colour depth; 60 Hz is flicker-free to the eye.
pub const MIN_REFRESH_RATE: u8 = 60;
/// Double buffering: a whole frame is composed off-screen and swapped, so a
/// 128x32 blit never tears. Costs a second DMA framebuffer (~32 KB).
pub const DOUBLE_BUFFER: bool = true;

/// 0..255. The panels are extremely bright; this is a living-room level.
pub const BRIGHTNESS: u8 = 96;

// ---------------------------------------------------------------------------
// microSD, SPI (not SDMMC) on VSPI/SPI2. hardware/PINOUT.md, "microSD".
// ---------------------------------------------------------------------------
pub const SD_SCK: u32 = 18;
pub const SD_MISO: u32 = 19;
pub const SD_MOSI: u32 = 23;
pub const SD_CS: u32 = 5;
/// Where the card is mounted. Manifest paths are card-absolute, so a manifest
/// entry `/gifs/X.gif` opens as `/sdcard/gifs/X.gif`.
pub const SD_MOUNT: &str = "/sdcard";
/// FATFS file descriptors. Two are live in steady state: the manifest
/// index's handle on `filelist.txt` and the GIF the engine is playing.
pub const SD_MAX_FILES: usize = 4;
/// Conservative: the carrier's SD wiring is long and unshielded.
pub const SD_SPEED_KHZ: u32 = 20_000;

// ---------------------------------------------------------------------------
// Playlist
// ---------------------------------------------------------------------------
/// The stock card's playlist file, read from the card root.
pub const MANIFEST_PATH: &str = "/sdcard/filelist.txt";

// The manifest is indexed, never loaded.
//
// `FileCatalog::build` makes one streaming pass over `MANIFEST_PATH` and
// keeps a `u32` byte offset per line that passes `PLATFORMS`: 4 bytes per
// playable entry, ~40 KB for the whole 10,162-entry card, against the ~6 MB
// a `Vec<Entry>` would want (~590 B/entry, measured on this part). The file
// handle stays open for the life of the run and the engine seeks back into
// it for one `Entry` at a time, only when it is about to open the next GIF.
// There is therefore nothing to tune here: the playlist is the whole card.

/// When non-empty, only these platform prefixes play (e.g. `&["MD", "SNES"]`).
/// Empty means "everything on the card", which is the shipped behaviour.
pub const PLATFORMS: &[&str] = &[];

/// How many recently played entries the shuffle refuses to repeat.
pub const NO_REPEAT_WINDOW: usize = 32;

// ---------------------------------------------------------------------------
// Loop timing
// ---------------------------------------------------------------------------
/// Nothing to draw yet: back off briefly rather than spinning the CPU.
pub const IDLE_SLEEP: Duration = Duration::from_millis(5);
/// A frame whose deadline has already passed: yield the tick, don't skid.
pub const CATCHUP_SLEEP: Duration = Duration::from_millis(1);
/// Nothing playable, or no card: log and retry on this cadence, forever.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(5);
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
