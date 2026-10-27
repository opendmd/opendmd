//! Every tunable in one place: pin map, panel timing, playlist policy.
//!
//! The pin numbers are measured, not guessed — see `hardware/PINOUT.md` in the
//! repository root (continuity walk, 2026-09-22, DMDos board v3).

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
// Panel timing (clock edge, latch blanking, driver chip, data clock, minimum
// refresh) is a setting now: `opendmd_core::panel_timing::PanelTiming`, NVS
// `panel`, `/api/config` `panel`, Settings -> Advanced -> Panel timing. Its
// defaults are the constants that used to live here (positive edge, 1 clock
// of blanking, SHIFTREG, 8 MHz, 60 Hz). A change is tried for
// `panel_timing::TRIAL` and reverts unless kept; see docs/firmware.md.
//
// The driver chip is unknown, so the default is a plain shift register.
// **If the panel stays completely dark, try FM6126A first** — it is by far
// the most common panel that needs an init sequence, and it shows as "no
// output at all" rather than as corruption.
// ---------------------------------------------------------------------------
/// Single-buffered on purpose. Double buffering composes a frame off-screen
/// and swaps, so a blit can never tear — but it costs a second DMA framebuffer
/// plus descriptors (~40 KB of heap, measured: 80 KB -> 120 KB free). A 128x32
/// blit takes ~1 ms against a 10 ms refresh, and eyes-on-panel testing across
/// scrolls, sprites and logos found no visible tearing (2026-09-22). It is
/// there (2026-10-26: a rolling seam on scrolling text, Insert_Coin.gif;
/// 78-93% of blits on the bench straddle a refresh), and the fix that costs
/// no heap is pacing the blit to the scan (`panel_sync`,
/// `panel.rs`), not a second buffer.
pub const DOUBLE_BUFFER: bool = false;

/// 0..255. The panels are extremely bright; this is a living-room level.
pub const BRIGHTNESS: u8 = 96;

/// The default for the `max_brightness` setting, and the ceiling during
/// [`SAFE_START`]: the highest brightness taken as safe on the reference
/// rig's power supply. The panels and the ESP32 share one 5 V supply.
///
/// Measured 2026-09-29 on the rig (DMDPower supply, both panels, the
/// all-white GIF as a one-GIF playlist: the worst case), each trial either
/// 60 s under `make api-load` (3 workers, WiFi transmitting) or a reboot
/// into white at that level watched to 90 s uptime:
///
/// | brightness | trials | failures |
/// |---|---|---|
/// | 144 | 2 load + 5 boot | 0 |
/// | 160 | 5 load + 20 boot on white, + 1 load + 3 boot on the normal playlist | 0 (per-trial rate < ~9% at 95%) |
/// | 168 | 2 load + 5 boot (+ 1 step) | 3 brownout resets, mostly under load |
/// | 192 | — | froze the chip at the IDF's default detector level; a clean brownout reset at level 7 |
///
/// 144 is one measurement step (16) below the highest clean level and 24
/// below the first failing one: the error bar. The margin will differ with
/// other supplies, cables and panels; Settings warns above this value, and
/// safe start plus the level-7 brownout detector make a too-high setting
/// recoverable (the device resets and is reachable for ~30 s).
pub const BRIGHTNESS_SAFE_MAX: u8 = 144;

