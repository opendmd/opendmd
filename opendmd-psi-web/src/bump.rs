//! The module's allocator: a bump pointer over pages grown on demand, which
//! starts over whenever no allocation is live.
//!
//! Everything long-lived is a static (`STATE`), so what the heap holds is
//! the compiler's tokens and nodes (a few KB, all freed before `compile`
//! returns) and the strings and arrays crossing to JS (freed by the glue once
//! copied). Between calls nothing is live and the next allocation starts at
//! the bottom again; the high-water mark is one compile's worth. A `realloc`
//! of the newest block grows it in place. (std's dlmalloc and panic
//! machinery were ~9 KB of the gzipped module.)

use core::alloc::{GlobalAlloc, Layout};
use core::arch::wasm32::{memory_grow, memory_size};
use core::cell::UnsafeCell;

const PAGE: usize = 65_536;

struct Bump(UnsafeCell<Inner>);

struct Inner {
    /// The heap's bottom (0: not yet set up), the next free byte, the end of
    /// memory, the newest block and how many blocks are live.
    base: usize,
    next: usize,
    end: usize,
    last: usize,
    live: usize,
}

// SAFETY: wasm here is single-threaded.
unsafe impl Sync for Bump {}

#[global_allocator]
static HEAP: Bump = Bump(UnsafeCell::new(Inner {
    base: 0,
    next: 0,
    end: 0,
    last: 0,
    live: 0,
}));

impl Inner {
    fn take(&mut self, l: Layout) -> *mut u8 {
        if self.base == 0 {
            // Above everything there is: the statics and the stack.
            self.base = memory_size(0) * PAGE;
            self.next = self.base;
            self.end = self.base;
        }
        let at = (self.next + l.align() - 1) & !(l.align() - 1);
        let Some(top) = at.checked_add(l.size()) else {
            return core::ptr::null_mut();
        };
        if top > self.end {
            let pages = (top - self.end).div_ceil(PAGE);
            if memory_grow(0, pages) == usize::MAX {
                return core::ptr::null_mut();
            }
            self.end += pages * PAGE;
        }
        self.next = top;
        self.last = at;
        self.live += 1;
        at as *mut u8
    }
}

unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        (*self.0.get()).take(l)
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        let h = &mut *self.0.get();
        h.live -= 1;
        if h.live == 0 {
            h.next = h.base;
        } else if p as usize == h.last && p as usize + l.size() == h.next {
            h.next = h.last;
        }
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, size: usize) -> *mut u8 {
        let h = &mut *self.0.get();
        if p as usize == h.last && p as usize + l.size() == h.next {
            // The newest block: grow (or shrink) it where it is.
            let top = h.last + size;
            if top > h.end {
                let pages = (top - h.end).div_ceil(PAGE);
                if memory_grow(0, pages) == usize::MAX {
                    return core::ptr::null_mut();
                }
                h.end += pages * PAGE;
            }
            h.next = top;
            return p;
        }
        let q = h.take(Layout::from_size_align_unchecked(size, l.align()));
        if !q.is_null() {
            core::ptr::copy_nonoverlapping(p, q, l.size().min(size));
            self.dealloc(p, l);
        }
        q
    }
}
