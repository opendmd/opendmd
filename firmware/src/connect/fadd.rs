//! A Formula added by a remote controller (`formula-add`, `formula-part`;
//! protocol 5.7): the card's half of `opendmd_core::connect::fadd`. Built
//! only with `connect-formula-add` (test images: never a release).
//!
//! **Only signed Formulas.** The core's parser checks the gallery's
//! signature over the add's argument stem, sizes and hashes
//! (`fadd::admit`, `gallery_sig::check`) right after the argument rules and
//! before anything here runs; [`Client::formula_add`] and the landing take
//! the [`Signed`] witness only that check makes, and the core holds an add
//! only with a witness over exactly its bytes. Any refusal is `not-allowed`.
//!
//! - **Before any card read:** the add's own rate, the same add signed
//!   again (no card read at all), another key's add under way, the adds
//!   started in the last day of uptime (16, in RAM), a LAN transfer or
//!   update under way (`busy`).
//! - **One directory pass** (no lookup per name): the Formulas on the card
//!   (at most [`fadd::MAX_FORMULAS`] sources, else `quota`) and the taken
//!   stems (a `.psi` or `.psb`); where it lands is the stem or the
//!   next free `-2` … `-9`. A taken stem whose `.psi` has the signed
//!   source's length is hashed, and if it is the signed source it is the
//!   same Formula: `done`, nothing written.
//! - **The bytes** go to `/formulas/.add-<id>.psi.part` and `.psb.part`
//!   (dot names: never listed or played; gone when the add is dropped or
//!   stale, and at boot).
//! - **The last part:** both hashes (the core's), the source UTF-8 with no
//!   NUL, the program `PSB` + this ABI and every program in it loaded and
//!   checked (`psb::verify`), behind a heap gate (the bytes, one program, 20
//!   KB for the rest; below it, or a load short of heap, `busy`: the same
//!   `formula-add` signed again tries again).
//! - **Landing:** the compiled file renamed to `<stem>.psb` first, then the
//!   source to `<stem>.psi` (a rename never replaces a file); a failed
//!   second rename removes the first. Then the `.psb` is given its source's
//!   FAT date (`f_utime`, as `card_formulas` dates its own), so the mount
//!   pass takes the signed program as current whatever the clock did during
//!   the add; it plays (on the JIT where it takes it) at once. A restart
//!   between the renames leaves a `.psb` with no source: never listed or
//!   played (both go by the `.psi`).

use super::client::{core_now, Client};
use crate::config;
use opendmd_core::connect::fadd::{self, AddArgs, Part, Pre, Signed};
use opendmd_core::connect::keys::KeyId;
use opendmd_core::connect::proto::Refusal;
use std::io::{Read, Write};

type Done = Result<Option<String>, Refusal>;

fn dir() -> String {
    format!("{}/formulas", config::SD_MOUNT)
}

fn part_path(id: u32, p: Part) -> String {
    format!("{}/.add-{id}.{}.part", dir(), if p == Part::Source { "psi" } else { "psb" })
}

fn remove_parts(id: u32) {
    let _ = std::fs::remove_file(part_path(id, Part::Source));
    let _ = std::fs::remove_file(part_path(id, Part::Psb));
}

/// A LAN transfer (an upload, a stream, an update's) shares the card.
fn lan_busy() -> bool {
    crate::cache_builder::transfer_in_flight() || crate::stream::uploads() > 0
}

