//! OpenDMD core: everything that is not a device driver.
//!
//! No hardware, windowing or filesystem dependencies. `std` is used freely
//! (the ESP32 firmware target is `std`-capable via ESP-IDF), but the only
//! outside crates are `gif` (the fallback decoder, and the metadata the
//! tests read back) and `log`.

// Code that runs once a minute or once a change (the sky, the input
// table), optimised for size on the ESP32 (`#[optimize(size)]`: the app's
// flash is the scarce thing there).
#![cfg_attr(target_arch = "xtensa", feature(optimize_attribute))]

pub mod access;
pub mod admit;
pub mod auto_prepare;
pub mod boot;
pub mod brightness;
pub mod canvas;
pub mod card;
pub mod card_speed;
pub mod chan;
pub mod compositor;
#[cfg(feature = "connect")]
pub mod connect;
pub mod engine;
pub mod fatdir;
pub mod fatread;
pub mod filter;
pub mod follow;
mod fonts;
pub mod frametime;
pub mod fx;
#[cfg(feature = "psi")]
pub mod fx_psi;
pub mod gifslim;
pub mod history;
pub mod hostname;
pub mod http_client;
pub mod improv;
pub mod index;
pub mod inputs;
pub mod layout;
pub mod look;
pub mod manifest;
pub mod names_file;
#[cfg(feature = "ota")]
pub mod ota;
pub mod overrides;
pub mod panel_test;
pub mod panel_timing;
pub mod player;
#[cfg(feature = "tags")]
pub mod query;
pub mod readahead;
pub mod restarts;
pub mod scan;
pub mod schedule;
pub mod scheduler;
pub mod settings_file;
pub mod sidecar;
pub mod sky;
pub mod synth;
#[cfg(feature = "tags")]
pub mod tags;
pub mod tree;
pub mod text;
pub mod updater_picture;
pub mod wifi_login;
pub mod www_pack;

pub use canvas::{Canvas, DirtyMap, Rgb, HEIGHT, WIDTH};
pub use compositor::{Kind as TransitionKind, Transition};
pub use engine::{
    Catalog, DwellPolicy, Engine, FileCatalog, FormulaKeep, NowPlaying, ReadSeek, SharedStr, Source, TickOutcome,
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
/// The Psi language (the `psi` feature): see [`fx_psi`].
#[cfg(feature = "psi")]
pub use opendmd_psi as psi;
pub use overrides::{Override, OverrideSet, Overrides};
pub use player::{FrameSource, GifSource};
pub use scheduler::{Mode, Scheduler};

#[cfg(feature = "tween")]
pub mod tween;
