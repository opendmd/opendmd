//! Every restart leaves a reason: the last [`SLOTS`] restarts, why each was
//! made and the chip's reset cause after it, kept in RTC slow memory
//! (`.rtc_noinit`, beside `sdhealth`'s records and the log ring) and shown
//! in `/api/status` (`restarts`, and `reset_note` when `sdhealth` has none).
//! The ring's logic is `opendmd_core::restarts` (host-tested); this is the
//! memory and the hooks that write it:
//!
//! - [`crate::restart`] writes the planned restart's [`Reason`] (every
//!   caller names one) with the uptime and heap, just before the reset.
//! - The Rust panic hook ([`install_panic_hook`], before the log ring's
//!   flush) writes a `panic` entry with the panic's `file:line` and the
//!   start of its message: no allocation (std has already formatted the
//!   message), no heap walk, fixed buffers, word stores.
//! - FreeRTOS's stack-overflow hook ([`vApplicationStackOverflowHook`],
//!   replacing IDF's weak one) writes a `stack_overflow` entry with the
//!   task's name through `ring::push_minimal`, then aborts as IDF's does.
//! - The `diag-heap` build's corruption aborts write `diag_abort` first.
//! - The heap's failed-allocation callback keeps the last failure (size,
//!   uptime, free heap): Rust's out-of-memory abort passes no hook of ours,
//!   so a crash soon after one names it (perhaps a deliberate probe).
//! - The heartbeat ([`alive`]) keeps its uptime and heap: the figures for a
//!   crash that wrote nothing.
//!
//! [`boot`] pairs the newest entry with `esp_reset_reason` once, first
//! thing in `main`, and logs the result. A power-on clears the ring.
//!
//! Only plain word loads and stores touch RTC slow memory (see
//! `sdhealth.rs`); two writers at the same instant (a panic on each core)
//! can spoil a slot, which the checksum then drops.

use core::ffi::c_char;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

use esp_idf_sys as sys;
use opendmd_core::restarts::{
    self as ring, AllocFail, Entry, Figures, Reason, Reset, Slot, Store, SLOTS, WORDS,
};

use crate::httpd::json_string;

#[link_section = ".rtc_noinit"]
static mut RING: [Slot; SLOTS] = [[0; WORDS]; SLOTS];
/// The last heartbeat's figures (`ring::encode_alive`).
#[link_section = ".rtc_noinit"]
static mut ALIVE: [u32; 5] = [0; 5];
/// The last failed allocation (`ring::encode_alloc`).
#[link_section = ".rtc_noinit"]
static mut ALLOC: [u32; 6] = [0; 6];

/// This boot's entry's sequence number (0 before [`boot`]).
static BOOT_SEQ: AtomicU32 = AtomicU32::new(0);
/// The panic's entry is written (the hook is installed twice, see
/// [`install_panic_hook`]).
static PANIC_RECORDED: AtomicBool = AtomicBool::new(false);

/// A word of RTC memory as an atomic, for plain loads and stores only.
fn word(p: *mut u32) -> &'static AtomicU32 {
    // SAFETY: a 4-aligned word of one of the statics above, only ever
    // accessed through this view (loads and stores, never RMW).
    unsafe { AtomicU32::from_ptr(p) }
}

fn load<const N: usize>(p: *mut [u32; N]) -> [u32; N] {
    let mut out = [0; N];
    for (i, w) in out.iter_mut().enumerate() {
        // SAFETY: `i < N`, inside the array.
        *w = word(unsafe { (p as *mut u32).add(i) }).load(Relaxed);
    }
    out
}

fn store<const N: usize>(p: *mut [u32; N], v: &[u32; N]) {
    for (i, w) in v.iter().enumerate() {
        // SAFETY: `i < N`, inside the array.
        word(unsafe { (p as *mut u32).add(i) }).store(*w, Relaxed);
    }
}

/// The ring in RTC memory.
struct Rtc;

impl Store for Rtc {
    #[inline(never)]
    fn get(&self, slot: usize, w: usize) -> u32 {
        // SAFETY: `slot < SLOTS`, `w < WORDS` (the core's loops); a raw
        // pointer, no reference.
        word(unsafe { &raw mut RING[slot][w] }).load(Relaxed)
    }
    #[inline(never)]
    fn put(&mut self, slot: usize, w: usize, v: u32) {
        // SAFETY: as above.
        word(unsafe { &raw mut RING[slot][w] }).store(v, Relaxed)
    }
}

fn uptime_ms() -> u32 {
    // SAFETY: argument-free read of the IDF's microsecond clock.
    (unsafe { sys::esp_timer_get_time() } / 1000) as u32
}

/// Uptime and heap now.
fn figures() -> Figures {
    Figures {
        uptime_ms: uptime_ms(),
        heap_free: crate::state::free_heap(),
        largest: crate::state::largest_free_block() as u32,
    }
}

