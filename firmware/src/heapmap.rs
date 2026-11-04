//! The heap map (feature `heapmap`, a diagnostic build only: `make
//! fw-release FW_RELEASE_FEATURES=heapmap`, which also turns on FreeRTOS's
//! trace facility for the task table). Off, every entry point here is an
//! empty inline function and nothing is compiled in.
//!
//! What it measures, and where it says it:
//!
//! - **Checkpoints** ([`cp`]): one console line at each start-up step (the
//!   first time a name is passed), with the free heap and largest free block
//!   for `MALLOC_CAP_8BIT`, `MALLOC_CAP_INTERNAL` and `MALLOC_CAP_DMA`, the
//!   change since the previous checkpoint (whichever task logged it), and the
//!   bytes the Rust allocator holds. `heapmap: cp ...`.
//! - **Tasks**: every FreeRTOS task, its core, priority, stack size (the heap
//!   block its stack is, found by the heap walk; `static` when the stack is
//!   not in the heap) and its stack's high-water mark. `heapmap: task ...`.
//! - **Census**: a walk of every heap block, split into the Rust allocator's
//!   (each carries an 8 B header naming its call site, below) and C's (FatFs,
//!   lwIP, Wi-Fi, FreeRTOS, the panel driver), with the largest C blocks by
//!   address and size, a histogram of free blocks, and the Rust call sites
//!   holding the most bytes. `heapmap: census|free|cblk|site ...`.
//!
//! The Rust call sites: in this build the global allocator notes every
//! live Rust allocation's pointer and call site in fixed tables (no header
//! in the block, so a block freed across the Rust/C line stays harmless). A
//! call site is the return addresses of the first [`DEPTH`] frames above the
//! allocator, so `tools/heapmap-report.py` can name each with addr2line and
//! the ELF and group them by owner. The tables (~10 KB of .bss) and the
//! reporting task's stack are measurement overhead: subtract them.
//!
//! The census and the task table go to the console ~60 s after boot and
//! every [`PERIOD_S`] s after, from a task of their own on CPU0 (its stack is
//! in the task table: subtract it too). `/api/status` carries a short
//! `"heapmap"` object; `/api/status?heapmap=1` answers the whole map as JSON,
//! fresh, sent in chunks.

