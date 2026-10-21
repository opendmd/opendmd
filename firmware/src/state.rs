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

use crate::chan as mpsc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Which flash layout this build is for, compiled in by `build.rs`
/// (`/api/status` `variant`, the boot banner).
pub const FLASH_VARIANT: &str = env!("OPENDMD_FLASH_VARIANT");

/// The firmware's version: **the one source of truth** for `/api/status`,
/// `GET /api/firmware`, the boot banner and the post-update check. It is
/// the app descriptor's (`"<version> <variant>"`, which the Makefile writes
/// from `FW_VERSION`, by default `Cargo.toml`'s), i.e. what a bundle for
/// this image carries in its header; [`BUILD_VERSION`] (the same
/// `FW_VERSION`, passed to `build.rs`) if the descriptor is in another form.
pub fn firmware_version() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        let desc = running_version();
        opendmd_core::ota::version_from_app_desc(&desc, FLASH_VARIANT)
            .unwrap_or(BUILD_VERSION)
            .to_string()
    })
}

/// `FW_VERSION` at build time (`build.rs`: `OPENDMD_FW_VERSION`, else the
/// crate's version).
pub const BUILD_VERSION: &str = env!("OPENDMD_FW_VERSION");

/// The running firmware's version, as the bootloader records it in the app
/// descriptor (`"<version> 4mb"` for `make fw-release`, `git describe` for a
/// bench build).
pub fn running_version() -> String {
    // SAFETY: `esp_app_get_description` returns a pointer into the app image
    // header, which is mapped for the life of the program.
    unsafe {
        let desc = esp_idf_sys::esp_app_get_description();
        if desc.is_null() {
            return "unknown".to_string();
        }
        std::ffi::CStr::from_ptr((*desc).version.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

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

/// Largest single allocation currently possible from byte-addressable heap.
pub fn largest_free_block() -> usize {
    // SAFETY: a read-only query with a capability mask argument.
    unsafe { esp_idf_sys::heap_caps_get_largest_free_block(esp_idf_sys::MALLOC_CAP_8BIT) }
}

/// The phosphor glow holds its 12 KB copy now (`/api/status` `glow_held`),
/// and Follow RePlayOS's attract mode shows its live item (`attract_live`,
/// the path from the `attract` setting) instead of a playlist. Flags the
/// main loop sets rather than [`Status`] fields, so the looks pass costs
/// the status object no heap; `look` and `glow` are read from the settings.
pub static GLOW_HELD: AtomicBool = AtomicBool::new(false);
pub static ATTRACT_LIVE: AtomicBool = AtomicBool::new(false);

/// Safe start (`main.rs` `SafeStart`): the uptime second at which the
/// brightness ceiling lifts from `min(max_brightness, BRIGHTNESS_SAFE_MAX)`
/// to `max_brightness`, 0 once it has (`/api/status` `safe_start_until`).
pub static SAFE_START_UNTIL: AtomicU32 = AtomicU32::new(0);
/// The network worker has published an address (joined the WiFi, or raised
/// the setup AP): safe start's window is counted from here too.
pub static NET_UP: AtomicBool = AtomicBool::new(false);

/// The prepared copy the player opened ahead for the next file
/// (`Engine::ahead_step`, `sd::SdSource::open_ahead`), as
/// `layout::prepared_key` of its card path (no mount point); 0 for none. Set
/// when the copy opens and cleared when that handle closes
/// (`frametime::Counted`), so it covers the copy while it waits and while it
/// plays. The handlers that replace or delete a file check it
/// (`http::skip_if_playing`), as they check the path on screen: FAT must not
/// have an open file replaced or unlinked. A key, not the path, so setting
/// it allocates nothing on the display loop.
///
/// `SeqCst` on both sides, with `cache_builder::busy()`'s count: a handler
/// takes `busy()` and then reads this, the open stores this and then reads
/// `transfer_in_flight()` (giving up if it is set), so either the handler
/// sees the key and asks for the copy back, or the open sees the transfer
/// and opens nothing.
pub static AHEAD_KEY: AtomicU32 = AtomicU32::new(0);

/// Whether `fs_path` (under the mount point) is the copy opened ahead, or
/// the GIF whose copy it is: a handler given a GIF's path that touches its
/// copy too (a delete, a `forget`, a move) must have both let go.
pub fn is_ahead(fs_path: &str) -> bool {
    use opendmd_core::layout::{cache_path, prepared_key};
    let key = AHEAD_KEY.load(Ordering::SeqCst);
    if key == 0 {
        return false;
    }
    let rel = fs_path
        .strip_prefix(crate::config::SD_MOUNT)
        .unwrap_or(fs_path);
    prepared_key(rel) == key || cache_path(rel).is_some_and(|c| prepared_key(&c) == key)
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
    /// The main loop's clock (`Duration`s below are since this); a handler
    /// reads it to stamp `t_ms` and to extrapolate the position.
    pub epoch: Instant,
    /// The frame on the glass: where it starts in the file's timeline
    /// (`NowPlaying::elapsed`, loops included), its 1-based number in the
    /// file, when it went up (after the blit) and when the next one is due.
    /// Written by the display loop with the other per-frame fields, from
    /// values it already holds; `/api/status` turns them into `elapsed_ms`
    /// as of the moment it is built, so the loop does no extra work for it.
    pub frame_elapsed: Duration,
    pub frame: u64,
    pub frame_shown_at: Duration,
    pub frame_due_at: Duration,
    /// When the pause began (valid while `paused`): the position is frozen
    /// there.
    pub paused_at: Duration,
    pub entries: usize,
    /// The manifest actually in play (filesystem path), as of the last
    /// playlist build: `/sdcard/filelist.txt` for `set` = `""`. Always a list
    /// of GIF paths; prepared copies are resolved per file by `SdSource`.
    pub manifest: String,
    /// `"building"` while the main task is (re)building the playlist index —
    /// several seconds for the stock card, during which a config PUT or a
    /// manifest install is waiting for its reply — `"ready"` once an engine is
    /// playing, `"unavailable"` if the last build failed (no card, no
    /// manifest, nothing matched) and the loop is retrying.
    pub index_state: &'static str,
    // Heap: `floor` is our own sampled low-water mark, which is the one to
    // trust; `idf_min` is esp_get_minimum_free_heap_size(), printed only so the
    // discrepancy stays visible (it is the sum of each region's own minimum).
    pub heap_free: u32,
    pub heap_floor: u32,
    pub heap_idf_min: u32,
    /// Who was running on each core, and the largest free block, when
    /// `heap_floor` was reached — a hint (`frametime::FloorAt`).
    pub heap_floor_at: Option<crate::frametime::FloorAt>,
    // The display's health. These three are the acceptance metrics for the
    // networking phase: if serving files costs the panel anything, it shows up
    // here first.
    pub refresh_hz: u8,
    pub deadline_misses: u64,
    pub worst_lateness_ms: u64,
    /// The misses by cause and by lateness (`frametime`).
    pub misses: crate::frametime::Misses,
    pub gifs_started: u64,
    pub sd_mounted: bool,
    /// The mounted card's filesystem answered the boot task's probe (a
    /// `read_dir` of the root). False with `sd_mounted` true: a card that
    /// mounts at the SD level whose filesystem cannot be read (exFAT, a
    /// 64 GB card as sold), which is never handed to the player.
    pub sd_readable: bool,
    pub sd_capacity_bytes: u64,
    /// Why the card is not mounted (the last attempt's error), why a
    /// mounted one cannot be read, or why it stopped (a card failure).
    pub sd_error: Option<String>,
    /// Mount attempts so far (1 on a normal boot).
    pub sd_attempts: u32,
    /// A format of the unreadable card is asked for or running (`POST
    /// /api/card/format` sets it, the boot task clears it when done).
    pub sd_formatting: bool,
    /// Why the last format failed, until the next one is asked for.
    pub sd_format_error: Option<String>,
    /// Why the chip last reset (`esp_reset_reason`), e.g. `panic`, `poweron`.
    pub reset_reason: &'static str,
    /// The playlist's resolved overrides (`.meta`), and how many of them hide
    /// an entry.
    pub overrides: usize,
    pub hidden: usize,
    // The schedule, as last applied by the main loop.
    /// `"normal"`, `"dim"` or `"off"` (night mode).
    pub mode: &'static str,
    /// When night mode ends, `"HH:MM"` local.
    pub mode_until: Option<String>,
    /// The rule in force (by position), the playlist it chose (`None`: the
    /// saved one) and when it ends.
    pub rule: Option<usize>,
    pub rule_set: Option<String>,
    pub rule_until: Option<String>,
    /// While a manual wake has the schedule suspended: until when, `"HH:MM"`.
    pub wake_until: Option<String>,

    /// The zone's display name (`Europe/Dublin`).
    pub tz_name: String,
    /// The panel timing: saved, running, a trial and its deadline, a staged
    /// trial (`/api/config` `panel*`), published by the main task.
    pub panel: opendmd_core::panel_timing::Tuner,
    /// Recently played (`opendmd_core::history`): the main loop records each
    /// start with the path above; `/api/status?recent=1` lists it and
    /// [`Command::Back`] steps through it. ~16 B here, 640 B of text from the
    /// first start.
    pub recent: opendmd_core::history::History,
}

impl Status {
    pub fn new(version: &'static str, hostname: String, epoch: Instant) -> Self {
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
            epoch,
            frame_elapsed: Duration::ZERO,
            frame: 0,
            frame_shown_at: Duration::ZERO,
            frame_due_at: Duration::ZERO,
            paused_at: Duration::ZERO,
            entries: 0,
            manifest: String::new(),
            index_state: "building",
            heap_free: 0,
            heap_floor: 0,
            heap_idf_min: 0,
            heap_floor_at: None,
            refresh_hz: 0,
            deadline_misses: 0,
            worst_lateness_ms: 0,
            misses: Default::default(),
            gifs_started: 0,
            sd_mounted: false,
            sd_readable: false,
            sd_capacity_bytes: 0,
            sd_error: None,
            sd_attempts: 0,
            sd_formatting: false,
            sd_format_error: None,
            reset_reason: "unknown",
            overrides: 0,
            hidden: 0,
            mode: "normal",
            mode_until: None,
            rule: None,
            rule_set: None,
            rule_until: None,
            wake_until: None,
            tz_name: String::new(),
            panel: opendmd_core::panel_timing::Tuner::boot(None, None),
            recent: opendmd_core::history::History::new(),
        }
    }
}

impl Status {
    /// A card the device can use: mounted, and its filesystem readable.
    pub fn sd_usable(&self) -> bool {
        self.sd_mounted && self.sd_readable
    }

    /// Where the file on screen is, in its own timeline, at `t` (since
    /// `epoch`): the frame's start plus the time it has been up, but never
    /// past its hold — a frame that is late (a deadline miss) is still the
    /// one on the glass, so the position waits with it. Frozen while paused.
    pub fn position_at(&self, t: Duration) -> Duration {
        if self.frame == 0 {
            return Duration::ZERO;
        }
        let at = if self.paused { self.paused_at } else { t };
        let hold = self.frame_due_at.saturating_sub(self.frame_shown_at);
        self.frame_elapsed + at.saturating_sub(self.frame_shown_at).min(hold)
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
    /// Overrides resolved into the playlist (its `.meta`), when rebuilt,
    /// and how many of them hide an entry.
    pub overrides: usize,
    pub hidden: usize,
}

impl Applied {
    /// Nothing was rebuilt; `entries` is what the running engine already had.
    pub fn unchanged(entries: usize) -> Self {
        Applied {
            entries,
            index: None,
            index_ms: 0,
            overrides: 0,
            hidden: 0,
        }
    }
}

/// The reply channel every command that changes state carries.
pub type Reply = mpsc::Sender<Result<Applied, String>>;

/// The error [`Command::ApplyConfig`] replies with when the requested set's
/// manifest does not open; the handler turns exactly this into a 404.
pub const SET_NOT_FOUND: &str = "set not found";

/// Why [`Command::PlayNow`] did not start the file. Each carries the message.
#[derive(Debug)]
pub enum PlayError {
    /// The file did not open (404); what was on screen keeps playing.
    NotFound(String),
    /// It opened but is not a GIF (415); the schedule moves on.
    Unplayable(String),
    /// There is no playlist running to interrupt (503).
    NoPlaylist,
}

/// Work for the main task. Everything the web API can change goes through here.
pub enum Command {
    /// End the current GIF at the next tick.
    Skip,
    /// Let go of the copy opened ahead for the next file (a handler is
    /// about to replace or delete it: `http::skip_if_playing`); the pick
    /// stays and opens when it plays. Answered once the handle is closed
    /// (the loop reads commands only between frames, so a long frame delay
    /// is waited out, not guessed at).
    ReleaseAhead(mpsc::Sender<()>),
    /// Toggle pause and report the resulting state.
    TogglePause(mpsc::Sender<bool>),
    /// Play this card-absolute path (`/gifs/…`; its prepared copy if there is
    /// one, exactly as for a playlist entry) now, then carry on
    /// with the schedule (`Engine::play_now`). The open happens on the main
    /// task, between frames; the reply is `Err` if it failed, in which case
    /// what was on screen keeps playing.
    PlayNow {
        path: String,
        reply: mpsc::Sender<Result<(), PlayError>>,
    },
    /// Previous: play the entry of Recently played one older than the one on
    /// screen (`History::back`), like `PlayNow`; `NotFound` if there is none.
    Back(mpsc::Sender<Result<(), PlayError>>),
    /// Persist a new configuration and apply what can be applied live.
    ///
    /// NVS is owned by the main task, so the handler cannot write it itself.
    /// Brightness is one FFI call and dwell is a setter on the running engine;
    /// a change of platform filter, active set or shuffle mode rebuilds the
    /// engine, and a change of WiFi credentials only takes effect at the next
    /// boot. Replies `Ok(entries)` or `Err(message)`.
    ///
    /// A set change is checked **before anything is saved**: the new manifest
    /// is opened first and, if it will not open, the reply is
    /// [`SET_NOT_FOUND`] with NVS and the running engine untouched. (Saving
    /// first used to leave the board with no playlist, retrying forever,
    /// across reboots.)
    ///
    /// `panel` is the request's panel-timing operation (`panel`,
    /// `panel_confirm`), carried out after the save (`Tuner::apply`); a
    /// confirm with no trial running is refused first with
    /// [`opendmd_core::panel_timing::NO_TRIAL`], nothing saved.
    ApplyConfig {
        settings: Box<crate::nvs_config::Settings>,
        panel: Option<opendmd_core::panel_timing::PanelOp>,
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
    /// Put a playlist's overrides file (`.meta`) in place — `temp` renamed
    /// over `dest` — or remove it (`temp: None`), then reindex if it belongs
    /// to the playlist in play. Exactly the manifest install's sequence
    /// (engine dropped, `.idx` removed first), because the index carries the
    /// resolved overrides and a fingerprint of this file.
    InstallMeta {
        temp: Option<String>,
        dest: String,
        reply: Reply,
    },
    /// Store a new schedule (validated JSON, `""` = none) in NVS and
    /// evaluate it at once.
    ///
    /// (Every reply here is a [`Reply`]: each distinct channel type costs
    /// several KB of code, and the image is close to its slot.)
    SetSchedule { json: String, reply: Reply },
    /// Suspend the schedule for this many minutes (0 resumes it now). The
    /// local `"HH:MM"` it lasts until is in `Status::wake_until` by the time
    /// the reply is sent.
    Wake { minutes: u32, reply: Reply },
    /// `POST /api/panel/test`: start (or restart) the panel-timing test of
    /// this setting, or end it (`None`), putting back what ran before
    /// (`opendmd_core::panel_test`). Refused during a panel trial
    /// ([`opendmd_core::panel_test::TRIAL_RUNNING`]) or with the panel off
    /// for the night ([`opendmd_core::panel_test::PANEL_OFF`]).
    PanelTest {
        setting: Option<opendmd_core::panel_test::Setting>,
        reply: Reply,
    },
    /// Follow RePlayOS: what the network worker's poller decided (attract,
    /// a game started, its context index is ready, back to normal). No
    /// reply; see `follow::Runner`.
    Follow(crate::follow::Event),
}
