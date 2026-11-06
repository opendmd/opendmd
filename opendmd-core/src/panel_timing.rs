//! Panel timing: the HUB75 driver's knobs as settings, and the trial that
//! keeps a bad value from sticking.
//!
//! The firmware drives the panel through mrcodetastic's
//! ESP32-HUB75-MatrixPanel-DMA. Five of its options matter for a fussy
//! panel (a marginal driver chip or cable shows wrong colours on some
//! pictures only): the clock edge data is sampled on, the OE blanking
//! around LAT, the driver chip (some need an init sequence), the data clock
//! and the minimum refresh rate. [`PanelTiming`] holds them, with the
//! values the firmware has always used as the defaults.
//!
//! Two of them change live without touching the DMA buffers:
//! [`PanelTiming::clk_phase`] is the clock pin's inversion in the GPIO
//! matrix and [`PanelTiming::latch_blanking`] only re-renders the OE bits.
//! The other three need the driver re-initialised (a driver's init
//! sequence, the I2S clock, the DMA descriptor chain), which the firmware
//! does only at boot, on a whole heap ([`PanelTiming::same_boot_values`]).
//!
//! [`Tuner`] is the state machine behind `PUT /api/config`'s `panel` keys:
//! a change is a *trial* that reverts after [`TRIAL`] unless confirmed,
//! like a PC's display-mode dialog. A live trial lives in RAM only; a trial
//! that needs a restart is staged in NVS for exactly one boot (the boot
//! removes it), so whatever goes wrong — a dark panel, a crash, a power
//! cut — the boot after is on the saved settings. Pure logic: the firmware
//! executes the [`Effects`] it returns, the tests drive it with uptimes.

use std::time::Duration;

/// How long a trial runs before it reverts. A boot trial counts from the
/// later of boot and the network coming up (the confirmation has to come
/// over it).
pub const TRIAL: Duration = Duration::from_secs(60);

/// OE blanking clocks either side of LAT: the library's range
/// (`MAX_LAT_BLANKING`; 0 would underflow its loop).
pub const LATCH_MIN: u8 = 1;
pub const LATCH_MAX: u8 = 4;
/// The I2S data clocks the library offers on the ESP32, MHz (its `HZ_10M`
/// is 8 MHz too).
pub const I2S_SPEEDS: [u8; 3] = [8, 16, 20];
/// `min_refresh` range, Hz.
pub const REFRESH_MIN: u8 = 30;
pub const REFRESH_MAX: u8 = 240;

/// The panel's shift-register chip (`HUB75_I2S_CFG::shift_driver`, in its
/// order: [`Driver::code`] is the library's value).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Driver {
    /// A plain shift register (ICN2037, …): no init sequence. The default.
    #[default]
    Shiftreg,
    Fm6124,
    /// The common chip that needs an init sequence: no init shows as a
    /// dark panel, not as corruption.
    Fm6126a,
    Icn2038s,
    /// Always clocked on the rising edge (the library forces it).
    Mbi5124,
    /// Always clocked on the rising edge, a 3-clock latch.
    Dp3246,
}

impl Driver {
    pub const ALL: [Driver; 6] = [
        Driver::Shiftreg,
        Driver::Fm6124,
        Driver::Fm6126a,
        Driver::Icn2038s,
        Driver::Mbi5124,
        Driver::Dp3246,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Driver::Shiftreg => "shiftreg",
            Driver::Fm6124 => "fm6124",
            Driver::Fm6126a => "fm6126a",
            Driver::Icn2038s => "icn2038s",
            Driver::Mbi5124 => "mbi5124",
            Driver::Dp3246 => "dp3246",
        }
    }

    /// Case-insensitive.
    pub fn from_name(s: &str) -> Option<Driver> {
        Driver::ALL
            .into_iter()
            .find(|d| d.name().eq_ignore_ascii_case(s.trim()))
    }

    /// The library's `shift_driver` value (and the shim's
    /// `HUB75_DRIVER_*`).
    pub fn code(self) -> i32 {
        self as i32
    }

    /// The library clocks this chip on the rising edge whatever is asked.
    pub fn forces_rising_edge(self) -> bool {
        matches!(self, Driver::Mbi5124 | Driver::Dp3246)
    }
}

