//! OpenDMD core: everything that is not a device driver.
//!
//! No hardware, windowing or filesystem dependencies. `std` is used freely
//! (the ESP32 firmware target is `std`-capable via ESP-IDF), but the only
//! outside crates are `gif` (the fallback decoder, and the metadata the
//! tests read back), `color_quant` (the recoder's quantiser, already a
//! dependency of `gif`) and `log`.

pub mod canvas;
pub mod compositor;
pub mod engine;
pub mod filter;
pub mod follow;
pub mod gifslim;
pub mod index;
pub mod layout;
pub mod manifest;
pub mod overrides;
pub mod player;
pub mod schedule;
pub mod scheduler;
pub mod synth;
pub mod text;
pub mod www_pack;

pub use canvas::{Canvas, DirtyMap, Rgb, HEIGHT, WIDTH};
pub use compositor::{Kind as TransitionKind, Transition};
pub use engine::{
    Catalog, DwellPolicy, Engine, FileCatalog, NowPlaying, ReadSeek, Source, TickOutcome,
    OFF_PLAYLIST,
};
pub use filter::Filter;
pub use gifslim::{
    recode_gif, recode_gif_streaming, GifSlim, RecodeOptions, RecodeStats, PREPARED,
};
pub use index::{
    write_index, write_index_resolving, write_index_with_overrides, IndexHeader, IndexedCatalog,
};
pub use manifest::{
    entry_at, index_matching, normalize_title, parse_manifest, render_manifest, scan, Entry,
};
pub use overrides::{Override, OverrideSet, Overrides};
pub use player::{FrameSource, GifSource};
pub use scheduler::{Mode, Scheduler};

#[cfg(feature = "tween")]
pub mod tween;
