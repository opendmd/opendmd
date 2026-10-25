//! Formulas compile on a stack of their own. Psi's compiler is a recursive
//! descent parser bounded at `psi::MAX_DEPTH` levels; at its worst (calls
//! nested to the cap, a misspelt name at the bottom) it used 10,048 B of
//! stack on the ESP32 (`psi-probe`, 2026-10-25). Its callers are the main
//! task (16 KB, ~8 KB of it in use where a path opens) and the HTTP
//! server's handlers (6.5 KB: an attract effect is checked there), so a
//! deep formula typed by anyone on the network could overflow either and
//! restart the player. Installed with `fx_psi::set_compiler` at the top of
//! `main`: every compile `opendmd_core::fx_psi` makes (opening `@fx:psi`,
//! the Formula look, the checks before saving) runs here, on a scoped
//! thread with [`config::PSI_COMPILE_STACK`] of stack taken from the heap
//! for the compile only. The caller waits (a compile is 0.1–10 ms).

use crate::config;
use opendmd_core::psi;

/// [`opendmd_core::fx_psi::Compiler`]: `psi::compile_into` on its own stack.
/// No thread (the heap can't spare the stack right now): an error the user
/// sees, nothing compiled.
pub fn compile(src: &str, prog: &mut psi::Program) -> Result<psi::Info, psi::Error> {
    std::thread::scope(|s| {
        let busy = || psi::Error {
            start: 0,
            end: 0,
            msg: "the player is short of memory just now: try again in a moment".to_string(),
        };
        match std::thread::Builder::new()
            .name("psi-compile".into())
            .stack_size(config::PSI_COMPILE_STACK)
            .spawn_scoped(s, || psi::compile_into(src, prog))
        {
            Ok(h) => h.join().unwrap_or_else(|_| Err(busy())),
            Err(e) => {
                log::warn!(
                    "formula: no thread to compile on ({e}); heap {} B free",
                    unsafe { esp_idf_sys::esp_get_free_heap_size() }
                );
                Err(busy())
            }
        }
    })
}
