//! Schedules and night mode: given the rules and a local time, what should
//! the panel be doing — which playlist, how bright, or dark?
//!
//! The firmware stores a schedule as JSON in NVS and parses it into these
//! types (the core has no `serde`); this module is the evaluator, so it can
//! be tested on the host.
//!
//! - **Rules** are tried in order and the **first** whose days and window
//!   contain the time wins. A rule may name a playlist (`set`, the same
//!   values as the config's `set`: `""` = all files) and/or a brightness;
//!   whatever it leaves out stays as configured.
//! - **Night mode** is one window, every day, and beats every rule: `off`
//!   (panel dark, playback paused) or `dim` (the level given; playback
//!   carries on with whatever playlist applies).
//! - A window `from..to` is half-open, in minutes since local midnight. If
//!   `to <= from` it runs past midnight, and the *start* day is the one its
//!   `days` are checked against (a Friday 22:00–02:00 rule is on at 01:00
//!   on Saturday). `from == to` is the whole day.

/// Minutes in a day.
pub const DAY: u16 = 24 * 60;

/// A local wall-clock time, as the evaluator needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// ISO weekday: 1 = Monday .. 7 = Sunday.
    pub weekday: u8,
    /// Minutes since local midnight, `0..DAY`.
    pub minute: u16,
}

impl LocalTime {
    /// From `struct tm` style fields: `tm_wday` (0 = Sunday), hour, minute.
    pub fn from_tm(wday_sunday0: i32, hour: i32, min: i32) -> Self {
        let weekday = if wday_sunday0 <= 0 {
            7
        } else {
            wday_sunday0.min(6) as u8
        };
        LocalTime {
            weekday,
            minute: (hour.clamp(0, 23) * 60 + min.clamp(0, 59)) as u16,
        }
    }
}

/// A `from..to` window in minutes since midnight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub from: u16,
    pub to: u16,
}

impl Window {
    /// Is `t` inside the window, where `days` is the bitmask of start days
    /// allowed (bit `d - 1` for ISO day `d`)?
    fn contains(&self, days: u8, t: LocalTime) -> bool {
        let on = |day: u8| days & (1 << (day - 1)) != 0;
        let yesterday = if t.weekday == 1 { 7 } else { t.weekday - 1 };
        if self.from == self.to {
            on(t.weekday)
        } else if self.from < self.to {
            on(t.weekday) && (self.from..self.to).contains(&t.minute)
        } else {
            (on(t.weekday) && t.minute >= self.from) || (on(yesterday) && t.minute < self.to)
        }
    }
}

/// One rule: on these days, in this window, play this / at this brightness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Bit `d - 1` for ISO day `d` (Monday = bit 0).
    pub days: u8,
    pub window: Window,
    /// The playlist, as a config `set` value; `None` keeps the configured one.
    pub set: Option<String>,
    /// `None` keeps the configured brightness.
    pub brightness: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NightMode {
    /// Panel dark (blanked, brightness 0), playback paused.
    Off,
    /// Panel at the night brightness, playback carries on.
    Dim,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Night {
    pub window: Window,
    pub mode: NightMode,
    /// The level for [`NightMode::Dim`].
    pub brightness: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schedule {
    pub rules: Vec<Rule>,
    pub night: Option<Night>,
}

/// What the panel is doing, as status reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Normal,
    Dim,
    Off,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Normal => "normal",
            Mode::Dim => "dim",
            Mode::Off => "off",
        }
    }
}

/// The evaluator's answer for one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decision {
    pub mode: Mode,
    /// When night mode ends (minutes since midnight), while it is on.
    pub mode_until: Option<u16>,
    /// The rule in force, by position.
    pub rule: Option<usize>,
    /// When that rule's window ends.
    pub rule_until: Option<u16>,
    /// The playlist the rule selects (`None`: the configured one).
    pub set: Option<String>,
    /// The brightness to use (`None`: the configured one); the night level
    /// in `Dim`, the rule's otherwise.
    pub brightness: Option<u8>,
}