/// The five knobs. `Default` is what the firmware has always run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelTiming {
    /// true: data clocked on the rising edge (the library's default since
    /// 2021); false: the falling edge.
    pub clk_phase: bool,
    /// OE blanking clocks either side of LAT, [`LATCH_MIN`]..=[`LATCH_MAX`].
    pub latch_blanking: u8,
    pub driver: Driver,
    /// The I2S data clock, MHz: one of [`I2S_SPEEDS`].
    pub i2s_speed: u8,
    /// The minimum refresh rate the library trades colour-depth timing
    /// for, Hz.
    pub min_refresh: u8,
}

impl Default for PanelTiming {
    fn default() -> Self {
        PanelTiming {
            clk_phase: true,
            latch_blanking: 1,
            driver: Driver::Shiftreg,
            i2s_speed: 8,
            min_refresh: 60,
        }
    }
}

impl PanelTiming {
    /// Whether the values only a boot can apply (driver, data clock,
    /// minimum refresh) are the same: if so, `other` can be reached from
    /// `self` live.
    pub fn same_boot_values(&self, other: &PanelTiming) -> bool {
        self.driver == other.driver
            && self.i2s_speed == other.i2s_speed
            && self.min_refresh == other.min_refresh
    }

    /// `self`'s boot values with `live`'s clock edge and blanking.
    pub fn with_live(&self, live: &PanelTiming) -> PanelTiming {
        PanelTiming {
            clk_phase: live.clk_phase,
            latch_blanking: live.latch_blanking,
            ..*self
        }
    }

    /// What the library actually runs: the chips that force the rising
    /// edge get it.
    pub fn effective(&self) -> PanelTiming {
        PanelTiming {
            clk_phase: self.clk_phase || self.driver.forces_rising_edge(),
            ..*self
        }
    }

    fn valid(&self) -> bool {
        (LATCH_MIN..=LATCH_MAX).contains(&self.latch_blanking)
            && I2S_SPEEDS.contains(&self.i2s_speed)
            && (REFRESH_MIN..=REFRESH_MAX).contains(&self.min_refresh)
    }

    /// The NVS form: `clk,latch,driver,mhz,hz`, e.g. `1,1,shiftreg,8,60`.
    pub fn to_nvs(&self) -> String {
        format!(
            "{},{},{},{},{}",
            u8::from(self.clk_phase),
            self.latch_blanking,
            self.driver.name(),
            self.i2s_speed,
            self.min_refresh
        )
    }

    /// [`to_nvs`](Self::to_nvs) read back; anything malformed or out of
    /// range is `None` (the caller uses the defaults).
    pub fn from_nvs(s: &str) -> Option<PanelTiming> {
        let mut it = s.trim().split(',');
        let clk_phase = match it.next()? {
            "1" => true,
            "0" => false,
            _ => return None,
        };
        let t = PanelTiming {
            clk_phase,
            latch_blanking: it.next()?.parse().ok()?,
            driver: Driver::from_name(it.next()?)?,
            i2s_speed: it.next()?.parse().ok()?,
            min_refresh: it.next()?.parse().ok()?,
        };
        (it.next().is_none() && t.valid()).then_some(t)
    }

    /// `{"clk_phase":true,"latch_blanking":1,"driver":"shiftreg","i2s_speed":8,"min_refresh":60}`
    pub fn json(&self) -> String {
        format!(
            r#"{{"clk_phase":{},"latch_blanking":{},"driver":"{}","i2s_speed":{},"min_refresh":{}}}"#,
            self.clk_phase,
            self.latch_blanking,
            self.driver.name(),
            self.i2s_speed,
            self.min_refresh
        )
    }
}

