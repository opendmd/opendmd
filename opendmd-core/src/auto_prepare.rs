//! When the player prepares GIFs by itself (firmware `cache_builder.rs`,
//! `main::builder_pace`): the pure part, here so it is tested on the host.
//!
//! The `auto_prepare` setting:
//!
//! - **`always`** (the default since 1.0.0-beta.11): full speed while
//!   nothing animates (night mode has the panel dark, or the player is
//!   paused), and a slow **drip** while any GIF plays — a prepared one or an
//!   original — except during a transition, or for a while after a late
//!   frame ([`DripGuard`]). A fresh card gets its copies without a night
//!   schedule.
//! - **`idle`**: full speed while nothing animates, never while playing.
//! - **`off`**: never.
//!
//! **Why drip now runs over originals too.** It used to need a *prepared*
//! file on screen: before the 512 B FatFs buffers (2026-10-24, ~30 KB back
//! everywhere) an original playing left ~62-64 KB free, under the drip
//! start threshold (`BUILDER_MIN_HEAP` + the task, 76.4 KB), while a copy
//! left 76-80 KB. With them an original leaves ~92-94 KB, more than a copy
//! did when the drip was designed and measured; the builder's own heap gate
//! (start threshold, abort floor) is unchanged and still decides each file.
//!
//! **Older settings.** Every save writes every field, so a device that ever
//! saved anything holds the old default `idle` without anyone having chosen
//! it. The setting moved to a new NVS key ([`AutoPrepare::from_nvs`]): the
//! old key's `off` and `always` (which only a person could have set) are
//! kept, its `idle` reads as the new default.

use std::time::Duration;

use crate::frametime::{When, HEADLINE_MS};

/// The `auto_prepare` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum AutoPrepare {
    /// Only while nothing animates (night mode dark, paused).
    Idle = 0,
    /// Also slowly while playing. The default.
    #[default]
    Always = 1,
    /// Never.
    Off = 2,
}

/// The 400 for a bad `auto_prepare`.
pub const INVALID: &str = "auto_prepare must be idle, always or off";

impl AutoPrepare {
    /// A value `PUT /api/config` accepts.
    pub fn parse(s: &str) -> Option<AutoPrepare> {
        match s {
            "idle" => Some(AutoPrepare::Idle),
            "always" => Some(AutoPrepare::Always),
            "off" => Some(AutoPrepare::Off),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AutoPrepare::Idle => "idle",
            AutoPrepare::Always => "always",
            AutoPrepare::Off => "off",
        }
    }

    /// Back from `self as u8` (an atomic's copy).
    pub fn from_u8(v: u8) -> AutoPrepare {
        match v {
            0 => AutoPrepare::Idle,
            2 => AutoPrepare::Off,
            _ => AutoPrepare::Always,
        }
    }

    /// The setting from NVS: `saved`, the current key, wins; else the old
    /// key's `legacy` value if a person chose it (`off`, `always`); else —
    /// nothing saved, the old default `idle`, or garbage — the default.
    pub fn from_nvs(saved: Option<&str>, legacy: Option<&str>) -> AutoPrepare {
        if let Some(mode) = saved.and_then(AutoPrepare::parse) {
            return mode;
        }
        match legacy.and_then(AutoPrepare::parse) {
            Some(AutoPrepare::Idle) | None => AutoPrepare::default(),
            Some(chosen) => chosen,
        }
    }
}

/// How fast the builder may work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// Nothing animates: as fast as the card allows.
    Full,
    /// A GIF plays: a pause before every 2 KB of card work.
    Drip,
    Stop(Hold),
}

/// Why the pace says stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// `auto_prepare` is `off`.
    Off,
    /// `idle`, and a GIF plays.
    Playing,
    /// A transition runs: its 24 KB and the encoder do not both fit.
    Transition,
    /// A late frame while dripping: held off for a while ([`DripGuard`]).
    LateFrame,
}

/// What the panel is doing, as the pace needs it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Panel {
    /// Night mode has the panel dark.
    pub dark: bool,
    /// The player is paused (or has nothing to animate).
    pub paused: bool,
    /// A transition runs.
    pub transitioning: bool,
}

