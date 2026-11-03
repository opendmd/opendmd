//! Starting a task pinned to CPU0: [`spawn_core0`] (a FreeRTOS task: no std
//! thread's leftovers), and [`spawn_core0_joinable`] (a std thread, for the
//! one task that is joined): the one place `ThreadSpawnConfiguration`
//! (`esp_pthread_set_cfg`, per calling thread) is set for a spawn and put
//! back, whatever happened, so nothing else lands on CPU0 with it.

use esp_idf_hal::cpu::Core;
use esp_idf_hal::task::thread::ThreadSpawnConfiguration;
use std::io;

/// Spawn `f` as task `name` (NUL-terminated) on CPU0 with `stack` bytes at
/// FreeRTOS `priority`: a FreeRTOS task of its own, not a `std::thread`.
///
/// **No `std::thread` for a task that ends.** Every std thread that ends on
/// ESP-IDF leaves ~190 B behind: its `Arc<Inner>` (48 B, a reference still
/// held), its parker's pthread mutex (16 B) and FreeRTOS semaphore (92 B),
/// its name (heap-map census, 2026-11-03: 62-66 new blocks for 20 short
/// tasks). Tasks started again and again (the read-ahead once a GIF, the
/// cache builder, the settings saver, the card-speed probe) filled the heap
/// with small blocks: the spare's floor went 37.5 -> 13.2 KB in ~4 min of
/// the emulator. A FreeRTOS task that deletes itself has none of that: its
/// control block and stack are freed by the idle task. (What runs in it
/// must not ask for `std::thread::current()` or park: none of these tasks
/// does; `std::thread::sleep` and the std locks need neither.)
pub fn spawn_core0(name: &'static [u8], stack: usize, priority: u8, f: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    use esp_idf_sys as sys;
    unsafe extern "C" fn entry(arg: *mut core::ffi::c_void) {
        // SAFETY: `arg` is the `Box<Box<dyn FnOnce>>` leaked below, taken
        // back once, here.
        let f = unsafe { Box::from_raw(arg as *mut Box<dyn FnOnce() + Send>) };
        f();
        // A FreeRTOS task must not return: deleted, its memory freed by the
        // idle task.
        unsafe { sys::vTaskDelete(core::ptr::null_mut()) };
    }
    if name.last() != Some(&0) {
        return Err(io::Error::other("task name not NUL-terminated"));
    }
    let arg = Box::into_raw(Box::new(f));
    let mut task: sys::TaskHandle_t = core::ptr::null_mut();
    // SAFETY: `entry` takes `arg` back; `name` is NUL-terminated and static.
    let made = unsafe {
        sys::xTaskCreatePinnedToCore(
            Some(entry),
            name.as_ptr() as *const core::ffi::c_char,
            stack as u32,
            arg as *mut core::ffi::c_void,
            priority as u32,
            &mut task,
            0,
        )
    };
    if made != 1 {
        // SAFETY: not handed to any task.
        drop(unsafe { Box::from_raw(arg) });
        return Err(io::Error::new(io::ErrorKind::OutOfMemory, "no heap for the task"));
    }
    Ok(())
}

/// [`spawn_core0`], keeping the thread's handle (a task that is stopped and
/// joined: the captive portal's DNS server).
pub fn spawn_core0_joinable(
    name: &'static [u8],
    stack: usize,
    priority: u8,
    f: Box<dyn FnOnce() + Send>,
) -> io::Result<std::thread::JoinHandle<()>> {
    // The heap map (a diagnostic build): a checkpoint as the task starts
    // and as it ends, the first time each name does.
    #[cfg(feature = "heapmap")]
    let f: Box<dyn FnOnce() + Send> = {
        let tag = std::str::from_utf8(name.strip_suffix(b"\0").unwrap_or(name)).unwrap_or("?");
        Box::new(move || {
            crate::heapmap::cp_k("task start: ", tag);
            f();
            crate::heapmap::cp_k("task end: ", tag);
        })
    };
    let previous = ThreadSpawnConfiguration::get();
    let set = ThreadSpawnConfiguration {
        name: Some(name),
        stack_size: stack,
        priority,
        pin_to_core: Some(Core::Core0),
        ..Default::default()
    }
    .set();
    let spawned = match set {
        Ok(()) => std::thread::Builder::new()
            .name(String::from_utf8_lossy(name.strip_suffix(b"\0").unwrap_or(name)).into_owned())
            .stack_size(stack)
            .spawn(f),
        Err(e) => Err(io::Error::other(e)),
    };
    let _ = previous.unwrap_or_default().set();
    spawned
}
