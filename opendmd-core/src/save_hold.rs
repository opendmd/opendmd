//! When settings changed while a Formula plays reach the flash.
//!
//! An NVS write stops the flash cache on both cores for milliseconds (tens
//! when a page is erased), so a Formula drawn against the panel's refresh
//! misses a frame or a few. Settings changed while one plays are applied at
//! once and held in RAM; they are written as soon as nothing draws that a
//! write would disturb (a GIF, the panel dark, paused, no playlist, no
//! frame for a second), and otherwise once they have waited [`HOLD_MAX`],
//! just after a frame has gone to the panel (the stall then falls in that
//! frame's time on screen), or [`HOLD_MARGIN`] after that whatever the
//! display does. A GIF or Formula played by hand stays until something
//! else plays, so a hold without the bound could last for hours, and a
//! power cut would lose the change.
//! The caller passes `now` (any epoch it keeps).

use std::time::Duration;

/// The longest settings wait while a Formula plays.
pub const HOLD_MAX: Duration = Duration::from_secs(60);
/// Past [`HOLD_MAX`] by this, they are written whatever the display does
/// (no frame came to write them after).
pub const HOLD_MARGIN: Duration = Duration::from_secs(5);
/// A frame within this long is a display drawing now; none, and nothing
/// draws that a write could disturb (a card pulled, a panel test, an update
/// screen, a stalled engine).
pub const FRAME_RECENT: Duration = Duration::from_secs(1);

/// What the display does, as the loop sees it at `now`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Display {
    /// What plays is a Formula.
    pub formula: bool,
    /// The panel is dark (night mode off).
    pub dark: bool,
    /// Playback is paused.
    pub paused: bool,
    /// There is an engine (a playlist running).
    pub engine: bool,
    /// When the last frame went to the panel, if one has.
    pub last_frame: Option<Duration>,
}

impl Display {
    /// A write now could cost a Formula's frames: one draws, unpaused, on a
    /// lit panel, a frame gone up within [`FRAME_RECENT`].
    pub fn disturbable(&self, now: Duration) -> bool {
        self.formula
            && !self.dark
            && !self.paused
            && self.engine
            && self.last_frame.is_some_and(|t| now.saturating_sub(t) < FRAME_RECENT)
    }
}

/// Settings not yet written: since when.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaveHold {
    since: Option<Duration>,
}

impl SaveHold {
    /// Settings are waiting at `now` (`pending`), or none are (they were
    /// written, here or elsewhere): note it. The wait counts from the
    /// first `now` that saw them.
    pub fn note(&mut self, pending: bool, now: Duration) {
        if !pending {
            self.since = None;
        } else if self.since.is_none() {
            self.since = Some(now);
        }
    }

    /// Whether to write them now: at once when nothing a write would
    /// disturb draws ([`Display::disturbable`]); once they have waited
    /// [`HOLD_MAX`], just after a frame went to the panel (`after_frame`);
    /// and past [`HOLD_MAX`] + [`HOLD_MARGIN`] whatever the display does.
    pub fn due(&self, now: Duration, display: Display, after_frame: bool) -> bool {
        self.since.is_some_and(|t| {
            let waited = now.saturating_sub(t);
            !display.disturbable(now) || (after_frame && waited >= HOLD_MAX) || waited >= HOLD_MAX + HOLD_MARGIN
        })
    }

    /// They were written.
    pub fn written(&mut self) {
        self.since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    /// A Formula drawing now, its last frame at `at`.
    fn drawing(at: Duration) -> Display {
        Display { formula: true, engine: true, last_frame: Some(at), ..Display::default() }
    }

    #[test]
    fn nothing_waiting_is_never_due() {
        let h = SaveHold::default();
        assert!(!h.due(S(1000), Display::default(), true));
    }

    #[test]
    fn a_gif_or_a_dark_panel_writes_at_once() {
        let mut h = SaveHold::default();
        h.note(true, S(10));
        assert!(h.due(S(10), Display { formula: false, ..drawing(S(10)) }, false), "a GIF");
        assert!(h.due(S(10), Display { dark: true, ..drawing(S(10)) }, false), "the panel dark");
    }

    /// The cases where `formula_on` stays true but nothing draws: paused,
    /// no engine (a card pulled), no recent frame (a panel test, an
    /// update screen): written at once, not held.
    #[test]
    fn paused_no_engine_or_no_frame_writes_at_once() {
        let mut h = SaveHold::default();
        h.note(true, S(10));
        assert!(h.due(S(10), Display { paused: true, ..drawing(S(10)) }, false), "paused");
        assert!(h.due(S(10), Display { engine: false, ..drawing(S(10)) }, false), "no engine");
        assert!(h.due(S(12), drawing(S(10)), false), "no frame for 2 s");
        assert!(h.due(S(10), Display { last_frame: None, ..drawing(S(10)) }, false), "no frame yet");
        assert!(!h.due(S(10), drawing(S(10)), false), "a Formula drawing: held");
    }

    #[test]
    fn a_formula_holds_them_a_minute_then_writes_after_a_frame() {
        let mut h = SaveHold::default();
        h.note(true, S(10));
        // Later changes don't push the write back.
        h.note(true, S(40));
        assert!(!h.due(S(69), drawing(S(69)), true), "held under a minute");
        assert!(!h.due(S(70), drawing(S(70)), false), "a minute, but not just after a frame");
        assert!(h.due(S(70), drawing(S(70)), true), "a minute, just after a frame");
        assert!(h.due(S(75), drawing(S(75)), false), "the minute and the margin: whatever the display does");
        h.written();
        assert!(!h.due(S(200), drawing(S(200)), true));
        // Written elsewhere (a restart's flush, a GIF): the wait starts again.
        h.note(true, S(300));
        h.note(false, S(301));
        h.note(true, S(350));
        assert!(!h.due(S(400), drawing(S(400)), true) && h.due(S(410), drawing(S(410)), true));
    }

    /// A Formula kept on the panel for hours (played by hand): a change is
    /// in flash within a minute and a frame, however long it stays.
    #[test]
    fn a_kept_formula_never_holds_them_past_a_minute() {
        let mut h = SaveHold::default();
        let mut written = None;
        // The display loop: a frame every 20 ms, the change at 1 h.
        let mut t = Duration::from_secs(3600);
        while t < Duration::from_secs(3 * 3600) {
            h.note(true, t);
            if h.due(t, drawing(t), true) {
                h.written();
                written = Some(t);
                break;
            }
            t += Duration::from_millis(20);
        }
        let w = written.expect("written");
        assert!(w - Duration::from_secs(3600) <= HOLD_MAX + Duration::from_millis(20), "{w:?}");
    }
}
