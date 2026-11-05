//! The Formula look: a Psi formula drawn over whatever the player shows
//! (`opendmd_core::fx_psi::FormulaLook`), from two places —
//!
//! - the **Look setting's formula** (`look_formula` in `/api/config`, kept
//!   in NVS): over every GIF, transition and title card, until it is cleared;
//! - **Show on panel, over the GIF** from the Formulas editor (`POST
//!   /api/play` with `"over":true`): over the file on screen until the
//!   playlist moves on to another (the same lifetime as Show on panel),
//!   in place of the setting's meanwhile, followed by live edits.
//!
//! Where it goes in the frame: decode (or a transition's step) → **the
//! formula** → `Panel::blit` (the tint at the push, then the glow). Before
//! the tint because a formula wants the GIF's real colours (`ur ug ub`,
//! colour grades) and the tint is the panel's own colour filter; before the
//! glow because the glow models the LEDs fading from whatever was shown.
//! After the transition, so a look stays put while one file dissolves into
//! the next.
//!
//! **Heap.** The program (2.8 KB) is held while a formula is set; its
//! buffers (the VM's 8 KB and its 12 KB picture) only while the main loop
//! sees `TAKE_HEAP` free when it draws (asked again a second after a
//! refusal, at each file, and between frames once the heap is back), and
//! they are given back under [`config::GLOW_YIELD_HEAP`] (after the glow's
//! copy; never during a transition: the transition gate counts the look
//! instead, `config::LOOK_TRANSITION_MARGIN`, and cuts the dissolve), in
//! night mode's dark, for the panel test, while paused (the panel keeps
//! the last picture; taken again on resume) and while the editor's formula
//! replaces the setting's. Without them the picture goes up as it is.
//!
//! **The glow** fades towards what the panel shows: [`Looks::shown`] (the
//! look's picture while it is held and drawn) is what every glow call gets.
//!
//! Built without the `psi` feature this is a stub that passes the picture
//! through.

use crate::config;
use crate::state::PlayError;
use opendmd_core::canvas::Canvas;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// The buffers are taken only with this much free: theirs (20.5 KB) and
/// the glow's yield level's worth (20 KB) and a margin, so a transition
/// (24 KB) still usually fits beside them.
#[cfg(feature = "psi")]
pub const TAKE_HEAP: usize = 48 * 1024;

/// What is drawn over the picture (`/api/status`, the still gate): 0
/// nothing, 1 the setting's formula, 2 the editor's (over the GIF). A
/// saved formula that does not build (an `@fx:` value from before no-fx,
/// or one a later build refuses) is 0: nothing is drawn, so nothing holds
/// the heap or keeps a Formula from its still cache (consolea-a: such a
/// value on the spare cost Time gate its cache, 5.1 → 14.4 ms); its
/// reason is [`ERROR`].
pub static SOURCE: AtomicU8 = AtomicU8::new(0);

/// The look's buffers are held (`main::transition_gate` counts them).
pub static HELD: AtomicBool = AtomicBool::new(false);

/// Why the setting's formula is not drawn (it no longer compiles, e.g.
/// after a firmware change): `/api/status` `formula_look.error`.
pub static ERROR: Mutex<String> = Mutex::new(String::new());

#[cfg(feature = "psi")]
pub use with_psi::Looks;
#[cfg(not(feature = "psi"))]
pub use without::Looks;

/// The `look_formula` value checked before it is saved: `""` (none) or a
/// formula's path that compiles, lets the GIF show through and fits over
/// GIFs. The error is the compiler's sentence.
pub fn check(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Ok(());
    }
    // (Held only while it is on the panel: a setting would outlive it.)
    if opendmd_core::synth::is_sent(path) {
        return Err("save the Formula first".into());
    }
    #[cfg(feature = "psi")]
    {
        build(path).map(drop).map_err(|e| e.msg)
    }
    #[cfg(not(feature = "psi"))]
    Err(crate::state::NO_FORMULAS.to_string())
}

