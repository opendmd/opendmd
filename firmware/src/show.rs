//! What the panel shows while nothing plays, on the main task: the start-up
//! show from the moment the panel lights, then the status card whenever
//! there is no card to play from (`opendmd_core::boot` draws both).
//!
//! **When the show ends.** It runs at least `boot_show`'s minimum (1.3 s
//! short, 5.6 s full): a handover from the boot task that arrives earlier
//! waits for it ([`Screen::may_end`]), one that arrives later ends it at
//! once — so the first GIF comes at `max(minimum, card ready)`, with the
//! configured transition from the show's last frame. After
//! `opendmd_core::boot::CAP_MS` (8 s) without the card, or at the minimum
//! once the boot task has said there is no card, the status card takes over
//! (arriving from the show), and stays until something plays. `off`: the
//! panel stays dark until the first GIF, as before; the status card still
//! comes up when there is no card.
//!
//! **What it costs.** Nothing on the heap: the frames are drawn into the
//! main loop's canvas, the text comes from `Status` into a 16-byte buffer
//! every 250 ms, and the log lines are formatted into the log ring. A show
//! frame is ~4-6 ms (the tunnel, the stars and the name) plus a full blit.
//!
//! **After an update** (`Screen::new`'s `updated`: the first boot of a
//! version other than the one NVS says booted last, `nvs_config::Config::note_booted`)
//! the show's status line says "Updated to v<version>" for
//! `opendmd_core::boot::UPDATED_MS` (3 s) instead of the version, and the
//! show's minimum grows to match (3.9 s short, 6.85 s full; `off` plays the
//! short show for that boot). Every other boot is as before.
//!
//! **While a firmware update comes in** ([`Updating`], held by
//! `ota::receive` from the moment the bundle's header and signature are
//! accepted): the main loop shows a still message instead of anything else
//! ("Updating / to v… / Don't unplug", then "Installing / … / Don't
//! unplug" once it is staged, for the second before the restart into the
//! updater, which shows the same picture while it copies: its build-time
//! copy, `opendmd_core::updater_picture`, a dot stepping under it; with an
//! updater from before that, `crate::ota::updater_lights_panel` false, the
//! middle line says "Dark while it installs" instead). The
//! guard's drop (a refused, cut-short or failed upload) takes it down and
//! playback resumes. One relaxed atomic load per loop iteration otherwise.
//!
//! **A Formula as the show** (`boot_formula`, [`Formula`]): drawn in the
//! wordmark's place by `opendmd_core::boot::FormulaShow` (the frames
//! `@boot?formula=…` draws), for the same length, the information line over
//! it only when there is something to say (`Join WiFi`, a card problem, the
//! address, "Updated to"): never the version. A bundled or inline Formula
//! is built before the boot task starts (it needs neither the card nor
//! the network: its ~10-14 KB taken while the heap is whole); one on the
//! card waits (dark) for the boot task, which reads its `.psb` once the
//! card is mounted ([`Screen::card_formula`]: the display loop never
//! touches the card before the handover), and plays the wordmark if the
//! card is not there in [`CARD_WAIT_MS`]. Missing, broken, too heavy for
//! the show (`boot::FORMULA_MAX_EST_US`) or out of memory, the wordmark
//! plays, with a log line and `/api/status` `boot_formula.error`; short of
//! memory it is built again a second later (`fx_psi::look_retry`), the
//! wordmark meanwhile. The Formula is let go when the show ends (the
//! status card or the first GIF). `off` plays no show, Formula or not.
//!
//! **The log.** One line per beat of the show (`boot show: warp at 0 ms`,
//! `name`, `jump`, `status line`, `name settled`, `hold`), the card's
//! readiness, the status card's text whenever it changes, and the
//! handover — so a capture shows what the panel showed.

use opendmd_core::boot::{self, Info, ShowLength, UpdateStage, CAP_MS};
use opendmd_core::canvas::Canvas;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
#[cfg(feature = "psi")]
use std::time::Instant as RetryAt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config;
use crate::panel::Panel;
use crate::state::Status;

