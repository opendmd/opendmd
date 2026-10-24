//! The Formula look: a Psi formula drawn over whatever the player shows
//! (`opendmd_core::fx_psi::FormulaLook`), from two places —
//!
//! - the **Look setting's formula** (`look_formula` in `/api/config`, kept
//!   in NVS): over every GIF, transition and live item, until it is cleared;
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
//! **Heap.** The program (1.8 KB) is held while a formula is set; its
//! buffers (the VM's 8 KB and its 12 KB picture) only while the main loop
//! sees [`TAKE_HEAP`] free when it draws, and they are given back under
//! [`config::GLOW_YIELD_HEAP`] (after the glow's copy), in night mode's
//! dark, for the panel test, and while the editor's formula replaces the
//! setting's. Without them the picture goes up as it is.
//!
//! Built without the `psi` feature this is a stub that passes the picture
//! through.

use crate::config;
use crate::state::PlayError;
use opendmd_core::canvas::Canvas;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

/// The buffers are taken only with this much free: theirs (20.5 KB) and
/// the glow's yield level's worth (20 KB) and a margin, so a transition
/// (24 KB) still usually fits beside them.
#[cfg(feature = "psi")]
pub const TAKE_HEAP: usize = 48 * 1024;

/// What is drawn over the picture, for `/api/status`: 0 nothing, 1 the
/// setting's formula, 2 the editor's (over the GIF).
pub static SOURCE: AtomicU8 = AtomicU8::new(0);

#[cfg(feature = "psi")]
pub use with_psi::Looks;
#[cfg(not(feature = "psi"))]
pub use without::Looks;

/// The `look_formula` value checked before it is saved: `""` (none) or a
/// formula's path that compiles and fits over GIFs. The error is the
/// compiler's sentence.
pub fn check(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Ok(());
    }
    #[cfg(feature = "psi")]
    {
        opendmd_core::fx_psi::FormulaLook::from_path(path)
            .map(drop)
            .map_err(|e| e.msg)
    }
    #[cfg(not(feature = "psi"))]
    Err("this build cannot draw formulas".to_string())
}

#[cfg(feature = "psi")]
mod with_psi {
    use super::*;
    use crate::state::free_heap;
    use opendmd_core::fx_psi::FormulaLook;

    #[derive(Default)]
    pub struct Looks {
        /// The setting's.
        saved: Option<FormulaLook>,
        /// The editor's, and the ordinal of the file it was shown over.
        over: Option<(FormulaLook, u64)>,
        /// Buffers refused since the last file started (asked again then).
        refused: bool,
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
        }

        fn active(&mut self) -> Option<&mut FormulaLook> {
            match &mut self.over {
                Some((l, _)) => Some(l),
                None => self.saved.as_mut(),
            }
        }

        /// The setting's formula (`""`: none). One that does not build
        /// (checked when saved, so a formula a later build refuses) is
        /// logged and left out.
        #[inline(never)]
        pub fn set_saved(&mut self, path: &str, display: &mut Canvas) {
            if let Some(mut l) = self.saved.take() {
                l.release(display);
            }
            if !path.is_empty() {
                match FormulaLook::from_path(path) {
                    Ok(l) => self.saved = Some(l),
                    Err(e) => log::warn!("formula look: not drawn: {}", e.msg),
                }
            }
            self.refused = false;
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
                self.refused = false;
                self.publish();
                return Ok(());
            }
            let Some(ordinal) = ordinal else {
                return Err(PlayError::NoPlaylist);
            };
            match &mut self.over {
                Some((l, at)) if *at == ordinal => {
                    l.retune(path).map_err(|e| PlayError::BadFormula(e.msg))?;
                }
                _ if live => return Err(PlayError::MovedOn),
                _ => {
                    let l =
                        FormulaLook::from_path(path).map_err(|e| PlayError::BadFormula(e.msg))?;
                    if let Some((mut old, _)) = self.over.take() {
                        old.release(display);
                    }
                    // Its buffers in place of the setting's.
                    if let Some(s) = &mut self.saved {
                        s.release(display);
                    }
                    self.over = Some((l, ordinal));
                    self.refused = false;
                }
            }
            self.publish();
            Ok(())
        }

        /// A file started (`ordinal`): the editor's formula goes once the
        /// playlist has moved on; buffers refused before are asked again.
        #[inline(never)]
        pub fn started(&mut self, ordinal: u64, display: &mut Canvas) {
            if self.over.as_ref().is_some_and(|(_, at)| *at != ordinal) {
                if let Some((mut l, _)) = self.over.take() {
                    l.release(display);
                }
                log::info!("formula look: the editor's formula is off (the playlist moved on)");
                self.publish();
            }
            self.refused = false;
        }

        /// What the panel shows over `display` at `now` (see
        /// `FormulaLook::apply`): its buffers taken first if the heap has
        /// room. `decode_us`: the frame's decoding.
        #[inline(never)]
        pub fn apply<'a>(
            &'a mut self,
            now: Duration,
            display: &'a mut Canvas,
            decode_us: u32,
        ) -> &'a mut Canvas {
            let refused = self.refused;
            let take_failed = match self.active() {
                Some(l) if !l.is_held() && !refused => {
                    (free_heap() as usize) < TAKE_HEAP || !l.take()
                }
                _ => false,
            };
            if take_failed {
                self.refused = true;
                log::info!(
                    "formula look: no room for its {} B ({} B free); the picture goes up as it is",
                    FormulaLook::BUFFER_BYTES,
                    free_heap()
                );
            }
            match self.active() {
                Some(l) => l.apply(now, display, decode_us),
                None => display,
            }
        }

        /// When the look must be drawn whatever the picture does (its own
        /// frame), if it is drawn at all.
        #[inline(never)]
        pub fn due(&self) -> Option<Duration> {
            match &self.over {
                Some((l, _)) => l.due(),
                None => self.saved.as_ref().and_then(FormulaLook::due),
            }
        }

        /// Give the buffers back (night mode's dark panel, the panel
        /// test); taken again when it draws next.
        #[inline(never)]
        pub fn release(&mut self, display: &mut Canvas) {
            if let Some(l) = self.active().filter(|l| l.is_held()) {
                l.release(display);
            }
        }

        /// The panel showed something else (a status card): every row is
        /// drawn again next time.
        #[inline(never)]
        pub fn redraw(&mut self) {
            if let Some(l) = self.active() {
                l.redraw();
            }
        }

        /// The heap is low: give the buffers back. True if it did.
        #[inline(never)]
        pub fn yield_heap(&mut self, display: &mut Canvas) -> bool {
            if free_heap() as usize >= config::GLOW_YIELD_HEAP {
                return false;
            }
            match self.active().filter(|l| l.is_held()) {
                Some(l) => {
                    l.release(display);
                    self.refused = true;
                    true
                }
                None => false,
            }
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
            Err(PlayError::Unplayable(
                "this build cannot draw formulas".into(),
            ))
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
        pub fn due(&self) -> Option<Duration> {
            None
        }
        pub fn release(&mut self, _: &mut Canvas) {}
        pub fn redraw(&mut self) {}
        pub fn yield_heap(&mut self, _: &mut Canvas) -> bool {
            let _ = (config::GLOW_YIELD_HEAP, &SOURCE, Ordering::Relaxed);
            false
        }
    }
}