/// A validated `panel` object from `PUT /api/config`: what it changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PanelPatch {
    pub clk_phase: Option<bool>,
    pub latch_blanking: Option<u8>,
    pub driver: Option<Driver>,
    pub i2s_speed: Option<u8>,
    pub min_refresh: Option<u8>,
}

impl PanelPatch {
    /// Check the fields as sent (numbers wide, so an out-of-range value
    /// gets the range message rather than a JSON error).
    pub fn new(
        clk_phase: Option<bool>,
        latch_blanking: Option<i64>,
        driver: Option<&str>,
        i2s_speed: Option<i64>,
        min_refresh: Option<i64>,
    ) -> Result<PanelPatch, String> {
        let in_range =
            |v: i64, lo: u8, hi: u8| u8::try_from(v).ok().filter(|v| (lo..=hi).contains(v));
        let latch_blanking = match latch_blanking {
            None => None,
            Some(v) => Some(in_range(v, LATCH_MIN, LATCH_MAX).ok_or_else(|| {
                format!("panel.latch_blanking must be {LATCH_MIN}..={LATCH_MAX}, not {v}")
            })?),
        };
        let driver = match driver {
            None => None,
            Some(s) => Some(Driver::from_name(s).ok_or_else(|| {
                format!(
                    "panel.driver must be {}, not {s:?}",
                    Driver::ALL.map(Driver::name).join(", ")
                )
            })?),
        };
        let i2s_speed = match i2s_speed {
            None => None,
            Some(v) => Some(
                u8::try_from(v)
                    .ok()
                    .filter(|v| I2S_SPEEDS.contains(v))
                    .ok_or_else(|| format!("panel.i2s_speed must be 8, 16 or 20 (MHz), not {v}"))?,
            ),
        };
        let min_refresh = match min_refresh {
            None => None,
            Some(v) => Some(in_range(v, REFRESH_MIN, REFRESH_MAX).ok_or_else(|| {
                format!("panel.min_refresh must be {REFRESH_MIN}..={REFRESH_MAX} (Hz), not {v}")
            })?),
        };
        Ok(PanelPatch {
            clk_phase,
            latch_blanking,
            driver,
            i2s_speed,
            min_refresh,
        })
    }

    pub fn apply(&self, base: PanelTiming) -> PanelTiming {
        PanelTiming {
            clk_phase: self.clk_phase.unwrap_or(base.clk_phase),
            latch_blanking: self.latch_blanking.unwrap_or(base.latch_blanking),
            driver: self.driver.unwrap_or(base.driver),
            i2s_speed: self.i2s_speed.unwrap_or(base.i2s_speed),
            min_refresh: self.min_refresh.unwrap_or(base.min_refresh),
        }
    }
}

/// What a `PUT /api/config` asks of the panel settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelOp {
    /// `panel: {...}`: try these (on top of the trial in progress, else
    /// the saved settings).
    Try(PanelPatch),
    /// `panel: {...}` with `panel_confirm: true` (the same values a try
    /// would run), or `panel: null` (`None`: the defaults): saved at once,
    /// no trial.
    Save(Option<PanelPatch>),
    /// `panel_confirm: true`: keep the trial running.
    Confirm,
    /// `panel_confirm: false`: undo now — the saved settings, and no
    /// staged trial.
    Revert,
}

