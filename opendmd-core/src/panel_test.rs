//! The guided panel-timing test (Settings → Advanced → Panel timing → Test
//! panel timing): the firmware draws patterns that show a marginal panel's
//! wrong dots and flips one live setting between two values, A and B, so a
//! user can say which looks clean without knowing what the setting is.
//!
//! The patterns are the four that exposed a real panel's fault (a fixed
//! patch of wrong dots with the data clocked on the rising edge, clean on the
//! falling one; a white/black checkerboard, 0x80/0x7F checkerboards and flat
//! colours did not): a 1 px magenta/green checkerboard, 1 px white/black
//! columns, a 1 px checkerboard of two muted complements, and a 1 px
//! checkerboard of `v`/`255 - v` grey with `v = row × 8`. Each is shown under
//! A for [`SIDE`], then under B for [`SIDE`], back to back, so the dots come
//! and go on the same picture; a small "A" or "B" in the bottom-right corner
//! (a 5 × 7 box, away from where a scan-half fault shows) says which.
//!
//! Only the two settings that apply live are tested ([`Setting`]): the
//! clock edge (A rising, the default; B falling) and the latch blanking (A
//! what runs; B two clocks more, at most 4, from 4 two fewer: [`variants`]). A test ends by itself after
//! [`LIMIT`] (the browser may have gone), and ending it — however — puts back
//! what ran before ([`PanelTest::restore`]); it never saves anything. Pure
//! logic: the firmware draws what [`PanelTest::step`] says and applies its
//! timing through the panel's live path.

use std::time::Duration;

use crate::canvas::{Canvas, Rgb, HEIGHT, WIDTH};
use crate::panel_timing::{PanelTiming, Tuner, LATCH_MAX, LATCH_MIN};

/// How long one pattern is shown under one value.
pub const SIDE: Duration = Duration::from_secs(2);
/// A test ends by itself after this (restarting it starts the clock again).
pub const LIMIT: Duration = Duration::from_secs(120);

/// The live setting under test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    ClkPhase,
    LatchBlanking,
}

impl Setting {
    /// The `/api/config` `panel` key.
    pub fn name(self) -> &'static str {
        match self {
            Setting::ClkPhase => "clk_phase",
            Setting::LatchBlanking => "latch_blanking",
        }
    }

    pub fn from_name(s: &str) -> Option<Setting> {
        [Setting::ClkPhase, Setting::LatchBlanking]
            .into_iter()
            .find(|v| v.name() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

/// The test patterns, full screen, in the order shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// 1 px checkerboard, magenta (255,0,255) / green (0,255,0).
    MagentaGreen,
    /// 1 px columns, white / black.
    Columns,
    /// 1 px checkerboard, (83,72,108) / (172,183,147).
    Muted,
    /// 1 px checkerboard of grey `v` / `255 - v`, `v = row × 8`.
    Ramp,
}

pub const PATTERNS: [Pattern; 4] = [
    Pattern::MagentaGreen,
    Pattern::Columns,
    Pattern::Muted,
    Pattern::Ramp,
];

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb { r, g, b }
}

/// One pattern pixel.
pub fn pixel(p: Pattern, x: usize, y: usize) -> Rgb {
    let odd = (x + y) % 2 == 1;
    match p {
        Pattern::MagentaGreen if odd => rgb(0, 255, 0),
        Pattern::MagentaGreen => rgb(255, 0, 255),
        Pattern::Columns if x.is_multiple_of(2) => rgb(255, 255, 255),
        Pattern::Columns => Rgb::BLACK,
        Pattern::Muted if odd => rgb(172, 183, 147),
        Pattern::Muted => rgb(83, 72, 108),
        Pattern::Ramp => {
            let v = (y * 8).min(255) as u8;
            let v = if odd { 255 - v } else { v };
            rgb(v, v, v)
        }
    }
}