/// Safe start: how long after boot the panel is held at
/// `min(max_brightness, BRIGHTNESS_SAFE_MAX)` before the saved
/// `max_brightness` takes over — counted from the later of boot and the
/// network coming up (joined, or the setup AP), so a device whose saved
/// brightness browns it out is still reachable for this long after any
/// reset. See "Safe start" in CLAUDE.md.
pub const SAFE_START: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// microSD, SPI (not SDMMC) on VSPI (`SPI3`, IO_MUX pins). hardware/PINOUT.md, "microSD".
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
/// serving or the upload it is streaming to the card; the FAT index keeps
/// its own file open (`fatidx.rs`), hence 10 (552 B each with 512 B sectors).
pub const SD_MAX_FILES: usize = 10;
/// The SD bus clock. The SPI master divides 80 MHz by a whole number, so
/// the only clocks are 40, 26.7, 20, 16 … MHz (the nearest is taken: 26 and
/// 32 MHz both give 26.7). 40 MHz needs the IO_MUX pins, i.e. `SPI3` (see
/// `sd.rs`); through the GPIO matrix the driver refuses anything over
/// 26.7 MHz. 40 MHz is the top: the next divider is 80 MHz itself, over the
/// card's 50 MHz high-speed limit.
///
/// Stepped on the rig (2026-09-28, the carrier's long unshielded wiring,
/// `filelist.txt` playing, Dissolve on; each step: 3 min idle, 90
/// back-to-back `GET /filelist.txt` in three runs of 30 (the `0x107`
/// repro), `api-load WORKERS=3` 120 s, 10 `gifs/Consoles` GETs, 10 min idle,
/// plus 40 copies rebuilt by the cache builder):
///
/// | clock | card errors / `0x107` | api-load misses (read) | ≥ 250 ms late | `gifs/Consoles` scan |
/// |---|---|---|---|---|
/// | 20 MHz (`SPI2`, before) | — (short run) | +81 (74) | 31 | 142 ms per 1,000 entries |
/// | 26.7 MHz (`SPI2`) | 0 / 0 of 90 | +60 (39) | 20 | 127 ms |
/// | 40 MHz, `SPI2` | driver refuses the clock (see "When the card will not mount") | | | |
/// | **40 MHz (`SPI3`)** | **0 / 0 of 90** | +75 (69) | 20 | **115 ms** |
///
/// Every step mounted on the first attempt and passed its 10 min idle.
/// Directory scans are mostly per-sector latency, not bus time: 2x the
/// clock is ~19% off a scan. See "Frame time" in CLAUDE.md.
pub const SD_SPEED_KHZ: u32 = 40_000;
/// Per-command timeout for every mount attempt after the first (the IDF
/// default, used by the first, is 1 s): a card still finishing what a reset
/// interrupted may take longer to come out of busy. See `sd::SdBus`.
pub const SD_RETRY_CMD_TIMEOUT_MS: u32 = 3_000;
/// Mount attempts at boot before the network and the web server are started
/// anyway; the boot task (`boot.rs`) keeps trying every [`RETRY_INTERVAL`]
/// after that.
pub const SD_BOOT_ATTEMPTS: u32 = 2;

// Card failure while running (`sdhealth.rs`, CLAUDE.md "When the card will
// not mount"). Only card-class I/O errors count (`EIO`, `ETIMEDOUT`,
// `ENODEV` from FATFS: a command timeout, a CRC error, a card gone), never
// a missing file or a GIF that will not decode.

/// Card I/O errors within [`SD_FAIL_WINDOW`] that mean the card, not a
/// file, is failing (at least one must be on a file that had read fine).
pub const SD_FAIL_TRIP: usize = 3;
/// The window [`SD_FAIL_TRIP`] errors must fall in.
pub const SD_FAIL_WINDOW: Duration = Duration::from_secs(60);
/// The clock a card that failed at [`SD_SPEED_KHZ`] mounts at from then on
/// (under both the SD default-speed 25 MHz every card supports and the
/// GPIO matrix's 26.7 MHz cap, so the driver can never refuse it at the
/// clock switch). Kept per card, by its CID serial, in NVS.
pub const SD_SPEED_FALLBACK_KHZ: u32 = 20_000;
/// At most this many restarts for card failure within [`SD_REBOOT_WINDOW`];
/// after that the device stays up without the card and remounts it every
/// [`SD_REMOUNT_INTERVAL`] instead.
pub const SD_REBOOT_CAP: u32 = 3;
pub const SD_REBOOT_WINDOW: Duration = Duration::from_secs(15 * 60);
/// How often a device that has used up its restarts tries the card again;
/// also how often a card that mounts but cannot be read
/// ([`SD_UNREADABLE`]) is remounted, to notice it being swapped.
pub const SD_REMOUNT_INTERVAL: Duration = Duration::from_secs(60);

