//! Console logging off the display core.
//!
//! A UART line at 115200 baud costs ~87 µs a byte and the console write
//! blocks its caller: a heartbeat (~850 B) held the display loop ~70 ms and
//! cost a frame deadline every time it printed, which is why the loop used
//! to defer its lines until after a frame's sleep. Now every line — the
//! Rust `log` macros ([`RingLogger`]) and ESP-IDF's own `ESP_LOGx` (the
//! [`esp_log_set_vprintf`] hook, [`vprintf_hook`]) — is formatted by its
//! caller straight into a fixed ring, and a low-priority task on CPU0 (the
//! FreeRTOS timer service task, running [`drain`] every 5 ms) writes the
//! ring to the console. A caller never waits for the UART.
//!
//! - **The ring is [`RING`] bytes of RTC slow memory** (`.rtc_noinit`, 8 KB
//!   on this chip and otherwise unused but for the SD test hook's 8 B): no
//!   heap and no DRAM. Both cores can reach it; only plain loads and stores
//!   touch it (the one compare-and-swap, the writers' spinlock, is a DRAM
//!   `portMUX`).
//! - **Lines stay whole.** A writer measures its line (a counting pass over
//!   the format), reserves that many contiguous bytes under a spinlock (a
//!   few dozen instructions), formats into them outside it, then marks the
//!   record committed. The drain prints committed records in reservation
//!   order and waits at the first one still being written.
//! - **Full is a drop, never a wait**: the newest line is dropped and
//!   counted ([`drops`]; `log_drops` in `/api/status`), and the drain prints
//!   `N log lines dropped` once it has caught up.
//! - **Raw records** ([`raw`]) carry bytes that must reach the wire as
//!   they are: Improv Serial frames (`improv.rs`), which the browser
//!   installer reads only between whole lines. The drain writes them
//!   straight to the UART's FIFO (`improv::uart_raw`), without the console
//!   VFS's `\n` → `\r\n`, in their place among the lines.
//! - **Timestamps are the caller's** (`esp_log_timestamp` at format time:
//!   IDF's macros take it before calling us, [`RingLogger`] as it formats),
//!   so the capture's order and times stay true across tasks.
//! - **Before [`start`], lines are written synchronously** as before (the
//!   boot banner, the card mount, the index), and so are panic messages and
//!   aborts, which never come through here: Rust's panic output goes to
//!   stderr and IDF's panic handler prints through the ROM. A Rust panic
//!   first prints whatever the ring still holds ([`start`]'s hook), so the
//!   lines leading up to it are not lost.

use core::ffi::{c_char, c_int};
use core::fmt::{self, Write as _};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use esp_idf_hal::interrupt::IsrCriticalSection;
use esp_idf_sys as sys;

/// The ring's size in bytes (a power of two; lines longer than half of it
/// are cut).
pub const RING: usize = 4096;
/// The longest line kept whole; the rest of a longer one is cut.
const MAX_LINE: usize = 1536;
/// A record: an 8-byte header — state (byte 0), kind (byte 1: [`TEXT`] or
/// [`RAW`]), the reserved payload capacity (bytes 2-3), the written length
/// (bytes 4-5) — then the payload,
/// the whole padded to 8 bytes, so a gap at the end of the ring always has
/// room for a filler header.
const HDR: usize = 8;
const WRITING: u8 = 0;
const COMMITTED: u8 = 1;
/// Filler: the rest of the ring up to its end is unused; go to 0.
const WRAP: u8 = 2;
/// Record kinds: a log line (through the console VFS), raw bytes
/// (straight to the UART).
const TEXT: u8 = 0;
const RAW: u8 = 1;

#[repr(C, align(8))]
struct Buf([u8; RING]);

// RTC slow memory keeps its contents across a software reset and is garbage
// after power-on; either way nothing is read before it is written.
#[link_section = ".rtc_noinit"]
static mut BUF: Buf = Buf([0; RING]);

/// Bytes ever reserved (writers, under [`LOCK`]) and ever released (the
/// drain). `TAIL - HEAD` is the ring's fill; both wrap at 2^32, which is a
/// multiple of [`RING`].
static TAIL: AtomicU32 = AtomicU32::new(0);
static HEAD: AtomicU32 = AtomicU32::new(0);
static LOCK: IsrCriticalSection = IsrCriticalSection::new();
static DROPS: AtomicU32 = AtomicU32::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);
/// The ring's highest fill seen by the drain, bytes.
static HIGH: AtomicU32 = AtomicU32::new(0);
/// The timer service task's stack high-water mark, as the drain last saw
/// it (bytes never used).
static STACK_LOW: AtomicU32 = AtomicU32::new(u32::MAX);