impl Schedule {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty() && self.night.is_none()
    }

    /// What should be happening at `t`.
    pub fn evaluate(&self, t: LocalTime) -> Decision {
        let mut d = Decision::default();
        if let Some((i, rule)) = self
            .rules
            .iter()
            .enumerate()
            .find(|(_, r)| r.window.contains(r.days, t))
        {
            d.rule = Some(i);
            d.rule_until = Some(rule.window.to);
            d.set = rule.set.clone();
            d.brightness = rule.brightness;
        }
        if let Some(night) = &self.night {
            if night.window.contains(0x7f, t) {
                d.mode_until = Some(night.window.to);
                match night.mode {
                    NightMode::Off => {
                        d.mode = Mode::Off;
                        d.brightness = Some(0);
                    }
                    NightMode::Dim => {
                        d.mode = Mode::Dim;
                        d.brightness = Some(night.brightness);
                    }
                }
            }
        }
        d
    }
}

/// `"HH:MM"` (24-hour, `00:00`..`23:59`; `24:00` is accepted as midnight)
/// to minutes since midnight.
pub fn parse_hhmm(s: &str) -> Option<u16> {
    let (h, m) = s.trim().split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    match (h, m) {
        (24, 0) => Some(0),
        (0..=23, 0..=59) => Some(h * 60 + m),
        _ => None,
    }
}