// The card's clock (`card_speed`, `cardspeed.rs`).
const _: () = assert!(SD_SPEED_KHZ == opendmd_core::card_speed::FULL_KHZ);
const _: () = assert!(SD_SPEED_FALLBACK_KHZ == opendmd_core::card_speed::SAFE_KHZ);
/// How long after the handover the probe of a card at full speed starts
/// (`cardspeed::probe_soon`): past the first GIF's open and the network's
/// start.
pub const SD_PROBE_DELAY: Duration = Duration::from_secs(3);
/// What the probe reads: this much from sector 8192 (4 MiB, past the
/// partition table's gap: where a card formatted for SD, or by the device,
/// has its boot sector and FAT). The errors are the bus's, not the data's.
pub const SD_PROBE_START: u32 = 8192;
pub const SD_PROBE_BYTES: u32 = 4 * 1024 * 1024;
/// In reads of this many 512 B sectors (the buffer, 2 KB, is the probe
/// task's, freed with it)...
pub const SD_PROBE_CHUNK: u32 = 4;
/// ... with this pause after each, so the card is the player's most of
/// the time.
pub const SD_PROBE_PAUSE: Duration = Duration::from_millis(2);
/// The probe task's stack.
pub const SD_PROBE_STACK: usize = 4 * 1024;
/// The probe starts only with this much largest free block: its task
/// (4 KB stack, ~0.4 KB TCB) and its 2 KB buffer, with the rest of the
/// heap's margin left (CLAUDE.md, "Heap ceiling").
pub const SD_PROBE_HEAP: usize = 12 * 1024;
/// While the cache builder's task is up or a transfer shares the card the
/// probe waits (polling this often), for at most this long in all, then
/// gives up: their own reads slow a card that cannot keep up just the same.
pub const SD_PROBE_BUSY_POLL: Duration = Duration::from_millis(250);
pub const SD_PROBE_WAIT: Duration = Duration::from_secs(60);
/// The largest free internal block below which the clock is not changed
/// in place (the SPI device is removed and added again: a few hundred
/// bytes, and a failed add would leave the card with no device at all).
pub const SD_CLOCK_HEAP: usize = 2048;
/// `sd.error` for a card that mounts at the SD level but whose filesystem
/// does not answer (every access `ENODEV`): exFAT, which the FATFS build
/// does not include, i.e. a 64 GB card as it comes. The web app formats it
/// FAT32 (`POST /api/card/format`, which works on any size). CLAUDE.md,
/// "When the card will not mount".
pub const SD_UNREADABLE: &str = "card not readable: it needs FAT32; format it from the web app";
/// The format's work buffer: `f_mkfs` writes the FAT in pieces of this
/// size. Measured on the spare's 64 GB card: 63.6 s with 4 KB, 62.6 s with
/// 16 KB — the card's write time, not the pieces, sets the pace.
pub const SD_FORMAT_BUF: usize = 4096;
/// How long a planned restart (`crate::restart`) waits for the log ring to
/// reach the console first.
pub const RESTART_LOG_FLUSH: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// Playlist
// ---------------------------------------------------------------------------
/// The stock card's playlist file, read from the card root.
pub const MANIFEST_PATH: &str = "/sdcard/filelist.txt";
/// A manifest from before prepared copies were resolved automatically (a
/// playlist of paths into the retired pre-rendered format's tree), once the
/// default when present. It is **ignored** now (logged once at boot):
/// `filelist.txt` is the default, and [`crate::sd::SdSource`] plays each
/// GIF's prepared copy when it exists. Kept only so it can be recognised;
/// `set` may not name it.
pub const OBSOLETE_MANIFEST: &str = "filelist.odm.txt";

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
/// With the formula look's buffers held (20.5 KB, `formula_look.rs`) the
/// transition gate also keeps this much more: the look is not given back
/// during a transition (it is the user's chosen setting and the dissolve is
/// cosmetic), so the dissolve is cut sooner instead and the network side's
/// transients still find room. Measured on the device later.
pub const LOOK_TRANSITION_MARGIN: usize = 8 * 1024;

/// The next file is opened ahead (`Engine::ahead_step`, "The next file,
/// ahead" in CLAUDE.md) only while a step leaves this much free after the
/// bytes it takes ([`AHEAD_OPEN_BYTES`], [`AHEAD_PARSE_BYTES`]) and a 16 KB
/// block exists. Derived from the transition gate's arithmetic so a copy
/// held ahead never costs the switch its transition: the gate wants the
/// transition's 24 KB plus [`TRANSITION_HEAP_RESERVE`] free with the new
/// file's tables already allocated — which they are either way, opened
/// ahead or at the switch, so they are counted once, here — and the file
/// on screen gives back at least a copy's ~4 KB at the switch. Above the
/// ~20 KB the network side's transients need while it is held.
pub const AHEAD_HEAP_RESERVE: usize = 24 * 1024 + TRANSITION_HEAP_RESERVE - 4 * 1024;
/// Heap an open copy's handle takes (the `File`, FATFS's `FIL` with its
/// 512 B sector buffer, the boxed reader): rounded up from the probe's
/// measurement.
pub const AHEAD_OPEN_BYTES: usize = 1536;
/// Heap the copy's decoder takes when its header is read: gifslim's 2 KB
/// read buffer, the 1.5 KB of cap-9 tables, the row, a palette of up to
/// 768 B, allocator overhead.
pub const AHEAD_PARSE_BYTES: usize = 5 * 1024;
/// What an original's decoder takes on top of that when it is held ahead
/// (opened through the FAT index): cap-12 LZW tables instead of cap 9's.
pub const AHEAD_ORIGINAL_BYTES: usize = 11 * 1024;
/// A copy held ahead is let go (the pick is kept) when the main loop sees
/// the free heap under this: the network side's transients come first, as
/// for the glow's copy ([`GLOW_YIELD_HEAP`]).
pub const AHEAD_YIELD_HEAP: usize = 20 * 1024;
/// A step ahead runs between frames only if the next frame is at least this
/// far off: the step's worst case on the card (rig and spare, probe builds)
/// plus a margin. The pick (an index read and a manifest line), the copy's
/// open (the bucket lookup, the open and its magic) and the header (a 2 KB
/// read and the tables).
pub const AHEAD_PICK_SLACK: Duration = Duration::from_millis(9);
pub const AHEAD_OPEN_SLACK: Duration = Duration::from_millis(16);
pub const AHEAD_PARSE_SLACK: Duration = Duration::from_millis(7);

