//! Allocator wrapper metering heap allocation. Counts requested byte sizes, so
//! the numbers are deterministic (independent of OS/page size/RSS) and reserved
//! (a `vec![0; N]` counts N at once). `allocated()` = cumulative volume;
//! `peak()` = live high-water mark.
#![no_std]

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);

/// Cumulative bytes ever requested (allocation activity).
pub fn allocated() -> u64 {
    TOTAL.load(Ordering::Relaxed) as u64
}

/// High-water mark of simultaneously-live bytes (reserved footprint).
pub fn peak() -> u64 {
    PEAK.load(Ordering::Relaxed) as u64
}

#[inline]
fn on_alloc(size: usize) {
    TOTAL.fetch_add(size, Ordering::Relaxed);
    let now = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    let mut p = PEAK.load(Ordering::Relaxed);
    while now > p {
        match PEAK.compare_exchange_weak(p, now, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(e) => p = e,
        }
    }
}

#[inline]
fn on_free(size: usize) {
    LIVE.fetch_sub(size, Ordering::Relaxed);
}

/// Wraps any allocator and records allocation telemetry around it.
pub struct Tracking<A>(pub A);

unsafe impl<A: GlobalAlloc> GlobalAlloc for Tracking<A> {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = self.0.alloc(l);
        if !p.is_null() {
            on_alloc(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        on_free(l.size());
        self.0.dealloc(p, l);
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = self.0.alloc_zeroed(l);
        if !p.is_null() {
            on_alloc(l.size());
        }
        p
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let np = self.0.realloc(p, l, new);
        if !np.is_null() {
            if new >= l.size() {
                on_alloc(new - l.size());
            } else {
                on_free(l.size() - new);
            }
        }
        np
    }
}
