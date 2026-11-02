//! Starting a task pinned to CPU0: the one place `ThreadSpawnConfiguration`
//! (`esp_pthread_set_cfg`, per calling thread) is set for a spawn and put
//! back, whatever happened, so nothing else lands on CPU0 with it.

use esp_idf_hal::cpu::Core;
use esp_idf_hal::task::thread::ThreadSpawnConfiguration;
use std::io;

/// Spawn `f` as task `name` (NUL-terminated, also the Rust thread's name)
/// on CPU0 with `stack` bytes at FreeRTOS `priority`.
pub fn spawn_core0(name: &'static [u8], stack: usize, priority: u8, f: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    spawn_core0_joinable(name, stack, priority, f).map(drop)
}

/// [`spawn_core0`], keeping the thread's handle (a task that is stopped and
/// joined: the captive portal's DNS server).
pub fn spawn_core0_joinable(
    name: &'static [u8],
    stack: usize,
    priority: u8,
    f: Box<dyn FnOnce() + Send>,
) -> io::Result<std::thread::JoinHandle<()>> {
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
