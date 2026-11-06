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
//! copy, `opendmd_core::updater_picture`, a dot stepping under it). The
//! guard's drop (a refused, cut-short or failed upload) takes it down and
//! playback resumes. One relaxed atomic load per loop iteration otherwise.
//!
//! **The log.** One line per beat of the show (`boot show: warp at 0 ms`,
//! `name`, `jump`, `status line`, `name settled`, `hold`), the card's
//! readiness, the status card's text whenever it changes, and the
//! handover — so a capture shows what the panel showed.

use opendmd_core::boot::{self, Info, ShowLength, UpdateStage, CAP_MS};
use opendmd_core::canvas::Canvas;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
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
    let stage = if stage == INSTALLING {
        UpdateStage::Installing
    } else {
        UpdateStage::Receiving
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

    /// Staged and the updater chosen: the restart is next.
    pub fn installing(&self) {
        STAGE.store(INSTALLING, Relaxed);
        crate::wake::kick();
    }
}

impl Drop for Updating {
    fn drop(&mut self) {
        STAGE.store(NO_UPDATE, Relaxed);
        crate::wake::kick();
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
}

impl Screen {
    pub fn new(len: ShowLength, updated: bool) -> Screen {
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
                let p = boot::phase(self.len, t);
                if p != self.phase {
                    self.phase = p;
                    log::info!("boot show: {p} at {t} ms");
                }
                boot::show(canvas, self.len, t, &info);
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