/// The card's first line when the card failed again with no restart left.
pub const CARD_FAILING: &str = "The card is failing";
/// ... and when the card is there but its playlist will not play.
const NO_PLAYLIST: &str = "No playlist on the card";
const EMPTY_PLAYLIST: &str = "The playlist is empty";
/// ... and when the card mounts but its filesystem cannot be read (exFAT:
/// `config::SD_UNREADABLE`), which the web app can format ...
const CARD_UNREADABLE: &str = "Card unreadable: format in app";
/// ... and while it does.
const CARD_FORMATTING: &str = "Formatting the card";
// The small font's 4 px advance: 32 characters fill the 128 px panel.
const _: () = assert!(CARD_UNREADABLE.len() <= 32 && CARD_FORMATTING.len() <= 32);
/// How often the text is read from status.
const TEXT_EVERY: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scene {
    /// The show since `Screen::started` (the panel dark for `off`).
    Show,
    /// The status card since `since`; `from`: arriving from the show (its
    /// length, and its time when the card took over).
    Card {
        since: Instant,
        from: Option<(ShowLength, u32)>,
    },
    /// Something plays.
    Playing,
}

/// What the status line and the card say, read from `Status`: no heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Text {
    ip: [u8; 16],
    ip_len: u8,
    ap: bool,
    note: &'static str,
}

impl Text {
    const NONE: Text = Text {
        ip: [0; 16],
        ip_len: 0,
        ap: false,
        note: "",
    };

    fn info(&self) -> Info<'_> {
        Info {
            version: crate::state::firmware_version(),
            ip: std::str::from_utf8(&self.ip[..usize::from(self.ip_len)]).unwrap_or(""),
            ap: if self.ap { config::AP_SSID } else { "" },
            note: self.note,
            updated: false,
        }
    }

    /// `problem`: what the main loop knows is wrong (the card failed, no
    /// playlist); otherwise the card's state as the boot task reports it.
    fn read(status: &Mutex<Status>, problem: Option<&'static str>, ready: bool) -> Text {
        let s = status.lock().unwrap();
        let mut t = Text::NONE;
        if s.wifi == "sta" {
            let n = s.ip.len().min(16);
            t.ip[..n].copy_from_slice(&s.ip.as_bytes()[..n]);
            t.ip_len = n as u8;
        }
        t.ap = s.wifi == "ap";
        t.note = match problem {
            Some(p) => p,
            None if ready => "",
            None if s.sd_formatting => CARD_FORMATTING,
            None if s.sd_mounted && !s.sd_readable => CARD_UNREADABLE,
            None if s.sd_mounted => "Loading the playlist",
            None => s.sd_error.as_deref().map_or("Reading the card", plain),
        };
        t
    }
}

/// A mount error in plain words. Without a card-detect pin a missing card
/// and one that does not answer look the same (a command timeout).
fn plain(e: &str) -> &'static str {
    if e.starts_with("card failed repeatedly") {
        "The card keeps failing"
    } else if e.contains("FAT") {
        "The card can't be read"
    } else {
        "Insert a microSD card"
    }
}

fn ms(d: Duration) -> u32 {
    d.as_millis().min(u128::from(u32::MAX)) as u32
}

// --- A firmware update's message ---------------------------------------------

/// [`update_stage`]'s values.
pub const NO_UPDATE: u8 = 0;
const RECEIVING: u8 = 1;
pub const INSTALLING: u8 = 2;
/// Installing, with an updater in `ota_0` that leaves the panel dark
/// (`crate::ota::updater_lights_panel` false): the screen says so.
pub const INSTALLING_DARK: u8 = 3;
static STAGE: AtomicU8 = AtomicU8::new(NO_UPDATE);
/// The version being installed (the bundle's, at most 32 bytes as its
/// header holds it): written before [`STAGE`] goes up, read by the main
/// loop when it draws.
static TO: Mutex<([u8; 32], u8)> = Mutex::new(([0; 32], 0));

/// What the panel should show for a firmware update: [`NO_UPDATE`], or a
/// stage to draw with [`draw_update`]. A relaxed load: the main loop's one
/// cost of this while no update runs.
#[inline(always)]
pub fn update_stage() -> u8 {
    STAGE.load(Relaxed)
}