/// The ring's first byte. Only the address is taken here; every access goes
/// through the reservation protocol below.
fn base() -> *mut u8 {
    (&raw mut BUF).cast::<u8>()
}

/// The state byte of the record at ring offset `at`.
fn state(at: usize) -> &'static AtomicU8 {
    // SAFETY: `at` is a record start inside the ring (8-aligned), and a
    // state byte is only ever accessed through this atomic view.
    unsafe { AtomicU8::from_ptr(base().add(at)) }
}

fn put_u16(at: usize, v: u16) {
    // SAFETY: inside the ring; the header is the reserving writer's until
    // it commits.
    unsafe { core::ptr::copy_nonoverlapping(v.to_le_bytes().as_ptr(), base().add(at), 2) }
}

fn set_kind(at: usize, kind: u8) {
    // SAFETY: inside the ring; the header is the reserving writer's.
    unsafe { *base().add(at + 1) = kind }
}

fn kind(at: usize) -> u8 {
    // SAFETY: inside the ring, a committed record's header.
    unsafe { *base().add(at + 1) }
}

fn get_u16(at: usize) -> u16 {
    let mut b = [0u8; 2];
    // SAFETY: inside the ring, a committed record's header.
    unsafe { core::ptr::copy_nonoverlapping(base().add(at), b.as_mut_ptr(), 2) };
    u16::from_le_bytes(b)
}

const fn padded(len: usize) -> usize {
    (HDR + len + 7) & !7
}

/// A reserved record: `cap` bytes of payload at ring offset `at + HDR`,
/// of which `len` are written. Dropping it commits it — also if the
/// formatting panicked, so one bad line cannot stall the drain.
struct Slot {
    at: usize,
    cap: usize,
    /// The line's full length (it was cut if that is over `cap`).
    want: usize,
    len: usize,
}

impl Slot {
    fn payload(&self) -> &'static mut [u8] {
        // SAFETY: the reservation made these bytes this writer's alone until
        // the commit; they are inside the ring (reservations never wrap).
        unsafe { core::slice::from_raw_parts_mut(base().add(self.at + HDR), self.cap) }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let len = self.len.min(self.cap);
        if len > 0 && len < self.want {
            // Cut short: keep the line a line.
            self.payload()[len - 1] = b'\n';
        }
        put_u16(self.at + 4, len as u16);
        state(self.at).store(COMMITTED, Ordering::Release);
    }
}

