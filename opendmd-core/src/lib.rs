//! OpenDMD core: everything that is not a device driver.
//!
//! No hardware, windowing or filesystem dependencies. `std` is used freely
//! (the ESP32 firmware target is `std`-capable via ESP-IDF), but the only
//! outside crates are `gif` (streaming decoder) and `log`.

pub mod canvas;
pub mod engine;
pub mod filter;
pub mod manifest;
pub mod player;
pub mod scheduler;

pub use canvas::{Canvas, Rgb, HEIGHT, WIDTH};
pub use engine::{DwellPolicy, Engine, NowPlaying, Source, TickOutcome};
pub use filter::Filter;
pub use manifest::{normalize_title, parse_manifest, render_manifest, Entry};
pub use player::GifSource;
pub use scheduler::{Mode, Scheduler};
