//! Late frames: what a miss is counted as, and the counters.
//!
//! The display loop (firmware `main.rs`, `frametime.rs`) judges each frame
//! when it is on the glass: late when the file's next frame was already due.
//! Every late frame is counted against its iteration's largest phase
//! ([`Cause`]) and in a lateness bucket ([`LATE_BUCKETS`]), whatever its size.
//!
//! **The headline** counts only what a person could see, and says when it
//! happened: a frame [`HEADLINE_MS`] or more late, either **steady** (a GIF
//! playing on) or at a **switch** (the new file's first frame, a transition
//! running, or within [`SWITCH_WINDOW`] of the new file's first frame going
//! up; see [`when`]). Switch misses are the new file's open and first decode
//! taking longer than its first frame's delay — a GIF change, which a
//! transition is covering anyway.

use std::time::Duration;

/// What a missed frame's iteration spent most of its time on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    Open,
    Read,
    Decode,
    Transition,
    Blit,
    Log,
    Other,
    Wake,
}

/// The names, in [`Cause`] order (heartbeat and JSON keys).
pub const CAUSES: [&str; 8] = [
    "open",
    "read",
    "decode",
    "transition",
    "blit",
    "log",
    "other",
    "wake",
];

/// Lateness buckets, ms: under 5, 20, 50, 100, 250, 500, and the rest.
pub const LATE_BUCKETS: [&str; 7] = ["<5", "<20", "<50", "<100", "<250", "<500", ">=500"];
const LATE_EDGES: [u64; 6] = [5, 20, 50, 100, 250, 500];

/// A late frame counts in the headline (`steady`, `switch`) from this many
/// ms late. Under it nothing shows: the panel's own refresh is 10 ms, and on
/// a test panel every steady-play miss but the rare slow card read was 0-1 ms.
pub const HEADLINE_MS: u64 = 5;

/// How long after a new file's first frame went up a miss still counts as
/// the switch's. On a test panel (21.5 min, 161 files, Dissolve 400) every miss
/// came within ~360 ms of the first frame and none in steady play; the
/// default transition runs 383-485 ms. 500 ms covers it without a
/// transition too (a cut, or one the heap gate refused).
pub const SWITCH_WINDOW: Duration = Duration::from_millis(500);

/// When a late frame happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// A GIF playing on.
    Steady,
    /// A GIF change: its first frame, a transition, or just after.
    Switch,
}

/// Classify a late frame: `first_frame` (the tick that opened the file),
/// `transitioning` (a transition running after the tick), and how long
/// since the file's first frame went up (`None`: nothing has started yet).
pub fn when(first_frame: bool, transitioning: bool, since_first: Option<Duration>) -> When {
    if first_frame || transitioning || since_first.is_none_or(|d| d < SWITCH_WINDOW) {
        When::Switch
    } else {
        When::Steady
    }
}

/// One iteration's phases, µs.
#[derive(Default, Clone, Copy, Debug)]
pub struct Phases {
    pub wake: u32,
    pub log: u32,
    pub other: u32,
    pub open: u32,
    pub read: u32,
    /// The tick's own time (decode or composite).
    pub decode: u32,
    pub blit: u32,
    pub transitioning: bool,
}

impl Phases {
    /// The largest phase (the first of equals, in [`Cause`] order).
    pub fn cause(&self) -> Cause {
        let decode = if self.transitioning {
            Cause::Transition
        } else {
            Cause::Decode
        };
        [
            (Cause::Open, self.open),
            (Cause::Read, self.read),
            (decode, self.decode),
            (Cause::Blit, self.blit),
            (Cause::Log, self.log),
            (Cause::Other, self.other),
            (Cause::Wake, self.wake),
        ]
        .into_iter()
        .fold(
            (Cause::Other, 0),
            |best, (c, us)| if us > best.1 { (c, us) } else { best },
        )
        .0
    }
}

/// Misses by cause and by lateness (all of them), and the headline counts
/// (≥ [`HEADLINE_MS`], steady or at a switch), since boot.
#[derive(Default, Clone, Copy, Debug)]
pub struct Misses {
    pub by_cause: [u32; 8],
    pub late: [u32; 7],
    pub steady: u32,
    pub switch: u32,
    /// The latest steady-play frame, ms (any size).
    pub worst_steady_ms: u64,
}

impl Misses {
    pub fn record(&mut self, cause: Cause, late_ms: u64, when: When) {
        self.by_cause[cause as usize] += 1;
        let b = LATE_EDGES.iter().take_while(|&&e| late_ms >= e).count();
        self.late[b] += 1;
        match when {
            When::Steady => {
                self.worst_steady_ms = self.worst_steady_ms.max(late_ms);
                self.steady += (late_ms >= HEADLINE_MS) as u32;
            }
            When::Switch => self.switch += (late_ms >= HEADLINE_MS) as u32,
        }
    }

