//! `formula_arena` (a dev setting, `/api/config`, in force from the next
//! boot): a Formula's memory from one block set aside at boot, before the
//! network takes any, so it lands in the same place every time — the A/B
//! for the upload residual's two modes (`membank`, display-isolation
//! 2026-11-02). Off, this is the system allocator and one load an
//! allocation.
//!
//! The arena has a slot for each of a Formula's allocations: its one
//! allocation (program and VM scratch), its ink plane and its paint plane.
//! opendmd-core names the allocation it is about to make
//! (`fx_psi::ARENA_HINT`); one of a slot's exact size, with the hint set and
//! the slot free, is served from it. Anything else, and everything when the
//! slot is taken, is the system allocator's. A freed pointer inside the
//! arena gives its slot back.

#[cfg(feature = "psi")]
mod on {
    use opendmd_core::fx_psi as fx;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering::Relaxed};

    const fn up8(n: usize) -> usize {
        (n + 7) & !7
    }
    /// The slots: (offset, bytes).
    const SLOTS: [(usize, usize); 3] = [
        (0, fx::FX_BYTES),
        (up8(fx::FX_BYTES), fx::INK_BYTES),
        (up8(fx::FX_BYTES) + up8(fx::INK_BYTES), fx::PLANE_BYTES),
    ];
    const BYTES: usize = up8(fx::FX_BYTES) + up8(fx::INK_BYTES) + up8(fx::PLANE_BYTES);

    static BASE: AtomicUsize = AtomicUsize::new(0);
    static TAKEN: AtomicU8 = AtomicU8::new(0);
    /// Served from the arena, and wanted but busy (a slot already taken).
    pub static HITS: AtomicUsize = AtomicUsize::new(0);
    pub static BUSY: AtomicUsize = AtomicUsize::new(0);

    pub struct Alloc;

    /// `emu-census` builds: the bytes the Rust allocator holds for its
    /// callers (as asked), for the low watch (`net_worker`).
    #[cfg(feature = "emu-census")]
    pub static RUST_LIVE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
    #[cfg(feature = "emu-census")]
    #[inline(always)]
    fn note(d: isize) {
        RUST_LIVE.fetch_add(d, Relaxed);
    }
    #[cfg(not(feature = "emu-census"))]
    #[inline(always)]
    fn note(_d: isize) {}

    // The heap map's allocator (`heapmap`, a diagnostic build) is the
    // global one there, and serves the arena's slots through `claim`.
    #[cfg_attr(not(feature = "heapmap"), global_allocator)]
    #[allow(dead_code)]
    static A: Alloc = Alloc;

    #[inline(never)]
    pub fn claim(l: Layout) -> Option<*mut u8> {
        let base = BASE.load(Relaxed);
        if base == 0 || l.align() > 8 {
            return None;
        }
        let slot = match fx::ARENA_HINT.load(Relaxed) {
            fx::ARENA_FX if l.size() == fx::FX_BYTES => 0,
            fx::ARENA_CANVAS if l.size() == fx::INK_BYTES => 1,
            fx::ARENA_CANVAS if l.size() == fx::PLANE_BYTES => 2,
            _ => return None,
        };
        let bit = 1u8 << slot;
        if TAKEN.fetch_or(bit, Relaxed) & bit != 0 {
            BUSY.fetch_add(1, Relaxed);
            return None;
        }
        HITS.fetch_add(1, Relaxed);
        Some((base + SLOTS[slot].0) as *mut u8)
    }

    /// Give a slot back if `p` is one: true.
    pub fn release(p: *mut u8) -> bool {
        let base = BASE.load(Relaxed);
        let a = p as usize;
        if base == 0 || a < base || a >= base + BYTES {
            return false;
        }
        if let Some(k) = SLOTS.iter().position(|&(off, _)| base + off == a) {
            TAKEN.fetch_and(!(1u8 << k), Relaxed);
        }
        true
    }

    // Out of line, each: inlined into every allocation in the image (LTO)
    // they cost ~220 KB of flash.
    unsafe impl GlobalAlloc for Alloc {
        #[inline(never)]
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            note(l.size() as isize);
            if let Some(p) = claim(l) {
                return p;
            }
            #[cfg(all(esp32s3, feature = "psram-cold"))]
            if let Some(p) = super::cold::alloc(l, false) {
                return p;
            }
            // SAFETY: as the caller's.
            unsafe { System.alloc(l) }
        }

        #[inline(never)]
        unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
            note(l.size() as isize);
            if let Some(p) = claim(l) {
                // SAFETY: the slot is `l.size()` bytes of the arena, ours now.
                unsafe { p.write_bytes(0, l.size()) };
                return p;
            }
            #[cfg(all(esp32s3, feature = "psram-cold"))]
            if let Some(p) = super::cold::alloc(l, true) {
                return p;
            }
            // SAFETY: as the caller's.
            unsafe { System.alloc_zeroed(l) }
        }

        #[inline(never)]
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            note(-(l.size() as isize));
            #[cfg(all(esp32s3, feature = "psram-cold"))]
            if super::cold::free(p, l) {
                return;
            }
            if !release(p) {
                // SAFETY: not the arena's, so the system's.
                unsafe { System.dealloc(p, l) }
            }
        }

        #[inline(never)]
        unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
            note(new as isize - l.size() as isize);
            #[cfg(all(esp32s3, feature = "psram-cold"))]
            if super::cold::holds(p) {
                // A cold block moves as a new one of its own (so the
                // counts stay right): PSRAM again if it may, else internal.
                // SAFETY: the caller's alignment, a size it asks for.
                let to = unsafe { Layout::from_size_align_unchecked(new, l.align()) };
                let n = match super::cold::alloc(to, false) {
                    Some(n) => n,
                    // SAFETY: as the caller's.
                    None => unsafe { System.alloc(to) },
                };
                if !n.is_null() {
                    // SAFETY: both blocks are at least the smaller size.
                    unsafe { n.copy_from_nonoverlapping(p, l.size().min(new)) };
                    super::cold::free(p, l);
                }
                return n;
            }
            // An internal block growing past the threshold moves out too.
            #[cfg(all(esp32s3, feature = "psram-cold"))]
            if new > l.size() && new >= super::cold::MIN && !release_check(p) {
                // SAFETY: the caller's alignment, a size it asks for.
                let to = unsafe { Layout::from_size_align_unchecked(new, l.align()) };
                if let Some(n) = super::cold::alloc(to, false) {
                    // SAFETY: the new block is the larger; the old is the system's.
                    unsafe {
                        n.copy_from_nonoverlapping(p, l.size());
                        System.dealloc(p, l);
                    }
                    return n;
                }
            }
            if !release_check(p) {
                // SAFETY: not the arena's, so the system's.
                return unsafe { System.realloc(p, l, new) };
            }
            // (No arena allocation is ever resized: a Box each.)
            // SAFETY: a fresh block, the old one's bytes copied, then freed.
            unsafe {
                let n = System.alloc(Layout::from_size_align_unchecked(new, l.align()));
                if !n.is_null() {
                    n.copy_from_nonoverlapping(p, l.size().min(new));
                    release(p);
                }
                n
            }
        }
    }

    pub fn release_check(p: *mut u8) -> bool {
        let base = BASE.load(Relaxed);
        let a = p as usize;
        base != 0 && a >= base && a < base + BYTES
    }

    /// Set the arena aside (at boot, before the network starts).
    pub fn reserve() {
        // SAFETY: a plain allocation, kept for the life of the program.
        let p = unsafe { System.alloc(Layout::from_size_align(BYTES, 8).unwrap()) };
        if p.is_null() {
            log::warn!("formula arena: {BYTES} B not available; off this boot");
            return;
        }
        BASE.store(p as usize, Relaxed);
        log::info!("formula arena: {BYTES} B at {:p} ({})", p, crate::membank::pool(p as usize));
    }

    /// (address, bytes), (0, 0) when off.
    pub fn span() -> (usize, usize) {
        match BASE.load(Relaxed) {
            0 => (0, 0),
            b => (b, BYTES),
        }
    }
}