/// Draw the message for `stage` (from [`update_stage`], not [`NO_UPDATE`]).
pub fn draw_update(canvas: &mut Canvas, stage: u8) {
    let to = TO.lock().unwrap_or_else(|p| p.into_inner());
    let v = std::str::from_utf8(&to.0[..usize::from(to.1)]).unwrap_or("");
    let stage = match stage {
        INSTALLING => UpdateStage::Installing,
        INSTALLING_DARK => UpdateStage::InstallingDark,
        _ => UpdateStage::Receiving,
    };
    boot::update_notice(canvas, stage, v);
}

/// A firmware update is being received: the panel says so while this
/// lives. Dropped (any early return: refused, cut short, a card error), the
/// message goes and playback resumes; on success the restart comes first.
pub struct Updating(());

impl Updating {
    pub fn begin(version: &str) -> Updating {
        {
            let mut to = TO.lock().unwrap_or_else(|p| p.into_inner());
            let n = version.len().min(32);
            to.0[..n].copy_from_slice(&version.as_bytes()[..n]);
            to.1 = n as u8;
        }
        STAGE.store(RECEIVING, Relaxed);
        crate::wake::kick();
        Updating(())
    }

    /// Staged and the updater chosen: the restart is next. `lit`: the
    /// updater shows the same picture while it installs; else the screen
    /// says the panel goes dark.
    pub fn installing(&self, lit: bool) {
        STAGE.store(if lit { INSTALLING } else { INSTALLING_DARK }, Relaxed);
        crate::wake::kick();
    }
}

impl Drop for Updating {
    fn drop(&mut self) {
        STAGE.store(NO_UPDATE, Relaxed);
        crate::wake::kick();
    }
}

// --- The start-up show's Formula ------------------------------------------------

/// How long a card Formula's show stays dark waiting for the card (ms into
/// the show) before the wordmark plays instead.
pub const CARD_WAIT_MS: u32 = 500;

/// What `/api/status` says of the start-up show's Formula this boot
/// (`boot_formula`): [`NO_FORMULA`] (none set, or the show `off`), played,
/// waiting for the card, or the wordmark in its place, and why
/// ([`BOOT_ERROR`]).
static BOOT_STATE: AtomicU8 = AtomicU8::new(NO_FORMULA);
pub const NO_FORMULA: u8 = 0;
pub const PLAYED: u8 = 1;
pub const WAITING: u8 = 2;
pub const FELL_BACK: u8 = 3;
/// Why the wordmark played in the Formula's place.
pub static BOOT_ERROR: Mutex<String> = Mutex::new(String::new());
/// A Formula plays as the start-up show: its still cache is never taken
/// (`main::still_gate`: 16 KB while the boot task opens the playlist's
/// index on the heap; the show's Formulas fit its frames without it).
pub static FORMULA_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// [`BOOT_STATE`].
pub fn boot_state() -> u8 {
    BOOT_STATE.load(Relaxed)
}

/// The start-up show's Formula as the show has it.
pub enum Formula {
    /// The wordmark (none set, or it fell back).
    Wordmark,
    /// A card Formula: dark until the boot task reads it.
    #[cfg_attr(not(feature = "psi"), allow(dead_code))]
    Waiting,
    /// Short of memory: built again at `at`, from `psb` (a card Formula's
    /// bytes, with its options) or else the path.
    #[cfg(feature = "psi")]
    Retry { at: RetryAt, psb: Option<(Vec<u8>, String)> },
    #[cfg(feature = "psi")]
    Playing(Box<boot::FormulaShow>),
}

impl Formula {
    /// The Formula `path` names (a `boot_formula` value) for a show of
    /// `len`: built now if it is bundled or inline (before the boot task
    /// starts, on a whole heap), waited for if it is on the card. Logged,
    /// and said in the status.
    pub fn new(path: &str, len: ShowLength) -> Formula {
        if path.is_empty() || len == ShowLength::Off {
            BOOT_STATE.store(NO_FORMULA, Relaxed);
            return Formula::Wordmark;
        }
        #[cfg(feature = "psi")]
        {
            if opendmd_core::synth::formula_file(path).is_some() {
                log::info!("boot show: formula {}, from the card once it is mounted", crate::short(path));
                BOOT_STATE.store(WAITING, Relaxed);
                return Formula::Waiting;
            }
            Formula::built(path, boot::FormulaShow::open(path), None)
        }
        #[cfg(not(feature = "psi"))]
        {
            fell_back(path, crate::state::NO_FORMULAS);
            Formula::Wordmark
        }
    }