/// The label: a black box [`LABEL_W`] × [`LABEL_H`] in the bottom-right
/// corner, a 3 × 5 white letter inside.
pub const LABEL_W: usize = 5;
pub const LABEL_H: usize = 7;
const GLYPHS: [[u8; 5]; 2] = [
    // A
    [0b010, 0b101, 0b111, 0b101, 0b101],
    // B
    [0b110, 0b101, 0b110, 0b101, 0b110],
];

/// The whole picture: `p` with `side`'s label. Marks every pixel.
pub fn draw(canvas: &mut Canvas, p: Pattern, side: Side) {
    let (x0, y0) = (WIDTH - LABEL_W, HEIGHT - LABEL_H);
    let glyph = &GLYPHS[side as usize];
    let (px, dirty) = canvas.pixels_and_dirty_mut();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            px[y * WIDTH + x] = if x >= x0 && y >= y0 {
                let (gx, gy) = (x - x0, y - y0);
                let on = (1..4).contains(&gx)
                    && (1..6).contains(&gy)
                    && (glyph[gy - 1] >> (3 - gx)) & 1 == 1;
                if on {
                    rgb(255, 255, 255)
                } else {
                    Rgb::BLACK
                }
            } else {
                pixel(p, x, y)
            };
        }
    }
    dirty.mark_all();
}

/// A and B for `setting`, from `base` (what runs): the clock edge rising
/// (the default) then falling; the latch blanking as it is, then two clocks
/// more (at most [`LATCH_MAX`]; from the maximum, two fewer).
pub fn variants(setting: Setting, base: &PanelTiming) -> (PanelTiming, PanelTiming) {
    match setting {
        Setting::ClkPhase => (
            PanelTiming {
                clk_phase: true,
                ..*base
            },
            PanelTiming {
                clk_phase: false,
                ..*base
            },
        ),
        Setting::LatchBlanking => {
            let l = base.latch_blanking;
            let b = if l < LATCH_MAX {
                (l + 2).min(LATCH_MAX)
            } else {
                LATCH_MAX - 2
            };
            debug_assert!((LATCH_MIN..=LATCH_MAX).contains(&b));
            (
                *base,
                PanelTiming {
                    latch_blanking: b,
                    ..*base
                },
            )
        }
    }
}

/// What is on the panel `elapsed` into a test, and how long until it
/// changes: each pattern under A for [`SIDE`], then under B, then the next.
pub fn at(elapsed: Duration) -> (Pattern, Side, Duration) {
    let side_ms = SIDE.as_millis() as u64;
    let ms = elapsed.as_millis() as u64;
    let slot = ms / side_ms;
    let pattern = PATTERNS[(slot / 2) as usize % PATTERNS.len()];
    let side = if slot.is_multiple_of(2) {
        Side::A
    } else {
        Side::B
    };
    (pattern, side, Duration::from_millis(side_ms - ms % side_ms))
}

/// What the firmware does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Apply `timing` live, draw `pattern` with `side`'s label; nothing
    /// changes for `wait`.
    Draw {
        pattern: Pattern,
        side: Side,
        timing: PanelTiming,
        wait: Duration,
    },
    /// What is shown stays for `wait`.
    Wait(Duration),
    /// Time is up: put [`PanelTest::restore`] back and end the test.
    End,
}

/// A test in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelTest {
    pub setting: Setting,
    pub a: PanelTiming,
    pub b: PanelTiming,
    /// What ran before the test, and runs again after it.
    pub restore: PanelTiming,
    started: Duration,
    shown: Option<(Pattern, Side)>,
}

impl PanelTest {
    /// Start testing `setting` at uptime `now`; `running` is what the panel
    /// runs (the saved settings: the firmware refuses a test during a
    /// trial).
    pub fn start(setting: Setting, running: PanelTiming, now: Duration) -> PanelTest {
        let (a, b) = variants(setting, &running);
        PanelTest {
            setting,
            a,
            b,
            restore: running,
            started: now,
            shown: None,
        }
    }