/// `psram-cold` (ESP32-S3 measurement builds only, S3 port phase 1): the
/// cold, bursty buffers in PSRAM. Every Rust allocation of [`cold::MIN`]
/// bytes or more made on a task other than the display's (the HTTP
/// handlers' transfers, the cache builder, the Formula compiler's task, the
/// network worker) is taken from the PSRAM pool first, internal RAM if that
/// fails; the display task's (canvas, decoders, looks, the Formula's
/// buffers) stay internal. A blunt switch for measuring what moving them
/// frees, not a placement policy: a compiled Formula made on the compiler's
/// task and read by the display per frame lands in PSRAM too.
#[cfg(all(esp32s3, feature = "psram-cold"))]
pub mod cold {
    use esp_idf_sys as sys;
    use std::alloc::Layout;
    use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering::Relaxed};

    /// The smallest allocation moved.
    pub const MIN: usize = 1024;
    /// The display task (`set_display_task`, at boot); 0: nothing moves yet.
    static DISPLAY: AtomicUsize = AtomicUsize::new(0);
    /// Blocks taken from PSRAM, bytes held there now and at most, and
    /// allocations that wanted PSRAM and got internal RAM.
    pub static TAKEN: AtomicUsize = AtomicUsize::new(0);
    pub static LIVE: AtomicIsize = AtomicIsize::new(0);
    pub static PEAK: AtomicIsize = AtomicIsize::new(0);
    pub static FELL_BACK: AtomicUsize = AtomicUsize::new(0);

    /// The calling task is the display's (`main`, before the loop starts).
    pub fn set_display_task() {
        // SAFETY: the calling task's own handle.
        DISPLAY.store(unsafe { sys::xTaskGetCurrentTaskHandle() } as usize, Relaxed);
    }

    /// The PSRAM's data mapping (`membank::pool`'s `PSRAM`).
    pub fn holds(p: *mut u8) -> bool {
        (0x3C00_0000..0x3E00_0000).contains(&(p as usize))
    }

    #[inline(never)]
    pub fn alloc(l: Layout, zeroed: bool) -> Option<*mut u8> {
        // (heap_caps_malloc aligns to 4: larger alignments stay the
        // system allocator's.)
        if l.size() < MIN || l.align() > 4 {
            return None;
        }
        let d = DISPLAY.load(Relaxed);
        // SAFETY: the calling task's own handle.
        if d == 0 || unsafe { sys::xTaskGetCurrentTaskHandle() } as usize == d {
            return None;
        }
        let caps = sys::MALLOC_CAP_SPIRAM | sys::MALLOC_CAP_8BIT;
        // SAFETY: plain allocations with a capability mask.
        let p = unsafe {
            if zeroed {
                sys::heap_caps_calloc(1, l.size(), caps)
            } else {
                sys::heap_caps_malloc(l.size(), caps)
            }
        } as *mut u8;
        if p.is_null() {
            FELL_BACK.fetch_add(1, Relaxed);
            return None;
        }
        TAKEN.fetch_add(1, Relaxed);
        let now = LIVE.fetch_add(l.size() as isize, Relaxed) + l.size() as isize;
        PEAK.fetch_max(now, Relaxed);
        Some(p)
    }

    /// Free `p` if it is a PSRAM block: true.
    #[inline(never)]
    pub fn free(p: *mut u8, l: Layout) -> bool {
        if !holds(p) {
            return false;
        }
        LIVE.fetch_sub(l.size() as isize, Relaxed);
        // SAFETY: a block heap_caps_malloc gave (`alloc`).
        unsafe { sys::heap_caps_free(p.cast()) };
        true
    }

    /// `{"min":N,"taken":N,"live":N,"peak":N,"fell_back":N}`.
    pub fn json() -> String {
        format!(
            r#"{{"min":{MIN},"taken":{},"live":{},"peak":{},"fell_back":{}}}"#,
            TAKEN.load(Relaxed),
            LIVE.load(Relaxed),
            PEAK.load(Relaxed),
            FELL_BACK.load(Relaxed)
        )
    }
}

#[cfg(all(feature = "psram-cold", not(esp32s3)))]
compile_error!("psram-cold is an ESP32-S3 measurement switch (VARIANT=s3)");
#[cfg(all(feature = "psram-cold", any(feature = "heapmap", not(feature = "psi"))))]
compile_error!("psram-cold works through the arena's allocator: not with heapmap, nor without psi");

#[cfg(feature = "psi")]
pub use on::{reserve, span, BUSY, HITS};
#[cfg(all(feature = "psi", feature = "emu-census"))]
pub use on::RUST_LIVE;
#[cfg(all(feature = "psi", feature = "heapmap"))]
pub use on::{claim, release, release_check as holds};