/// Minutes since midnight to `"HH:MM"`.
pub fn format_hhmm(minute: u16) -> String {
    let m = minute % DAY;
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// ISO weekday numbers (1..=7) to a bitmask; `None` if any is out of range.
/// An empty list is every day.
pub fn days_mask(days: &[u8]) -> Option<u8> {
    if days.is_empty() {
        return Some(0x7f);
    }
    days.iter().try_fold(0u8, |mask, d| match d {
        1..=7 => Some(mask | 1 << (d - 1)),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: u8 = 1;
    const FRI: u8 = 5;
    const SAT: u8 = 6;
    const SUN: u8 = 7;

    fn at(weekday: u8, hhmm: &str) -> LocalTime {
        LocalTime {
            weekday,
            minute: parse_hhmm(hhmm).unwrap(),
        }
    }

    fn window(from: &str, to: &str) -> Window {
        Window {
            from: parse_hhmm(from).unwrap(),
            to: parse_hhmm(to).unwrap(),
        }
    }

    fn rule(days: &[u8], from: &str, to: &str, set: Option<&str>, brightness: Option<u8>) -> Rule {
        Rule {
            days: days_mask(days).unwrap(),
            window: window(from, to),
            set: set.map(str::to_string),
            brightness,
        }
    }

    fn schedule() -> Schedule {
        Schedule {
            rules: vec![
                rule(
                    &[1, 2, 3, 4, 5],
                    "18:00",
                    "23:00",
                    Some("consoles"),
                    Some(96),
                ),
                rule(&[5], "22:00", "02:00", Some("party"), None),
                rule(&[6, 7], "10:00", "12:00", None, Some(40)),
            ],
            night: Some(Night {
                window: window("23:30", "07:00"),
                mode: NightMode::Off,
                brightness: 8,
            }),
        }
    }

    #[test]
    fn nothing_matches_outside_every_window() {
        let d = schedule().evaluate(at(MON, "12:00"));
        assert_eq!(d, Decision::default());
        assert_eq!(
            Schedule::default().evaluate(at(MON, "12:00")),
            Decision::default()
        );
    }

    #[test]
    fn a_rule_selects_its_playlist_and_brightness_until_its_end() {
        let d = schedule().evaluate(at(MON, "18:00"));
        assert_eq!(d.mode, Mode::Normal);
        assert_eq!(d.rule, Some(0));
        assert_eq!(d.set.as_deref(), Some("consoles"));
        assert_eq!(d.brightness, Some(96));
        assert_eq!(d.rule_until, parse_hhmm("23:00"));
        // Half-open: 23:00 is outside.
        assert_eq!(schedule().evaluate(at(MON, "23:00")).rule, None);
        // Not on a Saturday.
        assert_eq!(schedule().evaluate(at(SAT, "18:30")).rule, None);
    }

    #[test]
    fn first_matching_rule_wins() {
        // Friday 22:30 matches both the weekday rule and the Friday one.
        let d = schedule().evaluate(at(FRI, "22:30"));
        assert_eq!(d.rule, Some(0));
        assert_eq!(d.set.as_deref(), Some("consoles"));
    }

    #[test]
    fn an_overnight_rule_belongs_to_the_day_it_starts() {
        // Friday's 22:00-02:00 is on at 01:00 on Saturday...
        let d = schedule().evaluate(at(SAT, "01:00"));
        assert_eq!(d.rule, Some(1));
        assert_eq!(d.set.as_deref(), Some("party"));
        assert_eq!(d.brightness, Some(0), "and night mode is on too");
        assert_eq!(d.mode, Mode::Off);
        // ...but not at 01:00 on Friday (Thursday is not in its days).
        let s = Schedule {
            rules: vec![rule(&[5], "22:00", "02:00", Some("party"), None)],
            night: None,
        };
        assert_eq!(s.evaluate(at(FRI, "01:00")).rule, None);
        assert_eq!(s.evaluate(at(SAT, "01:59")).rule, Some(0));
        assert_eq!(s.evaluate(at(SAT, "02:00")).rule, None);
        // Sunday -> Monday wraps the week.
        let s = Schedule {
            rules: vec![rule(&[SUN], "23:00", "01:00", Some("late"), None)],
            night: None,
        };
        assert_eq!(s.evaluate(at(MON, "00:30")).rule, Some(0));
    }

    #[test]
    fn night_mode_beats_every_rule_and_reports_its_end() {
        let d = schedule().evaluate(at(MON, "23:45"));
        assert_eq!(d.mode, Mode::Off);
        assert_eq!(d.mode_until, parse_hhmm("07:00"));
        assert_eq!(d.brightness, Some(0));
        let d = schedule().evaluate(at(SUN, "06:59"));
        assert_eq!(d.mode, Mode::Off);
        assert_eq!(schedule().evaluate(at(SUN, "07:00")).mode, Mode::Normal);

        let mut dim = schedule();
        dim.night = Some(Night {
            window: window("22:00", "23:00"),
            mode: NightMode::Dim,
            brightness: 8,
        });
        let d = dim.evaluate(at(MON, "22:15"));
        assert_eq!(d.mode, Mode::Dim);
        assert_eq!(d.brightness, Some(8));
        // Dim keeps playing the rule's playlist.
        assert_eq!(d.set.as_deref(), Some("consoles"));
    }

    #[test]
    fn a_zero_length_window_is_the_whole_day() {
        let s = Schedule {
            rules: vec![rule(&[SAT], "00:00", "00:00", Some("weekend"), None)],
            night: None,
        };
        assert_eq!(s.evaluate(at(SAT, "00:00")).rule, Some(0));
        assert_eq!(s.evaluate(at(SAT, "23:59")).rule, Some(0));
        assert_eq!(s.evaluate(at(SUN, "00:00")).rule, None);
    }

    #[test]
    fn times_and_days_parse_strictly() {
        assert_eq!(parse_hhmm("07:00"), Some(420));
        assert_eq!(parse_hhmm("7:05"), Some(425));
        assert_eq!(parse_hhmm("23:59"), Some(1439));
        assert_eq!(parse_hhmm("24:00"), Some(0));
        for bad in [
            "", "7", "24:01", "12:60", "12:5", "ab:cd", "-1:00", "123:00",
        ] {
            assert_eq!(parse_hhmm(bad), None, "{bad}");
        }
        assert_eq!(format_hhmm(425), "07:05");
        assert_eq!(format_hhmm(DAY), "00:00");
        assert_eq!(days_mask(&[1, 2, 3, 4, 5]), Some(0b001_1111));
        assert_eq!(days_mask(&[]), Some(0x7f));
        assert_eq!(days_mask(&[0]), None);
        assert_eq!(days_mask(&[8]), None);
    }

    #[test]
    fn local_time_from_tm_fields() {
        assert_eq!(LocalTime::from_tm(0, 7, 30), at(SUN, "07:30"));
        assert_eq!(LocalTime::from_tm(1, 0, 0), at(MON, "00:00"));
        assert_eq!(LocalTime::from_tm(6, 23, 59), at(SAT, "23:59"));
    }
}