#[cfg(not(feature = "heapmap"))]
mod off {
    /// A start-up step: nothing in this build.
    #[inline(always)]
    pub fn cp(_name: &'static str) {}
    /// A start-up step with a prefix: nothing in this build.
    #[inline(always)]
    #[allow(dead_code)]
    pub fn cp_k(_prefix: &'static str, _name: &'static str) {}
    /// A step that took heap: nothing in this build.
    #[inline(always)]
    pub fn cp_if_moved(_name: &'static str, _before: u32) {}
    /// Nothing in this build.
    #[inline(always)]
    pub fn start() {}
    /// No `"heapmap"` key in this build.
    #[inline(always)]
    pub fn status_brief() -> Option<String> {
        None
    }
    /// No map in this build.
    #[inline(always)]
    pub fn part(_k: usize) -> Option<String> {
        None
    }
}
#[cfg(not(feature = "heapmap"))]
pub use off::*;

#[cfg(feature = "heapmap")]
pub use on::*;

#[cfg(feature = "heapmap")]
mod on {
    use esp_idf_sys as sys;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::fmt::Write as _;
    use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering::Relaxed};

    /// Return addresses kept per call site.
    pub const DEPTH: usize = 5;
    /// Call sites in the table (the rest are counted in `overflow`).
    pub const SITES: usize = 256;
    /// The census's period after the first one (~60 s after boot).
    pub const PERIOD_S: u64 = 180;
    /// Checkpoints kept for `/api/status?heapmap=1` (all are logged).
    const CPS: usize = 56;
    /// The site index of allocations whose site did not fit the table.
    const OVERFLOW: u16 = SITES as u16;

    // --- The allocator -------------------------------------------------

    struct Site {
        pcs: [AtomicU32; DEPTH],
        live: AtomicU32,
        count: AtomicU32,
    }
    #[allow(clippy::declare_interior_mutable_const)]
    const SITE0: Site = Site {
        pcs: [const { AtomicU32::new(0) }; DEPTH],
        live: AtomicU32::new(0),
        count: AtomicU32::new(0),
    };
    static TABLE: [Site; SITES + 1] = [SITE0; SITES + 1];
    /// Bytes the Rust allocator holds for its callers (as asked), the
    /// allocations, and the most bytes ever held.
    static LIVE: AtomicU32 = AtomicU32::new(0);
    static COUNT: AtomicU32 = AtomicU32::new(0);
    static PEAK: AtomicU32 = AtomicU32::new(0);

    #[repr(C)]
    struct Frame {
        pc: u32,
        sp: u32,
        next_pc: u32,
        exc_frame: *const core::ffi::c_void,
    }
    extern "C" {
        fn esp_backtrace_get_start(pc: *mut u32, sp: *mut u32, next_pc: *mut u32);
        fn esp_backtrace_get_next_frame(frame: *mut Frame) -> bool;
    }

    /// A windowed return address as the call's own address (IDF's
    /// `esp_cpu_process_stack_pc`).
    fn pc_of(pc: u32) -> u32 {
        let pc = if pc & 0x8000_0000 != 0 { (pc & 0x3FFF_FFFF) | 0x4000_0000 } else { pc };
        pc.wrapping_sub(3)
    }

    /// The return addresses above the allocator: frame 0 is the function
    /// that allocated (with the allocation shims inlined into it).
    #[inline(never)]
    fn backtrace() -> [u32; DEPTH] {
        let mut pcs = [0u32; DEPTH];
        let mut f = Frame { pc: 0, sp: 0, next_pc: 0, exc_frame: core::ptr::null() };
        // SAFETY: fills `f` with this stack's first frame (spilling the
        // register windows); the walk only reads this task's stack.
        unsafe { esp_backtrace_get_start(&mut f.pc, &mut f.sp, &mut f.next_pc) };
        // Skipped: `take`'s own frame. The next is the allocating function
        // (`__rust_alloc` and `Alloc::alloc` are inlined into it: addr2line
        // -i names it after them).
        // SAFETY: as above.
        if f.next_pc == 0 || !unsafe { esp_backtrace_get_next_frame(&mut f) } {
            return pcs;
        }
        for slot in pcs.iter_mut() {
            if f.next_pc == 0 {
                break;
            }
            // SAFETY: as above.
            if !unsafe { esp_backtrace_get_next_frame(&mut f) } {
                break;
            }
            *slot = pc_of(f.pc);
        }
        pcs
    }

    fn site_of(pcs: &[u32; DEPTH]) -> u16 {
        // The first address marks a slot taken (1 while it is being filled).
        let key = match pcs[0] {
            0 | 1 => 2,
            k => k,
        };
        let mut h: u32 = 0x811C_9DC5;
        for &p in pcs {
            h = (h ^ p).wrapping_mul(0x0100_0193);
        }
        let start = (h as usize) % SITES;
        let fill = |s: &Site, from: u32| {
            if s.pcs[0].compare_exchange(from, 1, Relaxed, Relaxed).is_err() {
                return false;
            }
            for (d, &p) in s.pcs[1..].iter().zip(&pcs[1..]) {
                d.store(p, Relaxed);
            }
            s.pcs[0].store(key, Relaxed);
            true
        };
        // A site with nothing live left may be taken over (the table holds
        // the sites with bytes, not every site there ever was): the first
        // such on the way, if the site is not found before an empty slot.
        let mut idle = None;
        for i in 0..SITES {
            let k = (start + i) % SITES;
            let s = &TABLE[k];
            let have = s.pcs[0].load(Relaxed);
            if have == 0 {
                if let Some((j, was)) = idle {
                    if fill(&TABLE[j], was) {
                        return j as u16;
                    }
                }
                if fill(s, 0) {
                    return k as u16;
                }
                continue;
            }
            if have == key && s.pcs[1..].iter().zip(&pcs[1..]).all(|(a, &b)| a.load(Relaxed) == b) {
                return k as u16;
            }
            if idle.is_none() && have != 1 && s.count.load(Relaxed) == 0 {
                idle = Some((k, have));
            }
        }
        if let Some((j, was)) = idle {
            if fill(&TABLE[j], was) {
                return j as u16;
            }
        }
        OVERFLOW
    }

    // Which site each live Rust allocation is from: an open-addressed
    // table keyed by the pointer (no header in the block: memory that
    // crosses between Rust and C, freed by the other, stays harmless).
    // A slot is empty (0), being filled (1), freed (2, reused by inserts;
    // lookups go past it) or holds a pointer.
    const SLOTS: usize = 768;
    const EMPTY: u32 = 0;
    const BUSY: u32 = 1;
    const TOMB: u32 = 2;
    static PTRS: [AtomicU32; SLOTS] = [const { AtomicU32::new(0) }; SLOTS];
    static SITE_OF: [AtomicU16; SLOTS] = [const { AtomicU16::new(0) }; SLOTS];
    /// Allocations not in the table (it was full), and frees of pointers
    /// that were not in it (C's, or those).
    static FULL: AtomicU32 = AtomicU32::new(0);
    static MISSES: AtomicU32 = AtomicU32::new(0);

    fn slot0(p: u32) -> usize {
        ((p >> 2).wrapping_mul(0x9E37_79B1) >> 12) as usize % SLOTS
    }

    fn insert(p: u32, site: u16) -> bool {
        let h = slot0(p);
        for i in 0..SLOTS {
            let k = (h + i) % SLOTS;
            let v = PTRS[k].load(Relaxed);
            if (v == EMPTY || v == TOMB) && PTRS[k].compare_exchange(v, BUSY, Relaxed, Relaxed).is_ok() {
                SITE_OF[k].store(site, Relaxed);
                PTRS[k].store(p, Relaxed);
                return true;
            }
        }
        FULL.fetch_add(1, Relaxed);
        false
    }

    /// Take `p` out of the table: its site.
    fn remove(p: u32) -> Option<u16> {
        let h = slot0(p);
        for i in 0..SLOTS {
            let k = (h + i) % SLOTS;
            let v = PTRS[k].load(Relaxed);
            if v == EMPTY {
                break;
            }
            if v == p {
                let site = SITE_OF[k].load(Relaxed);
                if PTRS[k].compare_exchange(p, TOMB, Relaxed, Relaxed).is_ok() {
                    return Some(site);
                }
            }
        }
        MISSES.fetch_add(1, Relaxed);
        None
    }

    fn note_alloc(p: *mut u8, size: usize) {
        if p.is_null() {
            return;
        }
        let site = site_of(&backtrace());
        if !insert(p as u32, site) {
            return;
        }
        let size = size as u32;
        let t = &TABLE[site as usize];
        t.live.fetch_add(size, Relaxed);
        t.count.fetch_add(1, Relaxed);
        let now = LIVE.fetch_add(size, Relaxed) + size;
        COUNT.fetch_add(1, Relaxed);
        PEAK.fetch_max(now, Relaxed);
    }

    fn note_free(p: *mut u8, size: usize) {
        let Some(site) = remove(p as u32) else { return };
        let size = size as u32;
        let t = &TABLE[site.min(OVERFLOW) as usize];
        t.live.fetch_sub(size, Relaxed);
        t.count.fetch_sub(1, Relaxed);
        LIVE.fetch_sub(size, Relaxed);
        COUNT.fetch_sub(1, Relaxed);
    }

    /// The live pointers, sorted (for the walk, which must not allocate).
    /// The live pointers and their sites, sorted (for the walk, which
    /// must not allocate).
    fn live_ptrs() -> Vec<(u32, u16)> {
        let mut v: Vec<(u32, u16)> = PTRS
            .iter()
            .zip(&SITE_OF)
            .map(|(a, s)| (a.load(Relaxed), s.load(Relaxed)))
            .filter(|&(p, _)| p > TOMB)
            .collect();
        v.sort_unstable();
        v
    }

    pub struct Alloc;

    #[global_allocator]
    static A: Alloc = Alloc;

    impl Alloc {
        #[inline(never)]
        unsafe fn take(&self, l: Layout, zeroed: bool) -> *mut u8 {
            #[cfg(feature = "psi")]
            if let Some(p) = crate::arena::claim(l) {
                if zeroed {
                    // SAFETY: the slot is `l.size()` bytes of the arena, ours now.
                    unsafe { p.write_bytes(0, l.size()) };
                }
                return p;
            }
            // SAFETY: as the caller's.
            let p = unsafe { if zeroed { System.alloc_zeroed(l) } else { System.alloc(l) } };
            note_alloc(p, l.size());
            p
        }
    }

    // Out of line, each (as `arena`'s): inlined into every allocation in the
    // image they would cost a great deal of flash.
    unsafe impl GlobalAlloc for Alloc {
        #[inline(never)]
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            unsafe { self.take(l, false) }
        }

        #[inline(never)]
        unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
            unsafe { self.take(l, true) }
        }