/// The look a `look_formula` value names: inline (`@formula?psb=…`), a
/// bundled Formula by name (`@formula:<name>`) or the user's on the card
/// (`/formulas/<stem>.psi`, read here, on the main task, like any file the
/// player opens).
#[cfg(feature = "psi")]
pub fn build(path: &str) -> Result<opendmd_core::fx_psi::FormulaLook, opendmd_core::psi::Error> {
    use opendmd_core::fx_psi::FormulaLook;
    match opendmd_core::synth::formula_file(path) {
        Some((file, opts)) => FormulaLook::from_bytes(&crate::formulas::read_card(file)?, opts),
        None => FormulaLook::from_path(path),
    }
}

/// Whether a `look_formula` value is read from the card (built again once
/// the card is mounted: at boot the look is set before it is).
pub fn on_card(path: &str) -> bool {
    opendmd_core::synth::formula_file(path).is_some()
}

#[cfg(feature = "psi")]
mod with_psi {
    use super::*;
    use crate::state::free_heap;
    use opendmd_core::fx_psi::FormulaLook;
    use std::time::Instant;

    /// A refused take is asked again no sooner than this.
    const RETRY: Duration = Duration::from_secs(1);
    /// After a yield to low heap, the buffers are taken again only once
    /// the heap has stayed at `TAKE_HEAP` for this long (the spare, 3
    /// downloads at once: free swings above 48 KB between requests, and a
    /// 1 s retry took and gave back the buffers in a loop).
    const CALM: Duration = Duration::from_secs(10);

    #[derive(Default)]
    pub struct Looks {
        /// The setting's.
        saved: Option<FormulaLook>,
        /// The editor's, and the ordinal of the file it was shown over.
        over: Option<(FormulaLook, u64)>,
        /// When the buffers were last refused (the heap short).
        refused: Option<Instant>,
        /// When they were last given back to low heap, and since when the
        /// heap has been back at `TAKE_HEAP` (a yield's calm).
        yielded: Option<Instant>,
        calm_since: std::cell::Cell<Option<Instant>>,
    }

    impl Looks {
        pub fn new() -> Looks {
            Looks::default()
        }

        fn publish(&self) {
            let s = if self.over.is_some() {
                2
            } else if self.saved.is_some() {
                1
            } else {
                0
            };
            SOURCE.store(s, Ordering::Relaxed);
            HELD.store(self.active_ref().is_some_and(FormulaLook::is_held), Ordering::Relaxed);
        }

        fn active(&mut self) -> Option<&mut FormulaLook> {
            match &mut self.over {
                Some((l, _)) => Some(l),
                None => self.saved.as_mut(),
            }
        }

        fn active_ref(&self) -> Option<&FormulaLook> {
            match &self.over {
                Some((l, _)) => Some(l),
                None => self.saved.as_ref(),
            }
        }

        /// The buffers may be asked for now: not held, the heap has room,
        /// and no refusal in the last second.
        fn may_take(&self) -> bool {
            if !self.active_ref().is_some_and(|l| !l.is_held())
                || self.refused.is_some_and(|t| t.elapsed() < RETRY)
            {
                return false;
            }
            let room = free_heap() as usize >= TAKE_HEAP;
            if self.yielded.is_none() {
                return room;
            }
            // After a yield: the heap at TAKE_HEAP for CALM, unbroken.
            if !room {
                self.calm_since.set(None);
                return false;
            }
            let since = self.calm_since.get().unwrap_or_else(Instant::now);
            self.calm_since.set(Some(since));
            since.elapsed() >= CALM
        }

        /// The setting's formula (`""`: none). One that does not build
        /// (checked when saved, so a formula a later build refuses) is
        /// logged, left out, and its reason goes to the status.
        #[inline(never)]
        pub fn set_saved(&mut self, path: &str, display: &mut Canvas) {
            if let Some(mut l) = self.saved.take() {
                l.release(display);
            }
            let mut why = String::new();
            if !path.is_empty() {
                match super::build(path) {
                    Ok(l) => self.saved = Some(l),
                    Err(e) => {
                        log::warn!(
                            "formula look: the saved look does not build, none drawn ({}): {}",
                            e.msg,
                            crate::short(path)
                        );
                        why = e.msg;
                    }
                }
            }
            if let Ok(mut e) = ERROR.lock() {
                *e = why;
            }
            self.refused = None;
            self.yielded = None;
            display.dirty_mut().mark_all();
            self.publish();
        }

