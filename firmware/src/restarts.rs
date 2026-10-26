//! Every restart leaves a reason: the last [`SLOTS`] restarts, why each was
//! made and the chip's reset cause after it, kept in RTC slow memory
//! (`.rtc_noinit`, beside `sdhealth`'s records and the log ring) and shown
//! in `/api/status` (`restarts`, and `reset_note` when `sdhealth` has none).
//! The ring's logic is `opendmd_core::restarts` (host-tested); this is the
//! memory and the hooks that write it:
//!
//! - [`crate::restart`] writes the planned restart's [`Reason`] (every
//!   caller names one) with the uptime and heap, just before the reset.
//! - The Rust panic hook ([`install`]) writes a `panic` entry with the
//!   panic's `file:line` and the start of its message: no heap (std has
//!   already formatted the message), fixed buffers, word stores.
//! - FreeRTOS's stack-overflow hook ([`vApplicationStackOverflowHook`],
//!   replacing IDF's weak one) writes a `stack_overflow` entry with the
//!   task's name, then aborts as IDF's does.
//! - The heap's failed-allocation callback keeps the last failure (size,
//!   uptime, free heap): Rust's out-of-memory abort passes no hook of ours,
//!   so a crash just after one names it as the likely cause.
//! - The heartbeat ([`alive`]) keeps its uptime and heap: the figures for a
//!   crash that wrote nothing.
//!
//! [`boot`] pairs the newest entry with `esp_reset_reason` once, first
//! thing in `main`, and logs the result. A power-on clears the ring.
//!
//! Only plain word loads and stores touch RTC slow memory (see
//! `sdhealth.rs`); two writers at the same instant (a panic on each core)
//! can spoil a slot, which the checksum then drops.

use core::ffi::{c_char, CStr};
use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

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
    fn read(&self, i: usize) -> Slot {
        // SAFETY: `i < SLOTS` (the core's loops); a raw pointer, no reference.
        load(unsafe { &raw mut RING[i] })
    }
    #[inline(never)]
    fn write(&mut self, i: usize, s: &Slot) {
        // SAFETY: as above.
        store(unsafe { &raw mut RING[i] }, s)
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

/// Pair the last entry with this boot's reset, log it, then install the
/// hooks. Once, first thing in `main` (before anything can fail an
/// allocation: the previous run's record is read first).
#[inline(never)]
pub fn boot(reset: Reset) {
    let alive = ring::decode_alive(&load(&raw mut ALIVE));
    let alloc = ring::decode_alloc(&load(&raw mut ALLOC));
    store(&raw mut ALIVE, &[0; 5]);
    store(&raw mut ALLOC, &[0; 6]);
    let e = ring::boot(&mut Rtc, reset, alive, alloc);
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
    install();
}

/// The panic hook and the failed-allocation callback.
fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        panicked(info);
        previous(info);
    }));
    // SAFETY: registers a static function with the callback's signature.
    unsafe { sys::heap_caps_register_failed_alloc_callback(Some(alloc_failed)) };
}

/// The panic's entry: no allocation (the heap queries take its lock only
/// briefly, and no Rust code panics holding it).
#[inline(never)]
fn panicked(info: &std::panic::PanicHookInfo) {
    let mut e = Entry::new(Reason::Panic, Some(figures()));
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
/// own message and abort.
///
/// # Safety
/// Called by FreeRTOS with the task's NUL-terminated name.
#[no_mangle]
pub unsafe extern "C" fn vApplicationStackOverflowHook(
    _task: sys::TaskHandle_t,
    name: *mut c_char,
) {
    let name = if name.is_null() {
        "?"
    } else {
        // SAFETY: FreeRTOS's NUL-terminated task name.
        unsafe { CStr::from_ptr(name) }.to_str().unwrap_or("?")
    };
    let mut e = Entry::new(Reason::StackOverflow, Some(figures_light()));
    e.set_text(format_args!("task {name}"));
    ring::push(&mut Rtc, e);
    let mut msg = Msg([0; 80], 0);
    let _ = write!(
        msg,
        "***ERROR*** A stack overflow in task {name} has been detected."
    );
    let end = msg.1.min(msg.0.len() - 1);
    msg.0[end] = 0;
    // SAFETY: a NUL-terminated buffer; does not return.
    unsafe { sys::esp_system_abort(msg.0.as_ptr() as *const c_char) }
}

/// A fixed buffer for the abort message (room left for the NUL).
struct Msg([u8; 80], usize);

impl core::fmt::Write for Msg {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(self.0.len() - 1 - self.1);
        self.0[self.1..self.1 + n].copy_from_slice(&s.as_bytes()[..n]);
        self.1 += n;
        Ok(())
    }
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
    if ring::mark_update(&mut Rtc) {
        log::info!("reset reason: the restart out of the updater (firmware_update)");
    }
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