    /// Whole seconds left before it ends by itself.
    pub fn left(&self, now: Duration) -> u32 {
        (self.started + LIMIT)
            .saturating_sub(now)
            .as_secs_f32()
            .ceil() as u32
    }

    /// Every main-loop iteration while the test runs.
    pub fn step(&mut self, now: Duration) -> Step {
        let elapsed = now.saturating_sub(self.started);
        if elapsed >= LIMIT {
            return Step::End;
        }
        let (pattern, side, wait) = at(elapsed);
        let wait = wait.min(LIMIT - elapsed);
        if self.shown == Some((pattern, side)) {
            return Step::Wait(wait);
        }
        self.shown = Some((pattern, side));
        Step::Draw {
            pattern,
            side,
            timing: match side {
                Side::A => self.a,
                Side::B => self.b,
            },
            wait,
        }
    }
}

/// The API's 409s.
pub const TRIAL_RUNNING: &str = "a panel trial is running: keep or undo it first";
pub const PANEL_OFF: &str = "the panel is off (night mode)";
pub const STAGED: &str = "A panel change is waiting for a restart: restart or undo it first";
pub const RISING_ONLY: &str =
    "this driver chip always uses the rising edge: test latch_blanking instead";

/// Why a test of `setting` may not start now, if it may not: a trial runs
/// (what runs would not be the saved timing), a boot-time change is staged
/// (a pick's save would commit it too), night mode has the panel off, or
/// the driver chip ignores the clock edge.
pub fn refusal(setting: Setting, tuner: &Tuner, panel_off: bool) -> Option<&'static str> {
    if tuner.trial.is_some() {
        Some(TRIAL_RUNNING)
    } else if tuner.staged.is_some() {
        Some(STAGED)
    } else if panel_off {
        Some(PANEL_OFF)
    } else if setting == Setting::ClkPhase && tuner.running.driver.forces_rising_edge() {
        Some(RISING_ONLY)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn patterns_are_the_ones_that_showed_the_fault() {
        let m = rgb(255, 0, 255);
        let g = rgb(0, 255, 0);
        assert_eq!(pixel(Pattern::MagentaGreen, 0, 0), m);
        assert_eq!(pixel(Pattern::MagentaGreen, 1, 0), g);
        assert_eq!(pixel(Pattern::MagentaGreen, 0, 1), g);
        assert_eq!(pixel(Pattern::MagentaGreen, 1, 1), m);
        assert_eq!(pixel(Pattern::Columns, 0, 5), rgb(255, 255, 255));
        assert_eq!(pixel(Pattern::Columns, 1, 5), Rgb::BLACK);
        assert_eq!(pixel(Pattern::Columns, 2, 6), rgb(255, 255, 255));
        assert_eq!(pixel(Pattern::Muted, 4, 2), rgb(83, 72, 108));
        assert_eq!(pixel(Pattern::Muted, 5, 2), rgb(172, 183, 147));
        // v = row × 8, and its complement on the odd squares.
        assert_eq!(pixel(Pattern::Ramp, 0, 0), rgb(0, 0, 0));
        assert_eq!(pixel(Pattern::Ramp, 1, 0), rgb(255, 255, 255));
        assert_eq!(pixel(Pattern::Ramp, 0, 10), rgb(80, 80, 80));
        assert_eq!(pixel(Pattern::Ramp, 1, 10), rgb(175, 175, 175));
        assert_eq!(pixel(Pattern::Ramp, 1, 31), rgb(248, 248, 248));
        assert_eq!(pixel(Pattern::Ramp, 0, 31), rgb(7, 7, 7));
    }

    #[test]
    fn draw_fills_the_screen_and_labels_a_small_corner() {
        let mut c = Canvas::new();
        c.dirty_mut().clear();
        draw(&mut c, Pattern::MagentaGreen, Side::A);
        assert_eq!(c.dirty().count() as usize, WIDTH * HEIGHT);
        let mut label = 0;
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let in_box = x >= WIDTH - LABEL_W && y >= HEIGHT - LABEL_H;
                if in_box {
                    label += 1;
                    let p = c.get(x, y);
                    assert!(p == Rgb::BLACK || p == rgb(255, 255, 255));
                } else {
                    assert_eq!(c.get(x, y), pixel(Pattern::MagentaGreen, x, y), "{x},{y}");
                }
            }
        }
        // 35 of 4,096 pixels.
        assert_eq!(label, 35);
        // The letters differ, and each has lit pixels only inside the box's
        // 1 px border.
        let lit = |c: &Canvas| {
            let mut v = Vec::new();
            for y in HEIGHT - LABEL_H..HEIGHT {
                for x in WIDTH - LABEL_W..WIDTH {
                    if c.get(x, y) != Rgb::BLACK {
                        v.push((x, y));
                    }
                }
            }
            v
        };
        let a = lit(&c);
        draw(&mut c, Pattern::Ramp, Side::B);
        let b = lit(&c);
        assert_ne!(a, b);
        for (x, y) in a.iter().chain(&b) {
            assert!((WIDTH - 4..WIDTH - 1).contains(x) && (HEIGHT - 6..HEIGHT - 1).contains(y));
        }
        assert_eq!(a.len(), 10); // A: 1 + 2 + 3 + 2 + 2
        assert_eq!(b.len(), 10); // B: 2 + 2 + 2 + 2 + 2
    }

    #[test]
    fn variants_change_only_the_setting() {
        let saved = PanelTiming {
            clk_phase: false,
            latch_blanking: 2,
            ..Default::default()
        };
        let (a, b) = variants(Setting::ClkPhase, &saved);
        assert!(a.clk_phase && !b.clk_phase);
        assert_eq!((a.latch_blanking, b.latch_blanking), (2, 2));
        let (a, b) = variants(Setting::LatchBlanking, &saved);
        assert_eq!(a, saved);
        assert_eq!(
            b,
            PanelTiming {
                latch_blanking: 4,
                ..saved
            }
        );
        for (l, want) in [(1, 3), (2, 4), (3, 4), (4, 2)] {
            let base = PanelTiming {
                latch_blanking: l,
                ..Default::default()
            };
            assert_eq!(
                variants(Setting::LatchBlanking, &base).1.latch_blanking,
                want
            );
        }
        for s in [Setting::ClkPhase, Setting::LatchBlanking] {
            assert_eq!(Setting::from_name(s.name()), Some(s));
            let (a, b) = variants(s, &PanelTiming::default());
            assert!(a.same_boot_values(&b), "never a value that needs a restart");
            assert_ne!(a, b);
        }
        assert_eq!(Setting::from_name("driver"), None);
    }

    #[test]
    fn each_pattern_under_a_then_b() {
        let want = [
            (Pattern::MagentaGreen, Side::A),
            (Pattern::MagentaGreen, Side::B),
            (Pattern::Columns, Side::A),
            (Pattern::Columns, Side::B),
            (Pattern::Muted, Side::A),
            (Pattern::Muted, Side::B),
            (Pattern::Ramp, Side::A),
            (Pattern::Ramp, Side::B),
            (Pattern::MagentaGreen, Side::A),
        ];
        for (i, w) in want.into_iter().enumerate() {
            let t = SIDE * i as u32;
            let (p, s, wait) = at(t);
            assert_eq!((p, s), w, "slot {i}");
            assert_eq!(wait, SIDE);
            let (p, s, wait) = at(t + SIDE - MS(1));
            assert_eq!((p, s), w);
            assert_eq!(wait, MS(1));
        }
    }

    #[test]
    fn a_test_draws_on_changes_and_ends_by_itself() {
        let saved = PanelTiming::default();
        let t0 = Duration::from_secs(1000);
        let mut t = PanelTest::start(Setting::ClkPhase, saved, t0);
        assert_eq!(t.left(t0), 120);
        let Step::Draw {
            pattern,
            side,
            timing,
            wait,
        } = t.step(t0)
        else {
            panic!("first step draws");
        };
        assert_eq!(
            (pattern, side, wait),
            (Pattern::MagentaGreen, Side::A, SIDE)
        );
        assert!(timing.clk_phase);
        // Nothing new until the side changes.
        assert_eq!(t.step(t0 + MS(500)), Step::Wait(MS(1500)));
        let Step::Draw { side, timing, .. } = t.step(t0 + SIDE) else {
            panic!("B");
        };
        assert_eq!(side, Side::B);
        assert!(!timing.clk_phase);
        assert_eq!(
            timing,
            PanelTiming {
                clk_phase: false,
                ..saved
            }
        );
        // Ends at the limit, and stays ended; what goes back is the saved.
        assert!(matches!(
            t.step(t0 + LIMIT - MS(1)),
            Step::Draw { .. } | Step::Wait(_)
        ));
        assert_eq!(t.step(t0 + LIMIT), Step::End);
        assert_eq!(t.step(t0 + LIMIT * 2), Step::End);
        assert_eq!(t.left(t0 + LIMIT), 0);
        assert_eq!(t.restore, saved);
        // A wait never runs past the end.
        let mut t = PanelTest::start(Setting::LatchBlanking, saved, Duration::ZERO);
        let _ = t.step(LIMIT - MS(300));
        assert_eq!(t.step(LIMIT - MS(100)), Step::Wait(MS(100)));
    }

    #[test]
    fn refused_when_a_test_would_mislead_or_commit_too_much() {
        use crate::panel_timing::{PanelOp, PanelPatch};
        let s = Duration::from_secs;
        let t = Tuner::boot(None, None);
        assert_eq!(refusal(Setting::ClkPhase, &t, false), None);
        assert_eq!(refusal(Setting::ClkPhase, &t, true), Some(PANEL_OFF));
        let mut tr = t;
        let p = PanelPatch::new(Some(false), None, None, None, None).unwrap();
        tr.apply(PanelOp::Try(p), s(1)).unwrap();
        assert_eq!(
            refusal(Setting::LatchBlanking, &tr, false),
            Some(TRIAL_RUNNING)
        );
        let mut st = t;
        let p = PanelPatch::new(None, None, Some("fm6126a"), None, None).unwrap();
        st.apply(PanelOp::Try(p), s(1)).unwrap();
        assert!(st.staged.is_some() && st.trial.is_none());
        assert_eq!(refusal(Setting::LatchBlanking, &st, false), Some(STAGED));
        let mbi = Tuner::boot(Some("1,1,mbi5124,8,60"), None);
        assert_eq!(refusal(Setting::ClkPhase, &mbi, false), Some(RISING_ONLY));
        assert_eq!(refusal(Setting::LatchBlanking, &mbi, false), None);
    }

    #[test]
    fn restarting_starts_the_clock_again_and_restores_the_same() {
        let saved = PanelTiming {
            clk_phase: false,
            latch_blanking: 3,
            ..Default::default()
        };
        let t = PanelTest::start(Setting::ClkPhase, saved, Duration::ZERO);
        let mut t2 = PanelTest::start(Setting::LatchBlanking, saved, Duration::from_secs(100));
        assert_eq!(t.restore, t2.restore);
        assert_eq!(t2.left(Duration::from_secs(100)), 120);
        assert!(matches!(
            t2.step(Duration::from_secs(150)),
            Step::Draw { .. }
        ));
        let (a, b) = (t2.a, t2.b);
        assert_eq!((a.latch_blanking, b.latch_blanking), (3, 4));
        assert!(
            !a.clk_phase && !b.clk_phase,
            "stage 2 keeps the saved clock edge"
        );
    }
}