        #[inline(never)]
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            #[cfg(feature = "psi")]
            if crate::arena::release(p) {
                return;
            }
            note_free(p, l.size());
            // SAFETY: as the caller's.
            unsafe { System.dealloc(p, l) }
        }

        #[inline(never)]
        unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
            #[cfg(feature = "psi")]
            if crate::arena::holds(p) {
                // (No arena allocation is ever resized: a Box each.)
                // SAFETY: a fresh block, the old one's bytes copied, then freed.
                unsafe {
                    let n = self.take(Layout::from_size_align_unchecked(new, l.align()), false);
                    if !n.is_null() {
                        n.copy_from_nonoverlapping(p, l.size().min(new));
                        self.dealloc(p, l);
                    }
                    return n;
                }
            }
            // SAFETY: as the caller's.
            let n = unsafe { System.realloc(p, l, new) };
            if n.is_null() {
                return n;
            }
            // The site stays the first allocation's.
            let site = remove(p as u32);
            let old = l.size() as u32;
            match site {
                Some(site) if insert(n as u32, site) => {
                    let t = &TABLE[site.min(OVERFLOW) as usize];
                    if new as u32 >= old {
                        let d = new as u32 - old;
                        t.live.fetch_add(d, Relaxed);
                        let now = LIVE.fetch_add(d, Relaxed) + d;
                        PEAK.fetch_max(now, Relaxed);
                    } else {
                        let d = old - new as u32;
                        t.live.fetch_sub(d, Relaxed);
                        LIVE.fetch_sub(d, Relaxed);
                    }
                }
                Some(site) => {
                    let t = &TABLE[site.min(OVERFLOW) as usize];
                    t.live.fetch_sub(old, Relaxed);
                    t.count.fetch_sub(1, Relaxed);
                    LIVE.fetch_sub(old, Relaxed);
                    COUNT.fetch_sub(1, Relaxed);
                }
                None => {}
            }
            n
        }
    }

    // --- Heap figures ---------------------------------------------------

    #[derive(Clone, Copy, Default)]
    struct Heap {
        free8: u32,
        big8: u32,
        free_int: u32,
        big_int: u32,
        free_dma: u32,
        big_dma: u32,
    }

    fn heap_now() -> Heap {
        // SAFETY: read-only queries of IDF's heap accounting.
        unsafe {
            Heap {
                free8: sys::heap_caps_get_free_size(sys::MALLOC_CAP_8BIT) as u32,
                big8: sys::heap_caps_get_largest_free_block(sys::MALLOC_CAP_8BIT) as u32,
                free_int: sys::heap_caps_get_free_size(sys::MALLOC_CAP_INTERNAL) as u32,
                big_int: sys::heap_caps_get_largest_free_block(sys::MALLOC_CAP_INTERNAL) as u32,
                free_dma: sys::heap_caps_get_free_size(sys::MALLOC_CAP_DMA) as u32,
                big_dma: sys::heap_caps_get_largest_free_block(sys::MALLOC_CAP_DMA) as u32,
            }
        }
    }

    fn uptime_ms() -> u32 {
        // SAFETY: a plain clock read.
        (unsafe { sys::esp_timer_get_time() } / 1000) as u32
    }

    fn task_name() -> [u8; 12] {
        let mut out = [0u8; 12];
        // SAFETY: the calling task's own name, a NUL-terminated C string.
        let p = unsafe { sys::pcTaskGetName(core::ptr::null_mut()) };
        if !p.is_null() {
            let s = unsafe { core::ffi::CStr::from_ptr(p) }.to_bytes();
            let n = s.len().min(11);
            out[..n].copy_from_slice(&s[..n]);
        }
        out
    }

    fn name_str(n: &[u8; 12]) -> &str {
        let end = n.iter().position(|&b| b == 0).unwrap_or(12);
        core::str::from_utf8(&n[..end]).unwrap_or("?")
    }

    // --- Checkpoints ----------------------------------------------------

    #[derive(Clone, Copy)]
    struct Cp {
        prefix: &'static str,
        name: &'static str,
        task: [u8; 12],
        t_ms: u32,
        heap: Heap,
        rust: u32,
        rust_n: u32,
    }
    static CP_LOCK: AtomicBool = AtomicBool::new(false);
    static mut CP_TABLE: [Option<Cp>; CPS] = [None; CPS];
    static CP_N: AtomicUsize = AtomicUsize::new(0);
    static CP_SEEN: AtomicU32 = AtomicU32::new(0);
    static LAST_FREE: AtomicU32 = AtomicU32::new(0);
    static LAST_INT: AtomicU32 = AtomicU32::new(0);
    static LAST_DMA: AtomicU32 = AtomicU32::new(0);
    static LAST_RUST: AtomicU32 = AtomicU32::new(0);

    /// CP_LOCK (a sleep, not a yield: a higher-priority task must not spin
    /// on a lower one that holds it).
    fn lock() {
        while CP_LOCK.compare_exchange(false, true, Relaxed, Relaxed).is_err() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn delta(now: u32, last: &AtomicU32) -> i64 {
        let was = last.swap(now, Relaxed);
        if was == 0 {
            0
        } else {
            now as i64 - was as i64
        }
    }

    /// A start-up step: logged and kept the first time `name` is passed.
    #[inline(never)]
    pub fn cp(name: &'static str) {
        cp_k("", name);
    }

    /// [`cp`] with a prefix (`"task start: "` and a task's name).
    #[inline(never)]
    pub fn cp_k(prefix: &'static str, name: &'static str) {
        // One at a time (a checkpoint is never on a hot path twice).
        lock();
        let n = CP_N.load(Relaxed);
        // SAFETY: only read or written with CP_LOCK held.
        let table = unsafe { &mut *core::ptr::addr_of_mut!(CP_TABLE) };
        let seen = table[..n.min(CPS)].iter().flatten().any(|c| c.name == name && c.prefix == prefix);
        // (Past the table: logged, not kept; a repeat is then not caught.)
        if seen {
            CP_LOCK.store(false, Relaxed);
            return;
        }
        let heap = heap_now();
        let c = Cp {
            prefix,
            name,
            task: task_name(),
            t_ms: uptime_ms(),
            heap,
            rust: LIVE.load(Relaxed),
            rust_n: COUNT.load(Relaxed),
        };
        if n < CPS {
            table[n] = Some(c);
        }
        CP_N.store(n + 1, Relaxed);
        let k = CP_SEEN.fetch_add(1, Relaxed);
        let (d8, di, dd, dr) = (
            delta(heap.free8, &LAST_FREE),
            delta(heap.free_int, &LAST_INT),
            delta(heap.free_dma, &LAST_DMA),
            delta(c.rust, &LAST_RUST),
        );
        CP_LOCK.store(false, Relaxed);
        log::info!(
            "heapmap: cp {k:02} {:<34} t={} ms [{}] 8bit {} ({d8:+}) big {} | int {} ({di:+}) big {} | dma {} ({dd:+}) big {} | rust {} ({dr:+}) in {}",
            format!("{prefix}{name}"),
            c.t_ms,
            name_str(&c.task),
            heap.free8,
            heap.big8,
            heap.free_int,
            heap.big_int,
            heap.free_dma,
            heap.big_dma,
            c.rust,
            c.rust_n
        );
    }

    /// A step that may or may not take heap (a tick that loads an index
    /// now and then): a checkpoint only if it took 2 KB or more since
    /// `before` (the 8-bit free heap read before it).
    pub fn cp_if_moved(name: &'static str, before: u32) {
        // SAFETY: a read of IDF's heap accounting.
        let now = unsafe { sys::heap_caps_get_free_size(sys::MALLOC_CAP_8BIT) } as u32;
        if now + 2048 <= before {
            cp(name);
        }
    }

    // --- Tasks ----------------------------------------------------------

    struct TaskRow {
        name: [u8; 12],
        core: i32,
        prio: u32,
        state: u32,
        stack_base: usize,
        unused: u32,
    }

    fn tasks() -> Vec<TaskRow> {
        // SAFETY: FreeRTOS's own snapshot into a buffer of the size it says
        // it needs (plus room for tasks made meanwhile).
        unsafe {
            let n = sys::uxTaskGetNumberOfTasks() as usize + 4;
            let mut v: Vec<sys::TaskStatus_t> = Vec::with_capacity(n);
            let got = sys::uxTaskGetSystemState(v.as_mut_ptr(), n as _, core::ptr::null_mut()) as usize;
            v.set_len(got.min(n));
            v.iter()
                .map(|t| {
                    let mut name = [0u8; 12];
                    if !t.pcTaskName.is_null() {
                        let s = core::ffi::CStr::from_ptr(t.pcTaskName).to_bytes();
                        let k = s.len().min(11);
                        name[..k].copy_from_slice(&s[..k]);
                    }
                    TaskRow {
                        name,
                        core: sys::xTaskGetCoreID(t.xHandle) as i32,
                        prio: t.uxBasePriority as u32,
                        state: t.eCurrentState as u32,
                        stack_base: t.pxStackBase as usize,
                        unused: t.usStackHighWaterMark as u32,
                    }
                })
                .collect()
        }
    }

    // --- The heap walk --------------------------------------------------

    const TOP_C: usize = 28;
    const TOP_R: usize = 16;
    /// Free-block size classes' upper bounds (the last is everything bigger).
    const FREE_CLASSES: [u32; 10] = [32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384];
    const MAX_STACKS: usize = 32;

    #[derive(Default)]
    struct Walk {
        used: u32,
        used_n: u32,
        rust: u32,
        rust_n: u32,
        c: u32,
        c_n: u32,
        free: u32,
        free_n: u32,
        largest: u32,
        free_hist: [u32; FREE_CLASSES.len() + 1],
        free_hist_b: [u32; FREE_CLASSES.len() + 1],
        /// C's size histogram, the same classes (count, bytes).
        c_hist: [u32; FREE_CLASSES.len() + 1],
        c_hist_b: [u32; FREE_CLASSES.len() + 1],
        /// The largest C blocks: (address, bytes), biggest first.
        top_c: [(usize, u32); TOP_C],
        /// Every used C block (address, bytes), when wanted (capacity
        /// reserved before the walk: the callback cannot allocate).
        c_list: Vec<(u32, u32)>,
        /// The Rust allocator's live pointers, sorted (in).
        rust_ptrs: Vec<(u32, u16)>,
        /// The largest Rust blocks: (address, bytes, site), biggest first.
        top_rust: [(usize, u32, u16); TOP_R],
        /// Task stack bases to find (in), their blocks' sizes (out).
        stacks: [(usize, u32); MAX_STACKS],
        stack_n: usize,
        stack_bytes: u32,
        regions: [(usize, usize, u32, u32); 8],
        region_n: usize,
        /// Islands: runs of used blocks between two free blocks of at least
        /// [`ISLAND_FREE`] each (what splits the free space), the
        /// [`ISLANDS`] with the most free on their smaller side.
        islands: [Island; ISLANDS],
        /// The run in progress: the free block before it (bytes, 0: none),
        /// its region, and the run itself.
        run_free_before: u32,
        run_region: usize,
        run: Island,
    }

    const ISLANDS: usize = 20;
    const ISLAND_FREE: u32 = 512;
    const ISLAND_MAX: u32 = 8192;
    const ISLAND_BLOCKS: usize = 3;

    #[derive(Default, Clone, Copy)]
    struct Island {
        /// The smaller and larger free blocks either side.
        free_lo: u32,
        free_hi: u32,
        bytes: u32,
        n: u32,
        /// Its first blocks: (address, bytes, Rust site or u16::MAX for C).
        blocks: [(u32, u32, u16); ISLAND_BLOCKS],
    }

    fn class(size: u32) -> usize {
        FREE_CLASSES.iter().position(|&c| size <= c).unwrap_or(FREE_CLASSES.len())
    }

    unsafe extern "C" fn walker(
        heap: sys::walker_heap_into_t,
        b: sys::walker_block_info_t,
        user: *mut core::ffi::c_void,
    ) -> bool {
        // SAFETY: `user` is the `Walk`, alive for the walk; the callback
        // runs with the heap locked and must not allocate (it does not).
        let w = unsafe { &mut *(user as *mut Walk) };
        let size = b.size as u32;
        let (hs, he) = (heap.start as usize, heap.end as usize);
        let r = match w.regions[..w.region_n].iter().position(|r| r.0 == hs) {
            Some(r) => r,
            None if w.region_n < w.regions.len() => {
                w.regions[w.region_n] = (hs, he, 0, 0);
                w.region_n += 1;
                w.region_n - 1
            }
            None => w.regions.len() - 1,
        };
        if r != w.run_region {
            w.run_region = r;
            w.run_free_before = 0;
            w.run = Island::default();
        }
        if !b.used {
            // A run between two big enough free blocks is an island.
            let (before, run) = (w.run_free_before, w.run);
            if run.n > 0 && before >= ISLAND_FREE && size >= ISLAND_FREE && run.bytes <= ISLAND_MAX {
                let mut isl = run;
                isl.free_lo = before.min(size);
                isl.free_hi = before.max(size);
                if isl.free_lo > w.islands[ISLANDS - 1].free_lo {
                    let mut j = ISLANDS - 1;
                    while j > 0 && w.islands[j - 1].free_lo < isl.free_lo {
                        w.islands[j] = w.islands[j - 1];
                        j -= 1;
                    }
                    w.islands[j] = isl;
                }
            }
            w.run_free_before = size;
            w.run = Island::default();
            w.free += size;
            w.free_n += 1;
            w.largest = w.largest.max(size);
            let k = class(size);
            w.free_hist[k] += 1;
            w.free_hist_b[k] += size;
            w.regions[r].3 += size;
            return true;
        }
        w.regions[r].2 += size;
        w.used += size;
        w.used_n += 1;
        let p = b.ptr as usize;
        let site = w
            .rust_ptrs
            .binary_search_by_key(&(p as u32), |e| e.0)
            .map_or(u16::MAX, |i| w.rust_ptrs[i].1);
        if (w.run.n as usize) < ISLAND_BLOCKS {
            w.run.blocks[w.run.n as usize] = (p as u32, size, site);
        }
        w.run.n += 1;
        w.run.bytes += size;
        if let Ok(i) = w.rust_ptrs.binary_search_by_key(&(p as u32), |e| e.0) {
            w.rust += size;
            w.rust_n += 1;
            if size > w.top_rust[TOP_R - 1].1 {
                let mut j = TOP_R - 1;
                while j > 0 && w.top_rust[j - 1].1 < size {
                    w.top_rust[j] = w.top_rust[j - 1];
                    j -= 1;
                }
                w.top_rust[j] = (p, size, w.rust_ptrs[i].1);
            }
            return true;
        }
        w.c += size;
        w.c_n += 1;
        if w.c_list.len() < w.c_list.capacity() {
            w.c_list.push((p as u32, size));
        }
        let k = class(size);
        w.c_hist[k] += 1;
        w.c_hist_b[k] += size;
        if let Some(s) = w.stacks[..w.stack_n].iter_mut().find(|s| s.0 == p) {
            s.1 = size;
            w.stack_bytes += size;
        }
        if size > w.top_c[TOP_C - 1].1 {
            let mut i = TOP_C - 1;
            while i > 0 && w.top_c[i - 1].1 < size {
                w.top_c[i] = w.top_c[i - 1];
                i -= 1;
            }
            w.top_c[i] = (p, size);
        }
        true
    }

    fn walk(stack_bases: &[usize]) -> Box<Walk> {
        walk_listing(stack_bases, 0)
    }

    /// [`walk`], listing up to `list` C blocks.
    fn walk_listing(stack_bases: &[usize], list: usize) -> Box<Walk> {
        let mut w = Box::<Walk>::default();
        w.c_list = Vec::with_capacity(list);
        w.rust_ptrs = live_ptrs();
        for (k, &b) in stack_bases.iter().take(MAX_STACKS).enumerate() {
            w.stacks[k] = (b, 0);
            w.stack_n = k + 1;
        }
        // SAFETY: the callback only writes the `Walk` passed as its user data.
        unsafe { sys::heap_caps_walk_all(Some(walker), (&mut *w as *mut Walk).cast()) };
        w
    }

    // --- The report -----------------------------------------------------

    fn state_name(s: u32) -> &'static str {
        match s {
            0 => "running",
            1 => "ready",
            2 => "blocked",
            3 => "suspended",
            4 => "deleted",
            _ => "?",
        }
    }

    /// The table's sites with bytes, most bytes first: (index, live, count).
    fn top_sites(n: usize) -> (Vec<(u16, u32, u32)>, usize) {
        let mut v: Vec<(u16, u32, u32)> = (0..=SITES)
            .filter_map(|k| {
                let t = &TABLE[k];
                let (live, count) = (t.live.load(Relaxed), t.count.load(Relaxed));
                (count > 0).then_some((k as u16, live, count))
            })
            .collect();
        let used = (0..SITES).filter(|&k| TABLE[k].pcs[0].load(Relaxed) != 0).count();
        // Unstable: the stable sort keeps a 4 KB scratch buffer on the stack
        // (more than this task or httpd can spare).
        v.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        v.truncate(n);
        (v, used)
    }

    fn pcs_of(k: u16) -> [u32; DEPTH] {
        let mut out = [0u32; DEPTH];
        if (k as usize) < SITES {
            for (o, p) in out.iter_mut().zip(&TABLE[k as usize].pcs) {
                *o = p.load(Relaxed);
            }
        }
        out
    }

    fn pause() {
        // The log ring is 4 KB: a census is many lines, written a few at a time.
        std::thread::sleep(std::time::Duration::from_millis(15));
    }

    /// The task table and the census, to the console.
    fn report(seq: u32) {
        let t_s = uptime_ms() / 1000;
        let rows = tasks();
        let bases: Vec<usize> = rows.iter().map(|r| r.stack_base).collect();
        let w = walk(&bases);
        let size_of = |base: usize| w.stacks[..w.stack_n].iter().find(|s| s.0 == base).map(|s| s.1).unwrap_or(0);
        log::info!("heapmap: report {seq} at {t_s} s: {} tasks", rows.len());
        pause();
        let mut stack_total = 0u32;
        for r in &rows {
            let size = size_of(r.stack_base);
            stack_total += size;
            log::info!(
                "heapmap: task {:<12} cpu{} p{:<2} {:<9} stack {} unused {} B (base {:#010x})",
                name_str(&r.name),
                if r.core == 0 || r.core == 1 { r.core.to_string() } else { "*".into() },
                r.prio,
                state_name(r.state),
                if size > 0 { format!("{size} B heap") } else { "static".to_string() },
                r.unused,
                r.stack_base
            );
            pause();
        }
        let h = heap_now();
        log::info!(
            "heapmap: census at {t_s} s: used {} B in {} blocks (rust {} B in {} blocks: {} B asked in {} allocs, peak {} B, table full {} misses {}; c {} B in {} blocks, of it {} B task stacks), free {} B in {} blocks, largest {} B (8bit free {} big {}, int free {} big {}, dma free {} big {})",
            w.used,
            w.used_n,
            w.rust,
            w.rust_n,
            LIVE.load(Relaxed),
            COUNT.load(Relaxed),
            PEAK.load(Relaxed),
            FULL.load(Relaxed),
            MISSES.load(Relaxed),
            w.c,
            w.c_n,
            stack_total,
            w.free,
            w.free_n,
            w.largest,
            h.free8,
            h.big8,
            h.free_int,
            h.big_int,
            h.free_dma,
            h.big_dma
        );
        pause();
        for r in &w.regions[..w.region_n] {
            log::info!(
                "heapmap: region {:#010x}..{:#010x} ({} B): used {} B, free {} B",
                r.0,
                r.1,
                r.1 - r.0,
                r.2,
                r.3
            );
            pause();
        }
        let mut line = String::new();
        for (k, &c) in FREE_CLASSES.iter().enumerate() {
            let _ = write!(line, " <={c}:{}/{}B", w.free_hist[k], w.free_hist_b[k]);
        }
        let k = FREE_CLASSES.len();
        let _ = write!(line, " more:{}/{}B", w.free_hist[k], w.free_hist_b[k]);
        log::info!("heapmap: free blocks{line}");
        pause();
        line.clear();
        for (k, &c) in FREE_CLASSES.iter().enumerate() {
            let _ = write!(line, " <={c}:{}/{}B", w.c_hist[k], w.c_hist_b[k]);
        }
        let _ = write!(line, " more:{}/{}B", w.c_hist[k], w.c_hist_b[k]);
        log::info!("heapmap: c blocks{line}");
        pause();
        for chunk in w.top_c.chunks(7) {
            line.clear();
            for &(a, s) in chunk.iter().filter(|c| c.1 > 0) {
                let tag = match bases.iter().position(|&b| b == a) {
                    Some(i) => format!("={}", name_str(&rows[i].name)),
                    None => String::new(),
                };
                let _ = write!(line, " {a:#010x}:{s}{tag}");
            }
            if !line.is_empty() {
                log::info!("heapmap: cblk{line}");
                pause();
            }
        }
        for chunk in w.top_rust.chunks(8) {
            line.clear();
            for &(a, s, site) in chunk.iter().filter(|c| c.1 > 0) {
                let _ = write!(line, " {a:#010x}:{s}@{site}");
            }
            if !line.is_empty() {
                log::info!("heapmap: rblk{line}");
                pause();
            }
        }
        let mut isites: Vec<u16> = Vec::new();
        for isl in w.islands.iter().filter(|i| i.n > 0) {
            line.clear();
            for &(a, sz, site) in isl.blocks.iter().take(isl.n as usize) {
                if site == u16::MAX {
                    let tag = match bases.iter().position(|&b| b == a as usize) {
                        Some(i) => format!("={}", name_str(&rows[i].name)),
                        None => String::new(),
                    };
                    let _ = write!(line, " {a:#010x}:{sz}c{tag}");
                } else {
                    let _ = write!(line, " {a:#010x}:{sz}@{site}");
                    if !isites.contains(&site) {
                        isites.push(site);
                    }
                }
            }
            log::info!(
                "heapmap: island {} B in {} between free {} and {}:{line}",
                isl.bytes,
                isl.n,
                isl.free_lo,
                isl.free_hi
            );
            pause();
        }
        for k in isites {
            line.clear();
            for p in pcs_of(k) {
                let _ = write!(line, " {p:#010x}");
            }
            log::info!("heapmap: isite {k:3}:{line}");
            pause();
        }
        let (top, used) = top_sites(48);
        log::info!(
            "heapmap: sites {used} of {SITES} used, overflow {} B in {}",
            TABLE[SITES].live.load(Relaxed),
            TABLE[SITES].count.load(Relaxed)
        );
        pause();
        for (k, live, count) in top {
            line.clear();
            for p in pcs_of(k) {
                let _ = write!(line, " {p:#010x}");
            }
            log::info!("heapmap: site {k:3} {live:6} B in {count:4}:{line}");
            pause();
        }
        // SAFETY: a read of this task's own high-water mark.
        let unused = unsafe { sys::uxTaskGetStackHighWaterMark(core::ptr::null_mut()) };
        log::info!("heapmap: report {seq} end (its task's stack {unused} B unused)");
    }

    #[inline(never)]
    /// The spawn test: what a short-lived task (`task::spawn_core0`, a
    /// Rust std thread that returns at once) leaves behind once it is gone,
    /// over `n` of them — the read-ahead task is such a task, started again
    /// for each file. Logged as `heapmap: spawn test ...`.
    fn spawn_test(n: u32) {
        static DONE: AtomicU32 = AtomicU32::new(0);
        let settle = || std::thread::sleep(std::time::Duration::from_millis(400));
        settle();
        // The C blocks' list, only with the heap to spare for it (~8 KB).
        let list = if heap_now().big8 > 24 * 1024 { 1000 } else { 0 };
        let mut before = walk_listing(&[], list);
        before.c_list.sort_unstable();
        let (r0, c0) = (COUNT.load(Relaxed), LIVE.load(Relaxed));
        let mut spawned = 0;
        for _ in 0..n {
            let want = DONE.load(Relaxed) + 1;
            let r = crate::task::spawn_core0(b"hm-spawn\0", 2048, 2, Box::new(|| {
                DONE.fetch_add(1, Relaxed);
            }));
            if r.is_err() {
                break;
            }
            spawned += 1;
            while DONE.load(Relaxed) < want {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            // The idle task frees a deleted task's stack and TCB.
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        settle();
        let after = walk_listing(&[], list);
        // The C blocks that are new, with their first words (what they are).
        let mut shown = 0;
        for &(a, sz) in &after.c_list {
            if before.c_list.binary_search(&(a, sz)).is_ok() || shown >= 24 {
                continue;
            }
            shown += 1;
            // SAFETY: a used block of at least `sz` bytes, read while it
            // may be in use elsewhere (a diagnostic's snapshot).
            let words: Vec<String> = (0..(sz as usize / 4).min(6))
                .map(|i| format!("{:08x}", unsafe { ((a as usize + 4 * i) as *const u32).read_volatile() }))
                .collect();
            log::info!("heapmap: spawn test new c block {a:#010x}:{sz} {}", words.join(" "));
            pause();
        }
        let (r1, c1) = (COUNT.load(Relaxed), LIVE.load(Relaxed));
        log::info!(
            "heapmap: spawn test: {spawned} short tasks left c {:+} B in {:+} blocks, rust {:+} B in {:+} allocs (walk rust {:+} B in {:+} blocks), free {:+} B",
            after.c as i64 - before.c as i64,
            after.c_n as i64 - before.c_n as i64,
            c1 as i64 - c0 as i64,
            r1 as i64 - r0 as i64,
            after.rust as i64 - before.rust as i64,
            after.rust_n as i64 - before.rust_n as i64,
            after.free as i64 - before.free as i64
        );
    }

    fn run() {
        let mut seq = 0u32;
        std::thread::sleep(std::time::Duration::from_secs(45));
        spawn_test(20);
        std::thread::sleep(std::time::Duration::from_secs(13));
        loop {
            report(seq);
            seq += 1;
            std::thread::sleep(std::time::Duration::from_secs(PERIOD_S));
        }
    }

    /// The reporting task (CPU0, priority 1, 6 KB of stack).
    pub fn start() {
        if let Err(e) = crate::task::spawn_core0(b"heapmap\0", 6 * 1024, 1, Box::new(run)) {
            log::warn!("heapmap: no task ({e})");
        }
    }

    /// `/api/status`'s `"heapmap"` value: a few counters, no walk.
    pub fn status_brief() -> Option<String> {
        let h = heap_now();
        Some(format!(
            r#"{{"rust_live":{},"rust_allocs":{},"rust_peak":{},"free":{},"largest":{},"checkpoints":{},"full":"/api/status?heapmap=1"}}"#,
            LIVE.load(Relaxed),
            COUNT.load(Relaxed),
            PEAK.load(Relaxed),
            h.free8,
            h.big8,
            CP_N.load(Relaxed)
        ))
    }

    /// The whole map as JSON, in parts (each a few KB at most, sent as one
    /// chunk each): `{"heapmap":{...}}` once they are joined.
    pub fn part(k: usize) -> Option<String> {
        let mut s = String::new();
        match k {
            0 => {
                let h = heap_now();
                let _ = write!(
                    s,
                    r#"{{"heapmap":{{"t_ms":{},"version":"{}","depth":{},"sites_max":{},"rust":{{"live":{},"allocs":{},"peak":{},"table_full":{},"free_misses":{}}},"now":{{"free":{},"largest":{},"free_int":{},"largest_int":{},"free_dma":{},"largest_dma":{}}},"checkpoints":["#,
                    uptime_ms(),
                    crate::state::firmware_version(),
                    DEPTH,
                    SITES,
                    LIVE.load(Relaxed),
                    COUNT.load(Relaxed),
                    PEAK.load(Relaxed),
                    FULL.load(Relaxed),
                    MISSES.load(Relaxed),
                    h.free8,
                    h.big8,
                    h.free_int,
                    h.big_int,
                    h.free_dma,
                    h.big_dma
                );
            }
            1 | 2 => {
                // Checkpoints, half each (copied out one at a time: the
                // table is ~4 KB, too much for the httpd task's stack).
                let n = CP_N.load(Relaxed).min(CPS);
                let (lo, hi) = if k == 1 { (0, n.min(CPS / 2)) } else { (n.min(CPS / 2), n) };
                for i in lo..hi {
                    lock();
                    // SAFETY: read with CP_LOCK held.
                    let c = unsafe { (*core::ptr::addr_of!(CP_TABLE))[i] };
                    CP_LOCK.store(false, Relaxed);
                    let Some(c) = c else { continue };
                    let _ = write!(
                        s,
                        r#"{}{{"name":"{}{}","task":"{}","t_ms":{},"free":{},"largest":{},"free_int":{},"largest_int":{},"free_dma":{},"largest_dma":{},"rust":{},"rust_allocs":{}}}"#,
                        if i == 0 { "" } else { "," },
                        c.prefix,
                        c.name,
                        name_str(&c.task),
                        c.t_ms,
                        c.heap.free8,
                        c.heap.big8,
                        c.heap.free_int,
                        c.heap.big_int,
                        c.heap.free_dma,
                        c.heap.big_dma,
                        c.rust,
                        c.rust_n
                    );
                }
                if k == 2 {
                    s.push_str("],");
                }
            }
            3 => {
                let rows = tasks();
                let bases: Vec<usize> = rows.iter().map(|r| r.stack_base).collect();
                let w = walk(&bases);
                s.push_str(r#""tasks":["#);
                for (i, r) in rows.iter().enumerate() {
                    let size = w.stacks[..w.stack_n].iter().find(|x| x.0 == r.stack_base).map(|x| x.1).unwrap_or(0);
                    let _ = write!(
                        s,
                        r#"{}{{"name":"{}","core":{},"prio":{},"state":"{}","stack":{},"unused":{}}}"#,
                        if i == 0 { "" } else { "," },
                        name_str(&r.name),
                        if r.core == 0 || r.core == 1 { r.core } else { -1 },
                        r.prio,
                        state_name(r.state),
                        size,
                        r.unused
                    );
                }
                let _ = write!(
                    s,
                    r#"],"walk":{{"used":{},"used_blocks":{},"rust":{},"rust_blocks":{},"c":{},"c_blocks":{},"stacks":{},"free":{},"free_blocks":{},"largest":{},"classes":{:?},"free_hist":{:?},"free_hist_bytes":{:?},"c_hist":{:?},"c_hist_bytes":{:?},"top_c":["#,
                    w.used,
                    w.used_n,
                    w.rust,
                    w.rust_n,
                    w.c,
                    w.c_n,
                    w.stack_bytes,
                    w.free,
                    w.free_n,
                    w.largest,
                    FREE_CLASSES,
                    w.free_hist,
                    w.free_hist_b,
                    w.c_hist,
                    w.c_hist_b
                );
                for (i, &(a, sz)) in w.top_c.iter().filter(|c| c.1 > 0).enumerate() {
                    let tag = bases.iter().position(|&b| b == a).map(|i| name_str(&rows[i].name).to_string());
                    let _ = write!(
                        s,
                        r#"{}{{"at":"{a:#010x}","bytes":{sz}{}}}"#,
                        if i == 0 { "" } else { "," },
                        tag.map(|t| format!(r#","stack_of":"{t}""#)).unwrap_or_default()
                    );
                }
                s.push_str(r#"],"top_rust":["#);
                for (i, &(a, sz, site)) in w.top_rust.iter().filter(|c| c.1 > 0).enumerate() {
                    let _ = write!(s, r#"{}{{"at":"{a:#010x}","bytes":{sz},"site":{site}}}"#, if i == 0 { "" } else { "," });
                }
                s.push_str("]},");
            }
            4..=7 => {
                // The sites, 12 a part, most bytes first.
                let (top, used) = top_sites(48);
                let from = (k - 4) * 12;
                if k == 4 {
                    let _ = write!(
                        s,
                        r#""sites_used":{used},"overflow":{{"live":{},"count":{}}},"sites":["#,
                        TABLE[SITES].live.load(Relaxed),
                        TABLE[SITES].count.load(Relaxed)
                    );
                }
                for (i, &(site, live, count)) in top.iter().enumerate().skip(from).take(12) {
                    let pcs: Vec<String> = pcs_of(site).iter().map(|p| format!(r#""{p:#010x}""#)).collect();
                    let _ = write!(
                        s,
                        r#"{}{{"site":{site},"live":{live},"count":{count},"pcs":[{}]}}"#,
                        if i == 0 { "" } else { "," },
                        pcs.join(",")
                    );
                }
                if k == 7 {
                    s.push_str("]}}");
                }
            }
            _ => return None,
        }
        Some(s)
    }
}