/// Phosphor glow (`glow` short/long) keeps a 12 KB copy of the panel while
/// it is lit (`Panel::blit`). It takes the copy only with at least this much
/// free (so ~28 KB stay for everything else), and gives it back as soon as
/// the main loop sees the free heap under [`GLOW_YIELD_HEAP`] — the network
/// side's transients (an upload, three downloads at once) come first. Before
/// the yield, `make api-load WORKERS=3` with glow on took the sampled floor
/// from 16.9 KB (glow off) to 4.7 KB (largest block 1.5 KB) on a probe build.
pub const GLOW_TAKE_HEAP: usize = 40 * 1024;
pub const GLOW_YIELD_HEAP: usize = 20 * 1024;

/// A formula's still cache (16 KB a value it works out from the pixel's
/// position alone, `opendmd_psi::Still`: the same picture either way,
/// faster with it) is taken only if this much stays free after it: the
/// network side's transients (~20 KB, as the glow's and the copy ahead's
/// yields assume) and a margin. A transition never meets it: the engine
/// drops the formula on screen before the next file opens (so a switch away
/// pays nothing for the cache), and a switch to a formula takes the
/// transition's buffers before its first frame, where the cache is taken
/// (refused then, it is asked again a second later). It is given back as
/// soon as the main loop's next frame sees the free heap under
/// [`STILL_SHED_HEAP`], and never held while a formula look is set (the look
/// has the heap first). Was 48 KB in p2probe-a: with ~80-90 KB free while a
/// formula played, Tunnel's 32 KB never got in.
#[cfg(feature = "psi")]
pub const STILL_KEEP_HEAP: usize = 32 * 1024;
#[cfg(feature = "psi")]
pub const STILL_SHED_HEAP: usize = 24 * 1024;

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

/// How often the network worker tries again to give the mDNS responder the
/// display's name after that failed (`net_worker::MdnsName`).
pub const MDNS_RETRY: Duration = Duration::from_secs(5);
/// How long the network worker listens for another device answering to a
/// name (`net_worker::name_check`, an mDNS A query). A responder answers a
/// unique record at once; the query ends early on the first answer.
pub const NAME_QUERY: Duration = Duration::from_millis(1200);
/// How long a handler waits for that answer (`GET /api/name-check`, a
/// `PUT /api/config` that changes the name), on the httpd task: the query
/// plus room for the worker to finish what it was doing (a Follow poll).
/// A worker busy for longer (a connect: up to 34 s) gets no say: the check
/// reads "could not ask" and a save goes ahead, and the worker skips a
/// query that would end after the handler stopped waiting.
pub const NAME_CHECK_WAIT: Duration = Duration::from_secs(3);
/// Improv Serial (`improv.rs`): the join for WiFi details sent over USB
/// ends within this of their arrival (every wait in `wifi.rs` is clipped to
/// it). ESP Web Tools waits 45 s from sending them: 34 s, then the save
/// ([`IMPROV_SAVE_TIMEOUT`], 8 s at worst), then starting mDNS and SNTP
/// (well under a second) puts the answer at ~43 s at worst.
pub const IMPROV_ANSWER_WITHIN: Duration = Duration::from_secs(34);
/// How long the network worker waits for the main task to save WiFi
/// details joined over USB.
pub const IMPROV_SAVE_TIMEOUT: Duration = Duration::from_secs(8);
/// An Improv job that waited this long for the network worker (busy with
/// its first connect, a cache count) is answered without being done: the
/// installer is about to give up on it (scans: 30 s; WiFi details: 45 s,
/// and a join needs ~20 s of [`IMPROV_ANSWER_WITHIN`]).
pub const IMPROV_STALE: Duration = Duration::from_secs(15);
/// A scan this recent answers an Improv scan again instead of a new one
/// (the installer rescans every 3 s while its WiFi form is open; a scan
/// takes the radio for ~2 s).
pub const IMPROV_SCAN_FRESH: Duration = Duration::from_secs(5);

