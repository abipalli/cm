//! HEAP_PEAK: peak live reserved heap over the full corpus, under a tracking
//! allocator. Deterministic (sums requested sizes; independent of OS/page/RSS).
//! Heap only (static/stack invisible) — a non-scoring diagnostic. Outside
//! src/algorithm/, so a submission cannot alter it. FROZEN.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
struct Tracking;
#[inline]
fn bump(sz: usize) {
    let now = LIVE.fetch_add(sz, Ordering::Relaxed) + sz;
    let mut p = PEAK.load(Ordering::Relaxed);
    while now > p {
        match PEAK.compare_exchange_weak(p, now, Ordering::Relaxed, Ordering::Relaxed) { Ok(_) => break, Err(e) => p = e }
    }
}
unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 { let p = System.alloc(l); if !p.is_null() { bump(l.size()); } p }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 { let p = System.alloc_zeroed(l); if !p.is_null() { bump(l.size()); } p }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { LIVE.fetch_sub(l.size(), Ordering::Relaxed); System.dealloc(p, l); }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 { let np = System.realloc(p, l, new); if !np.is_null() { if new >= l.size() { bump(new - l.size()); } else { LIVE.fetch_sub(l.size() - new, Ordering::Relaxed); } } np }
}
#[global_allocator]
static ALLOC: Tracking = Tracking;
fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "corpus".to_string());
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("cannot read corpus dir: {dir}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "bin").unwrap_or(false))
        .collect();
    files.sort();
    for f in &files {
        let data = std::fs::read(f).expect("read corpus file");
        std::hint::black_box(cm::compress(&data));
    }
    println!("HEAP_PEAK: {} (peak live reserved heap bytes over the full corpus; deterministic, heap-only diagnostic; lower is leaner)", PEAK.load(Ordering::Relaxed));
}