/// Uptime and free heap only: for the hooks that run where walking the heap
/// for its largest block is not wanted (a context switch, an allocation).
fn figures_light() -> Figures {
    Figures {
        uptime_ms: uptime_ms(),
        heap_free: crate::state::free_heap(),
        largest: 0,
    }
}

/// `esp_reset_reason`.
pub fn reset_now() -> Reset {
    // SAFETY: an argument-free query of the reset cause.
    #[allow(non_upper_case_globals)]
    match unsafe { sys::esp_reset_reason() } {
        sys::esp_reset_reason_t_ESP_RST_POWERON => Reset::PowerOn,
        sys::esp_reset_reason_t_ESP_RST_EXT => Reset::External,
        sys::esp_reset_reason_t_ESP_RST_SW => Reset::Software,
        sys::esp_reset_reason_t_ESP_RST_PANIC => Reset::Panic,
        sys::esp_reset_reason_t_ESP_RST_INT_WDT => Reset::IntWdt,
        sys::esp_reset_reason_t_ESP_RST_TASK_WDT => Reset::TaskWdt,
        sys::esp_reset_reason_t_ESP_RST_WDT => Reset::Wdt,
        sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP => Reset::DeepSleep,
        sys::esp_reset_reason_t_ESP_RST_BROWNOUT => Reset::Brownout,
        sys::esp_reset_reason_t_ESP_RST_SDIO => Reset::Sdio,
        _ => Reset::Unknown,
    }
}

/// Pair the last entry with this boot's reset, log it, seed the heartbeat's
/// record, then install the hooks. Once, first thing in `main` (before
/// anything can fail an allocation: the previous run's record is read
/// first).
#[inline(never)]
pub fn boot(reset: Reset) {
    let last = ring::decode_alive(&load(&raw mut ALIVE));
    let alloc = ring::decode_alloc(&load(&raw mut ALLOC));
    store(&raw mut ALIVE, &[0; 5]);
    store(&raw mut ALLOC, &[0; 6]);
    let e = ring::boot(&mut Rtc, reset, last, alloc);
    BOOT_SEQ.store(e.seq, Relaxed);
    let up = e.figures.map_or(String::new(), |f| {
        format!(
            ", after {} s up, heap {} B free (largest {} B)",
            f.uptime_ms / 1000,
            f.heap_free,
            f.largest
        )
    });
    let at = if e.text().is_empty() {
        String::new()
    } else {
        format!(", {}", e.text())
    };
    log::info!(
        "reset reason: {} ({}){up}{at}",
        reset.name(),
        e.reason.name()
    );
    // A crash before the first heartbeat still has figures.
    alive();
    install_panic_hook();
    // SAFETY: registers a static function with the callback's signature.
    unsafe { sys::heap_caps_register_failed_alloc_callback(Some(alloc_failed)) };
}

/// The panic hook: here at boot, and again after `logring::start` has
/// wrapped the first (its hook prints the log ring first), so the entry is
/// written before the flush. Whichever runs first writes it.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !PANIC_RECORDED.swap(true, Relaxed) {
            panicked(info);
        }
        previous(info);
    }));
}

/// The panic's entry: no allocation, and no heap walk (the heap may be what
/// is broken).
#[inline(never)]
fn panicked(info: &std::panic::PanicHookInfo) {
    let mut e = Entry::new(Reason::Panic, Some(figures_light()));
    let (file, line) = info.location().map_or(("?", 0), |l| (l.file(), l.line()));
    e.set_panic(file, line, info.payload_as_str().unwrap_or(""));
    ring::push(&mut Rtc, e);
}

/// The heap's failed-allocation callback: every failure, deliberate
/// (`try_reserve`) or not, overwrites the last.
unsafe extern "C" fn alloc_failed(size: usize, _caps: u32, _function: *const c_char) {
    let a = AllocFail {
        size: size as u32,
        at: figures_light(),
    };
    store(&raw mut ALLOC, &ring::encode_alloc(&a));
}

/// FreeRTOS's stack-overflow hook (IDF's is weak): the entry, then IDF's
/// own message and abort. Minimal on purpose (it runs in a context switch,
/// on a task whose stack has just overrun): `ring::push_minimal`, word
/// stores, byte copies — no slot on the stack, no `core::fmt`.
///
/// # Safety
/// Called by FreeRTOS with the task's NUL-terminated name.
#[no_mangle]
pub unsafe extern "C" fn vApplicationStackOverflowHook(
    _task: sys::TaskHandle_t,
    name: *mut c_char,
) {
    const NAME_MAX: usize = sys::configMAX_TASK_NAME_LEN as usize;
    let mut n = 0;
    while !name.is_null() && n < NAME_MAX && unsafe { *name.add(n) } != 0 {
        n += 1;
    }
    let name: &[u8] = if n == 0 {
        b"?"
    } else {
        // SAFETY: `n` bytes of the task's name, all before its NUL.
        unsafe { core::slice::from_raw_parts(name as *const u8, n) }
    };
    ring::push_minimal(
        &mut Rtc,
        Reason::StackOverflow,
        figures_light(),
        &[b"task ", name],
    );
    const HEAD: &[u8] = b"***ERROR*** A stack overflow in task ";
    const TAIL: &[u8] = b" has been detected.\0";
    let mut msg = [0u8; HEAD.len() + NAME_MAX + TAIL.len()];
    msg[..HEAD.len()].copy_from_slice(HEAD);
    msg[HEAD.len()..HEAD.len() + name.len()].copy_from_slice(name);
    msg[HEAD.len() + name.len()..HEAD.len() + name.len() + TAIL.len()].copy_from_slice(TAIL);
    // SAFETY: a NUL-terminated buffer; does not return.
    unsafe { sys::esp_system_abort(msg.as_ptr() as *const c_char) }
}