        /// The editor's formula over the file on screen (`ordinal`), or a
        /// live edit of it (`live`: [`PlayError::MovedOn`] once the
        /// playlist has moved on); `""` takes it off.
        #[inline(never)]
        pub fn show_over(
            &mut self,
            path: &str,
            live: bool,
            ordinal: Option<u64>,
            display: &mut Canvas,
        ) -> Result<(), PlayError> {
            if path.is_empty() {
                if let Some((mut l, _)) = self.over.take() {
                    l.release(display);
                }
                self.refused = None;
                self.yielded = None;
                self.publish();
                return Ok(());
            }
            let Some(ordinal) = ordinal else {
                return Err(PlayError::NoPlaylist);
            };
            let short = |e: &opendmd_core::psi::Error| e.msg == opendmd_core::psi::NO_MEMORY;
            // Retuned in place (`None`), or built anew: at once (`Some(false)`)
            // or after the live edit was short of memory (`Some(true)`).
            let fresh = match &mut self.over {
                Some((l, at)) if *at == ordinal => match l.retune(path) {
                    Err(e) if short(&e) => Some(true),
                    r => {
                        r.map_err(|e| PlayError::BadFormula(e.msg))?;
                        None
                    }
                },
                _ if live => return Err(PlayError::MovedOn),
                _ => Some(false),
            };
            if let Some(was_short) = fresh {
                // Short of memory beside the editor's formula on screen (a
                // live edit) or the one before: it goes, the setting's
                // buffers too, and the build is tried once more from there.
                // Refused again: no editor's formula (the setting's look,
                // if any, comes back) and the error.
                let first = if was_short { None } else { Some(super::build(path)) };
                let l = match first {
                    Some(Err(e))
                        if short(&e)
                            && (self.over.is_some() || self.saved.as_ref().is_some_and(FormulaLook::is_held)) =>
                    {
                        None
                    }
                    Some(r) => Some(r),
                    None => None,
                };
                let l = match l {
                    Some(r) => r,
                    None => {
                        log::info!("formula look: no room to compile beside the one drawn: letting it go, then again");
                        if let Some((mut old, _)) = self.over.take() {
                            old.release(display);
                        }
                        if let Some(s) = &mut self.saved {
                            s.release(display);
                        }
                        self.publish();
                        super::build(path)
                    }
                }
                .map_err(|e| PlayError::BadFormula(e.msg))?;
                if let Some((mut old, _)) = self.over.take() {
                    old.release(display);
                }
                // Its buffers in place of the setting's.
                if let Some(s) = &mut self.saved {
                    s.release(display);
                }
                self.over = Some((l, ordinal));
                self.refused = None;
                self.yielded = None;
            }
            self.publish();
            Ok(())
        }

        /// A file started (`ordinal`): the editor's formula goes once the
        /// playlist has moved on; the file's first frames do not steer the
        /// look's governor; buffers refused before are asked again.
        #[inline(never)]
        pub fn started(&mut self, ordinal: u64, display: &mut Canvas) {
            if self.over.as_ref().is_some_and(|(_, at)| *at != ordinal) {
                if let Some((mut l, _)) = self.over.take() {
                    l.release(display);
                }
                log::info!("formula look: the editor's formula is off (the playlist moved on)");
            }
            if let Some(l) = self.active() {
                l.new_file();
            }
            self.refused = None;
            self.publish();
        }