/// Network worker thread: pinned to CPU0, named. stripe-dashboard uses 16 KB
/// here; 12 KB is what fits alongside the panel, the manifest index and a
/// playing GIF, and the deepest path (scan -> connect) has been exercised on
/// the rig at this size.
pub const NET_WORKER_STACK: usize = 12 * 1024;
/// The boot task's stack (`boot.rs`, CPU0, alive only until the card is
/// handed over): the web app's CRC pass, the HTTP server's start, the mount
/// attempts and the playlist index (opened, or built: one streaming pass
/// with its buffers on the heap), as the main task (16 KB) did them before.
/// The `boot task done (stack N B unused)` line says what was left.
pub const BOOT_STACK: usize = 10 * 1024;
/// The httpd task's stack, and one contiguous internal allocation — which is
/// why the server is started before the first GIF is decoded. stripe-dashboard
/// needs 16 KB because its OTA handler buffers on the stack; ours reads into a
/// heap buffer. Measured high-water (the `httpd task: stack high-water` log
/// line) at 10 KB: 5.2 KB used over a device `web-smoke` pass (config,
/// schedule, playlist upload), 3.2 KB through a full `POST /api/ota` (gone). It was
/// 10 KB until file streaming moved to its own task, then 8 KB. Since uploads
/// moved there too (the upload path, with its log line, is what reached
/// 5.2 KB: it measures ~4.0 KB on the stream task), the deepest left is
/// parsing a config or schedule body: 4.1 KB used (invalid `PUT
/// /api/config` and `/api/schedule` bodies, which are parsed whole and then
/// refused); OTA's 3.2 KB went with OTA. 6.5 KB leaves ~2.4 KB over that, and gives
/// the stream task's extra 1.5 KB back.
pub const HTTP_STACK: usize = 6 * 1024 + 512;

/// The stack a formula is compiled on (`psi_stack.rs`): Psi's compiler
/// recurses to `psi::MAX_DEPTH` levels, ~430 B each on the ESP32. At the
/// old cap of 24 its worst used 11,808 B inside a thread (`psi-probe` on
/// the spare, 2026-10-26; 9,664 B on 2026-10-25, before the text
/// built-ins), more than this stack less its ~800 B of entry frames and
/// interrupts: a deep enough formula could restart the player. With the
/// cap at 18, ~9.2 KB (opendmd-psi's `tests/stack.rs` holds the host's
/// figure to a budget calibrated on that probe). Taken from the heap for
/// the compile only.
#[cfg(feature = "psi")]
pub const PSI_COMPILE_STACK: usize = 12 * 1024;
/// The file-streaming task's stack (`stream.rs`): `files::serve` and, since
/// uploads moved here, `files::stream_body_to` and the manifest/`.meta`
/// installs, chunk buffers on the heap, plus logging. Also one contiguous
/// internal allocation, taken at the same time as the httpd task's. Measured:
/// 2.9 KB used at most by downloads alone (stalled client, 4 MB GIF,
/// api-load, web-smoke, page-load bursts); 3.95 KB with uploads (656 B left
/// of 4.5 KB after card-syncs and a trickle abort, whose warn line is the
/// deepest path), so 6 KB, for ~2 KB spare — until the listings moved here
/// (2026-10-26): `/api/cache/list`'s bucket walk left **784 B** of 6 KB
/// (`stream_stack` in `/api/status`, the spare, its populated 11k-copy card,
/// 3 runs; a `gifs/Consoles` listing and `/api/cache/failed` went no
/// deeper), so 7 KB, for ~1.8 KB spare.
pub const STREAM_STACK: usize = 7 * 1024;
/// An upload (`PUT /api/files/…`, `/api/manifest`, `/api/sets/…`) is received
/// on the stream task, and gives up when one read waits this long for data.
/// A trickle does not get round it: [`UPLOAD_MAX_TIME`] caps the whole body.
pub const UPLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest one request body may take to arrive, counted from when its
/// reception starts (a queued upload's wait is not counted): 3 MB at the
/// ~100-200 KB/s a laptop gets is 15-30 s, so this only ends a client that
/// is trickling or stuck. `PUT /api/www` (a ~0.8 MB pack) has the same cap.
pub const UPLOAD_MAX_TIME: Duration = Duration::from_secs(120);
/// Everything streamed to or from the card moves in chunks this size: big
/// enough to keep SPI efficient, small enough that the display loop never
/// waits long for the FAT lock — and small enough to afford, because each
/// in-flight request holds one of these on a heap that has under 20 KB free
/// while a GIF is playing.
pub const IO_CHUNK: usize = 2048;
/// `PUT /api/www` sleeps this long after each 4 KB sector it erases, so the
/// display loop runs between erases (each stops both CPUs' flash caches for
/// ~15-20 ms). Costs ~4 s on a 0.8 MB pack (~190 sectors).
pub const WWW_ERASE_BREATHER: Duration = Duration::from_millis(20);
/// `PUT /api/www` reads its staged pack back from the card twice (the check,
/// then the copy into flash); a read the card fails is tried this many times
/// in all, the file opened again each time (`www::Staged`). Seen: a 64 GB
/// card at 40 MHz failed one read with a data CRC error, the rest read fine.
pub const WWW_CARD_READ_TRIES: u32 = 4;
/// The pause before a retry, times the attempt number (100, 200, 300 ms).
pub const WWW_CARD_RETRY_PAUSE: Duration = Duration::from_millis(100);
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
/// How long a transfer already under way waits between chunks for the heap
/// to climb back over [`HEAP_RESERVE`] before it gives up. Giving up cuts the
/// response short — the client sees fewer bytes than the `Content-Length`, or
/// a chunked body with no end — so it is always logged, with the path, the
/// bytes sent of the total and the free heap, and counted
/// (`streams_aborted` in `/api/status` and the heartbeat).
pub const HEAP_WAIT: Duration = Duration::from_secs(10);
/// Free heap a response must find before its first byte goes out: the
/// reserve plus 8 KB, the lwip buffers of one transfer in flight with room
/// to spare. Below it the request is answered `503` + `Retry-After: 2`
/// before any body, rather than started and then abandoned half way — a
/// refusal a client can retry, where a truncated body is a corrupt file.
pub const STREAM_START_HEAP: u32 = HEAP_RESERVE + 8 * 1024;

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
/// control sockets and a spare), + 1 for the captive-portal DNS server, + 3
/// for uploads, which leave httpd's sockets for the stream task's
/// (`stream.rs` asserts the sum).
///
/// It is not a safety limit on the heap, and was measured not to be (1, 2
/// and 3 were all tried when every download ran on the httpd task: none
/// stopped three concurrent multi-hundred KB downloads running the heap to
/// zero). What bounds the heap now is that only one file is ever being sent;
/// a waiting request costs its ~1.7 KB request copy and an idle TCP pcb.
///
/// **Why 7 and not the 4 the stream task needs.** httpd's LRU purge evicts
/// the least recently used *idle* keep-alive connection when a new one
/// arrives and every slot is taken. A browser keeps up to six connections
/// to one host and reuses whichever is idle, so with four slots the purge
/// regularly closed a connection at the instant the browser was sending
/// its next request on it: `ERR_EMPTY_RESPONSE`, seen on `GET /api/cache`
/// in 4 of 6 device smoke runs (2026-09-29), with nothing in the device's
/// log because nothing went wrong on its side. Seven slots hold the
/// browser's six plus one evictable, so the purge only ever closes a
/// connection the browser has abandoned. An idle keep-alive socket costs
/// a TCP pcb (~200 B) and nothing else until it is used.
pub const HTTP_MAX_SOCKETS: u16 = 7;

