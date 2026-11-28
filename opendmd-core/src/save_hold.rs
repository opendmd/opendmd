//! When settings changed while a Formula plays reach the flash.
//!
//! An NVS write stops the flash cache on both cores for milliseconds (tens
//! when a page is erased), so a Formula drawn against the panel's refresh
//! misses a frame or a few. Settings changed while one plays are applied at
//! once and held in RAM; they are written as soon as nothing plays that a
//! write would disturb (a GIF, the panel dark), and otherwise once they
//! have waited [`HOLD_MAX`], just after a frame has gone to the panel (the
//! stall then falls in that frame's time on screen). A GIF or Formula
//! played by hand stays until something else plays, so a hold without the
//! bound could last for hours, and a power cut would lose the change.
//! The caller passes `now` (any epoch it keeps).

use std::time::Duration;

/// The longest settings wait while a Formula plays.
pub const HOLD_MAX: Duration = Duration::from_secs(60);

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

    /// Whether to write them now: at once when `quiet` (nothing plays that
    /// a write would disturb); else once they have waited [`HOLD_MAX`] and
    /// a frame has just gone to the panel (`after_frame`).
    pub fn due(&self, now: Duration, quiet: bool, after_frame: bool) -> bool {
        self.since
            .is_some_and(|t| quiet || (after_frame && now.saturating_sub(t) >= HOLD_MAX))
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

    #[test]
    fn nothing_waiting_is_never_due() {
        let h = SaveHold::default();
        assert!(!h.due(S(1000), true, true));
    }

    #[test]
    fn a_gif_or_a_dark_panel_writes_at_once() {
        let mut h = SaveHold::default();
        h.note(true, S(10));
        assert!(h.due(S(10), true, false));
    }

    #[test]
    fn a_formula_holds_them_a_minute_then_writes_after_a_frame() {
        let mut h = SaveHold::default();
        h.note(true, S(10));
        // Later changes don't push the write back.
        h.note(true, S(40));
        assert!(!h.due(S(69), false, true), "held under a minute");
        assert!(!h.due(S(70), false, false), "a minute, but not just after a frame");
        assert!(h.due(S(70), false, true), "a minute, just after a frame");
        h.written();
        assert!(!h.due(S(200), false, true));
        // Written elsewhere (a restart's flush, a GIF): the wait starts again.
        h.note(true, S(300));
        h.note(false, S(301));
        h.note(true, S(350));
        assert!(!h.due(S(400), false, true) && h.due(S(410), false, true));
    }

    /// A Formula kept on the panel for hours (played by hand): a change is
    /// in flash within a minute and a frame, however long it stays.
    #[test]
    fn a_kept_formula_never_holds_them_past_a_minute() {
        let mut h = SaveHold::default();
        let mut written = None;
        // The display loop: a frame every 20 ms, the change at 1 h.
        let mut t = Duration::from_secs(3600);
        let mut pending = true;
        while t < Duration::from_secs(3 * 3600) {
            h.note(pending, t);
            if h.due(t, false, true) {
                h.written();
                pending = false;
                written = Some(t);
                break;
            }
            t += Duration::from_millis(20);
        }
        let w = written.expect("written");
        assert!(w - Duration::from_secs(3600) <= HOLD_MAX + Duration::from_millis(20), "{w:?}");
        assert!(!pending);
    }
}