        /// What the panel shows over `display` at `now` (see
        /// `FormulaLook::apply`): its buffers taken first if the heap has
        /// room. `decode_us`: the frame's decoding (not the file's open).
        #[inline(never)]
        pub fn apply<'a>(
            &'a mut self,
            now: Duration,
            display: &'a mut Canvas,
            decode_us: u32,
        ) -> &'a mut Canvas {
            if self.may_take() {
                let took = self.active().is_some_and(FormulaLook::take);
                if took {
                    self.yielded = None;
                    self.calm_since.set(None);
                } else {
                    self.refused = Some(Instant::now());
                    log::info!(
                        "formula look: no room for its {} B ({} B free); the picture goes up as it is",
                        self.active_ref()
                            .map_or(FormulaLook::BUFFER_BYTES, FormulaLook::buffer_bytes),
                        free_heap()
                    );
                }
                self.publish();
            }
            match self.active() {
                Some(l) => l.apply(now, display, decode_us),
                None => display,
            }
        }

        /// What the panel shows over `display` now (for the glow): the
        /// look's picture while it is held and drawn, else `display`.
        pub fn shown<'a>(&'a self, display: &'a Canvas) -> &'a Canvas {
            self.active_ref().and_then(FormulaLook::shown).unwrap_or(display)
        }

        /// When the look must be drawn between the player's frames,
        /// whatever the picture does: its own frame; at once while its
        /// buffers can be taken (after a refusal or a yield, once the heap
        /// is back). Never while a transition runs (`transitioning`): its
        /// steps draw the look, its own frames included
        /// (`FormulaLook::due_between`).
        #[inline(never)]
        pub fn due(&self, transitioning: bool) -> Option<Duration> {
            if transitioning {
                return None;
            }
            if self.may_take() {
                return Some(Duration::ZERO);
            }
            self.active_ref().and_then(|l| l.due_between(transitioning))
        }

        /// Its mean render time, µs (0 with none).
        pub fn avg_us(&self) -> u32 {
            self.active_ref().map_or(0, FormulaLook::avg_us)
        }

        /// A look is set (the setting's or the editor's).
        pub fn is_on(&self) -> bool {
            self.active_ref().is_some()
        }

        /// Give the buffers back (night mode's dark panel, the panel
        /// test); taken again when it draws next.
        #[inline(never)]
        pub fn release(&mut self, display: &mut Canvas) {
            if let Some(l) = self.active().filter(|l| l.is_held()) {
                l.release(display);
            }
            self.publish();
        }

        /// Paused (and the glow done fading): the buffers go, the panel
        /// keeps the look's last picture (nothing is pushed while paused);
        /// the next frame takes them again and draws every row.
        #[inline(never)]
        pub fn let_go(&mut self) {
            if let Some(l) = self.active().filter(|l| l.is_held()) {
                l.let_go();
                self.publish();
            }
        }

        /// The panel showed something else (a status card): every row is
        /// drawn again next time.
        pub fn redraw(&mut self) {
            if let Some(l) = self.active() {
                l.redraw();
            }
        }

        /// The heap is low (and no transition runs: during one, the gate
        /// counts the look instead): give the buffers back. True if it did.
        #[inline(never)]
        pub fn yield_heap(&mut self, display: &mut Canvas, transitioning: bool) -> bool {
            if transitioning || free_heap() as usize >= config::GLOW_YIELD_HEAP {
                return false;
            }
            let gave = match self.active().filter(|l| l.is_held()) {
                Some(l) => {
                    l.release(display);
                    true
                }
                None => false,
            };
            if gave {
                self.yielded = Some(Instant::now());
                self.calm_since.set(None);
                self.publish();
            }
            gave
        }
    }
}

#[cfg(not(feature = "psi"))]
mod without {
    use super::*;

    #[derive(Default)]
    pub struct Looks;

    impl Looks {
        pub fn new() -> Looks {
            Looks
        }
        pub fn set_saved(&mut self, _: &str, _: &mut Canvas) {}
        pub fn show_over(
            &mut self,
            _: &str,
            _: bool,
            _: Option<u64>,
            _: &mut Canvas,
        ) -> Result<(), PlayError> {
            Err(PlayError::Unplayable(crate::state::NO_FORMULAS.into()))
        }
        pub fn started(&mut self, _: u64, _: &mut Canvas) {}
        pub fn apply<'a>(
            &'a mut self,
            _: Duration,
            display: &'a mut Canvas,
            _: u32,
        ) -> &'a mut Canvas {
            display
        }
        pub fn shown<'a>(&'a self, display: &'a Canvas) -> &'a Canvas {
            display
        }
        pub fn due(&self, _: bool) -> Option<Duration> {
            None
        }
        pub fn avg_us(&self) -> u32 {
            0
        }
        #[allow(dead_code)]
        pub fn is_on(&self) -> bool {
            false
        }
        pub fn release(&mut self, _: &mut Canvas) {}
        pub fn let_go(&mut self) {}
        pub fn redraw(&mut self) {}
        pub fn yield_heap(&mut self, _: &mut Canvas, _: bool) -> bool {
            let _ = (config::GLOW_YIELD_HEAP, &SOURCE, &HELD, &ERROR, Ordering::Relaxed);
            false
        }
    }
}