/// Where the playlists live on the card. (The web app is not on the card: it
/// is a www pack in flash, `www.rs`.)
pub const SETS_DIR: &str = "/sdcard/sets";
/// Smooth motion's list (`/sets/.smooth`, one GIF path a line): the GIFs
/// whose prepared copy is the tweened one. The web app keeps it; the
/// firmware only renames a line when a GIF is moved (`POST /api/files/move`).
pub const SMOOTH_LIST: &str = "/sdcard/sets/.smooth";

// ---------------------------------------------------------------------------
// Playback cache (prepared copies)
// ---------------------------------------------------------------------------
/// Where prepared copies live: `cache/<Category>/<xx>/<name>.gif`, the
/// layout of `opendmd_core::layout` (`docs/cache-format.md`). `SdSource`
/// looks here before every GIF.
pub const CACHE_ROOT: &str = "/sdcard/cache";
/// The retired pre-rendered format's tree. Never read: its presence is
/// logged once at boot, and `DELETE /api/files/odm?recursive=1` removes it.
pub const LEGACY_ROOT: &str = "/sdcard/odm";
/// A recursive `DELETE` (`?recursive=1`, under `cache/` and the legacy tree
/// only) works on the httpd task for at most this long per request, then
/// answers `"done":false` and the client asks again.
pub const RECURSIVE_DELETE_BUDGET: Duration = Duration::from_secs(4);
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
/// The `/api/cache` count's pacing. It runs on the network worker at
/// priority 5 on CPU0, where the cache builder (priority 1) and the stream
/// task (2) only get the CPU when it blocks, and a `yield_now` only goes to
/// equal or higher priorities: flat out, a count mid-way through a builder
/// file starved it of the CPU until FATFS's 10 s lock wait gave up
/// (`card error ... Connection timed out`). So the count **sleeps**
/// [`CACHE_PACE_SLEEP`] after every bucket directory it reads and after
/// every [`CACHE_PACE_BYTES`] of manifest (~15% of CPU0 left to the lower
/// priorities, the lock free while it sleeps). Nothing on the heap.
pub const CACHE_PACE_BYTES: usize = 2048;
/// See [`CACHE_PACE_BYTES`]. Two ticks at `CONFIG_FREERTOS_HZ=1000`.
pub const CACHE_PACE_SLEEP: Duration = Duration::from_millis(2);
/// How long a count waits for the cache builder to finish the file it has
/// in hand before counting anyway (beside the encoder: a small key slice,
/// many manifest passes). See `cache::run_count`.
pub const CACHE_BUILDER_WAIT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// The cache builder (`cache_builder.rs`): the device preparing its own copies
// ---------------------------------------------------------------------------
/// The builder task's stack: `gifslim::recode_gif_streaming` and FATFS.
/// The heartbeat's high-water (`stack … B unused`): with the old
/// pre-rendered encoder through the `gif` decoder ~6.1 KB was used at most
/// (8.5 KB left 2.5 KB spare); the first run of this encoder at 8.5 KB left
/// only 700 B (gifslim then copied a local palette into a 768-byte stack
/// buffer on top of its 256-entry colour table; it reads it straight into
/// the table now). 9.5 KB keeps ~2.5 KB spare. Taken when the task starts
/// (only while there is work it may do) and given back when it exits;
/// every KB of it is a KB the drip pace needs free on top (its start
/// threshold is [`BUILDER_MIN_HEAP`] + this + 1 KB).
pub const BUILDER_STACK: usize = 9 * 1024 + 512;
/// Below the stream task (2): the builder runs only when nothing that
/// answers a client wants CPU0.
pub const BUILDER_PRIORITY: u8 = 1;
/// The encoder's peak heap for a 128x32 GIF, the largest over the whole
/// stock card by the host's count (`encodeprobe --heap`: mean 43,181 B, max
/// 43,648): gifslim at cap 12 as the source decoder (~15 KB), a canvas (12
/// KB), the previous frame's indices (4 KB), changed bits, the colour table,
/// the LZW dictionary at cap 9 and the write buffer. The rig agrees: ~66 KB
/// free with the task up and idle, 20.3-22.9 KB while encoding.
pub const BUILDER_ENCODER_PEAK: u32 = 43_648;
/// The same for the card's wider strays (132x32, 135x32): 56,483 B.
pub const BUILDER_WIDE_ENCODER_PEAK: u32 = 56_483;
/// Heap left free at the encoder's peak, for everything else on the board
/// at the same moment: WiFi's RX buffers, an lwip connection, a Follow
/// RePlayOS poll (~4-6 KB), a status reply, a heartbeat. With the old ~9-10
/// KB (52.5 KB thresholds against a 43.6 KB peak) a night run reached a
/// sampled floor of 5,916 B (idf-min 1,504 B) with the encoder at ~22 KB
/// free: something transient took ~16 KB on top of it.
pub const BUILDER_PEAK_MARGIN: u32 = 20 * 1024;
/// A file is only started at the full pace (paused, night mode "off", no
/// playlist) with this much heap free, the task already running: the
/// encoder's peak plus [`BUILDER_PEAK_MARGIN`] (64,128 B, 62.6 KB). The
/// task itself is only started with its stack + 1 KB more than this free
/// (74,880 B). At night on the rig the builder saw ~66 KB free between
/// files with the task up (~78 KB before it started), so both still pass —
/// with ~2 KB to spare per file: anything that takes 2 KB more for good
/// stops the night run (`paused: low memory`). Nothing animates at this
/// pace and the player opens nothing without stopping the builder first
/// (`player_opening`), and every transfer stops it too.
pub const BUILDER_MIN_HEAP_FULL: u32 = BUILDER_ENCODER_PEAK + BUILDER_PEAK_MARGIN;
/// [`BUILDER_MIN_HEAP_FULL`] at the drip pace (`auto_prepare = "always"`,
/// a prepared file playing): 1.5 KB more, as before, since the player's
/// frames and transitions go on beside it (65,664 B; the task starts at
/// 76,416 B). A prepared file on screen leaves ~74-78 KB free, so the drip
/// starts less often than it did at 54 KB; with an original on screen the
/// pace stops it anyway (`gif on screen`).
pub const BUILDER_MIN_HEAP: u32 = BUILDER_MIN_HEAP_FULL + 1536;
/// A GIF gifslim does not decode (interlaced frames, `Previous` disposal: 5
/// of the stock card's 11,047) is decoded with the `gif` crate instead only
/// if this much is free when gifslim gives up: the encoder then peaks at
/// ~84 KB instead of ~43 KB (`encodeprobe --heap`: the crate's ~41 KB, a
/// 12 KB `Previous` snapshot, a 16 KB block among them), and
/// [`BUILDER_ABORT_HEAP`] must still be left. On this board that is never
/// free while the builder runs, so in practice those files are recorded as
/// failed, "too large to prepare here", and the browser prepares them.
pub const BUILDER_FULL_DECODER_HEAP: u32 = 60 * 1024;
/// Mid-file, the encode is abandoned (and its heap freed) if free heap falls
/// under this: with [`BUILDER_PEAK_MARGIN`] a file only gets here if
/// something else took 8 KB more than the margin allows for.
pub const BUILDER_ABORT_HEAP: u32 = 12 * 1024;
/// Drip pace: the sleep before every 2 KB of card I/O and every `stat` while
/// a file plays (frames are CPU0 work and are not paced: the card is what the
/// builder shares with the player).
pub const BUILDER_DRIP_PAUSE: Duration = Duration::from_millis(5);
/// After a transfer ends the builder waits this long before starting again
/// (a browser fetching several files would otherwise have it start and
/// abandon a file between each).
pub const BUILDER_SETTLE: Duration = Duration::from_secs(2);
/// A file abandoned this many times in a row at the drip pace (it does not
/// fit in one file's turn on screen) is skipped while dripping and left for
/// the next full-speed pass.
pub const BUILDER_DRIP_TRIES: u32 = 2;
/// While the player waits for the builder to let go of its heap, the task
/// runs at this priority (httpd's and the network worker's).
pub const BUILDER_RELEASE_PRIORITY: u8 = 5;
/// How long the player's open waits, once the heap is back, for the task to
/// be gone too (its stack is what a GIF's transition would need).
pub const BUILDER_EXIT_WAIT: Duration = Duration::from_millis(20);
/// How long the player's open waits for the builder to free its heap.
pub const BUILDER_RELEASE_WAIT: Duration = Duration::from_millis(1000);
/// At most one start attempt this often. The task exits whenever it may not
/// work (giving its stack and TCB, ~9.4 KB of heap, back to the player) and
/// is started again by the main loop when it may.
pub const BUILDER_RESPAWN: Duration = Duration::from_secs(1);
/// The builder waits at most this long for a running `/api/cache` count
/// (see `cache_builder::work`, step 4) before starting a file anyway.
pub const BUILDER_COUNT_WAIT: Duration = Duration::from_secs(120);
/// After a card error (not the GIF's fault), wait this long before retrying.
pub const BUILDER_CARD_BACKOFF: Duration = Duration::from_secs(30);
/// The largest logical screen the builder will take on (the card's biggest
/// is 135x32; the encoder's buffers scale with it).
pub const BUILDER_MAX_PIXELS: usize = 2 * 128 * 32;
/// GIFs the builder could not prepare, one `path<TAB>reason` a line; skipped
/// by every later pass. Delete it to retry them.
pub const BUILDER_FAILED_PATH: &str = "/sdcard/cache/.failed";
/// At most this many failures are remembered (4 B each while a pass runs).
pub const BUILDER_MAX_FAILED: usize = 1024;