    /// A build's outcome: playing, or the wordmark (again in a second if
    /// it was only short of memory, from `psb` if it came from the card).
    #[cfg(feature = "psi")]
    fn built(
        path: &str,
        r: Result<boot::FormulaShow, opendmd_core::psi::Error>,
        psb: Option<(Vec<u8>, String)>,
    ) -> Formula {
        match r {
            Ok(s) => {
                log::info!("boot show: formula {}", crate::short(path));
                BOOT_STATE.store(PLAYED, Relaxed);
                FORMULA_SHOWN.store(true, Relaxed);
                if let Ok(mut e) = BOOT_ERROR.lock() {
                    e.clear();
                }
                Formula::Playing(Box::new(s))
            }
            Err(e) => match opendmd_core::fx_psi::look_retry(&e) {
                Some(d) => {
                    log::warn!("boot show: no room for formula {} yet ({}): the built-in show, again in a second", crate::short(path), e.msg);
                    Formula::Retry { at: RetryAt::now() + d, psb }
                }
                None => {
                    fell_back(path, &e.msg);
                    Formula::Wordmark
                }
            },
        }
    }
}

/// A `boot_formula` value checked before it is saved: `""` (the wordmark)
/// or a Formula that builds as the start-up show (compiles, fits the show:
/// `boot::FORMULA_MAX_EST_US`); one on the card read from it (so the card
/// must be ready). The error is the sentence why.
pub fn check_formula(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Ok(());
    }
    // (Held only while it is on the panel: a setting would outlive it.)
    if opendmd_core::synth::is_sent(path) {
        return Err("save the Formula first".into());
    }
    #[cfg(feature = "psi")]
    {
        let r = match opendmd_core::synth::formula_file(path) {
            Some((file, opts)) => crate::formulas::read_card(file).and_then(|b| boot::FormulaShow::from_bytes(&b, opts)),
            None => boot::FormulaShow::open(path),
        };
        r.map(drop).map_err(|e| e.msg)
    }
    #[cfg(not(feature = "psi"))]
    Err(crate::state::NO_FORMULAS.to_string())
}

/// The wordmark plays in `path`'s place, for `why`: logged and said.
fn fell_back(path: &str, why: &str) {
    log::warn!("boot show: formula {} does not build ({why}): the built-in show", crate::short(path));
    BOOT_STATE.store(FELL_BACK, Relaxed);
    if let Ok(mut e) = BOOT_ERROR.lock() {
        why.clone_into(&mut e);
    }
}

// --- The start-up show and the status card --------------------------------------

pub struct Screen {
    len: ShowLength,
    /// The first boot of a new version (its minimum is longer, and its
    /// status line says so).
    updated: bool,
    started: Instant,
    scene: Scene,
    /// The boot task found no card (for now).
    no_card: bool,
    /// Handed over, and nothing plays (no playlist on the card).
    over: bool,
    /// When the card and its playlist were ready, ms into the show.
    ready_at: Option<u32>,
    /// The show's beat last logged.
    phase: &'static str,
    drawn: bool,
    /// The card's first line while the card is there but nothing plays.
    pub playlist_note: &'static str,
    text: Text,
    text_at: Option<Instant>,
    /// The card's text last logged.
    logged: Option<Text>,
    /// The show's Formula (`boot_formula`), and the path it names.
    formula: Formula,
    path: String,
}

impl Screen {
    /// `formula`: [`Formula::new`] for `path`, made before the boot task
    /// started.
    pub fn new(len: ShowLength, updated: bool, formula: Formula, path: &str) -> Screen {
        let len = if updated { len.for_update() } else { len };
        let min = if updated {
            len.min_ms_updated()
        } else {
            len.min_ms()
        };
        log::info!(
            "boot show: {}{} ({min} ms at least; the status card after {CAP_MS} ms if the card is not ready)",
            len.name(),
            if updated {
                ", says 'Updated to' this boot"
            } else {
                ""
            },
        );
        Screen {
            len,
            updated,
            started: Instant::now(),
            scene: Scene::Show,
            no_card: false,
            over: false,
            ready_at: None,
            phase: "",
            drawn: false,
            playlist_note: NO_PLAYLIST,
            text: Text::NONE,
            text_at: None,
            logged: None,
            formula,
            path: path.to_string(),
        }
    }

