//! OpenDMD core: everything that is not a device driver.
//!
//! No hardware, windowing or filesystem dependencies. `std` is used freely
//! (the ESP32 firmware target is `std`-capable via ESP-IDF), but the only
//! outside crates are `gif` (streaming decoder), `color_quant` (the ODM
//! encoder's quantiser, already a dependency of `gif`) and `log`.

pub mod canvas;
pub mod engine;
pub mod filter;
pub mod index;
pub mod layout;
pub mod manifest;
pub mod odm;
pub mod player;
pub mod scheduler;

pub use canvas::{Canvas, DirtyMap, Rgb, HEIGHT, WIDTH};
pub use engine::{
    Catalog, DwellPolicy, Engine, FileCatalog, NowPlaying, ReadSeek, Source, TickOutcome,
    OFF_PLAYLIST,
};
pub use filter::Filter;
pub use index::{write_index, IndexHeader, IndexedCatalog};
pub use manifest::{
    entry_at, index_matching, normalize_title, parse_manifest, render_manifest, scan, Entry,
};
pub use odm::{encode_gif, EncodeStats, OdmSource};
pub use player::{FrameSource, GifSource};
pub use scheduler::{Mode, Scheduler};