// --- Follow RePlayOS (`follow.rs`) ---------------------------------------------

/// RePlayOS's REST port (`http://<pi>:55356/api/v1/`).
pub const FOLLOW_PORT: u16 = 55356;
/// Connect and read timeout for one poll.
pub const FOLLOW_TIMEOUT: Duration = Duration::from_secs(3);
/// The whole of one poll after the connect (request, headers, body) must
/// be over by then, however the server trickles (`follow::Deadlined`).
pub const FOLLOW_DEADLINE: Duration = Duration::from_secs(6);
/// A `get_status` reply is read (and scanned, `follow::StatusScan`) up to
/// this many bytes; a real one is ~300 B. The scan needs no buffer, so this
/// only bounds the time a strange reply can keep the worker reading.
pub const FOLLOW_MAX_BODY: usize = 16 * 1024;
/// Where the context indexes (and the logos list) are cached.
pub const FOLLOW_CTX_DIR: &str = "/sdcard/sets/.ctx";
/// At most this many context indexes are kept; the oldest go first.
pub const FOLLOW_CTX_MAX: usize = 64;
/// The title card is held this long at most for a context index that has
/// not arrived; then the playlist plays on (and the context swaps in when it
/// comes).
pub const FOLLOW_CONTEXT_WAIT: Duration = Duration::from_secs(30);
/// How often the poller logs its request count and heap figures.
pub const FOLLOW_REPORT: Duration = Duration::from_secs(300);

