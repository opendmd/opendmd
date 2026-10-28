//! When the player prepares GIFs by itself (firmware `cache_builder.rs`,
//! `main::builder_pace`): the pure part, here so it is tested on the host.
//!
//! The `auto_prepare` setting:
//!
//! - **`idle`** (the default): full speed while nothing animates (night
//!   mode has the panel dark, or the player is paused), never while
//!   playing. A fresh card plays fine as it is (originals: 0 steady misses)
//!   and is ready after ~3.7 h of the panel dark or paused (~2,700 files/h).
//! - **`always`** (opt-in): `idle`, plus a slow **drip** while a *prepared*
//!   GIF plays — not during a transition or the new GIF's first
//!   [`SWITCH_WINDOW`], and not for a while after a late frame the drip may
//!   have caused, steady or at a switch ([`DripGuard`]).
//! - **`off`**: never.
//!
//! **Why `idle` stays the default** (2026-10-28, spare, `filelist.txt` with
//! 2,714 of 10,162 prepared, unpaused, no night schedule, 20 min A/B
//! against `idle` each):
//!
//! - *builder-a, drip over originals too, default `always`.* An original
//!   on that card leaves 56-79 KB free (median ~67 KB; the ~100 KB of the
//!   512 B FatFs buffers' measurement was a 47-entry playlist of copies),
//!   against the drip's 76.4 KB start threshold; when it did start, the
//!   encoder took the heap to 23 KB free (floor 19.9 KB; `idle` 45.9 KB),
//!   it made 135 files in the first 10 min and 9 in the next (`low
//!   memory`), and it cost +24 switch misses and +1 steady (`idle`: +0).
//!   The encoder (~48-53 KB on the device) plus its 20 KB margin plus the
//!   task does not fit beside an original's decoder; a threshold that kept
//!   the playback floor would never let it start.
//! - *builder-b, drip over prepared files only, this guard.* Safe for the
//!   heap (min 59.2 KB; `idle` 60.1 KB), 0 steady misses, but +4 switch
//!   misses in 20 min (`idle` +0) for ~7 files (~21 an hour, `avg_drip_s`
//!   ~7.5; full speed ~2,700): reasons alternated `gif on screen` and `late
//!   frame`. Not worth being the default.
//!
//! The setting is the NVS key `auto_prepare`, as before: an explicit
//! choice carries over, the old default stays `idle`.

use std::time::Duration;

use crate::frametime::HEADLINE_MS;
pub use crate::frametime::SWITCH_WINDOW;

/// The `auto_prepare` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum AutoPrepare {
    /// Only while nothing animates (night mode dark, paused). The default.
    #[default]
    Idle = 0,
    /// Also slowly while a prepared GIF plays (opt-in).
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
            1 => AutoPrepare::Always,
            2 => AutoPrepare::Off,
            _ => AutoPrepare::Idle,
        }
    }
}

/// How fast the builder may work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// Nothing animates: as fast as the card allows.
    Full,
    /// A prepared GIF plays: a pause before every 2 KB of card work.
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
    /// A GIF change: a transition (its 24 KB and the encoder do not both
    /// fit), or the new GIF's first [`SWITCH_WINDOW`].
    Transition,
    /// An original plays: its decoder and the encoder do not both fit.
    GifOnScreen,
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
    /// The GIF on screen went up less than [`SWITCH_WINDOW`] ago (or
    /// nothing has yet): its first frames are the switch's.
    pub settling: bool,
    /// The GIF on screen is a prepared copy.
    pub prepared: bool,
}