/// Reserve room for a `len`-byte line, or count a drop.
fn reserve(len: usize) -> Option<Slot> {
    let cap = len.min(MAX_LINE);
    let need = padded(cap);
    let _cs = LOCK.enter();
    let tail = TAIL.load(Ordering::Relaxed);
    let used = tail.wrapping_sub(HEAD.load(Ordering::Acquire)) as usize;
    let at = tail as usize % RING;
    let to_end = RING - at;
    let (skip, at) = if to_end < need { (to_end, 0) } else { (0, at) };
    if used + skip + need > RING {
        DROPS.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    if skip > 0 {
        state(tail as usize % RING).store(WRAP, Ordering::Release);
    }
    state(at).store(WRITING, Ordering::Relaxed);
    set_kind(at, TEXT);
    put_u16(at + 2, cap as u16);
    TAIL.store(tail.wrapping_add((skip + need) as u32), Ordering::Release);
    Some(Slot {
        at,
        cap,
        want: len,
        len: 0,
    })
}

/// The next committed record's payload (and whether it is [`RAW`]), or
/// `None` if the ring is empty or the oldest record is still being
/// written. Drain side only.
fn peek() -> Option<(u32, &'static [u8], usize, bool)> {
    loop {
        let head = HEAD.load(Ordering::Relaxed);
        if head == TAIL.load(Ordering::Acquire) {
            return None;
        }
        let at = head as usize % RING;
        match state(at).load(Ordering::Acquire) {
            WRAP => {
                HEAD.store(head.wrapping_add((RING - at) as u32), Ordering::Release);
            }
            COMMITTED => {
                let len = get_u16(at + 4) as usize;
                let size = padded(get_u16(at + 2) as usize);
                // SAFETY: a committed record is immutable until HEAD passes it.
                let bytes = unsafe { core::slice::from_raw_parts(base().add(at + HDR), len) };
                return Some((head, bytes, size, kind(at) == RAW));
            }
            _ => return None,
        }
    }
}

fn release(head: u32, size: usize) {
    HEAD.store(head.wrapping_add(size as u32), Ordering::Release);
}

/// Lines dropped because the ring was full, since boot.
pub fn drops() -> u32 {
    DROPS.load(Ordering::Relaxed)
}

/// The heartbeat's part: `log: N dropped, ring high N B, drain stack N B
/// unused`, and `console at 115200 baud (Improv)` once a browser installer
/// switched it (`improv.rs`).
pub fn summary() -> String {
    let stack = match STACK_LOW.load(Ordering::Relaxed) {
        u32::MAX => "-".to_string(),
        n => n.to_string(),
    };
    format!(
        ", log: {} dropped, ring high {} B, drain stack {stack} B unused{}",
        drops(),
        HIGH.load(Ordering::Relaxed),
        if crate::improv::switched() {
            ", console at 115200 baud (Improv)"
        } else {
            ""
        }
    )
}

/// Write `bytes` to the console (fd 1, the UART through the VFS): one
/// `write` per line, which the UART driver's lock keeps whole against any
/// other writer. Blocks for the UART. (Straight to the fd rather than
/// through stdio's `fwrite` + `fflush`, as `EspLogger` wrote: the drain
/// runs on the timer service task's 2 KB stack, which that left 616 B of;
/// this leaves 824 B.)
fn console(bytes: &[u8]) {
    let mut rest = bytes;
    while !rest.is_empty() {
        // SAFETY: a plain write of a valid buffer to the console fd.
        let n = unsafe { sys::write(1, rest.as_ptr().cast(), rest.len()) };
        if n <= 0 {
            return;
        }
        rest = &rest[n as usize..];
    }
}

/// A record to the wire: a line through the console, raw bytes straight to
/// the UART.
/// The drain's last byte out was a `\n` (or nothing went out yet): raw
/// records must start a line, and a log line cut short at `MAX_LINE` does
/// not end with one.
static AT_LINE_START: AtomicBool = AtomicBool::new(true);

fn out(bytes: &[u8], raw: bool) {
    let Some(&last) = bytes.last() else { return };
    if raw {
        if !AT_LINE_START.load(Ordering::Relaxed) && bytes[0] != b'\n' {
            crate::improv::uart_raw(b"\n");
        }
        crate::improv::uart_raw(bytes);
    } else {
        console(bytes);
    }
    AT_LINE_START.store(last == b'\n', Ordering::Relaxed);
}

/// Queue `len` bytes to go to the UART exactly as `fill` writes them (it
/// gets a `len`-byte buffer and returns how many it wrote), in order with
/// the log lines: Improv Serial frames. Dropped and counted like a line if
/// the ring is full. Before [`start`], written at once.
pub fn raw(len: usize, fill: &mut dyn FnMut(&mut [u8]) -> usize) {
    if len == 0 || len > MAX_LINE {
        return;
    }
    if !STARTED.load(Ordering::Acquire) {
        let mut buf = vec![0u8; len];
        let n = fill(&mut buf).min(len);
        crate::improv::uart_raw(&buf[..n]);
        return;
    }
    let Some(mut slot) = reserve(len) else { return };
    set_kind(slot.at, RAW);
    let n = fill(slot.payload()).min(slot.cap);
    slot.len = n;
    slot.want = n;
}

/// [`console`] as a `fmt::Write`, for the lines before [`start`].
struct Console;

impl fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        console(s.as_bytes());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Writers
// ---------------------------------------------------------------------------

/// Counts what a format would write.
struct Count(usize);

impl fmt::Write for Count {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0 += s.len();
        Ok(())
    }
}

/// Writes into a slice, cutting what does not fit.
struct Fill<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl fmt::Write for Fill<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let n = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// The Rust `log` backend: the same line `EspLogger` printed (colour, level
/// letter, `esp_log_timestamp`, target), into the ring once [`start`] has
/// run, straight to the console before.
pub struct RingLogger;

pub static LOGGER: RingLogger = RingLogger;