    /// The boot task read the card Formula `path` (the card's `boot_formula`,
    /// which wins over NVS's as every card setting does): played from here
    /// if the show still runs, else the wordmark goes on (and why is said).
    #[cfg(feature = "psi")]
    pub fn card_formula(&mut self, path: &str, psb: Result<Vec<u8>, opendmd_core::psi::Error>) {
        if self.scene != Scene::Show || self.len == ShowLength::Off {
            return;
        }
        path.clone_into(&mut self.path);
        let opts = opendmd_core::synth::formula_file(path).map_or("", |(_, o)| o).to_string();
        self.formula = match psb {
            Ok(b) => {
                let r = boot::FormulaShow::from_bytes(&b, &opts);
                let keep = r.as_ref().err().and_then(opendmd_core::fx_psi::look_retry).map(|_| (b, opts));
                log::info!("boot show: the card's formula read at {} ms", self.t());
                Formula::built(path, r, keep)
            }
            Err(e) => {
                fell_back(path, &e.msg);
                Formula::Wordmark
            }
        };
    }

    /// The Formula is done with (the show ended): its memory goes back.
    fn let_go(&mut self) {
        match self.formula {
            Formula::Waiting => fell_back(&self.path, "the card was not ready in time"),
            #[cfg(feature = "psi")]
            Formula::Retry { .. } => fell_back(&self.path, opendmd_core::psi::NO_MEMORY),
            _ => {}
        }
        self.formula = Formula::Wordmark;
        FORMULA_SHOWN.store(false, Relaxed);
    }

    /// Before a show frame: a card Formula still waited for past
    /// [`CARD_WAIT_MS`] (or with no card) gives way to the wordmark; one
    /// short of memory is built again when due.
    fn step_formula(&mut self, t: u32) {
        match &mut self.formula {
            Formula::Waiting if t >= CARD_WAIT_MS || self.no_card => {
                let why = if self.no_card { "no card" } else { "the card was not ready in time" };
                fell_back(&self.path, why);
                self.formula = Formula::Wordmark;
            }
            #[cfg(feature = "psi")]
            Formula::Retry { at, psb } if RetryAt::now() >= *at => {
                let psb = psb.take();
                let r = match &psb {
                    Some((b, o)) => boot::FormulaShow::from_bytes(b, o),
                    None => boot::FormulaShow::open(&self.path),
                };
                let path = std::mem::take(&mut self.path);
                self.formula = Formula::built(&path, r, psb);
                self.path = path;
            }
            _ => {}
        }
    }

    fn t(&self) -> u32 {
        ms(self.started.elapsed())
    }

    /// The show's minimum this boot.
    fn min(&self) -> u32 {
        if self.updated {
            self.len.min_ms_updated()
        } else {
            self.len.min_ms()
        }
    }

    /// The boot task has no card (yet).
    pub fn no_card(&mut self) {
        self.no_card = true;
        log::info!("boot show: no card at {} ms", self.t());
        self.text_at = None;
    }

    /// The card and its playlist are ready (the handover may wait for the
    /// show's minimum).
    pub fn ready(&mut self) {
        let t = self.t();
        self.ready_at = Some(t);
        log::info!("boot show: card and playlist ready at {t} ms");
    }

    /// Whether a handover may happen now: the show has run its minimum.
    pub fn may_end(&self) -> bool {
        match self.scene {
            Scene::Show => self.len == ShowLength::Off || self.t() >= self.min(),
            _ => true,
        }
    }

    /// The handover (or a later build) happened: `playing`, or nothing to
    /// play (the card says why).
    pub fn handed_over(&mut self, playing: bool) {
        if playing {
            self.playing();
        } else {
            self.over = true;
            self.text_at = None;
        }
    }

