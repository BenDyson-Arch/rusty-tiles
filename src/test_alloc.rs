//! Test-only allocation accounting: a global allocator that, on a thread that asks for it,
//! tracks live bytes and their peak, and a meshoptimizer allocator routed through it, so a
//! test can bound everything one job allocates (meshopt's temporary arrays included).
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    ffi::c_void,
};

thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn note(delta: isize) {
    let _ = TRACK.try_with(|track| {
        if track.get() {
            LIVE.with(|live| {
                let now = live.get() + delta;
                live.set(now);
                PEAK.with(|peak| peak.set(peak.get().max(now)));
            });
        }
    });
}

pub(crate) struct Counting;

// SAFETY: every call forwards to `System` unchanged; the bookkeeping only reads sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            note(layout.size() as isize);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            note(layout.size() as isize);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        note(-(layout.size() as isize));
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // Counted as the new block before the old one is freed: a move's transient peak.
        note(size as isize);
        let q = unsafe { System.realloc(p, layout, size) };
        if q.is_null() {
            note(-(size as isize));
        } else {
            note(-(layout.size() as isize));
        }
        q
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const HEADER: usize = 16;

unsafe extern "C" fn meshopt_alloc(size: usize) -> *mut c_void {
    let layout = Layout::from_size_align(size + HEADER, HEADER).unwrap();
    // SAFETY: a nonzero layout; the size is stored in the header for the free.
    unsafe {
        let p = std::alloc::alloc(layout);
        assert!(!p.is_null());
        p.cast::<usize>().write(size);
        p.add(HEADER).cast()
    }
}

unsafe extern "C" fn meshopt_free(p: *mut c_void) {
    if p.is_null() {
        return;
    }
    // SAFETY: `p` came from `meshopt_alloc`, which wrote the size before it.
    unsafe {
        let base = p.cast::<u8>().sub(HEADER);
        let size = base.cast::<usize>().read();
        std::alloc::dealloc(
            base,
            Layout::from_size_align(size + HEADER, HEADER).unwrap(),
        );
    }
}

/// Route meshoptimizer's temporary allocations through the counting allocator (process
/// wide; harmless for other tests).
pub(crate) fn route_meshopt() {
    // SAFETY: both callbacks match meshopt's allocator contract (size in, pointer out; free
    // of a pointer it returned or null).
    unsafe { meshopt::ffi::meshopt_setAllocator(Some(meshopt_alloc), Some(meshopt_free)) };
}

/// `f`'s result and the peak bytes it held live on this thread (net of its frees).
pub(crate) fn peak<R>(f: impl FnOnce() -> R) -> (R, usize) {
    LIVE.with(|l| l.set(0));
    PEAK.with(|p| p.set(0));
    TRACK.with(|t| t.set(true));
    let result = f();
    TRACK.with(|t| t.set(false));
    (result, PEAK.with(|p| p.get()).max(0) as usize)
}