/// Install [`LOGGER`] as the `log` backend (synchronous until [`start`]).
/// The level is IDF's default (`CONFIG_LOG_DEFAULT_LEVEL`, info), as
/// `EspLogger` applied it: nothing here sets per-target levels.
pub fn init() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(match sys::CONFIG_LOG_DEFAULT_LEVEL {
            0 => log::LevelFilter::Off,
            1 => log::LevelFilter::Error,
            2 => log::LevelFilter::Warn,
            3 => log::LevelFilter::Info,
            4 => log::LevelFilter::Debug,
            _ => log::LevelFilter::Trace,
        });
    }
}

fn marker(level: log::Level) -> (&'static str, &'static str, &'static str) {
    let colours = sys::CONFIG_LOG_COLORS != 0;
    let (m, c) = match level {
        log::Level::Error => ("E", "\x1b[0;31m"),
        log::Level::Warn => ("W", "\x1b[0;33m"),
        log::Level::Info => ("I", "\x1b[0;32m"),
        log::Level::Debug => ("D", ""),
        log::Level::Trace => ("V", ""),
    };
    match (colours, c.is_empty()) {
        (true, false) => (c, m, "\x1b[0m"),
        _ => ("", m, ""),
    }
}

impl log::Log for RingLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let (pre, m, post) = marker(record.level());
        // SAFETY: argument-free read of the log clock.
        let ts = unsafe { sys::esp_log_timestamp() };
        let line = format_args!(
            "{pre}{m} ({ts}) {}: {}{post}\n",
            record.target(),
            record.args()
        );
        if !STARTED.load(Ordering::Acquire) {
            // Before the drain exists: synchronously, as EspLogger did.
            let _ = Console.write_fmt(line);
            return;
        }
        let mut n = Count(0);
        let _ = n.write_fmt(line);
        let Some(mut slot) = reserve(n.0) else { return };
        let mut w = Fill {
            buf: slot.payload(),
            len: 0,
        };
        let _ = w.write_fmt(line);
        slot.len = w.len;
    }

    fn flush(&self) {}
}

extern "C" {
    fn vsnprintf(buf: *mut c_char, n: usize, fmt: *const c_char, ap: sys::va_list) -> c_int;
}

/// IDF's log output (`esp_log_set_vprintf`): the line is formatted by the
/// caller into the ring. IDF formats a whole `ESP_LOGx` line — colour,
/// level, timestamp, tag, message, newline — in one call.
unsafe extern "C" fn vprintf_hook(fmt: *const c_char, ap: sys::va_list) -> c_int {
    // Xtensa's va_list is a three-word struct passed by value: each call
    // below walks its own copy of the arguments.
    let n = vsnprintf(core::ptr::null_mut(), 0, fmt, ap);
    if n <= 0 {
        return n;
    }
    // One more byte for vsnprintf's terminating NUL, which is not kept.
    let Some(mut slot) = reserve(n as usize + 1) else {
        return n;
    };
    slot.want = n as usize;
    let buf = slot.payload();
    let written = vsnprintf(buf.as_mut_ptr().cast(), buf.len(), fmt, ap);
    slot.len = (written.max(0) as usize).min(buf.len() - 1);
    n
}

// ---------------------------------------------------------------------------
// The drain
// ---------------------------------------------------------------------------

/// How often the drain looks at the ring, in FreeRTOS ticks (1 ms).
const PERIOD_TICKS: u32 = 5;
/// A drain call returns after writing this long (the UART write busy-waits
/// on its FIFO); the next one carries on, so IDLE0 gets its turn between.
const SLICE: Duration = Duration::from_millis(20);
/// FreeRTOS's `tmrCOMMAND_START` (`xTimerStart` is a macro).
const TMR_COMMAND_START: i32 = 1;