/// The add's directory pass: the stems taken (lower case) and how many
/// Formulas there are, with each taken `.psi`'s size.
fn scan() -> Option<(Vec<(String, u32)>, usize)> {
    use esp_idf_sys as sys;
    let path = std::ffi::CString::new("0:/formulas").ok()?;
    let mut taken = Vec::new();
    let mut sources = 0usize;
    // SAFETY: plain data FatFs fills, the directory closed before return.
    unsafe {
        let mut d: sys::FF_DIR = std::mem::zeroed();
        #[cfg(feature = "connect-test")]
        crate::cardspeed::worker_io::note_dir();
        let r = sys::f_opendir(&mut d, path.as_ptr());
        if r == sys::FRESULT_FR_NO_PATH {
            return Some((taken, 0));
        }
        if r != sys::FRESULT_FR_OK {
            return None;
        }
        let mut fi: sys::FILINFO = std::mem::zeroed();
        while sys::f_readdir(&mut d, &mut fi) == sys::FRESULT_FR_OK && fi.fname[0] != 0 {
            let Ok(name) = std::ffi::CStr::from_ptr(fi.fname.as_ptr()).to_str() else { continue };
            let l = name.to_ascii_lowercase();
            for ext in [".psi", ".psb"] {
                if let Some(st) = l.strip_suffix(ext) {
                    // (Only a `.psi`'s size matters: 0 for the others.)
                    taken.push((st.to_string(), if ext == ".psi" { fi.fsize as u32 } else { u32::MAX }));
                    if ext == ".psi" {
                        sources += 1;
                    }
                }
            }
            // (Bounded: past the cap the add is refused anyway.)
            if taken.len() > 3 * fadd::MAX_FORMULAS + 64 {
                break;
            }
        }
        sys::f_closedir(&mut d);
    }
    Some((taken, sources))
}

/// Where an add lands.
enum Target {
    /// A free stem: a new Formula.
    New(String),
    /// The same source is there, with a current `.psb`: done, nothing written.
    There(String),
    /// The same source is there but its `.psb` is missing or not current
    /// (not dated, a mark, another format): only the `.psb` is put back.
    Refill(String),
}

/// Whether `stem`'s `.psb` is current for its `.psi`: there, not a mark or
/// empty, this player's format, and not older than its source (FAT dates).
fn psb_current(stem: &str) -> bool {
    use esp_idf_sys as sys;
    let c = |e: &str| std::ffi::CString::new(format!("0:/formulas/{stem}.{e}")).ok();
    let (Some(src), Some(psb)) = (c("psi"), c("psb")) else { return false };
    // SAFETY: FILINFOs FatFs fills, NUL-terminated paths.
    let dated = unsafe {
        let (mut a, mut b): (sys::FILINFO, sys::FILINFO) = (std::mem::zeroed(), std::mem::zeroed());
        sys::f_stat(src.as_ptr(), &mut a) == sys::FRESULT_FR_OK
            && sys::f_stat(psb.as_ptr(), &mut b) == sys::FRESULT_FR_OK
            && b.fsize > 12
            && ((u32::from(b.fdate) << 16) | u32::from(b.ftime)) >= ((u32::from(a.fdate) << 16) | u32::from(a.ftime))
    };
    let mut h = [0u8; 4];
    dated
        && std::fs::File::open(format!("{}/{stem}.psb", dir())).and_then(|mut f| f.read_exact(&mut h)).is_ok()
        && &h[..3] == b"PSB"
        && h[3] == opendmd_core::psi::psb::ABI
}

/// Where `args` lands. Every taken stem among the stem and `-2` … `-9`
/// whose source has the signed length is hashed (at most nine), so the same
/// Formula is found wherever it landed before and never lands twice; else
/// the first free stem. The card is the add's for the pass
/// (`cache_builder::busy`).
fn target(args: &AddArgs) -> Result<Target, Refusal> {
    let _busy = crate::cache_builder::busy();
    let (taken, sources) = scan().ok_or(Refusal::Busy)?;
    if sources >= fadd::MAX_FORMULAS {
        return Err(Refusal::Quota);
    }
    log::info!("formula-add: looked on the card for {}", args.stem);
    let mut free = None;
    for stem in fadd::stem_choices(&args.stem) {
        let here: Vec<u32> = taken.iter().filter(|(s, _)| *s == stem).map(|&(_, n)| n).collect();
        if here.is_empty() {
            free = free.or(Some(stem));
            continue;
        }
        if here.contains(&args.src_size) {
            let f = std::fs::File::open(format!("{}/{stem}.psi", dir())).map_err(|_| Refusal::Busy)?;
            if fadd::sha256_of(f).ok() == Some(args.src_sha256) {
                return Ok(if psb_current(&stem) { Target::There(stem) } else { Target::Refill(stem) });
            }
        }
    }
    free.map(Target::New).ok_or(Refusal::Quota)
}

