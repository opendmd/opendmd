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
//!   caps, the task, uptime, free heap, what was going on and 12 return
//!   addresses), written straight to RTC memory. Rust's out-of-memory
//!   abort runs our alloc error hook (`oom_hook`), which latches that
//!   failure as the fatal one (nothing overwrites it after) and flags it,
//!   so the next boot names it as the abort; a crash soon after an
//!   unflagged one names it too (perhaps a deliberate probe).
//! - The heartbeat ([`alive`]) keeps its uptime, heap and [`activity_now`]:
//!   the figures for a crash that wrote nothing.
//! - Every entry carries what was going on ([`activity_now`]: the WiFi's
//!   state, the card builder, the FAT index build, a transition, transfers,
//!   the card's errors, the WiFi's reconnect tries), `activity` in status.
//! - A boot out of a crash keeps the crashed run's last log lines
//!   (`logring::keep_crash_tail`, `GET /api/log/last`), headed by the
//!   failed allocation's details when there was one.
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
    self as ring, Activity, ActivityParts, AllocFail, Entry, Figures, Reason, Reset, Slot, Store,
    ALIVE_WORDS, ALLOC_PCS, ALLOC_WORDS, SLOTS, TASK_NAME, WORDS,
};

use crate::httpd::json_string;

#[link_section = ".rtc_noinit"]
static mut RING: [Slot; SLOTS] = [[0; WORDS]; SLOTS];
/// The last heartbeat's figures (`ring::encode_alive`).
#[link_section = ".rtc_noinit"]
static mut ALIVE: [u32; ALIVE_WORDS] = [0; ALIVE_WORDS];
/// The last failed allocation (`ring::encode_alloc`).
#[link_section = ".rtc_noinit"]
static mut ALLOC: [u32; ALLOC_WORDS] = [0; ALLOC_WORDS];

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

/// What is going on now: atomics and RTC words only (no lock, no heap), so
/// the allocation callback, the panic hook and the stack-overflow hook can
/// call it.
pub fn activity_now() -> Activity {
    Activity::pack(&ActivityParts {
        wifi: crate::wifi::act(),
        builder: crate::cache_builder::holding(),
        fat_index: crate::fatidx::building(),
        transition: crate::state::TRANSITIONING.load(Relaxed),
        index_in_ram: crate::state::INDEX_IN_RAM.load(Relaxed),
        streams: crate::stream::transfers(),
        uploads: crate::stream::uploads(),
        card_errors: crate::sdhealth::error_count(),
        reconnect_tries: crate::wifi::reconnect_tries(),
    })
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
        sys::esp_reset_reason_t_ESP_RST_SDIO => Reset::Sdio, #[cfg(esp32s3)] r @ (sys::esp_reset_reason_t_ESP_RST_USB | sys::esp_reset_reason_t_ESP_RST_JTAG | sys::esp_reset_reason_t_ESP_RST_PWR_GLITCH) => s3_reset(r), // (S3 only, on this line so the lines below keep their numbers: `s3_reset`)
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
    store(&raw mut ALIVE, &[0; ALIVE_WORDS]);
    store(&raw mut ALLOC, &[0; ALLOC_WORDS]);
    let e = ring::boot(&mut Rtc, reset, last, alloc);
    BOOT_SEQ.store(e.seq, Relaxed);
    // The crashed run's last lines, before the log ring is written again.
    if reset == Reset::PowerOn {
        crate::logring::forget_crash_tail();
    } else if reset.is_crash()
        || matches!(
            e.reason,
            Reason::Panic | Reason::StackOverflow | Reason::Diag | Reason::Crash | Reason::WifiHeap | Reason::WifiStuck
        )
    {
        keep_crash_log(&e, last, alloc);
    }
    crate::logring::forget_last_tail();
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
        "reset reason: {} ({}){up}{at}{}{}",
        reset.name(),
        e.reason.name(),
        if e.activity.is_known() { "; during: " } else { "" },
        e.activity
    );
    // A crash before the first heartbeat still has figures.
    alive();
    install_panic_hook();
    // SAFETY: registers a static function with the callback's signature.
    unsafe { sys::heap_caps_register_failed_alloc_callback(Some(alloc_failed)) };
    std::alloc::set_alloc_error_hook(oom_hook);
}

