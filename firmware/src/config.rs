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
/// Single-buffered on purpose. Double buffering composes a frame off-screen
/// and swaps, so a blit can never tear — but it costs a second DMA framebuffer
/// plus descriptors (~40 KB of heap, measured: 80 KB -> 120 KB free). A 128x32
/// blit takes ~1 ms against a 10 ms refresh, and eyes-on-panel testing across
/// scrolls, sprites and logos found no visible tearing (2026-09-22). If a
/// future panel shows it, the cheaper fallback is double buffering at 6-bit
/// colour (48 KB instead of 64 KB), not going back to 8-bit double buffering.
pub const DOUBLE_BUFFER: bool = false;

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
/// FATFS file descriptors. Three are live in the player's steady state (the
/// catalog's handles on `filelist.txt` and `filelist.txt.idx`, and the GIF the
/// engine is playing); the httpd task needs a few more for the file it is
/// serving or the upload it is streaming to the card.
pub const SD_MAX_FILES: usize = 8;
/// Conservative: the carrier's SD wiring is long and unshielded.
pub const SD_SPEED_KHZ: u32 = 20_000;

// ---------------------------------------------------------------------------
// Playlist
// ---------------------------------------------------------------------------
/// The stock card's playlist file, read from the card root.
pub const MANIFEST_PATH: &str = "/sdcard/filelist.txt";
/// A manifest from before prepared copies were resolved automatically: the
/// same playlist with `/odm/...` paths, once the default when present. It is
/// **ignored** now (logged once at boot): `filelist.txt` is the default, and
/// [`crate::sd::SdSource`] plays each GIF's prepared copy when it exists.
/// Kept only so it can be recognised; `set` may not name it.
pub const OBSOLETE_ODM_MANIFEST: &str = "filelist.odm.txt";

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

/// Shuffle by default; `false` plays the manifest in order.
pub const SHUFFLE: bool = true;

/// Dwell policy defaults, in seconds. `opendmd_core::engine::DwellPolicy`
/// ends a GIF's turn once it has looped at least once *and* `min` has
/// elapsed, or unconditionally at `max`.
pub const DWELL_MIN_SECS: u64 = 3;
pub const DWELL_MAX_SECS: u64 = 30;

/// How a new file comes in, unless NVS says otherwise (`transition`, as
/// `kind:ms`). A transition holds 24 KB of buffers while it runs (see
/// `opendmd_core::compositor`), so the heap dips by that much for its
/// duration and nothing in between.
pub const TRANSITION: opendmd_core::compositor::Transition =
    opendmd_core::compositor::Transition::DEFAULT;
/// A transition only starts if the heap keeps at least this much free once
/// it has taken its 24 KB *and* the new GIF's decoder has taken its first
/// frame's ~34 KB (the transition begins before that frame is decoded);
/// otherwise the file cuts in. Measured on the rig (probe build, skipping
/// every 3 s): with a GIF playing the lowest free heap during a transition
/// was ~16 KB, one GIF (`NEW 2021/2a.gif`) left 4.9 KB, and during a
/// web-smoke run (files served, the cache counted) the LZW allocation of a
/// GIF's first frame failed and aborted the board — before this gate
/// counted the decoder. Refusals are the heartbeat's `transitions cut`.
pub const TRANSITION_HEAP_RESERVE: usize = 12 * 1024;

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

// ---------------------------------------------------------------------------
// Networking (phase 3a). Everything network-shaped runs on CPU0; the display
// loop stays on CPU1 and never calls into any of it.
// ---------------------------------------------------------------------------
/// Hostname (mDNS `<name>.local` and the DHCP client name) unless NVS says
/// otherwise.
pub const HOSTNAME: &str = "opendmd";

/// The timezone until one is chosen in Settings: UTC. A POSIX TZ string (what
/// newlib's `localtime_r` reads) and the name the UI shows for it.
pub const TZ: &str = "UTC0";
pub const TZ_NAME: &str = "UTC";