/// The pace for `mode` while the panel does what `panel` says; `late_hold`
/// is [`DripGuard::held`]. The builder adds its own conditions (heap,
/// transfers in flight, the player opening a file).
pub fn pace(mode: AutoPrepare, panel: Panel, late_hold: bool) -> Pace {
    match mode {
        AutoPrepare::Off => Pace::Stop(Hold::Off),
        _ if panel.dark || panel.paused => Pace::Full,
        AutoPrepare::Idle => Pace::Stop(Hold::Playing),
        AutoPrepare::Always if panel.transitioning => Pace::Stop(Hold::Transition),
        AutoPrepare::Always if late_hold => Pace::Stop(Hold::LateFrame),
        AutoPrepare::Always => Pace::Drip,
    }
}

/// The first hold after a late frame while dripping.
pub const GUARD_FIRST: Duration = Duration::from_secs(60);
/// Each further late frame doubles the hold, up to this.
pub const GUARD_MAX: Duration = Duration::from_secs(32 * 60);
/// A late frame this long after the last one starts again at
/// [`GUARD_FIRST`].
pub const GUARD_FORGIVE: Duration = Duration::from_secs(60 * 60);

/// The drip's safeguard: a frame a person could see late
/// ([`counts`]) while the builder drips holds the drip off — a minute at
/// first, doubling with each further one up to [`GUARD_MAX`], back to a
/// minute after an hour without one. Full speed (nothing animates) is not
/// affected. Times are on any monotonic clock (the firmware's boot epoch).
#[derive(Clone, Debug, Default)]
pub struct DripGuard {
    trips: u32,
    last: Option<Duration>,
    until: Duration,
}

/// A late frame the guard reacts to: one a person could see
/// ([`HEADLINE_MS`] or more) in steady play. A GIF change's own misses (the
/// new file's open and first frame) are the switch's, and the builder has
/// already been stopped for the open by then.
pub fn counts(late_ms: u64, when: When) -> bool {
    when == When::Steady && late_ms >= HEADLINE_MS
}

impl DripGuard {
    pub fn new() -> DripGuard {
        DripGuard::default()
    }

    /// A late frame that [`counts`] at `now`, with the builder dripping:
    /// the hold it starts.
    pub fn late_frame(&mut self, now: Duration) -> Duration {
        if self.last.is_some_and(|t| now.saturating_sub(t) >= GUARD_FORGIVE) {
            self.trips = 0;
        }
        self.trips = self.trips.saturating_add(1);
        self.last = Some(now);
        let hold = GUARD_FIRST
            .checked_mul(1u32 << (self.trips - 1).min(16))
            .map_or(GUARD_MAX, |d| d.min(GUARD_MAX));
        self.until = now + hold;
        hold
    }

    /// The drip is held off at `now`.
    pub fn held(&self, now: Duration) -> bool {
        self.last.is_some() && now < self.until
    }