// ---------------------------------------------------------------------------
// The FAT index (`fatidx.rs`)
// ---------------------------------------------------------------------------
/// Index misses (a `/gifs` open it could not serve) that mark it stale.
pub const FATIDX_STALE_REBUILD: u32 = 16;
/// Seconds after the last change through the API before a rebuild.
pub const FATIDX_QUIET_S: u32 = 30;
/// Seconds between rebuilds for staleness (a missing index is built at once).
pub const FATIDX_MIN_GAP_S: u32 = 600;
/// No build in the first seconds after boot: the start-up show, the first GIF.
pub const FATIDX_BOOT_DELAY_S: u32 = 20;
/// Free heap a build needs to start: its task's stack (8 KB), a 4 KB
/// sector batch, a 1 KB writer and the group (`FATIDX_GROUP`), over the
/// network side's transients.
pub const FATIDX_BUILD_HEAP: usize = 48 * 1024;
/// The build's task stack (it exists only while it builds): the walk goes
/// through FatFs, card-speed's transfer and error path (`cardspeed`:
/// a failed read is recovered and the clock slowed on the caller's task)
/// and the SD SPI driver, under two closures.
pub const FATIDX_STACK: usize = 10 * 1024;
/// The most records a build gathers per walk, in bytes (8 a record).
pub const FATIDX_GROUP: usize = 16 * 1024;