/// The drain: a periodic FreeRTOS software timer, so it runs on the timer
/// service task ("Tmr Svc": priority 1, pinned to CPU0 by
/// `CONFIG_FREERTOS_TIMER_TASK_AFFINITY_CPU0`, its 2 KB stack already
/// there) and costs no task of its own — the heap a task would take
/// (~2.9 KB) is heap the cache builder's night start threshold does not
/// have. Nothing else in this firmware uses FreeRTOS timers (the IDF and
/// esp-idf-svc timers are `esp_timer`s), so a callback that writes to the
/// UART for up to [`SLICE`] delays no one.
extern "C" fn drain(_: sys::TimerHandle_t) {
    static REPORTED: AtomicU32 = AtomicU32::new(0);
    let started = Instant::now();
    // Improv Serial: anything a browser installer sent (answers go into the
    // ring, so they go out below, this same call).
    crate::improv::poll();
    let fill = TAIL
        .load(Ordering::Relaxed)
        .wrapping_sub(HEAD.load(Ordering::Relaxed));
    HIGH.fetch_max(fill, Ordering::Relaxed);
    let mut wrote = false;
    while let Some((head, bytes, size, raw)) = peek() {
        out(bytes, raw);
        release(head, size);
        wrote = true;
        if started.elapsed() >= SLICE {
            break;
        }
    }
    let dropped = DROPS.load(Ordering::Relaxed);
    let reported = REPORTED.load(Ordering::Relaxed);
    if dropped != reported {
        let mut line = [0u8; 112];
        let mut w = Fill {
            buf: &mut line,
            len: 0,
        };
        let (pre, m, post) = marker(log::Level::Warn);
        // SAFETY: argument-free read of the log clock.
        let ts = unsafe { sys::esp_log_timestamp() };
        let _ = writeln!(
            w,
            "{pre}{m} ({ts}) opendmd_firmware::logring: {} log lines dropped ({} since boot){post}",
            dropped.wrapping_sub(reported),
            dropped
        );
        let len = w.len;
        console(&line[..len]);
        REPORTED.store(dropped, Ordering::Relaxed);
    }
    if wrote {
        // SAFETY: a query about the calling task (the timer service task);
        // a scan of its unused stack, so only after work.
        let unused = unsafe { sys::uxTaskGetStackHighWaterMark(core::ptr::null_mut()) };
        STACK_LOW.store(unused, Ordering::Relaxed);
    }
}

/// Start the drain, then route the Rust logger and IDF's `ESP_LOGx`
/// through the ring. Call once, early (before the network), on the main
/// task. On failure logging simply stays synchronous.
pub fn start() {
    // SAFETY: a static callback, a NUL-terminated literal name, no ID.
    let started = unsafe {
        let timer = sys::xTimerCreate(
            c"log-drain".as_ptr(),
            PERIOD_TICKS,
            1,
            core::ptr::null_mut(),
            Some(drain),
        );
        !timer.is_null()
            && sys::xTimerGenericCommand(
                timer,
                TMR_COMMAND_START,
                sys::xTaskGetTickCount(),
                core::ptr::null_mut(),
                u32::MAX,
            ) == 1
    };
    if !started {
        log::warn!("log drain timer did not start; logging stays synchronous");
        return;
    }
    STARTED.store(true, Ordering::Release);
    // SAFETY: installs a function with the vprintf_like_t signature.
    unsafe { sys::esp_log_set_vprintf(Some(vprintf_hook)) };
    // A Rust panic prints what the ring still holds first, synchronously.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        flush_now();
        previous(info);
    }));
    log::info!(
        "logging: {RING} B ring in RTC slow memory, drained to the console every {PERIOD_TICKS} ms by a timer on CPU0's timer service task"
    );
}

/// Print every committed line now, on the caller (a panic). Best effort:
/// the drain may print the record it was on a second time.
fn flush_now() {
    STARTED.store(false, Ordering::Release);
    while let Some((head, bytes, size, raw)) = peek() {
        out(bytes, raw);
        release(head, size);
    }
}

/// Wait, on the caller, until the drain has written every line logged so
/// far (and the UART's FIFO has emptied), for at most `timeout`: what a
/// deliberate restart does first so its reason reaches the console
/// (`sdhealth`'s card-failure restart). `false` if it timed out. Before
/// [`start`] every line was written synchronously, so there is nothing to
/// wait for.
pub fn flush(timeout: Duration) -> bool {
    if !STARTED.load(Ordering::Acquire) {
        return true;
    }
    let t = Instant::now();
    let tail = TAIL.load(Ordering::Acquire);
    // Everything reserved up to now has been released by the drain.
    while (tail.wrapping_sub(HEAD.load(Ordering::Acquire)) as i32) > 0 {
        if t.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // The drain's last write left up to a FIFO's worth (128 B, ~1.4 ms at
    // 921600 baud, ~11 ms at 115200 after an installer switched it) still
    // shifting out.
    std::thread::sleep(Duration::from_millis(20));
    true
}
