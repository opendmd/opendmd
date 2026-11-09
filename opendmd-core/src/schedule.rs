//! Schedules and night mode: given the rules and a local time, what should
//! the panel be doing — which playlist, how bright, or dark?
//!
//! The firmware stores a schedule as JSON in NVS and parses it into these
//! types (the core has no `serde`); this module is the evaluator, so it can
//! be tested on the host.
//!
//! - **Rules** are tried in order and the **first** whose days, dates and
//!   window contain the time wins. A rule may name a playlist (`set`, the
//!   same values as the config's `set`: `""` = all files) and/or a
//!   brightness; whatever it leaves out stays as configured. Its dates are
//!   a range of days of the year, inclusive, which may wrap over New Year
//!   (1 Dec–6 Jan); none is every day of the year.
//! - **Night mode** is one window every day, or from sunset to sunrise
//!   ([`NightWhen::Sun`], [`sun_night`]), and beats every rule: `off`
//!   (panel dark, playback paused) or `dim` (the level given; playback
//!   carries on with whatever playlist applies).
//! - A window `from..to` is half-open, in minutes since local midnight. If
//!   `to <= from` it runs past midnight, and the *start* day is the one its
//!   `days` and dates are checked against (a Friday 22:00–02:00 rule is on
//!   at 01:00 on Saturday; one for 31 October, at 01:00 on 1 November).
//!   `from == to` is the whole day.

use crate::sky::{self, Place};

/// Minutes in a day.
pub const DAY: u16 = 24 * 60;

/// A day of the year as `month * 32 + day` (1 January = 33): ordered like
/// the calendar, the year left out.
pub type MonthDay = u16;

/// A local wall-clock time, as the evaluator needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// ISO weekday: 1 = Monday .. 7 = Sunday.
    pub weekday: u8,
    /// Minutes since local midnight, `0..DAY`.
    pub minute: u16,
    /// The date, for rules with dates.
    pub date: MonthDay,
}

impl LocalTime {
    /// From `struct tm` style fields: `tm_wday` (0 = Sunday), hour, minute,
    /// `tm_mon` (0 = January) and `tm_mday`.
    pub fn from_tm(wday_sunday0: i32, hour: i32, min: i32, mon0: i32, mday: i32) -> Self {
        let weekday = if wday_sunday0 <= 0 {
            7
        } else {
            wday_sunday0.min(6) as u8
        };
        LocalTime {
            weekday,
            minute: (hour.clamp(0, 23) * 60 + min.clamp(0, 59)) as u16,
            date: (mon0.clamp(0, 11) as u16 + 1) * 32 + mday.clamp(1, 31) as u16,
        }
    }
}

/// The day before `d`. (The day before 1 March is taken as 29 February:
/// the year is not known, and a range that ends on 28 February misses only
/// the small hours of 1 March in a window that runs past midnight.)
fn yesterday(d: MonthDay) -> MonthDay {
    let (m, day) = (d / 32, d % 32);
    if day > 1 {
        return d - 1;
    }
    let m = if m <= 1 { 12 } else { m - 1 };
    m * 32 + days_in(m)
}

