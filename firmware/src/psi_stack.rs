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
//! `opendmd_core::fx_psi` makes (opening `@formula`, the Formula look, the
//! checks before saving) runs here, on a FreeRTOS task with
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
use opendmd_core::psi;

/// Run `f` on a task with [`config::PSI_COMPILE_STACK`] of stack, pinned
/// to the caller's core at the caller's priority, and wait for it. `None`:
/// no task (the heap can't spare the stack right now).
///
/// The caller's core: the main task's compiles stay on CPU1 with the
/// display (CPU0 is the network's), the HTTP server's on CPU0. The caller's
/// priority: the caller only waits, so the compile runs exactly when the
/// caller would have — neither ahead of the tasks the caller yields to nor
/// behind them.
///
/// **A FreeRTOS task, not a `std::thread`.** Until speed-d this spawned a
/// scoped std thread, and every compile left ~200 B of the heap behind
/// (psi-probe speed-c: 71 KB gone over the golden frames' ~320 compiles,
/// and the ~200-350 B "drift" per formula opened since the formula stack
/// came in: per-thread state the std thread's exit never gave back). A
/// task the caller deletes itself has nothing of the sort: its control
/// block and stack are freed by `vTaskDelete` at once, not later by an idle
/// task, because it is deleted only once suspended (pinned to the caller's
/// core it is never running while a pinned caller is; an unpinned caller
/// waits for it to suspend).
pub fn run<T: Send, F: FnOnce() -> T + Send>(f: F) -> Option<T> {
    use esp_idf_sys as sys;
    struct Job<F, T> {
        f: Option<F>,
        out: Option<T>,
        done: sys::QueueHandle_t,
    }
    unsafe extern "C" fn entry<F: FnOnce() -> T, T>(arg: *mut core::ffi::c_void) {
        // SAFETY: `arg` is the caller's `Job`, alive until it has taken
        // `done`, which is given only after the last use of `job` here.
        let job = unsafe { &mut *(arg as *mut Job<F, T>) };
        if let Some(f) = job.f.take() {
            job.out = Some(f());
        }
        let done = job.done;
        // (xSemaphoreGive: a send of no item to the back.)
        unsafe { sys::xQueueGenericSend(done, core::ptr::null(), 0, 0) };
        // Wait to be deleted by the caller (never returns: a FreeRTOS task
        // must not).
        loop {
            unsafe { sys::vTaskSuspend(core::ptr::null_mut()) };
        }
    }
    let core = esp_idf_hal::cpu::core() as i32;
    // SAFETY: a query about the calling task.
    let prio = unsafe { sys::uxTaskPriorityGet(core::ptr::null_mut()) }.clamp(1, 24);
    // (xSemaphoreCreateBinary: a queue of one empty item, type 3.)
    let done = unsafe { sys::xQueueGenericCreate(1, 0, 3) };
    if done.is_null() {
        return None;
    }
    let mut job = Job {
        f: Some(f),
        out: None,
        done,
    };
    let mut task: sys::TaskHandle_t = core::ptr::null_mut();
    // SAFETY: `entry::<F, T>` matches `job`'s type; the task is deleted
    // before `job` goes out of scope.
    let made = unsafe {
        sys::xTaskCreatePinnedToCore(
            Some(entry::<F, T>),
            c"psi-compile".as_ptr(),
            config::PSI_COMPILE_STACK as u32,
            &mut job as *mut Job<F, T> as *mut core::ffi::c_void,
            prio,
            &mut task,
            core,
        )
    };
    if made != 1 {
        unsafe { sys::vQueueDelete(done) };
        log::warn!(
            "formula: no task to compile on; heap {} B free",
            unsafe { sys::esp_get_free_heap_size() }
        );
        return None;
    }
    unsafe {
        sys::xQueueSemaphoreTake(done, u32::MAX);
        // Delete it only once it has suspended itself. Pinned to the
        // caller's core it always has by now when the caller is pinned (the
        // main task); a caller free to run on the other core (the HTTP
        // server's handlers) can wake while the task is still between its
        // give and its suspend: wait the moment out, so `vTaskDelete` never
        // meets a running task (which it would only queue for the idle
        // task to free).
        while sys::eTaskGetState(task) != sys::eTaskState_eSuspended {
            sys::vTaskDelay(1);
        }
        sys::vTaskDelete(task);
        sys::vQueueDelete(done);
    }
    job.out
}

/// The formula stack as it was until speed-d (a scoped `std::thread`), for
/// psi-probe's comparison of the two: what each compile leaves behind.
#[cfg(feature = "psi-probe")]
pub fn run_std_thread<T: Send>(f: impl FnOnce() -> T + Send) -> Option<T> {
    use esp_idf_hal::task::thread::ThreadSpawnConfiguration;
    let core: esp_idf_hal::cpu::Core = esp_idf_hal::cpu::core();
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
        std::thread::Builder::new()
            .name("psi-compile".into())
            .stack_size(config::PSI_COMPILE_STACK)
            .spawn_scoped(s, f)
            .ok()
            .and_then(|h| h.join().ok())
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