/// The pace for `mode` while the panel does what `panel` says; `late_hold`
/// is [`DripGuard::held`]. The builder adds its own conditions (heap,
/// transfers in flight, the player opening a file).
pub fn pace(mode: AutoPrepare, panel: Panel, late_hold: bool) -> Pace {
    match mode {
        AutoPrepare::Off => Pace::Stop(Hold::Off),
        _ if panel.dark || panel.paused => Pace::Full,
        AutoPrepare::Idle => Pace::Stop(Hold::Playing),
        AutoPrepare::Always if panel.transitioning || panel.settling => {
            Pace::Stop(Hold::Transition)
        }
        AutoPrepare::Always if !panel.prepared => Pace::Stop(Hold::GifOnScreen),
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

/// The drip's safeguard: a frame a person could see late ([`counts`])
/// while the builder drips, or at a GIF change that stopped it, holds the
/// drip off — a minute at first, doubling with each further one up to
/// [`GUARD_MAX`], back to a minute after an hour without one. Full speed
/// (nothing animates) is not affected. Times are on any monotonic clock
/// (the firmware's boot epoch). A drip that misses at every switch costs at
/// most ~7 late frames in its first hour and 2 an hour after
/// (`guard_bounds_a_drip_that_misses_at_every_switch`).
#[derive(Clone, Debug, Default)]
pub struct DripGuard {
    trips: u32,
    last: Option<Duration>,
    until: Duration,
}

/// A late frame the guard reacts to: one a person could see
/// ([`HEADLINE_MS`] or more), in steady play or at a GIF change — builder-a
/// showed the drip's cost is mostly at the switches (the player's open
/// waits for the builder to let go, and the next file is not opened ahead
/// while its task is up). Whether the drip is to blame is the caller's
/// part (firmware: the task was dripping, or let go of a drip, moments
/// before).
pub fn counts(late_ms: u64) -> bool {
    late_ms >= HEADLINE_MS
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
    fn default_is_idle() {
        assert_eq!(AutoPrepare::default(), AutoPrepare::Idle);
        // An unknown byte (the atomic before the first publish) too.
        assert_eq!(AutoPrepare::from_u8(9), AutoPrepare::Idle);
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

    fn panel(dark: bool, paused: bool, transitioning: bool) -> Panel {
        Panel {
            dark,
            paused,
            transitioning,
            settling: false,
            prepared: true,
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
        // Playing a prepared GIF: idle waits, always drips.
        assert_eq!(pace(Idle, playing, false), Pace::Stop(Hold::Playing));
        assert_eq!(pace(Always, playing, false), Pace::Drip);
        // Never over an original (builder-a: its decoder and the encoder
        // do not both fit), dark or paused excepted.
        let original = Panel { prepared: false, ..playing };
        assert_eq!(pace(Always, original, false), Pace::Stop(Hold::GifOnScreen));
        assert_eq!(pace(Idle, original, false), Pace::Stop(Hold::Playing));
        assert_eq!(pace(Always, Panel { paused: true, ..original }, false), Pace::Full);
        // Not through a transition or the new GIF's first frames, nor while
        // a late frame holds it off.
        assert_eq!(pace(Always, moving, false), Pace::Stop(Hold::Transition));
        let settling = Panel { settling: true, ..playing };
        assert_eq!(pace(Always, settling, false), Pace::Stop(Hold::Transition));
        assert_eq!(pace(Always, playing, true), Pace::Stop(Hold::LateFrame));
    }

    #[test]
    fn only_visible_misses_count() {
        assert!(counts(HEADLINE_MS));
        assert!(counts(400));
        assert!(!counts(HEADLINE_MS - 1));
        assert!(!counts(0));
    }

    /// Late frames a drip that causes one at every GIF change can cost,
    /// over `minutes`: builder-a's rate, +24 switch misses in 20 min of
    /// dripping (one every 50 s), each counted while the drip is not held.
    fn misses_with_guard(minutes: u64, every: Duration) -> u32 {
        let mut g = DripGuard::new();
        let (mut t, mut n) = (Duration::ZERO, 0);
        while t < S(minutes * 60) {
            if !g.held(t) {
                g.late_frame(t);
                n += 1;
            }
            t += every;
        }
        n
    }

    #[test]
    fn guard_bounds_a_drip_that_misses_at_every_switch() {
        // Unguarded that is 72 an hour; guarded, 7 in the first hour (at
        // 0, 1, 3, 7, 15, 31 and ~63 min less the steps) and then one per
        // 32 min hold: the hour never forgives while it keeps missing.
        let first = misses_with_guard(60, S(50));
        assert!(first <= 7, "{first} in the first hour");
        let day = misses_with_guard(24 * 60, S(50));
        assert!(day <= first + 2 * 23 + 1, "{day} in a day");
        // Even one at every frame-ish second: the same bound.
        assert!(misses_with_guard(60, S(1)) <= 7);
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