/// The crash log's head (`logring::keep_crash_tail`): the entry, and the
/// failed allocation beside it when it is the one named, with its caps and
/// return addresses (`addr2line -pfiaC -e <the release ELF>` names them;
/// the first few are the allocator's own frames).
#[inline(never)]
fn keep_crash_log(e: &Entry, alive: Option<(Figures, Activity)>, alloc: Option<AllocFail>) {
    struct Pcs([u32; ALLOC_PCS]);
    impl core::fmt::Display for Pcs {
        fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
            for pc in self.0.iter().take_while(|&&pc| pc != 0) {
                write!(f, " {pc:#010x}")?;
            }
            Ok(())
        }
    }
    let up = e.figures.map_or(0, |f| f.uptime_ms / 1000);
    let beat = alive.map_or(Activity(0), |(_, a)| a);
    match ring::recent_alloc(alive.map(|(f, _)| f), alloc) {
        Some(a) if e.reason == Reason::Crash => crate::logring::keep_crash_tail(
            e.seq,
            format_args!(
                "# crash log: restart {} ({}, reset {}), last heartbeat at {up} s; during: {}\n\
                 # last failed alloc: {} B, caps {:#x}, task {}, at {}.{:03} s, {} B free; during: {}\n\
                 # backtrace:{}\n",
                e.seq,
                e.reason.name(),
                e.reset.name(),
                beat,
                a.size,
                a.caps,
                a.task(),
                a.at.uptime_ms / 1000,
                a.at.uptime_ms % 1000,
                a.at.heap_free,
                a.activity,
                Pcs(a.pcs),
            ),
        ),
        _ => crate::logring::keep_crash_tail(
            e.seq,
            format_args!(
                "# crash log: restart {} ({}, reset {}), at {up} s; during: {}{}{}\n",
                e.seq,
                e.reason.name(),
                e.reset.name(),
                e.activity,
                if e.text().is_empty() { "" } else { "; where: " },
                e.text(),
            ),
        ),
    }
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
    e.activity = activity_now();
    let (file, line) = info.location().map_or(("?", 0), |l| (l.file(), l.line()));
    e.set_panic(file, line, info.payload_as_str().unwrap_or(""));
    ring::push(&mut Rtc, e);
}

/// The fatal failure is latched ([`oom_hook`]): later ones (the abort's
/// own, if any) do not overwrite its record.
static FATAL: AtomicBool = AtomicBool::new(false);

/// A word of [`ALLOC`].
fn alloc_word(i: usize) -> &'static AtomicU32 {
    // SAFETY: `i < ALLOC_WORDS` (the callers' indices), a word of the
    // static, only accessed through this view.
    word(unsafe { (&raw mut ALLOC as *mut u32).add(i % ALLOC_WORDS) })
}

/// The heap's failed-allocation callback: every failure, deliberate
/// (`try_reserve`) or not, overwrites the last, until the fatal one is
/// latched. No heap, no lock, nothing of the record on its stack: each
/// word goes straight to RTC memory as it is worked out (the task's name
/// from its TCB, atomics, the return addresses from IDF's own frame walk,
/// which checks every pointer before it follows it), the checksum folded
/// as it goes, the magic last. A few µs.
unsafe extern "C" fn alloc_failed(size: usize, caps: u32, _function: *const c_char) {
    if FATAL.load(Relaxed) {
        return;
    }
    let f = figures_light();
    alloc_word(0).store(0, Relaxed);
    let mut h = ring::alloc_fold(ring::ALLOC_FOLD_SEED, ring::ALLOC_MAGIC);
    let put = |i: usize, w: u32, h: &mut u32| {
        alloc_word(i).store(w, Relaxed);
        *h = ring::alloc_fold(*h, w);
    };
    for (i, w) in [size as u32, caps, f.uptime_ms, f.heap_free, 0, activity_now().0, 0].into_iter().enumerate() {
        put(1 + i, w, &mut h);
    }
    // SAFETY: argument-free queries; the name is NUL-terminated within
    // configMAX_TASK_NAME_LEN (16) bytes in the running task's TCB.
    let name: *const u8 = unsafe {
        if sys::xPortInIsrContext() != 0 {
            c"isr".as_ptr().cast()
        } else {
            sys::pcTaskGetName(core::ptr::null_mut()).cast()
        }
    };
    let mut ended = name.is_null();
    for k in 0..TASK_NAME / 4 {
        let mut w = [0u8; 4];
        for (j, b) in w.iter_mut().enumerate() {
            if !ended {
                // SAFETY: within the name, up to and including its NUL.
                *b = unsafe { *name.add(k * 4 + j) };
                ended = *b == 0;
            }
        }
        put(ring::ALLOC_TASK + k, u32::from_le_bytes(w), &mut h);
    }
    backtrace(&mut |k, pc| put(ring::ALLOC_PC0 + k, pc, &mut h));
    alloc_word(ALLOC_WORDS - 1).store(h, Relaxed);
    alloc_word(0).store(ring::ALLOC_MAGIC, Relaxed);
}

/// Rust's alloc error hook (`std::alloc::set_alloc_error_hook`): the
/// failure just recorded by [`alloc_failed`] is the fatal one. Latched
/// (no later failure overwrites it), flagged in the record (the next
/// boot's `where` says `the abort`), and the message std's own hook
/// prints. Then std aborts.
fn oom_hook(layout: std::alloc::Layout) {
    FATAL.store(true, Relaxed);
    if alloc_word(0).load(Relaxed) == ring::ALLOC_MAGIC {
        alloc_word(ring::ALLOC_FLAGS).store(ring::ALLOC_FATAL, Relaxed);
        let h = (0..ALLOC_WORDS - 1).fold(ring::ALLOC_FOLD_SEED, |h, i| ring::alloc_fold(h, alloc_word(i).load(Relaxed)));
        alloc_word(ALLOC_WORDS - 1).store(h, Relaxed);
    }
    // SAFETY: a NUL-terminated format and one unsigned argument; the ROM's
    // printf writes straight to the UART (the log ring may never drain).
    unsafe { sys::esp_rom_printf(c"memory allocation of %u bytes failed\n".as_ptr(), layout.size() as u32) };
}