/// What the firmware has to do after a [`Tuner`] call. Several can be set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// Set the clock edge and the blanking to these (the other values are
    /// what is running already).
    pub apply_live: Option<PanelTiming>,
    /// Write these as the saved settings (NVS `panel`).
    pub save: Option<PanelTiming>,
    /// Write (`Some(Some)`) or remove (`Some(None)`) the trial staged for
    /// the next boot (NVS `panel_try`).
    pub stage: Option<Option<PanelTiming>>,
    /// Restart (after the reply): a boot trial is being undone.
    pub restart: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrialKind {
    /// Applied live; undone live.
    Live,
    /// This boot runs a staged trial: undone by restarting on the saved
    /// settings.
    Boot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trial {
    pub kind: TrialKind,
    /// Uptime at which it reverts.
    pub until: Duration,
    /// A boot trial's deadline has been moved to network-up + [`TRIAL`].
    net_seen: bool,
}

/// The panel settings' state on the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tuner {
    /// What NVS holds (`panel`).
    pub saved: PanelTiming,
    /// What the driver runs: the boot values it was started with, the live
    /// values as last applied.
    pub running: PanelTiming,
    /// A trial in progress: `running` is not saved.
    pub trial: Option<Trial>,
    /// A trial waiting for the next boot (NVS `panel_try`).
    pub staged: Option<PanelTiming>,
}

impl Tuner {
    /// At boot, from NVS: the saved settings and a staged trial. A valid
    /// staged trial is run on trial (deadline [`TRIAL`] of uptime, moved
    /// later when the network comes up); either way the staged trial is
    /// consumed — the caller removes `panel_try` from NVS before the panel
    /// starts, so it runs for this one boot only.
    pub fn boot(saved: Option<&str>, staged: Option<&str>) -> Tuner {
        let saved = saved.and_then(PanelTiming::from_nvs).unwrap_or_default();
        match staged.and_then(PanelTiming::from_nvs) {
            Some(t) => Tuner {
                saved,
                running: t,
                trial: Some(Trial {
                    kind: TrialKind::Boot,
                    until: TRIAL,
                    net_seen: false,
                }),
                staged: None,
            },
            None => Tuner {
                saved,
                running: saved,
                trial: None,
                staged: None,
            },
        }
    }

    /// What a restart would run, when that differs from what runs now in
    /// the values only a boot applies: a staged trial (on trial), else saved
    /// boot values the driver is not running (a boot trial, or a boot value
    /// saved outright). `/api/config` `panel_next`.
    pub fn next_boot(&self) -> Option<PanelTiming> {
        self.staged
            .or_else(|| (!self.saved.same_boot_values(&self.running)).then_some(self.saved))
    }

    /// A panel setting is waiting for a restart ([`next_boot`](Self::next_boot)).
    pub fn restart_needed(&self) -> bool {
        self.next_boot().is_some()
    }

    /// Whole seconds left in the trial, rounded up.
    pub fn trial_left(&self, now: Duration) -> Option<u32> {
        self.trial
            .map(|t| t.until.saturating_sub(now).as_secs_f32().ceil() as u32)
    }

