//! Formulas compile on a stack of their own. Psi's compiler is a recursive
//! descent parser bounded at `psi::MAX_DEPTH` levels; at its worst (calls
//! nested to the cap, a misspelt name at the bottom) it used 9,664 B of
//! stack on the ESP32 on 2026-10-25, 11,808 B on 2026-10-26 (the cap then
//! 24; ~430 B a level), ~9.2 KB with the cap at 18 (beta.7's compiler
//! 13,152 B). Its callers are the main task (16 KB, ~8 KB
//! of it in use where a path opens) and the HTTP server's handlers (6.5 KB:
//! an attract effect is checked there), so a deep formula sent by anyone on
//! the network could overflow either and restart the player. Installed
//! with `fx_psi::set_compiler` at the top of `main`: every compile
//! `opendmd_core::fx_psi` makes (opening `@fx:psi`, the Formula look, the
//! checks before saving) runs here, on a scoped thread with
//! [`config::PSI_COMPILE_STACK`] (12 KB) of stack taken from the heap for
//! the compile only: the worst case leaves 1,820 B of it never touched on
//! the rig (the thread's entry frames, ~0.8 KB with interrupt frames, come
//! on top of the compiler's 9,664 B). The caller waits (a compile is 0.1–10 ms; `psi-probe`
//! reports the worst cases' times and the thread's real headroom).
//!
//! **Heap.** The stack (+ the thread's control block) is a dip for the
//! length of the compile. Compiled for the HTTP server (an attract effect,
//! the look's check before saving) it can come at any moment of playback:
//! for those milliseconds the main loop may see the heap under its 20 KB
//! yield thresholds (the glow and the look give their buffers back) and the
//! transition gate may cut a dissolve. Rare (a settings change) and brief.

use crate::config;
use esp_idf_hal::cpu::Core;
use esp_idf_hal::task::thread::ThreadSpawnConfiguration;
use opendmd_core::psi;

/// Run `f` on a thread with [`config::PSI_COMPILE_STACK`] of stack, pinned
/// to the caller's core at the caller's priority, and wait for it. `None`:
/// no thread (the heap can't spare the stack right now).
///
/// The caller's core: the main task's compiles stay on CPU1 with the
/// display (CPU0 is the network's), the HTTP server's on CPU0. The caller's
/// priority: the caller only waits in `join`, so the compile runs exactly
/// when the caller would have — neither ahead of the tasks the caller
/// yields to nor behind them.
pub fn run<T: Send>(f: impl FnOnce() -> T + Send) -> Option<T> {
    let core: Core = esp_idf_hal::cpu::core();
    // SAFETY: a query about the calling task.
    let prio = unsafe { esp_idf_sys::uxTaskPriorityGet(std::ptr::null_mut()) };
    let previous = ThreadSpawnConfiguration::get();
    let pinned = ThreadSpawnConfiguration {
        name: Some(b"psi-compile\0"),
        stack_size: config::PSI_COMPILE_STACK,
        priority: prio.clamp(1, 24) as u8,
        pin_to_core: Some(core),
        ..Default::default()
    }
    .set()
    .is_ok();
    let r = std::thread::scope(|s| {
        match std::thread::Builder::new()
            .name("psi-compile".into())
            .stack_size(config::PSI_COMPILE_STACK)
            .spawn_scoped(s, f)
        {
            // `join` fails only if the thread panicked, and the firmware is
            // built with panic = "abort": a panic restarts the player, so
            // this `.ok()` never sees an `Err`.
            Ok(h) => h.join().ok(),
            Err(e) => {
                log::warn!(
                    "formula: no thread to compile on ({e}); heap {} B free",
                    unsafe { esp_idf_sys::esp_get_free_heap_size() }
                );
                None
            }
        }
    });
    if pinned {
        match previous {
            Some(p) => {
                let _ = p.set();
            }
            None => {
                let _ = ThreadSpawnConfiguration::default().set();
            }
        }
    }
    r
}

/// [`opendmd_core::fx_psi::Compiler`]: `psi::compile_into` on its own stack.
/// No thread: an error the user sees, nothing compiled.
pub fn compile(src: &str, prog: &mut psi::Program) -> Result<psi::Info, psi::Error> {
    run(|| psi::compile_into(src, prog)).unwrap_or_else(|| {
        Err(psi::Error {
            start: 0,
            end: 0,
            msg: "the player is short of memory just now: try again in a moment".to_string(),
        })
    })
}
