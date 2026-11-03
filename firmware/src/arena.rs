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

    #[global_allocator]
    static A: Alloc = Alloc;

    #[inline(never)]
    fn claim(l: Layout) -> Option<*mut u8> {
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
    fn release(p: *mut u8) -> bool {
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
            if let Some(p) = claim(l) {
                return p;
            }
            // SAFETY: as the caller's.
            unsafe { System.alloc(l) }
        }

        #[inline(never)]
        unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
            if let Some(p) = claim(l) {
                // SAFETY: the slot is `l.size()` bytes of the arena, ours now.
                unsafe { p.write_bytes(0, l.size()) };
                return p;
            }
            // SAFETY: as the caller's.
            unsafe { System.alloc_zeroed(l) }
        }

        #[inline(never)]
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            if !release(p) {
                // SAFETY: not the arena's, so the system's.
                unsafe { System.dealloc(p, l) }
            }
        }

        #[inline(never)]
        unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
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

    fn release_check(p: *mut u8) -> bool {
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

#[cfg(feature = "psi")]
pub use on::{reserve, span, BUSY, HITS};