/// The bench's `?overflow=1` (`POST /api/reboot`): this task's stack canary
/// overwritten, as a real overrun would, then a yield, at which FreeRTOS
/// checks it and calls the hook above. No memory past the stack is touched.
#[cfg(feature = "bench")]
pub fn overflow_on_purpose() -> ! {
    // SAFETY: the calling task's own stack start; its lowest 16 bytes are
    // FreeRTOS's canary (configCHECK_FOR_STACK_OVERFLOW 2), unused by
    // anything else while the task runs well clear of them.
    unsafe {
        let start = sys::pxTaskGetStackStart(core::ptr::null_mut());
        core::ptr::write_bytes(start, 0, 16);
        for _ in 0..10 {
            sys::vTaskDelay(1);
        }
    }
    panic!("the stack-overflow check did not fire");
}

/// A `diag-heap` build's corruption abort: the entry (no heap walk), then
/// the caller aborts.
#[cfg(feature = "diag-heap")]
pub fn diag(site: &str) {
    let mut e = Entry::new(Reason::Diag, Some(figures_light()));
    e.set_text(format_args!("{site}"));
    ring::push(&mut Rtc, e);
}

/// A planned restart's entry ([`crate::restart`]).
#[inline(never)]
pub fn record(why: Reason) {
    ring::push(&mut Rtc, Entry::new(why, Some(figures())));
}

/// The heartbeat's figures, for a crash that writes nothing.
#[inline(never)]
pub fn alive() {
    store(&raw mut ALIVE, &ring::encode_alive(&figures()));
}

/// `ota::after_boot` found the updater's mark: a boot that recorded nothing
/// came out of the updater (after an update from firmware with no ring).
#[inline(never)]
pub fn booted_after_update() {
    // A new firmware: the FAT index's crash count was the old one's.
    crate::fatidx::forget_crashes();
    #[cfg(feature = "psi")]
    crate::card_formulas::forget_crashes();
    if ring::mark_update(&mut Rtc) {
        log::info!("reset reason: the restart out of the updater (firmware_update)");
    }
}

/// Whether the crash this boot came out of is one the code in `files` can
/// be blamed for (`opendmd_core::restarts::crash_in`).
pub fn this_boot_crash_in(files: &[&str], task: &str) -> bool {
    this_boot().is_some_and(|e| ring::crash_in(&e, files, task))
}

/// This boot's entry.
#[inline(never)]
fn this_boot() -> Option<Entry> {
    let seq = BOOT_SEQ.load(Relaxed);
    ring::newest_first(&Rtc).find(|e| e.seq == seq)
}

/// Why this boot's restart was made, in plain words (`reset_note`, when
/// `sdhealth` has none); `None` after a power-on.
pub fn reset_note() -> Option<&'static str> {
    this_boot().and_then(|e| e.note())
}

/// `"restarts":[…]`, newest first.
#[inline(never)]
#[optimize(size)] // (CPU0 / boot / config: no frame runs it — console-c)
pub fn status_json() -> String {
    let mut out = String::from(r#""restarts":["#);
    for (i, e) in ring::newest_first(&Rtc).enumerate() {
        if i > 0 {
            out.push(',');
        }
        // `reset` is null while pending (never for this boot's entry).
        let reset = match e.reset {
            Reset::Pending => "null".to_string(),
            r => format!(r#""{}""#, r.name()),
        };
        let _ = write!(out, r#"{{"reason":"{}","reset":{reset},"#, e.reason.name());
        match e.figures {
            Some(f) => {
                let _ = write!(
                    out,
                    r#""uptime_s":{},"heap_free":{},"largest":{}"#,
                    f.uptime_ms / 1000,
                    f.heap_free,
                    // 0: not measured (a stack overflow's hook).
                    if f.largest == 0 {
                        "null".to_string()
                    } else {
                        f.largest.to_string()
                    }
                );
            }
            None => out.push_str(r#""uptime_s":null,"heap_free":null,"largest":null"#),
        }
        if !e.text().is_empty() {
            let _ = write!(out, r#","where":{}"#, json_string(e.text()));
        }
        out.push('}');
    }
    out.push(']');
    out
}
