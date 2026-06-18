//! WebAssembly measurement shim — lives OUTSIDE src/algorithm/, so a submission
//! cannot touch the measurement path. Embeds the fixed metric corpus and exposes
//! one export that compresses a prefix of it; the runtime's fuel meter counts the
//! executed operators. The algorithm itself stays completely uninstrumented.
#[cfg(feature = "heap")]
mod heaptrack {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicU64, Ordering};
    static TOTAL: AtomicU64 = AtomicU64::new(0);
    pub struct T;
    unsafe impl GlobalAlloc for T {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 { let p = System.alloc(l); if !p.is_null() { TOTAL.fetch_add(l.size() as u64, Ordering::Relaxed); } p }
        unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 { let p = System.alloc_zeroed(l); if !p.is_null() { TOTAL.fetch_add(l.size() as u64, Ordering::Relaxed); } p }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) { System.dealloc(p, l); }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 { let np = System.realloc(p, l, new); if !np.is_null() && new > l.size() { TOTAL.fetch_add((new - l.size()) as u64, Ordering::Relaxed); } np }
    }
    #[global_allocator]
    static A: T = T;
    #[no_mangle]
    pub extern "C" fn cm_heap_bytes() -> u64 { TOTAL.load(Ordering::Relaxed) }
}

static CORPUS: &[u8] = include_bytes!("../../../tiny/sample.bin");

/// Compress the first `n` bytes of the embedded corpus; return the output length
/// (so the call can't be optimized away). The host runs this at two prefix
/// lengths and subtracts, cancelling the fixed one-time setup cost.
#[no_mangle]
pub extern "C" fn compress_prefix(n: u32) -> u32 {
    let n = (n as usize).min(CORPUS.len());
    cm::compress(&CORPUS[..n]).len() as u32
}

/// Compress `n` bytes of deterministic HIGH-ENTROPY data (a fixed splitmix64
/// stream). Used by the memory-traffic meter: unlike the repetitive text corpus,
/// high-entropy input forces the codec to touch many distinct context-table slots
/// and grow the CTW node store, so the *active* working set is large and the
/// cache model sees realistic miss traffic. The stream is fixed (seeded), so the
/// measurement stays deterministic and machine-independent.
#[no_mangle]
pub extern "C" fn compress_prefix_he(n: u32) -> u32 {
    let n = n as usize;
    let mut buf = vec![0u8; n];
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for b in buf.iter_mut() {
        // splitmix64
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        *b = (z >> 24) as u8;
    }
    cm::compress(&buf).len() as u32
}