    /// `misses: open 12 read 0 …; late <5:3 <20:1 …; >=5ms steady N switch N`
    pub fn heartbeat(&self) -> String {
        let mut s = String::from("misses:");
        for (name, n) in CAUSES.iter().zip(self.by_cause) {
            s.push_str(&format!(" {name} {n}"));
        }
        s.push_str("; late");
        for (name, n) in LATE_BUCKETS.iter().zip(self.late) {
            s.push_str(&format!(" {name}:{n}"));
        }
        s.push_str(&format!(
            "; >={HEADLINE_MS}ms steady {} switch {} (worst steady {} ms)",
            self.steady, self.switch, self.worst_steady_ms
        ));
        s
    }

    /// `{"steady":N,"switch":N,"threshold_ms":5,"window_ms":500,
    /// "worst_steady_ms":N,"by_cause":{"open":N,…},"late_ms":{"<5":N,…}}`
    pub fn json(&self) -> String {
        let obj = |names: &[&str], counts: &[u32]| {
            let fields: Vec<String> = names
                .iter()
                .zip(counts)
                .map(|(k, v)| format!(r#""{k}":{v}"#))
                .collect();
            format!("{{{}}}", fields.join(","))
        };
        format!(
            r#"{{"steady":{},"switch":{},"threshold_ms":{HEADLINE_MS},"window_ms":{},"worst_steady_ms":{},"by_cause":{},"late_ms":{}}}"#,
            self.steady,
            self.switch,
            SWITCH_WINDOW.as_millis(),
            self.worst_steady_ms,
            obj(&CAUSES, &self.by_cause),
            obj(&LATE_BUCKETS, &self.late)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Option<Duration> {
        Some(Duration::from_millis(n))
    }

    #[test]
    fn switch_or_steady() {
        // The first frame, a transition, or inside the window: a switch.
        assert_eq!(when(true, false, ms(0)), When::Switch);
        assert_eq!(when(true, false, ms(10_000)), When::Switch);
        assert_eq!(when(false, true, ms(1_500)), When::Switch);
        assert_eq!(when(false, false, ms(360)), When::Switch);
        assert_eq!(when(false, false, ms(499)), When::Switch);
        // Nothing started yet (a command's open before any frame).
        assert_eq!(when(false, false, None), When::Switch);
        // From the window's end on, steady.
        assert_eq!(when(false, false, ms(500)), When::Steady);
        assert_eq!(when(false, false, ms(60_000)), When::Steady);
    }

    #[test]
    fn headline_counts_from_threshold_raw_counts_everything() {
        let mut m = Misses::default();
        m.record(Cause::Open, 7, When::Switch);
        m.record(Cause::Open, 4, When::Switch);
        m.record(Cause::Blit, 1, When::Steady);
        m.record(Cause::Read, 5, When::Steady);
        m.record(Cause::Read, 22, When::Steady);
        assert_eq!((m.steady, m.switch), (2, 1));
        assert_eq!(m.worst_steady_ms, 22);
        assert_eq!(m.late, [2, 2, 1, 0, 0, 0, 0]);
        assert_eq!(m.by_cause[Cause::Open as usize], 2);
        assert_eq!(m.by_cause[Cause::Read as usize], 2);
        assert_eq!(m.by_cause.iter().sum::<u32>(), 5);
        // A switch miss never moves the steady worst.
        m.record(Cause::Open, 400, When::Switch);
        assert_eq!(m.worst_steady_ms, 22);
        assert_eq!(m.late[5], 1);
    }

    #[test]
    fn json_shape() {
        let mut m = Misses::default();
        m.record(Cause::Transition, 6, When::Switch);
        let j = m.json();
        assert!(j.starts_with(
            r#"{"steady":0,"switch":1,"threshold_ms":5,"window_ms":500,"worst_steady_ms":0,"by_cause":{"open":0"#
        ));
        assert!(j.contains(r#""transition":1"#));
        assert!(j.ends_with(r#""<500":0,">=500":0}}"#));
    }

    #[test]
    fn cause_is_the_largest_phase() {
        let p = Phases {
            open: 8_000,
            decode: 9_000,
            blit: 14,
            ..Default::default()
        };
        assert_eq!(p.cause(), Cause::Decode);
        let p = Phases {
            transitioning: true,
            ..p
        };
        assert_eq!(p.cause(), Cause::Transition);
        assert_eq!(Phases::default().cause(), Cause::Other);
    }
}