/// How often the main loop looks at the clock for the schedule. It acts only
/// when the local minute changes (or the schedule, the zone or a wake does).
pub const SCHEDULE_CHECK: Duration = Duration::from_secs(1);
/// The frame loop's sleep while night mode has the panel off: commands are
/// still drained every iteration, nothing is decoded.
pub const OFF_SLEEP: Duration = Duration::from_millis(50);
/// The longest `POST /api/wake` honours.
pub const WAKE_MAX_MINUTES: u32 = 24 * 60;
/// At most this many schedule rules.
pub const SCHEDULE_MAX_RULES: usize = 16;
/// The open setup network raised when there are no credentials, or when STA
/// association has failed [`STA_MAX_FAILURES`] times.
pub const AP_SSID: &str = "OpenDMD-Setup";
/// AP mode is an open network, so it does not stay up forever: the board
/// reboots on this cadence, which retries STA if credentials have since been
/// saved and otherwise cycles back into AP with a fresh timeout.
pub const AP_TIMEOUT: Duration = Duration::from_secs(600);
/// Consecutive STA connect failures before falling back to AP mode.
pub const STA_MAX_FAILURES: u32 = 2;
/// How often the worker re-checks that STA is still associated.
pub const WIFI_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Network worker thread: pinned to CPU0, named. stripe-dashboard uses 16 KB
/// here; 12 KB is what fits alongside the panel, the manifest index and a
/// playing GIF, and the deepest path (scan -> connect) has been exercised on
/// the rig at this size.
pub const NET_WORKER_STACK: usize = 12 * 1024;
/// The httpd task's stack, and one contiguous internal allocation — which is
/// why the server is started before the first GIF is decoded. stripe-dashboard
/// needs 16 KB because its OTA handler buffers on the stack; ours reads into a
/// heap buffer. Measured high-water (the `httpd task: stack high-water` log
/// line) at 10 KB: 5.2 KB used over a device `web-smoke` pass (config,
/// schedule, playlist upload), 3.2 KB through a full `POST /api/ota`. It was
/// 10 KB until file streaming moved to its own task; 8 KB gives back 2 KB of
/// the ~7 KB that task costs and leaves ~2.8 KB over the deepest measured.
pub const HTTP_STACK: usize = 8 * 1024;
/// The file-streaming task's stack (`stream.rs`): `files::serve` with its
/// chunk buffer on the heap, plus logging. Also one contiguous internal
/// allocation, taken at the same time as the httpd task's. Measured at 6 KB:
/// 2.9 KB used at most (stalled client, 4 MB GIF, api-load, web-smoke,
/// page-load bursts), so 4.5 KB keeps ~1.6 KB spare.
pub const STREAM_STACK: usize = 4608;
/// Everything streamed to or from the card moves in chunks this size: big
/// enough to keep SPI efficient, small enough that the display loop never
/// waits long for the FAT lock — and small enough to afford, because each
/// in-flight request holds one of these on a heap that has under 20 KB free
/// while a GIF is playing.
pub const IO_CHUNK: usize = 2048;
/// Contiguous heap needed for the in-RAM playlist index, the fallback when the
/// card will not take `<manifest>.idx`.
///
/// `FileCatalog`'s `Vec<u32>` grows by doubling, so indexing the stock card's
/// 10,133 entries (40,532 B) passes through a 64 KB allocation. That fits on a
/// fresh heap and never once WiFi is up, where the largest free block measured
/// 12-39 KB. The on-card index (the normal path) needs none of this; the check
/// only guards the fallback after boot, which reboots rather than let the
/// allocator abort the board.
pub const IN_RAM_INDEX_MIN_BLOCK: usize = 64 * 1024;