    /// Holds in the current run of late frames (back to 1 after
    /// [`GUARD_FORGIVE`] without one).
    pub fn trips(&self) -> u32 {
        self.trips
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    #[test]
    fn default_is_always() {
        assert_eq!(AutoPrepare::default(), AutoPrepare::Always);
    }

    #[test]
    fn parse_round_trips() {
        for m in [AutoPrepare::Idle, AutoPrepare::Always, AutoPrepare::Off] {
            assert_eq!(AutoPrepare::parse(m.as_str()), Some(m));
            assert_eq!(AutoPrepare::from_u8(m as u8), m);
        }
        assert_eq!(AutoPrepare::parse("sometimes"), None);
        assert_eq!(AutoPrepare::parse("Always"), None);
        assert!(INVALID.contains("idle, always or off"));
    }

    #[test]
    fn nvs_migration() {
        use AutoPrepare::*;
        // Nothing saved: the default.
        assert_eq!(AutoPrepare::from_nvs(None, None), Always);
        // The new key wins, whatever the old one says.
        assert_eq!(AutoPrepare::from_nvs(Some("idle"), Some("always")), Idle);
        assert_eq!(AutoPrepare::from_nvs(Some("off"), None), Off);
        assert_eq!(AutoPrepare::from_nvs(Some("always"), Some("off")), Always);
        // The old key: a choice is kept, the old default is not.
        assert_eq!(AutoPrepare::from_nvs(None, Some("off")), Off);
        assert_eq!(AutoPrepare::from_nvs(None, Some("always")), Always);
        assert_eq!(AutoPrepare::from_nvs(None, Some("idle")), Always);
        // Garbage in either: as if absent.
        assert_eq!(AutoPrepare::from_nvs(Some("x"), Some("off")), Off);
        assert_eq!(AutoPrepare::from_nvs(Some("x"), Some("x")), Always);
    }

    fn panel(dark: bool, paused: bool, transitioning: bool) -> Panel {
        Panel {
            dark,
            paused,
            transitioning,
        }
    }

    #[test]
    fn pace_table() {
        use AutoPrepare::*;
        let playing = panel(false, false, false);
        let moving = panel(false, false, true);
        // Off is off, whatever the panel does.
        for p in [playing, moving, panel(true, true, false), panel(false, true, false)] {
            assert_eq!(pace(Off, p, false), Pace::Stop(Hold::Off));
        }
        // Dark or paused: full speed in both other modes, a late hold or not.
        for m in [Idle, Always] {
            for late in [false, true] {
                assert_eq!(pace(m, panel(true, true, false), late), Pace::Full);
                assert_eq!(pace(m, panel(false, true, false), late), Pace::Full);
                assert_eq!(pace(m, panel(true, false, false), late), Pace::Full);
            }
        }
        // Playing: idle waits, always drips — over any GIF.
        assert_eq!(pace(Idle, playing, false), Pace::Stop(Hold::Playing));
        assert_eq!(pace(Always, playing, false), Pace::Drip);
        // Not through a transition, nor while a late frame holds it off.
        assert_eq!(pace(Always, moving, false), Pace::Stop(Hold::Transition));
        assert_eq!(pace(Always, playing, true), Pace::Stop(Hold::LateFrame));
    }

    #[test]
    fn only_visible_steady_misses_count() {
        assert!(counts(HEADLINE_MS, When::Steady));
        assert!(counts(400, When::Steady));
        assert!(!counts(HEADLINE_MS - 1, When::Steady));
        assert!(!counts(0, When::Steady));
        assert!(!counts(400, When::Switch));
    }

    #[test]
    fn guard_holds_and_doubles() {
        let mut g = DripGuard::new();
        assert!(!g.held(S(0)) && !g.held(S(10_000)));
        assert_eq!(g.late_frame(S(100)), S(60));
        assert!(g.held(S(100)) && g.held(S(159)) && !g.held(S(160)));
        assert_eq!(g.trips(), 1);
        // Each further one doubles, to the cap.
        assert_eq!(g.late_frame(S(200)), S(120));
        assert_eq!(g.late_frame(S(400)), S(240));
        assert_eq!(g.late_frame(S(700)), S(480));
        assert_eq!(g.late_frame(S(1200)), S(960));
        assert_eq!(g.late_frame(S(2200)), GUARD_MAX);
        assert_eq!(g.late_frame(S(2300)), GUARD_MAX);
        assert!(g.held(S(2300) + GUARD_MAX - S(1)) && !g.held(S(2300) + GUARD_MAX));
        // Many more never overflow.
        for i in 0..100 {
            g.late_frame(S(2400 + i));
        }
        assert!(g.held(S(2500)));
    }

    #[test]
    fn guard_forgives_after_a_quiet_hour() {
        let mut g = DripGuard::new();
        g.late_frame(S(0));
        g.late_frame(S(100));
        assert_eq!(g.trips(), 2);
        // Under an hour since the last: it keeps doubling.
        assert_eq!(g.late_frame(S(100) + GUARD_FORGIVE - S(1)), S(240));
        // An hour or more since the last: a minute again.
        let t = S(100) + GUARD_FORGIVE - S(1) + GUARD_FORGIVE;
        assert_eq!(g.late_frame(t), GUARD_FIRST);
        assert_eq!(g.trips(), 1);
    }
}