    /// Carry out `op` at uptime `now`. `Err` only for a confirm with no
    /// trial running (it may have just reverted); nothing changed then.
    pub fn apply(&mut self, op: PanelOp, now: Duration) -> Result<Effects, &'static str> {
        let mut fx = Effects::default();
        match op {
            PanelOp::Try(patch) => {
                let target = patch.apply(self.base());
                if target.same_boot_values(&self.running) {
                    // Live: on top of a boot trial it stays a boot trial
                    // (undoing it needs the restart either way).
                    let kind = self.trial.map_or(TrialKind::Live, |t| t.kind);
                    self.running = target;
                    self.trial = Some(Trial {
                        kind,
                        until: now + TRIAL,
                        net_seen: true,
                    });
                    fx.apply_live = Some(target);
                    if self.staged.take().is_some() {
                        fx.stage = Some(None);
                    }
                } else {
                    self.staged = Some(target);
                    fx.stage = Some(Some(target));
                }
            }
            PanelOp::Save(patch) => {
                let target = patch.map_or_else(PanelTiming::default, |p| p.apply(self.base()));
                self.saved = target;
                self.trial = None;
                fx.save = Some(target);
                if self.staged.take().is_some() {
                    fx.stage = Some(None);
                }
                // The live values now; the boot values at the next boot
                // (`restart_needed`) if they differ from what runs.
                let live = self.running.with_live(&target);
                if live != self.running {
                    self.running = live;
                    fx.apply_live = Some(live);
                }
            }
            PanelOp::Confirm => {
                if self.trial.take().is_none() {
                    return Err(NO_TRIAL);
                }
                self.saved = self.running;
                fx.save = Some(self.running);
            }
            PanelOp::Revert => fx = self.revert(),
        }
        Ok(fx)
    }

    /// What a change builds on: the trial in progress, else the staged
    /// trial, else the saved settings.
    fn base(&self) -> PanelTiming {
        if self.trial.is_some() {
            self.running
        } else {
            self.staged.unwrap_or(self.saved)
        }
    }

    /// Back to the saved settings: a live trial undone live, a boot trial
    /// by a restart; a staged trial dropped.
    fn revert(&mut self) -> Effects {
        let mut fx = Effects::default();
        if self.staged.take().is_some() {
            fx.stage = Some(None);
        }
        match self.trial.take().map(|t| t.kind) {
            Some(TrialKind::Boot) => fx.restart = true,
            Some(TrialKind::Live) => {
                let live = self.running.with_live(&self.saved);
                if live != self.running {
                    self.running = live;
                    fx.apply_live = Some(live);
                }
            }
            None => {}
        }
        fx
    }

    /// Every main-loop iteration: moves a boot trial's deadline to
    /// network-up + [`TRIAL`] once, and reverts a trial whose time is up
    /// (`Some` then).
    pub fn poll(&mut self, now: Duration, net_up: bool) -> Option<Effects> {
        let t = self.trial.as_mut()?;
        if !t.net_seen && net_up {
            t.net_seen = true;
            t.until = t.until.max(now + TRIAL);
        }
        (now >= t.until).then(|| self.revert())
    }
}

/// [`Tuner::apply`]'s error, and the API's 409 message.
pub const NO_TRIAL: &str = "no panel trial is running (it may have ended)";