/// The player's heap reserve: below this, the HTTP server stops serving files
/// rather than let the display loop fail an allocation.
///
/// This is not belt and braces, it is the load-bearing rule of the whole
/// phase. Opening a GIF needs a single 28,856 B block for the LZW decoder
/// (see "GIF heap budget" in the root CLAUDE.md), and free heap while playing
/// sits at 17-20 KB; three concurrent downloads were measured driving it to
/// zero, at which point the decoder's next allocation fails and Rust aborts
/// the board. So a file request that arrives under the reserve is refused with
/// 503, and a transfer already in flight pauses between chunks until the
/// player is clear. The display never waits for the network; the network waits
/// for the display.
///
/// The size is measured, not chosen. A transfer in flight is itself worth
/// ~11 KB of lwip buffers, so free heap *during* a large transfer sits at
/// 6-7 KB, and a reserve set anywhere near the 17-20 KB idle figure refuses
/// every request including the one that would have succeeded: at 12 KB the
/// load test got 4 responses out of 32, and at 8 KB a 450 KB manifest upload
/// throttled itself to death. 5 KB sits just above the cliff the aborts
/// happened at, so it throttles only when things are genuinely about to fail.
///
/// It is a mitigation, not a guarantee: lwip and the WiFi driver allocate on
/// their own account, outside any check we can make here.
///
/// Since the playlist index moved onto the card (~40 KB back), the sampled
/// floor under `api-load` is 38 KB with one client and 34 KB with three, so
/// the reserve no longer engages in normal use. It is left at the measured
/// cliff rather than raised: it is the backstop for when something else takes
/// that headroom.
pub const HEAP_RESERVE: u32 = 5 * 1024;
/// How long a stalled transfer will wait for the heap to recover before it
/// gives up and fails the request.
pub const HEAP_WAIT: Duration = Duration::from_secs(20);

/// Concurrent HTTP connections.
///
/// ESP-IDF's httpd is a single task, so this is not parallelism — it is how
/// many sockets, header buffers and lwip windows are alive at once. Files are
/// streamed one at a time by `stream.rs`, with up to `stream::QUEUE_DEPTH`
/// (2) more waiting, and each of those holds its socket marked in use, which
/// httpd's LRU purge will not evict. At least one socket must stay evictable
/// or a new connection can neither be accepted nor make room (see `stream`),
/// so this is 1 + 2 + 1 = 4; a compile-time assert there keeps it so.
/// `CONFIG_LWIP_MAX_SOCKETS` must be at least this + 3 (httpd's listening and
/// control sockets and a spare), + 1 for the captive-portal DNS server.
///
/// It is not a safety limit on the heap, and was measured not to be (1, 2
/// and 3 were all tried when every download ran on the httpd task: none
/// stopped three concurrent multi-hundred KB downloads running the heap to
/// zero). What bounds the heap now is that only one file is ever being sent;
/// a waiting request costs its ~1.7 KB request copy and an idle TCP pcb.
pub const HTTP_MAX_SOCKETS: u16 = 4;

/// The web app the browser runs, and the card-absolute roots the HTTP server
/// is allowed to read from or write to.
pub const WWW_ROOT: &str = "/sdcard/www";
pub const SETS_DIR: &str = "/sdcard/sets";
/// `git describe` of the web app on the card, written by `make web` and pushed
/// with it. Read at boot and whenever it is uploaded; `/api/status` reports it
/// as `www_version`.
pub const WWW_VERSION_PATH: &str = "/sdcard/www/version.txt";

// ---------------------------------------------------------------------------
// Playback cache (prepared copies)
// ---------------------------------------------------------------------------
/// Where prepared copies live: `odm/<Category>/<bb>/<name>.odm`, the bucketed
/// layout of `opendmd_core::layout`. `SdSource` looks here before every GIF.
pub const ODM_ROOT: &str = "/sdcard/odm";
/// How many prepared-copy keys (`u32` each) the `/api/cache` count holds at
/// once. The walk takes the buckets a slice at a time until this many keys are
/// held, then makes one pass over the manifest for that slice: 3,072 keys is
/// 12 KB of heap on the network worker, one pass for a card with a few thousand
/// copies and four for a fully prepared 11k-file card.
pub const CACHE_KEYS_PER_PASS: usize = 3072;
/// A cache recount waits until nothing has asked for one for this long, so a
/// browser uploading prepared copies one after another causes one count at the
/// end, not one per file.
pub const CACHE_DEBOUNCE: Duration = Duration::from_secs(3);