fn days_in(month: u16) -> u16 {
    match month {
        2 => 29,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `"MM-DD"` (`10-31`) to a [`MonthDay`]; 29 February is a day.
pub fn parse_month_day(s: &str) -> Option<MonthDay> {
    let (m, d) = s.trim().split_once('-')?;
    if m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (m, d): (u16, u16) = (m.parse().ok()?, d.parse().ok()?);
    ((1..=12).contains(&m) && (1..=days_in(m)).contains(&d)).then_some(m * 32 + d)
}

/// Is `d` in the inclusive range `from..=to`, which wraps over New Year
/// when `to < from`?
fn in_dates((from, to): (MonthDay, MonthDay), d: MonthDay) -> bool {
    if from <= to {
        (from..=to).contains(&d)
    } else {
        d >= from || d <= to
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
    /// allowed (bit `d - 1` for ISO day `d`) and `dates` the start dates?
    fn contains(&self, days: u8, dates: Option<(MonthDay, MonthDay)>, t: LocalTime) -> bool {
        let on = |day: u8, date: MonthDay| {
            days & (1 << (day - 1)) != 0 && dates.is_none_or(|r| in_dates(r, date))
        };
        let today = || on(t.weekday, t.date);
        let before = || on(if t.weekday == 1 { 7 } else { t.weekday - 1 }, yesterday(t.date));
        if self.from == self.to {
            today()
        } else if self.from < self.to {
            (self.from..self.to).contains(&t.minute) && today()
        } else {
            (t.minute >= self.from && today()) || (t.minute < self.to && before())
        }
    }
}

/// One rule: on these days, in this window, play this / at this brightness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Bit `d - 1` for ISO day `d` (Monday = bit 0).
    pub days: u8,
    /// The dates it runs on, inclusive (`None`: every day of the year).
    pub dates: Option<(MonthDay, MonthDay)>,
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

/// When night mode is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NightWhen {
    /// The same clock window every day.
    Clock(Window),
    /// From sunset to sunrise where the panel is (the saved location),
    /// starting `offset` minutes after sunset and ending as long before
    /// sunrise (a negative offset: before sunset, after sunrise). Off
    /// without a location.
    Sun { offset: i16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Night {
    pub when: NightWhen,
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

    /// Night mode by the sun ([`NightWhen::Sun`]), if that is the kind.
    pub fn sun_offset(&self) -> Option<i16> {
        match self.night?.when {
            NightWhen::Sun { offset } => Some(offset),
            NightWhen::Clock(_) => None,
        }
    }

    /// What should be happening at `t`. `sun`: for night mode by the sun,
    /// whether it is on now and when it ends, as the caller worked it out
    /// ([`sun_night`], [`sun_night_end`]); `None` (no location, no clock)
    /// leaves such a night mode off.
    pub fn evaluate(&self, t: LocalTime, sun: Option<SunNow>) -> Decision {
        let mut d = Decision::default();
        if let Some((i, rule)) = self
            .rules
            .iter()
            .enumerate()
            .find(|(_, r)| r.window.contains(r.days, r.dates, t))
        {
            d.rule = Some(i);
            d.rule_until = Some(rule.window.to);
            d.set = rule.set.clone();
            d.brightness = rule.brightness;
        }
        if let Some(night) = &self.night {
            let (on, until) = match night.when {
                NightWhen::Clock(w) => (w.contains(0x7f, None, t), Some(w.to)),
                NightWhen::Sun { .. } => sun.map_or((false, None), |s| (s.night, s.until)),
            };
            if on {
                d.mode_until = until;
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

/// Night mode by the sun at one moment ([`Schedule::evaluate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SunNow {
    pub night: bool,
    /// When it ends (minutes since local midnight), while it is on; `None`
    /// when the sun doesn't rise in the next day (a polar night).
    pub until: Option<u16>,
}

/// The sun's altitude at sunrise and sunset, −0.833° (the almanacs':
/// refraction and the sun's half-width), in Q32 turns.
const HORIZON: i32 = (-0.833 / 360.0 * 4_294_967_296.0) as i32;

/// Is the sun down (below [`HORIZON`]) at `unix`?
fn sun_down(unix: i64, place: Place) -> bool {
    sky::sun(unix, place).1 < HORIZON
}

/// Is night mode by the sun on at `unix` (UTC seconds), with `offset`
/// ([`NightWhen::Sun`])? The sun down `offset` minutes before and after now
/// (both of them for a later start and an earlier end, either for an earlier
/// start and a later end). Two positions of the sun: cheap enough to ask
/// once a minute. In UTC throughout, so a change of the clocks moves
/// nothing.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
#[inline(never)]
pub fn sun_night(unix: i64, place: Place, offset: i16) -> bool {
    let k = i64::from(offset) * 60;
    let (a, b) = (sun_down(unix - k, place), sun_down(unix + k, place));
    if offset >= 0 {
        a && b
    } else {
        a || b
    }
}

/// When a night by the sun that is on at `unix` ends: the first moment
/// [`sun_night`] is false, to the minute, within the next 26 hours (`None`
/// past that: a polar night). Half-hour steps then halving: at most 57
/// [`sun_night`]s, so the caller works it out once a night, not every
/// minute.
#[cfg_attr(target_arch = "xtensa", optimize(size))]
#[inline(never)]
pub fn sun_night_end(unix: i64, place: Place, offset: i16) -> Option<i64> {
    const STEP: i64 = 30 * 60;
    let mut lo = unix;
    let mut hi = (1..=52)
        .map(|k| unix + k * STEP)
        .find(|&t| !sun_night(t, place, offset))?;
    lo = lo.max(hi - STEP);
    // `lo` is night, `hi` is not: halve to the minute.
    while hi - lo > 60 {
        let mid = lo + (hi - lo) / 2;
        if sun_night(mid, place, offset) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(hi - hi.rem_euclid(60))
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
        on(weekday, "06-15", hhmm)
    }

    fn on(weekday: u8, date: &str, hhmm: &str) -> LocalTime {
        LocalTime {
            weekday,
            minute: parse_hhmm(hhmm).unwrap(),
            date: parse_month_day(date).unwrap(),
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
            dates: None,
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
                when: NightWhen::Clock(window("23:30", "07:00")),
                mode: NightMode::Off,
                brightness: 8,
            }),
        }
    }

    #[test]
    fn nothing_matches_outside_every_window() {
        let d = schedule().evaluate(at(MON, "12:00"), None);
        assert_eq!(d, Decision::default());
        assert_eq!(
            Schedule::default().evaluate(at(MON, "12:00"), None),
            Decision::default()
        );
    }

    #[test]
    fn a_rule_selects_its_playlist_and_brightness_until_its_end() {
        let d = schedule().evaluate(at(MON, "18:00"), None);
        assert_eq!(d.mode, Mode::Normal);
        assert_eq!(d.rule, Some(0));
        assert_eq!(d.set.as_deref(), Some("consoles"));
        assert_eq!(d.brightness, Some(96));
        assert_eq!(d.rule_until, parse_hhmm("23:00"));
        // Half-open: 23:00 is outside.
        assert_eq!(schedule().evaluate(at(MON, "23:00"), None).rule, None);
        // Not on a Saturday.
        assert_eq!(schedule().evaluate(at(SAT, "18:30"), None).rule, None);
    }

    #[test]
    fn first_matching_rule_wins() {
        // Friday 22:30 matches both the weekday rule and the Friday one.
        let d = schedule().evaluate(at(FRI, "22:30"), None);
        assert_eq!(d.rule, Some(0));
        assert_eq!(d.set.as_deref(), Some("consoles"));
    }

    #[test]
    fn an_overnight_rule_belongs_to_the_day_it_starts() {
        // Friday's 22:00-02:00 is on at 01:00 on Saturday...
        let d = schedule().evaluate(at(SAT, "01:00"), None);
        assert_eq!(d.rule, Some(1));
        assert_eq!(d.set.as_deref(), Some("party"));
        assert_eq!(d.brightness, Some(0), "and night mode is on too");
        assert_eq!(d.mode, Mode::Off);
        // ...but not at 01:00 on Friday (Thursday is not in its days).
        let s = Schedule {
            rules: vec![rule(&[5], "22:00", "02:00", Some("party"), None)],
            night: None,
        };
        assert_eq!(s.evaluate(at(FRI, "01:00"), None).rule, None);
        assert_eq!(s.evaluate(at(SAT, "01:59"), None).rule, Some(0));
        assert_eq!(s.evaluate(at(SAT, "02:00"), None).rule, None);
        // Sunday -> Monday wraps the week.
        let s = Schedule {
            rules: vec![rule(&[SUN], "23:00", "01:00", Some("late"), None)],
            night: None,
        };
        assert_eq!(s.evaluate(at(MON, "00:30"), None).rule, Some(0));
    }

    #[test]
    fn night_mode_beats_every_rule_and_reports_its_end() {
        let d = schedule().evaluate(at(MON, "23:45"), None);
        assert_eq!(d.mode, Mode::Off);
        assert_eq!(d.mode_until, parse_hhmm("07:00"));
        assert_eq!(d.brightness, Some(0));
        let d = schedule().evaluate(at(SUN, "06:59"), None);
        assert_eq!(d.mode, Mode::Off);
        assert_eq!(schedule().evaluate(at(SUN, "07:00"), None).mode, Mode::Normal);

        let mut dim = schedule();
        dim.night = Some(Night {
            when: NightWhen::Clock(window("22:00", "23:00")),
            mode: NightMode::Dim,
            brightness: 8,
        });
        let d = dim.evaluate(at(MON, "22:15"), None);
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
        assert_eq!(s.evaluate(at(SAT, "00:00"), None).rule, Some(0));
        assert_eq!(s.evaluate(at(SAT, "23:59"), None).rule, Some(0));
        assert_eq!(s.evaluate(at(SUN, "00:00"), None).rule, None);
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
    fn dates_hold_a_rule_to_part_of_the_year_and_may_wrap() {
        let dated = |from: &str, to: &str, set: &str| Rule {
            dates: Some((parse_month_day(from).unwrap(), parse_month_day(to).unwrap())),
            ..rule(&[], "00:00", "00:00", Some(set), None)
        };
        let s = Schedule {
            rules: vec![
                dated("10-01", "10-31", "filelist.hall"),
                dated("12-01", "12-26", "filelist.xmas"),
                dated("12-27", "01-06", "newyear"),
                rule(&[], "00:00", "00:00", Some("everyday"), None),
            ],
            night: None,
        };
        let set = |date: &str| s.evaluate(on(MON, date, "12:00"), None).set.unwrap();
        assert_eq!(set("09-30"), "everyday");
        assert_eq!(set("10-01"), "filelist.hall");
        assert_eq!(set("10-31"), "filelist.hall");
        assert_eq!(set("11-01"), "everyday");
        assert_eq!(set("12-01"), "filelist.xmas");
        assert_eq!(set("12-26"), "filelist.xmas");
        assert_eq!(set("12-31"), "newyear");
        assert_eq!(set("01-01"), "newyear");
        assert_eq!(set("01-06"), "newyear");
        assert_eq!(set("01-07"), "everyday");

        // A window past midnight belongs to its start date, as to its start day.
        let late = Schedule {
            rules: vec![Rule {
                dates: Some((parse_month_day("10-31").unwrap(), parse_month_day("10-31").unwrap())),
                ..rule(&[], "22:00", "02:00", Some("party"), None)
            }],
            night: None,
        };
        assert_eq!(late.evaluate(on(SAT, "10-31", "23:00"), None).rule, Some(0));
        assert_eq!(late.evaluate(on(SUN, "11-01", "01:00"), None).rule, Some(0));
        assert_eq!(late.evaluate(on(SAT, "10-31", "01:00"), None).rule, None);
        assert_eq!(late.evaluate(on(SUN, "11-01", "23:00"), None).rule, None);
    }

    #[test]
    fn month_days_parse_strictly_and_step_back() {
        assert_eq!(parse_month_day("10-31"), Some(10 * 32 + 31));
        assert_eq!(parse_month_day("02-29"), Some(2 * 32 + 29));
        for bad in ["", "1-01", "01-1", "02-30", "04-31", "13-01", "00-10", "10/31", "aa-bb"] {
            assert_eq!(parse_month_day(bad), None, "{bad}");
        }
        let md = |s| parse_month_day(s).unwrap();
        assert_eq!(yesterday(md("03-01")), md("02-29"));
        assert_eq!(yesterday(md("01-01")), md("12-31"));
        assert_eq!(yesterday(md("05-01")), md("04-30"));
        assert_eq!(yesterday(md("10-31")), md("10-30"));
    }

    /// Unix seconds of a UTC date and time.
    fn utc(y: i64, m: i64, d: i64, hh: i64, mm: i64) -> i64 {
        // Days from 1970-01-01 (Howard Hinnant's days_from_civil).
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        (era * 146_097 + doe - 719_468) * 86_400 + hh * 3600 + mm * 60
    }

    fn place(lat: f64, lon: f64) -> Place {
        Place::from_degrees(lat, lon).unwrap()
    }

    /// Within `min` minutes of `want`.
    fn near(got: Option<i64>, want: i64, min: i64) -> bool {
        got.is_some_and(|t| (t - want).abs() <= min * 60)
    }

    #[test]
    fn night_by_the_sun_runs_from_sunset_to_sunrise() {
        // London on 21 June 2026: sunset 20:21 UTC, sunrise 03:43 UTC.
        let london = place(51.51, -0.13);
        assert!(!sun_night(utc(2026, 6, 21, 12, 0), london, 0));
        assert!(!sun_night(utc(2026, 6, 21, 20, 15), london, 0));
        assert!(sun_night(utc(2026, 6, 21, 20, 30), london, 0));
        let end = sun_night_end(utc(2026, 6, 21, 23, 0), london, 0);
        assert!(near(end, utc(2026, 6, 22, 3, 43), 3), "{end:?}");
        assert_eq!(end.unwrap() % 60, 0, "to the minute");
        // 30 minutes after sunset to 30 before sunrise, and the other way.
        assert!(!sun_night(utc(2026, 6, 21, 20, 40), london, 30));
        assert!(sun_night(utc(2026, 6, 21, 21, 0), london, 30));
        assert!(near(sun_night_end(utc(2026, 6, 21, 23, 0), london, 30), utc(2026, 6, 22, 3, 13), 3));
        assert!(sun_night(utc(2026, 6, 21, 20, 0), london, -30));
        assert!(!sun_night(utc(2026, 6, 21, 19, 40), london, -30));
        assert!(near(sun_night_end(utc(2026, 6, 21, 23, 0), london, -30), utc(2026, 6, 22, 4, 13), 3));
        // Midwinter: a long night (sunrise 08:04 UTC).
        assert!(near(sun_night_end(utc(2026, 12, 21, 18, 0), london, 0), utc(2026, 12, 22, 8, 4), 3));
    }

    #[test]
    fn polar_night_and_polar_day() {
        for (lat, lon) in [(69.65, 18.96), (78.22, 15.65)] {
            let p = place(lat, lon);
            // Midwinter: dark all day, and no sunrise to end it.
            for h in 0..24 {
                assert!(sun_night(utc(2026, 12, 21, h, 0), p, 0), "{lat} December {h}:00");
            }
            assert_eq!(sun_night_end(utc(2026, 12, 21, 12, 0), p, 0), None, "{lat}");
            // Midsummer: never.
            for h in 0..24 {
                assert!(!sun_night(utc(2026, 6, 21, h, 0), p, 0), "{lat} June {h}:00");
                assert!(!sun_night(utc(2026, 6, 21, h, 30), p, -60), "{lat} June {h}:30");
            }
        }
    }

    #[test]
    fn a_change_of_the_clocks_moves_no_sunrise() {
        // Dublin's clocks go forward at 01:00 UTC on 29 March 2026 and back
        // at 01:00 UTC on 25 October: the nights either side end within a
        // couple of minutes of each other in UTC (the evaluator's caller
        // turns the moment into the local minute, summer time included).
        let dublin = place(53.35, -6.26);
        for (m, d) in [(3, 28), (10, 24)] {
            let ends: Vec<i64> = (0..3)
                .map(|k| sun_night_end(utc(2026, m, d - 1 + k, 23, 0), dublin, 0).unwrap())
                .collect();
            for w in ends.windows(2) {
                let gap = w[1] - w[0] - 86_400;
                assert!(gap.abs() <= 4 * 60, "{m}/{d}: {gap} s");
            }
            // Through the night of the change, one switch: off at sunrise.
            let mut was = true;
            let mut switches = 0;
            for k in 0..(10 * 60) {
                let now = sun_night(utc(2026, m, d, 23, 0) + k * 60, dublin, 0);
                switches += i32::from(now != was);
                was = now;
            }
            assert_eq!(switches, 1, "{m}/{d}");
        }
    }

    #[test]
    fn the_sun_decides_only_a_night_by_the_sun() {
        let mut s = schedule();
        s.night = Some(Night {
            when: NightWhen::Sun { offset: 0 },
            mode: NightMode::Dim,
            brightness: 8,
        });
        assert_eq!(s.sun_offset(), Some(0));
        // No location or clock: off.
        assert_eq!(s.evaluate(at(MON, "23:45"), None).mode, Mode::Normal);
        let dark = SunNow {
            night: true,
            until: parse_hhmm("07:42"),
        };
        let d = s.evaluate(at(MON, "12:00"), Some(dark));
        assert_eq!((d.mode, d.mode_until, d.brightness), (Mode::Dim, parse_hhmm("07:42"), Some(8)));
        let light = SunNow {
            night: false,
            until: None,
        };
        assert_eq!(s.evaluate(at(MON, "23:45"), Some(light)).mode, Mode::Normal);
        // A clock window pays no attention to it.
        assert_eq!(schedule().sun_offset(), None);
        assert_eq!(schedule().evaluate(at(MON, "12:00"), Some(dark)).mode, Mode::Normal);
    }

    #[test]
    fn local_time_from_tm_fields() {
        assert_eq!(LocalTime::from_tm(0, 7, 30, 5, 15), at(SUN, "07:30"));
        assert_eq!(LocalTime::from_tm(1, 0, 0, 5, 15), at(MON, "00:00"));
        assert_eq!(LocalTime::from_tm(6, 23, 59, 11, 31), on(SAT, "12-31", "23:59"));
    }
}