fn done(stem: &str) -> Option<String> {
    Some(format!(r#"{{"done":true,"stem":{}}}"#, opendmd_core::connect::json::quote(stem)))
}

/// What the source part is, read back from the card: `None` not UTF-8 or
/// with a NUL (or unreadable); else its length and SHA-256 (read in pieces).
fn source_read(path: &str) -> Option<(u32, [u8; 32])> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = fadd::Hash::default();
    let mut b = [0u8; 516];
    let (mut carry, mut total) = (0usize, 0u32);
    loop {
        let n = f.read(&mut b[carry..512]).ok()?;
        let len = carry + n;
        if n == 0 {
            return (carry == 0).then(|| (total, h.finish()));
        }
        h.update(&b[carry..len]);
        total += n as u32;
        if b[carry..len].contains(&0) {
            return None;
        }
        match std::str::from_utf8(&b[..len]) {
            Ok(_) => carry = 0,
            Err(e) if e.error_len().is_none() => {
                let ok = e.valid_up_to();
                carry = len - ok;
                b.copy_within(ok..len, 0);
            }
            Err(_) => return None,
        }
    }
}

/// The program: this player's format, every program in it loaded and
/// checked as anything from the network is (`psb::verify`). `Err`: the
/// heap had no room for a program (try again), not a refusal.
fn psb_ok(psb: &[u8]) -> Result<bool, ()> {
    use opendmd_core::psi::psb::{load_boxed, Which, ABI, NO_HEAP};
    if psb.len() < 6 || &psb[..3] != b"PSB" || psb[3] != ABI {
        return Ok(false);
    }
    for w in [Which::Play, Which::Look] {
        match load_boxed(psb, w) {
            Ok(p) => drop(p),
            Err(e) if e == NO_HEAP => return Err(()),
            Err(_) => return Ok(false),
        }
    }
    Ok(true)
}

/// The heap a check needs: the bytes, one program (its box is ~10.5 KB at
/// most) in one block, and what CPU0's other card work leaves for the
/// player and the network (`card_formulas`' 20 KB).
fn room(psb_size: usize) -> bool {
    crate::state::free_heap() as usize >= psb_size + CHECK_STACK + 12 * 1024 + 20 * 1024 && crate::state::largest_free_block() >= 12 * 1024
}

/// Give `psb` its source `psi`'s FAT date (`f_stat`, `f_utime`: no clock
/// involved). False: it could not.
fn date_like(psb: &str, psi: &str) -> bool {
    use esp_idf_sys as sys;
    let c = |p: &str| std::ffi::CString::new(format!("0:{}", p.strip_prefix(config::SD_MOUNT).unwrap_or(p))).ok();
    let (Some(src), Some(dst)) = (c(psi), c(psb)) else { return false };
    // SAFETY: a FILINFO FatFs fills, two NUL-terminated paths.
    unsafe {
        let mut fi: sys::FILINFO = std::mem::zeroed();
        sys::f_stat(src.as_ptr(), &mut fi) == sys::FRESULT_FR_OK && sys::f_utime(dst.as_ptr(), &fi) == sys::FRESULT_FR_OK
    }
}

/// The last plain command's check (µs), for the add's log line.
static PLAIN_CHECK_US: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// A board measurement (the log, any build with adds): how long an add's
/// command took to check, its own signature and the gallery's one after the
/// other, beside a plain command's (one signature).
pub(super) fn note_check_us(cmd: &opendmd_core::connect::proto::Command, us: u32) {
    use std::sync::atomic::Ordering::Relaxed;
    if matches!(cmd, opendmd_core::connect::proto::Command::FormulaAdd(..)) {
        // (Logged here, at the worker's top level, never inside the check:
        // a log line's formatting has its own stack depth.)
        let (vus, left) = (LAST_VERIFY_US.load(Relaxed), LAST_VERIFY_LEFT.load(Relaxed));
        if vus != 0 {
            log::info!("formula-add: the gallery's signature checked on a task of its own in {vus} us ({left} B of its stack left)");
        }
        log::info!("formula-add: checked in {us} us (the command's signature, then the gallery's; a plain command's check: {} us)", PLAIN_CHECK_US.load(Relaxed));
    } else {
        PLAIN_CHECK_US.store(us, Relaxed);
    }
    let gave_up = GAVE_UP.swap(0, Relaxed);
    if gave_up != 0 {
        log::warn!("formula-add: {gave_up} gallery check(s) not run or not waited for (no room, or over {VERIFY_WAIT_MS} ms): busy");
    }
}

/// The last verify's time (µs, 0: none since an add was taken) and its
/// task's stack left; verifies run, and ones not run or given up (logged by
/// [`note_check_us`], at the worker's top level).
static LAST_VERIFY_US: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static LAST_VERIFY_LEFT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static VERIFIES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static GAVE_UP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Gallery verifies run since boot (the emulator lane's flood count).
pub fn verifies() -> u32 {
    VERIFIES.load(std::sync::atomic::Ordering::Relaxed)
}

/// The gallery signature check's task's stack: an Ed25519 verify keeps 7
/// to 8.5 KB of frames (`ota.rs`), with room above it.
const VERIFY_STACK: usize = 11 * 1024;
/// The heap a verify's task is started with at least: its stack, 4 KB for
/// the rest it takes (its task's control block, the semaphore, the shared
/// run, the job) and the network's transients meanwhile, and the 20 KB the
/// player sheds its read-ahead under (`config::AHEAD_YIELD_HEAP`): the
/// heap never goes under that line for a verify.
const VERIFY_HEAP: usize = VERIFY_STACK + 4 * 1024 + crate::config::AHEAD_YIELD_HEAP;
/// How long the network worker waits for a verify (30 to 50 ms on the
/// board): past it, `busy`, and the task finishes on its own.
const VERIFY_WAIT_MS: u32 = 1000;

/// One verify's task and its waiter: everything the task touches is owned
/// here (the job, the result, the semaphore), shared by the two through an
/// `Arc`, so the waiter may give up and return while the task runs on.
struct VerifyRun {
    job: std::sync::Mutex<Option<Box<fadd::VerifyJob>>>,
    out: std::sync::Mutex<Option<Result<Signed, Refusal>>>,
    left: std::sync::atomic::AtomicU32,
    done: esp_idf_sys::QueueHandle_t,
}
// SAFETY: the semaphore handle is used from both tasks through FreeRTOS's
// own calls, which are thread-safe; the rest is Mutex/atomic.
unsafe impl Send for VerifyRun {}
unsafe impl Sync for VerifyRun {}

impl Drop for VerifyRun {
    fn drop(&mut self) {
        // SAFETY: the last owner; neither task uses the semaphore again.
        unsafe { esp_idf_sys::vQueueDelete(self.done) };
    }
}

unsafe extern "C" fn verify_entry(arg: *mut core::ffi::c_void) {
    use std::sync::atomic::Ordering::Relaxed;
    // SAFETY: `arg` is one `Arc<VerifyRun>` reference given to this task.
    let run = unsafe { std::sync::Arc::from_raw(arg as *const VerifyRun) };
    let job = run.job.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some(job) = job {
        let r = job.run();
        *run.out.lock().unwrap_or_else(|p| p.into_inner()) = Some(r);
    }
    // SAFETY: this task's own high-water mark; a give of the binary semaphore.
    unsafe {
        run.left.store(esp_idf_sys::uxTaskGetStackHighWaterMark(core::ptr::null_mut()), Relaxed);
        esp_idf_sys::xQueueGenericSend(run.done, core::ptr::null(), 0, 0);
    }
    drop(run);
    // SAFETY: a task ends by deleting itself (the idle task frees it).
    unsafe { esp_idf_sys::vTaskDelete(core::ptr::null_mut()) };
}

/// The core's [`fadd::Runner`]: the gallery's signature check on a task of
/// its own, never on the network worker's stack.
///
/// - **Heap:** started only with [`VERIFY_HEAP`] free and a block for its
///   stack, else `busy` (the core's cheap refusals and its verify rates
///   have run before anything is spent).
/// - **Priority and core:** the network worker's own priority, pinned to
///   its core (CPU0): above the cache builder (1), level with httpd, below
///   only WiFi and lwIP, which block on I/O. So the verify, CPU work with no
///   I/O and no lock, is never starved behind the work the worker yields
///   to, and the worker's wait is the verify's own time.
/// - **A bounded wait:** [`VERIFY_WAIT_MS`]; past it `busy`. The task owns
///   all it touches (`VerifyRun`, shared by `Arc`): nothing of the
///   worker's stack is borrowed, so giving up is safe, and the task frees
///   itself when it ends.
#[inline(never)]
fn run_verify(job: Box<fadd::VerifyJob>) -> Option<Result<Signed, Refusal>> {
    use esp_idf_sys as sys;
    use std::sync::atomic::Ordering::Relaxed;
    if (crate::state::free_heap() as usize) < VERIFY_HEAP || crate::state::largest_free_block() < VERIFY_STACK {
        GAVE_UP.fetch_add(1, Relaxed);
        return None;
    }
    // SAFETY: an empty binary semaphore (queue type 3, no items).
    let done = unsafe { sys::xQueueGenericCreate(1, 0, 3) };
    if done.is_null() {
        return None;
    }
    let run = std::sync::Arc::new(VerifyRun {
        job: std::sync::Mutex::new(Some(job)),
        out: std::sync::Mutex::new(None),
        left: std::sync::atomic::AtomicU32::new(0),
        done,
    });
    let t = std::time::Instant::now();
    let given = std::sync::Arc::into_raw(run.clone()) as *mut core::ffi::c_void;
    // SAFETY: a query about the calling task (the network worker).
    let prio = unsafe { sys::uxTaskPriorityGet(core::ptr::null_mut()) };
    let mut task: sys::TaskHandle_t = core::ptr::null_mut();
    // SAFETY: `verify_entry` takes the `Arc` reference `given`.
    let made = unsafe { sys::xTaskCreatePinnedToCore(Some(verify_entry), c"gallery-verify".as_ptr(), VERIFY_STACK as u32, given, prio, &mut task, esp_idf_hal::cpu::core() as i32) };
    if made != 1 {
        // SAFETY: the task never started: its reference is taken back.
        drop(unsafe { std::sync::Arc::from_raw(given as *const VerifyRun) });
        return None;
    }
    let ticks = (VERIFY_WAIT_MS * sys::configTICK_RATE_HZ / 1000).max(1);
    // SAFETY: a take of the run's own semaphore, alive while `run` is.
    if unsafe { sys::xQueueSemaphoreTake(run.done, ticks) } != 1 {
        GAVE_UP.fetch_add(1, Relaxed);
        return None;
    }
    let out = run.out.lock().unwrap_or_else(|p| p.into_inner()).take();
    VERIFIES.fetch_add(1, Relaxed);
    LAST_VERIFY_US.store(t.elapsed().as_micros() as u32, Relaxed);
    LAST_VERIFY_LEFT.store(run.left.load(Relaxed), Relaxed);
    out
}

/// At start (Connect's client): the gallery's check runs where [`run_verify`] puts it.
pub(super) fn install() {
    fadd::set_verify_runner(run_verify);
}

/// The emulator's Connect image only: the next landing's dating fails, on
/// purpose (`POST /api/test/clock?date_fail=1`).
#[cfg(feature = "connect-test")]
pub static DATE_FAIL_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn date_fails_on_purpose() -> bool {
    #[cfg(feature = "connect-test")]
    if DATE_FAIL_ONCE.swap(false, std::sync::atomic::Ordering::Relaxed) {
        log::warn!("clock-test: this landing's dating fails on purpose (connect-test)");
        return true;
    }
    false
}

/// What the checks' task did.
enum Landing {
    /// The heap had no room for a program's check: kept, tried again.
    Busy,
    /// Not a Formula this panel can play.
    Refused,
    /// A rename failed: nothing of the add is left.
    Failed,
    /// The bytes read back from the card are not the signed ones: a card
    /// fault; nothing lands.
    CardFault,
    /// In place; `dated`: the `.psb` given its source's date.
    Landed { dated: bool },
}

/// The checks' task's stack (the program loads, the source's pass, the
/// renames and the date).
const CHECK_STACK: usize = 8 * 1024;

impl Client {
    /// A turn's housekeeping: a stale add's part files go now.
    pub(super) fn formula_tick(&mut self) {
        if let Some(id) = self.adds.expire(core_now()) {
            log::info!("formula-add: an add dropped (10 minutes without a part, or past its whole life)");
            remove_parts(id);
        }
    }

    #[optimize(size)]
    #[inline(never)]
    pub(super) fn formula_add(&mut self, k: KeyId, args: AddArgs, signed: Signed) -> Done {
        if !self.link.status.lock().unwrap().sd_usable() {
            return Err(Refusal::NoCard);
        }
        if lan_busy() {
            return Err(Refusal::Busy);
        }
        self.formula_tick();
        let now = core_now();
        let id = args.id;
        match self.adds.pre(k, &args, now)? {
            Pre::Landed { stem } => return Ok(done(&stem)),
            Pre::Resume { next } => {
                if let Some((_, w)) = self.adds.whole(&k, id) {
                    let w = w.clone();
                    return self.formula_land(k, id, &w);
                }
                return Ok(Some(format!(r#"{{"next":{next}}}"#)));
            }
            Pre::New => {}
        }
        // The card is read now: counted against the card-read rate (an add
        // found already there too).
        self.adds.note_read(k, now);
        let (stem, refill) = match target(&args)? {
            Target::There(stem) => return Ok(done(&stem)),
            Target::New(stem) => (stem, false),
            Target::Refill(stem) => {
                log::info!("formula-add: /formulas/{stem}.psi is the signed source but its .psb is not current: putting the .psb back");
                (stem, true)
            }
        };
        // (Held only with a signature over exactly these bytes, and only
        // within the day's count, per key and in all.)
        if let Some(old) = self.adds.begin_as(k, args, signed, stem, refill, now)? {
            remove_parts(old);
        }
        let _busy = crate::cache_builder::busy();
        for p in [Part::Source, Part::Psb] {
            if std::fs::File::create(part_path(id, p)).is_err() {
                self.adds.drop_held();
                remove_parts(id);
                return Err(Refusal::Busy);
            }
        }
        Ok(Some(r#"{"next":0}"#.into()))
    }

    #[optimize(size)]
    #[inline(never)]
    pub(super) fn formula_part(&mut self, k: KeyId, id: u32, off: u32, data: Vec<u8>) -> Done {
        // No add admitted (none signed): refused before the card.
        if self.adds.held.is_none() {
            return Err(Refusal::NotAllowed);
        }
        if !self.link.status.lock().unwrap().sd_usable() {
            return Err(Refusal::NoCard);
        }
        // (A LAN transfer goes first: the part is sent again.)
        if lan_busy() {
            return Err(Refusal::Busy);
        }
        self.formula_tick();
        let p = match self.adds.part(k, id, off, &data, core_now()) {
            Ok(p) => p,
            Err(Refusal::BadArg) if self.adds.held.is_none() => {
                remove_parts(id);
                return Err(Refusal::BadArg);
            }
            Err(e) => return Err(e),
        };
        {
            let _busy = crate::cache_builder::busy();
            for (which, _, range) in &p.runs {
                let w = std::fs::OpenOptions::new().append(true).open(part_path(id, *which)).and_then(|mut f| f.write_all(&data[range.clone()]));
                if w.is_err() {
                    self.adds.drop_held();
                    remove_parts(id);
                    return Err(Refusal::Busy);
                }
            }
        }
        if p.complete {
            let Some((_, w)) = self.adds.whole(&k, id) else {
                return Err(Refusal::BadArg);
            };
            let w = w.clone();
            return self.formula_land(k, id, &w);
        }
        Ok(Some(format!(r#"{{"next":{}}}"#, p.next)))
    }

    /// The bytes are in and their hashes match: check, then land. Only with
    /// the gallery's signature over exactly these bytes.
    #[optimize(size)]
    #[inline(never)]
    fn formula_land(&mut self, k: KeyId, id: u32, signed: &Signed) -> Done {
        let Some((h, _)) = self.adds.whole(&k, id) else {
            return Err(Refusal::BadArg);
        };
        if !h.args.signed_by(signed) {
            return Err(Refusal::NotAllowed);
        }
        let (stem, psb_size, refill) = (h.target.clone(), h.args.psb_size as usize, h.refill);
        if lan_busy() || !room(psb_size) {
            // Kept: the same formula-add signed again tries again.
            return Err(Refusal::Busy);
        }
        // The card is this add's from its first read to its last rename.
        let _busy = crate::cache_builder::busy();
        let mut psb = Vec::new();
        let read = psb.try_reserve_exact(psb_size).is_ok()
            && std::fs::File::open(part_path(id, Part::Psb)).and_then(|f| f.take(psb_size as u64 + 1).read_to_end(&mut psb)).is_ok();
        if !read {
            return Err(Refusal::Busy);
        }
        // The checks (the bytes on the card read back and hashed against
        // the signature, the source's pass, the program's loads) and the
        // landing (the renames, the date) on a task of their own: FatFs's
        // lookups are deep, and the network worker's stack has none to spare.
        let d = dir();
        let (psb_to, psi_to) = (format!("{d}/{stem}.psb"), format!("{d}/{stem}.psi"));
        let (src_part, psb_part) = (part_path(id, Part::Source), part_path(id, Part::Psb));
        let (to_b, to_i) = (psb_to.clone(), psi_to.clone());
        let ((src_len, src_sha), (psb_len, psb_sha)) = ((signed.src().0, *signed.src().1), (signed.psb().0, *signed.psb().1));
        let run = crate::psi_stack::run_with_prio(CHECK_STACK, Some(1), move || {
            // The bytes as the card holds them, against the signature.
            let Some(src) = source_read(&src_part) else { return Landing::Refused };
            let psb_back = (psb.len() as u32, fadd::sha256_of(&psb[..]).unwrap_or_default());
            if src != (src_len, src_sha) || psb_back != (psb_len, psb_sha) || psb.len() != psb_size {
                return Landing::CardFault;
            }
            match psb_ok(&psb) {
                Err(()) => return Landing::Busy,
                Ok(false) => return Landing::Refused,
                Ok(true) => {}
            }
            if refill {
                // Only the `.psb`: the card's source is the signed one. The
                // stale one goes, the new one in its place.
                let _ = std::fs::remove_file(&src_part);
                let _ = std::fs::remove_file(&to_b);
                if std::fs::rename(&psb_part, &to_b).is_err() {
                    return Landing::Failed;
                }
            } else {
                // The program first, then its source; a rename never
                // replaces a file. A restart between the two leaves a `.psb`
                // with no source: never listed or played (both go by the
                // `.psi`), and removed by the next mount pass; the other
                // order could leave a source with no program, which a mount
                // pass would try to compile (the classic cannot, for the
                // largest). Either failing leaves nothing of this add.
                if std::fs::rename(&psb_part, &to_b).is_err() {
                    return Landing::Failed;
                }
                if std::fs::rename(&src_part, &to_i).is_err() {
                    let _ = std::fs::remove_file(&to_b);
                    return Landing::Failed;
                }
            }
            // The program as current as its source (the mount pass keeps it).
            Landing::Landed { dated: !date_fails_on_purpose() && date_like(&to_b, &to_i) }
        });
        let Some((landing, left)) = run else {
            // (No task to check on: the heap. Kept, tried again.)
            return Err(Refusal::Busy);
        };
        log::info!("formula-add: checked and landed on a task of its own ({left} B of its stack left)");
        match landing {
            Landing::Busy => return Err(Refusal::Busy),
            Landing::Refused => {
                log::warn!("formula-add: refused: not a Formula this panel can play");
                self.adds.drop_held();
                remove_parts(id);
                return Err(Refusal::BadArg);
            }
            Landing::CardFault => {
                log::warn!("formula-add: a card fault: the bytes read back from the card are not the signed ones; nothing landed");
                self.adds.drop_held();
                remove_parts(id);
                return Err(Refusal::BadArg);
            }
            Landing::Failed => {
                self.adds.drop_held();
                remove_parts(id);
                return Err(Refusal::Busy);
            }
            Landing::Landed { dated } => {
                if !dated {
                    log::warn!("formula-add: /formulas/{stem}.psb could not be given its source's date: the next start may prepare it again (a signed add of the same Formula puts it back)");
                }
            }
        }
        crate::fatidx::note_written(&psi_to);
        // (A board measurement: the network worker's stack never used yet,
        // with the add's checks and landing on it.)
        // SAFETY: a read of the calling task's own high-water mark.
        let unused = unsafe { esp_idf_sys::uxTaskGetStackHighWaterMark(core::ptr::null_mut()) };
        log::info!(
            "formula-add: landed as /formulas/{stem}.psi and its .psb (signed by gallery key {}, rev {}; the network worker's stack {unused} B never used)",
            signed.kid(),
            signed.rev()
        );
        self.adds.landed(&stem, core_now());
        Ok(done(&stem))
    }

    /// `k` was removed (remotely or on the LAN): its add goes, part files too.
    pub(super) fn formula_forget(&mut self, k: &KeyId) {
        if let Some(id) = self.adds.forget_key(k) {
            remove_parts(id);
        }
    }
}