/// Up to [`ALLOC_PCS`] return addresses from the frames above the heap's
/// failure path, innermost first, each to `each(k, pc)`; the rest 0:
/// `esp_backtrace_print`'s walk, without the printing. The first two
/// frames, the callback's and `heap_caps_alloc_failed`'s, are always the
/// same and not kept; the allocator's own (`heap_caps_malloc*`, `malloc`,
/// `__rust_alloc`) come next, then the caller.
#[inline(never)]
fn backtrace(each: &mut dyn FnMut(usize, u32)) {
    const SKIP: usize = 2;
    let mut k = 0;
    // SAFETY: IDF's frame walk over the calling task's own stack (the
    // window registers spilled by `esp_backtrace_get_start`); each step
    // checks the next frame's SP and PC before it is followed.
    unsafe {
        let mut f: sys::esp_backtrace_frame_t = core::mem::zeroed();
        sys::esp_backtrace_get_start(&mut f.pc, &mut f.sp, &mut f.next_pc);
        let mut alive = true;
        for _ in 0..SKIP {
            alive = alive && f.next_pc != 0 && sys::esp_backtrace_get_next_frame(&mut f);
        }
        while alive && k < ALLOC_PCS {
            if f.next_pc == 0 || !sys::esp_backtrace_get_next_frame(&mut f) {
                break;
            }
            // `esp_cpu_process_stack_pc` (an inline, not in the bindings):
            // the windowed call's PC in the top two bits, back 3 to the call.
            let pc = if f.pc & 0x8000_0000 != 0 { (f.pc & 0x3fff_ffff) | 0x4000_0000 } else { f.pc };
            each(k, pc.wrapping_sub(3));
            k += 1;
        }
    }
    while k < ALLOC_PCS {
        each(k, 0);
        k += 1;
    }
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
        activity_now(),
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
    e.activity = activity_now();
    e.set_text(format_args!("{site}"));
    ring::push(&mut Rtc, e);
}

/// A planned restart's entry ([`crate::restart`]).
#[inline(never)]
pub fn record(why: Reason) {
    let mut e = Entry::new(why, Some(figures()));
    e.activity = activity_now();
    // A card-failure restart says which transfer failed last (`where`).
    if why == Reason::CardFailure {
        if let Some(note) = crate::sdhealth::fault_note() {
            e.set_text(format_args!("{note}"));
        }
    }
    ring::push(&mut Rtc, e);
}

/// The heartbeat's figures, for a crash that writes nothing.
#[inline(never)]
pub fn alive() {
    store(&raw mut ALIVE, &ring::encode_alive(&figures(), activity_now()));
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

/// Why this boot's restart was made (`None`: unknown, or a power-on).
pub fn this_boot_reason() -> Option<Reason> {
    this_boot().map(|e| e.reason)
}

/// This boot's entry.
#[inline(never)]
fn this_boot() -> Option<Entry> {
    let seq = BOOT_SEQ.load(Relaxed);
    ring::newest_first(&Rtc).find(|e| e.seq == seq)
}

/// What this boot and those just before it say
/// (`opendmd_core::restarts::crash_streak`): the boots out of a crash since
/// the last clean boot (brownouts are power, not firmware: they neither
/// count nor end the run), and whether brownouts soon after start-up keep
/// restarting it. Remote control holds back 15 minutes after one crash or
/// such a run of brownouts, and stays off after two crashes until a clean
/// boot or a switch-on in Settings (`connect::client`), so a crash it
/// causes cannot loop and lock out an update over the LAN.
#[cfg(feature = "connect")]
pub fn crash_streak() -> ring::Streak {
    ring::crash_streak_in(&Rtc, BOOT_SEQ.load(Relaxed), reset_now())
}

/// Why this boot's restart was made, in plain words (`reset_note`, when
/// `sdhealth` has none); `None` after a power-on.
pub fn reset_note() -> Option<&'static str> { #[cfg(esp32s3)] if let Some(n) = crate::s3board::reset_note() { return Some(n); } // (power-safe first; on this line: s3board.rs)
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
        if e.activity.is_known() {
            let _ = write!(out, r#","activity":"{}""#, e.activity);
        }
        out.push('}');
    }
    out.push(']');
    out
}

/// The S3's own reset causes (`reset_now`): its USB-Serial-JTAG and JTAG
/// resets (a host opening the port, a debugger) are the reset button's kind,
/// the ring kept; a power glitch is the supply's, like a brownout. The
/// classic reports none of them. (At the end of the file, so the classic's
/// log lines keep their numbers.)
#[cfg(esp32s3)]
#[allow(non_upper_case_globals)]
fn s3_reset(r: sys::esp_reset_reason_t) -> Reset {
    match r {
        sys::esp_reset_reason_t_ESP_RST_PWR_GLITCH => Reset::Brownout,
        _ => Reset::External,
    }
}