#[cfg(test)]
mod tests {
    /// The vectors the updater's own parser (`ota/updater/main/panel_timing.c`,
    /// which reads this setting from NVS to drive the panel while it
    /// installs) is held to as well, by `make test`.
    #[test]
    fn the_updaters_vectors_parse_the_same() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../ota/updater/test/panel_timing_vectors.txt");
        let text = std::fs::read_to_string(path).expect("the updater's vectors");
        let mut n = 0;
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let Some((input, want)) = line.rsplit_once('|') else {
                continue;
            };
            let got = PanelTiming::from_nvs(input).map(|t| t.to_nvs());
            assert_eq!(got.as_deref().unwrap_or("-"), want, "{input:?}");
            n += 1;
        }
        assert!(n > 20, "{n} vectors");
    }

    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn patch(clk: Option<bool>, latch: Option<i64>, driver: Option<&str>) -> PanelPatch {
        PanelPatch::new(clk, latch, driver, None, None).unwrap()
    }

    #[test]
    fn defaults_are_the_old_constants() {
        let d = PanelTiming::default();
        assert!(d.clk_phase);
        assert_eq!(d.latch_blanking, 1);
        assert_eq!(d.driver, Driver::Shiftreg);
        assert_eq!(d.driver.code(), 0);
        assert_eq!(d.i2s_speed, 8);
        assert_eq!(d.min_refresh, 60);
        assert_eq!(
            d.json(),
            r#"{"clk_phase":true,"latch_blanking":1,"driver":"shiftreg","i2s_speed":8,"min_refresh":60}"#
        );
    }

    #[test]
    fn driver_names_and_codes() {
        for (i, d) in Driver::ALL.into_iter().enumerate() {
            assert_eq!(d.code(), i as i32);
            assert_eq!(Driver::from_name(d.name()), Some(d));
        }
        assert_eq!(Driver::from_name("FM6126A"), Some(Driver::Fm6126a));
        assert_eq!(Driver::from_name("ws2812"), None);
    }

    #[test]
    fn nvs_round_trip_and_rejects() {
        let t = PanelTiming {
            clk_phase: false,
            latch_blanking: 3,
            driver: Driver::Fm6126a,
            i2s_speed: 16,
            min_refresh: 120,
        };
        assert_eq!(t.to_nvs(), "0,3,fm6126a,16,120");
        assert_eq!(PanelTiming::from_nvs(&t.to_nvs()), Some(t));
        assert_eq!(
            PanelTiming::from_nvs("1,1,shiftreg,8,60"),
            Some(PanelTiming::default())
        );
        for bad in [
            "",
            "1,1,shiftreg,8",
            "1,1,shiftreg,8,60,1",
            "2,1,shiftreg,8,60",
            "1,0,shiftreg,8,60",
            "1,5,shiftreg,8,60",
            "1,1,nope,8,60",
            "1,1,shiftreg,10,60",
            "1,1,shiftreg,8,29",
            "1,1,shiftreg,8,241",
        ] {
            assert_eq!(PanelTiming::from_nvs(bad), None, "{bad}");
        }
    }

    #[test]
    fn patch_validation() {
        assert_eq!(
            PanelPatch::new(None, None, None, None, None),
            Ok(PanelPatch::default())
        );
        let p =
            PanelPatch::new(Some(false), Some(4), Some("icn2038s"), Some(20), Some(30)).unwrap();
        let t = p.apply(PanelTiming::default());
        assert_eq!(t.to_nvs(), "0,4,icn2038s,20,30");
        assert!(PanelPatch::new(None, Some(0), None, None, None)
            .unwrap_err()
            .contains("latch_blanking must be 1..=4"));
        assert!(PanelPatch::new(None, Some(-1), None, None, None).is_err());
        assert!(PanelPatch::new(None, Some(5), None, None, None).is_err());
        assert!(PanelPatch::new(None, None, Some("x"), None, None)
            .unwrap_err()
            .contains("shiftreg, fm6124, fm6126a, icn2038s, mbi5124, dp3246"));
        assert!(PanelPatch::new(None, None, None, Some(10), None)
            .unwrap_err()
            .contains("8, 16 or 20"));
        assert!(PanelPatch::new(None, None, None, Some(264), None).is_err());
        assert!(PanelPatch::new(None, None, None, None, Some(29)).is_err());
        assert!(PanelPatch::new(None, None, None, None, Some(241)).is_err());
        assert!(PanelPatch::new(None, None, None, None, Some(300)).is_err());
    }

    #[test]
    fn effective_forces_the_rising_edge() {
        let mut t = PanelTiming {
            clk_phase: false,
            ..Default::default()
        };
        assert!(!t.effective().clk_phase);
        t.driver = Driver::Mbi5124;
        assert!(t.effective().clk_phase);
        t.driver = Driver::Dp3246;
        assert!(t.effective().clk_phase);
    }

    #[test]
    fn boot_without_a_trial_runs_the_saved_settings() {
        let t = Tuner::boot(None, None);
        assert_eq!(t.running, PanelTiming::default());
        assert!(t.trial.is_none() && !t.restart_needed());
        let t = Tuner::boot(Some("0,2,shiftreg,8,60"), Some("garbage"));
        assert!(!t.running.clk_phase);
        assert_eq!(t.running.latch_blanking, 2);
        assert!(t.trial.is_none());
        // A damaged saved value is the defaults.
        assert_eq!(Tuner::boot(Some("x"), None).saved, PanelTiming::default());
    }

    #[test]
    fn live_trial_reverts_unless_confirmed() {
        let mut t = Tuner::boot(None, None);
        let fx = t
            .apply(PanelOp::Try(patch(Some(false), None, None)), S(100))
            .unwrap();
        let tried = PanelTiming {
            clk_phase: false,
            ..Default::default()
        };
        assert_eq!(fx.apply_live, Some(tried));
        assert_eq!((fx.save, fx.stage, fx.restart), (None, None, false));
        assert_eq!(t.trial_left(S(100)), Some(60));
        assert_eq!(t.poll(S(159), true), None);
        let fx = t.poll(S(160), true).unwrap();
        assert_eq!(fx.apply_live, Some(PanelTiming::default()));
        assert!(!fx.restart);
        assert_eq!(t.running, PanelTiming::default());
        assert!(t.trial.is_none());
        // Confirm with nothing running: refused, nothing changes.
        assert_eq!(t.apply(PanelOp::Confirm, S(161)), Err(NO_TRIAL));

        // Tried, then a second change on top, then kept.
        t.apply(PanelOp::Try(patch(Some(false), None, None)), S(200))
            .unwrap();
        t.apply(PanelOp::Try(patch(None, Some(3), None)), S(230))
            .unwrap();
        assert_eq!(t.running.to_nvs(), "0,3,shiftreg,8,60");
        assert_eq!(t.trial_left(S(230)), Some(60));
        let fx = t.apply(PanelOp::Confirm, S(240)).unwrap();
        assert_eq!(fx.save, Some(t.running));
        assert_eq!(t.saved, t.running);
        assert_eq!(t.poll(S(400), true), None);
    }

    #[test]
    fn undo_goes_back_to_the_saved_settings() {
        let mut t = Tuner::boot(Some("0,2,shiftreg,8,60"), None);
        t.apply(PanelOp::Try(patch(Some(true), Some(4), None)), S(10))
            .unwrap();
        let fx = t.apply(PanelOp::Revert, S(20)).unwrap();
        assert_eq!(fx.apply_live.unwrap().to_nvs(), "0,2,shiftreg,8,60");
        assert_eq!(t.running, t.saved);
        // Undo with nothing to undo: no effects.
        assert_eq!(t.apply(PanelOp::Revert, S(21)).unwrap(), Effects::default());
    }

    #[test]
    fn a_boot_value_is_staged_for_one_boot() {
        let mut t = Tuner::boot(None, None);
        let fx = t
            .apply(PanelOp::Try(patch(None, None, Some("fm6126a"))), S(50))
            .unwrap();
        let staged = PanelTiming {
            driver: Driver::Fm6126a,
            ..Default::default()
        };
        assert_eq!(fx.stage, Some(Some(staged)));
        assert_eq!(t.next_boot(), Some(staged));
        assert_eq!(fx.apply_live, None);
        assert!(t.trial.is_none() && t.restart_needed());
        // A further change builds on the staged one.
        let fx = t
            .apply(PanelOp::Try(patch(None, Some(2), None)), S(55))
            .unwrap();
        assert_eq!(fx.stage.unwrap().unwrap().to_nvs(), "1,2,fm6126a,8,60");

        // The next boot runs it on trial; the network extends the deadline.
        let mut b = Tuner::boot(None, Some("1,2,fm6126a,8,60"));
        assert_eq!(b.running.driver, Driver::Fm6126a);
        assert_eq!(b.trial.unwrap().kind, TrialKind::Boot);
        assert!(b.restart_needed()); // until kept: saved is shiftreg
        assert_eq!(b.next_boot(), Some(PanelTiming::default()));
        assert_eq!(b.poll(S(10), false), None);
        assert_eq!(b.poll(S(12), true), None); // now until 72 s
        assert_eq!(b.trial_left(S(12)), Some(60));
        assert_eq!(b.poll(S(71), true), None);
        let fx = b.poll(S(72), true).unwrap();
        assert!(fx.restart);
        assert_eq!(fx.save, None);

        // Kept instead: saved, no restart.
        let mut b = Tuner::boot(None, Some("1,2,fm6126a,8,60"));
        let fx = b.apply(PanelOp::Confirm, S(20)).unwrap();
        assert_eq!(fx.save.unwrap().driver, Driver::Fm6126a);
        assert!(!b.restart_needed() && !fx.restart);

        // Undone by hand: a restart.
        let mut b = Tuner::boot(None, Some("1,2,fm6126a,8,60"));
        assert!(b.apply(PanelOp::Revert, S(20)).unwrap().restart);
    }

    #[test]
    fn a_network_that_never_comes_still_reverts() {
        let mut b = Tuner::boot(None, Some("1,1,fm6124,8,60"));
        assert_eq!(b.poll(S(59), false), None);
        assert!(b.poll(S(60), false).unwrap().restart);
    }

    #[test]
    fn live_change_during_a_boot_trial_stays_a_boot_trial() {
        let mut b = Tuner::boot(None, Some("1,1,fm6124,8,60"));
        let fx = b
            .apply(PanelOp::Try(patch(Some(false), None, None)), S(30))
            .unwrap();
        assert_eq!(fx.apply_live.unwrap().to_nvs(), "0,1,fm6124,8,60");
        assert_eq!(b.trial.unwrap().kind, TrialKind::Boot);
        assert!(b.poll(S(90), true).unwrap().restart);
    }

    #[test]
    fn a_live_trial_drops_a_staged_one() {
        let mut t = Tuner::boot(None, None);
        t.apply(PanelOp::Try(patch(None, None, Some("fm6124"))), S(1))
            .unwrap();
        // Back to the running driver: live, and nothing waits for a boot.
        let fx = t
            .apply(
                PanelOp::Try(patch(Some(false), None, Some("shiftreg"))),
                S(2),
            )
            .unwrap();
        assert_eq!(fx.stage, Some(None));
        assert!(fx.apply_live.is_some());
        assert!(!t.restart_needed());
        assert_eq!(t.next_boot(), None);
    }

    #[test]
    fn keep_in_one_request_builds_on_the_trial() {
        let mut t = Tuner::boot(None, None);
        t.apply(PanelOp::Try(patch(Some(false), None, None)), S(1))
            .unwrap();
        let fx = t
            .apply(PanelOp::Save(Some(patch(None, Some(2), None))), S(2))
            .unwrap();
        assert_eq!(fx.save.unwrap().to_nvs(), "0,2,shiftreg,8,60");
        assert_eq!(fx.apply_live.unwrap().to_nvs(), "0,2,shiftreg,8,60");
        assert!(t.trial.is_none() && !t.restart_needed());
    }

    #[test]
    fn save_and_reset_to_defaults() {
        let mut t = Tuner::boot(Some("0,3,shiftreg,8,60"), None);
        t.apply(PanelOp::Try(patch(None, None, Some("fm6126a"))), S(1))
            .unwrap();
        let fx = t.apply(PanelOp::Save(None), S(2)).unwrap();
        assert_eq!(fx.save, Some(PanelTiming::default()));
        assert_eq!(fx.stage, Some(None));
        assert_eq!(fx.apply_live, Some(PanelTiming::default()));
        assert!(!t.restart_needed());

        // Saving a boot value: saved now, applied at the next boot.
        let fx = t
            .apply(
                PanelOp::Save(Some(
                    PanelPatch::new(None, None, None, None, Some(120)).unwrap(),
                )),
                S(3),
            )
            .unwrap();
        assert_eq!(fx.apply_live, None);
        assert!(t.restart_needed());
        assert_eq!(t.running.min_refresh, 60);

        // Reset during a boot trial whose boot values are the defaults:
        // nothing left to restart for.
        let mut b = Tuner::boot(Some("1,1,fm6126a,8,60"), Some("0,1,shiftreg,8,60"));
        let fx = b.apply(PanelOp::Save(None), S(5)).unwrap();
        assert!(b.trial.is_none() && !b.restart_needed() && !fx.restart);
        assert_eq!(fx.apply_live, Some(PanelTiming::default()));
        assert_eq!(b.poll(S(500), true), None);
    }
}