    /// Something plays (called every frame the engine draws).
    pub fn playing(&mut self) {
        if self.scene == Scene::Playing {
            return;
        }
        let t = self.t();
        let from = match self.scene {
            Scene::Show if self.len == ShowLength::Off => "dark panel",
            Scene::Show => "show",
            _ => "status card",
        };
        // The log's timestamps jump by ~1 s at the first line logged from
        // CPU0 (IDF adds the early clock once), so the uptime is given too.
        // SAFETY: an argument-free read of the system timer.
        let up = unsafe { esp_idf_sys::esp_timer_get_time() } / 1000;
        match self.ready_at.take() {
            Some(r) => log::info!(
                "boot show: {from} -> playlist at {t} ms, uptime {up} ms (card ready at {r} ms)"
            ),
            None => log::info!("boot show: {from} -> playlist at {t} ms, uptime {up} ms"),
        }
        self.scene = Scene::Playing;
        self.over = false;
        self.logged = None;
        self.let_go();
    }

    /// Why the card is there but nothing plays, from the build's error.
    pub fn playlist_problem(&mut self, error: &str) {
        self.playlist_note = if error.contains("matched no entries") {
            EMPTY_PLAYLIST
        } else {
            NO_PLAYLIST
        };
        self.text_at = None;
    }

    /// The last [`Screen::frame`] put something on the panel.
    pub fn drawn(&self) -> bool {
        self.drawn
    }

    fn show_card(&mut self, from: Option<(ShowLength, u32)>, t: u32) {
        log::info!(
            "boot show: status card at {t} ms{}",
            if from.is_some() {
                " (from the show)"
            } else {
                ""
            }
        );
        self.scene = Scene::Card {
            since: Instant::now(),
            from,
        };
        self.logged = None;
        self.text_at = None;
        self.let_go();
    }

    /// Draw the frame due now and blit it; returns how long to sleep before
    /// the next. `problem`: see [`Text::read`].
    pub fn frame(
        &mut self,
        panel: &mut Panel,
        canvas: &mut Canvas,
        status: &Mutex<Status>,
        problem: Option<&'static str>,
    ) -> Duration {
        let begun = Instant::now();
        let period = boot::frame_delay();
        let t = self.t();
        match self.scene {
            Scene::Playing => self.show_card(None, t),
            Scene::Show if self.len == ShowLength::Off => {
                if self.no_card || self.over {
                    self.show_card(None, t);
                }
            }
            Scene::Show => {
                if t >= CAP_MS || (self.no_card && t >= self.min()) || self.over {
                    self.show_card(Some((self.len, t)), t);
                }
            }
            Scene::Card { .. } => {}
        }
        if self.scene == Scene::Show && self.len != ShowLength::Off {
            self.step_formula(t);
        }
        if self.text_at.is_none_or(|at| at.elapsed() >= TEXT_EVERY) {
            self.text = Text::read(status, problem, self.ready_at.is_some() && !self.over);
            self.text_at = Some(begun);
        }
        let info = Info {
            updated: self.updated,
            ..self.text.info()
        };
        match self.scene {
            Scene::Show if self.len == ShowLength::Off => {
                // Dark: only waiting for the boot task, so look often.
                self.drawn = false;
                return Duration::from_millis(2);
            }
            Scene::Show => {
                match &mut self.formula {
                    #[cfg(feature = "psi")]
                    Formula::Playing(f) => {
                        if self.phase != "formula" {
                            self.phase = "formula";
                            log::info!("boot show: formula at {t} ms");
                        }
                        if !f.frame(canvas, self.len, t, &info) {
                            // Its frame is on the panel already.
                            return period.saturating_sub(begun.elapsed()).max(Duration::from_millis(1));
                        }
                    }
                    Formula::Waiting => {
                        // Dark until the card is read.
                        if self.drawn {
                            return Duration::from_millis(2);
                        }
                        canvas.clear();
                    }
                    _ => {
                        let p = boot::phase(self.len, t);
                        if p != self.phase {
                            self.phase = p;
                            log::info!("boot show: {p} at {t} ms");
                        }
                        boot::show(canvas, self.len, t, &info);
                    }
                }
            }
            Scene::Card { since, from } => {
                if self.logged != Some(self.text) {
                    self.logged = Some(self.text);
                    let (a, b) = info.card_lines();
                    log::info!(
                        "boot show: status card says '{}{}' / '{}{}'",
                        a[0],
                        a[1],
                        b[0],
                        b[1]
                    );
                }
                boot::card(canvas, ms(since.elapsed()), &info, from);
            }
            Scene::Playing => {}
        }
        panel.blit(canvas);
        self.drawn = true;
        period
            .saturating_sub(begun.elapsed())
            .max(Duration::from_millis(1))
    }
}
