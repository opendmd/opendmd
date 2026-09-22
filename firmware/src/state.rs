//! What the display loop publishes, and what it accepts back.
//!
//! The HTTP handlers run on the httpd task (CPU0) and the player runs on the
//! main task (CPU1). They meet in exactly two places, and nowhere else:
//!
//! - [`Status`] behind an `Arc<Mutex<_>>`, written by the main loop (cheap
//!   fields once per frame, the rest at each heartbeat or GIF change) and only
//!   ever read by the handlers.
//! - [`Command`] over an `mpsc` channel, sent by the handlers and drained by
//!   the main loop at the top of every iteration. Anything that touches the
//!   engine, the panel or a file the player has open happens *there*, on the
//!   main task, between frames.
//!
//! Commands that need an answer carry their own reply channel, so the handler
//! blocks on the httpd task (which is allowed to wait) instead of the display
//! loop blocking on the handler (which is not).

use std::sync::mpsc;

/// Free byte-addressable heap, in bytes.
///
/// `esp_get_free_heap_size` is `MALLOC_CAP_DEFAULT`, so the 27 KB IRAM-only
/// region does not inflate it — this is memory a `Vec<u8>` could actually use.
/// Both the display loop and the HTTP handlers read it: the loop to report the
/// floor, the handlers to decide whether they can afford to serve at all.
pub fn free_heap() -> u32 {
    // SAFETY: an argument-free read of ESP-IDF's heap accounting.
    unsafe { esp_idf_sys::esp_get_free_heap_size() }
}

/// The player's health and what it is doing, as `/api/status` reports it.
#[derive(Clone, Debug)]
pub struct Status {
    pub version: &'static str,
    pub uptime_secs: u64,
    pub ip: String,
    pub wifi: &'static str,
    pub hostname: String,
    // Now playing
    pub ordinal: u64,
    pub path: String,
    pub title: String,
    pub paused: bool,
    pub entries: usize,
    pub manifest: String,
    // Heap: `floor` is our own sampled low-water mark, which is the one to
    // trust; `idf_min` is esp_get_minimum_free_heap_size(), printed only so the
    // discrepancy stays visible (it is the sum of each region's own minimum).
    pub heap_free: u32,
    pub heap_floor: u32,
    pub heap_idf_min: u32,
    // The display's health. These three are the acceptance metrics for the
    // networking phase: if serving files costs the panel anything, it shows up
    // here first.
    pub refresh_hz: u8,
    pub deadline_misses: u64,
    pub worst_lateness_ms: u64,
    pub gifs_started: u64,
    pub sd_mounted: bool,
    pub sd_capacity_bytes: u64,
}

impl Status {
    pub fn new(version: &'static str, hostname: String) -> Self {
        Status {
            version,
            uptime_secs: 0,
            ip: String::new(),
            wifi: "disconnected",
            hostname,
            ordinal: 0,
            path: String::new(),
            title: String::new(),
            paused: false,
            entries: 0,
            manifest: String::new(),
            heap_free: 0,
            heap_floor: 0,
            heap_idf_min: 0,
            refresh_hz: 0,
            deadline_misses: 0,
            worst_lateness_ms: 0,
            gifs_started: 0,
            sd_mounted: false,
            sd_capacity_bytes: 0,
        }
    }
}

/// Where the playlist index a command ended up with came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexSource {
    /// `<manifest>.idx` was already on the card and current for this manifest
    /// length and filter; nothing was rebuilt.
    Loaded,
    /// A fresh `<manifest>.idx` was written (one streaming pass over the
    /// manifest) and renamed into place.
    Built,
    /// The card would not take the `.idx` (read-only, full), so the offsets are
    /// held in RAM as before — ~40 KB for the stock card.
    InRam,
}

impl IndexSource {
    pub fn as_str(self) -> &'static str {
        match self {
            IndexSource::Loaded => "loaded",
            IndexSource::Built => "built",
            IndexSource::InRam => "in-ram",
        }
    }
}

/// What came of a command that changes the playlist.
///
/// Every playlist change is applied in place now: the index lives on the card
/// (`<manifest>.idx`), so rebuilding it needs a 1 KB read buffer and a 2 KB
/// write buffer rather than the ~64 KB contiguous block the old in-RAM index
/// passed through — which is what used to force a reboot once WiFi was up.
/// `index` and `index_ms` say how the catalog was obtained and how long the
/// display was held for it.
#[derive(Clone, Copy, Debug)]
pub struct Applied {
    pub entries: usize,
    pub index: Option<IndexSource>,
    pub index_ms: u64,
}

impl Applied {
    /// Nothing was rebuilt; `entries` is what the running engine already had.
    pub fn unchanged(entries: usize) -> Self {
        Applied {
            entries,
            index: None,
            index_ms: 0,
        }
    }
}

/// The reply channel every command that changes state carries.
pub type Reply = mpsc::Sender<Result<Applied, String>>;

/// Work for the main task. Everything the web API can change goes through here.
pub enum Command {
    /// End the current GIF at the next tick.
    Skip,
    /// Toggle pause and report the resulting state.
    TogglePause(mpsc::Sender<bool>),
    /// Persist a new configuration and apply what can be applied live.
    ///
    /// NVS is owned by the main task, so the handler cannot write it itself.
    /// Brightness is one FFI call and dwell is a setter on the running engine;
    /// a change of platform filter, active set or shuffle mode rebuilds the
    /// engine, and a change of WiFi credentials only takes effect at the next
    /// boot. Replies `Ok(entries)` or `Err(message)`.
    ApplyConfig {
        settings: Box<crate::nvs_config::Settings>,
        reply: Reply,
    },
    /// Rebuild the engine and its on-card index — a fresh streaming pass over
    /// the manifest even if the `.idx` looks current, done between GIFs and
    /// never on the httpd task.
    Reindex(Reply),
    /// Swap an uploaded manifest into place and reindex.
    ///
    /// The engine holds `dest` (and `dest.idx`) open, so the httpd task must
    /// not touch them: it streams the upload to `temp` and hands the swap over
    /// here, where the engine is dropped first, the stale `.idx` is deleted,
    /// and only then is the manifest replaced. Replies `Ok(entries)` or
    /// `Err(message)`.
    InstallManifest {
        temp: String,
        dest: String,
        reply: Reply,
    },
}
