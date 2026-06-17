//! WebAssembly measurement shim (outside src/algorithm/). Compresses a prefix of
//! the embedded corpus under the tracking allocator; the host meters fuel + heap
//! bytes and reads peak linear memory.
use cm_telemetry::Tracking;
use std::alloc::System;

#[global_allocator]
static ALLOC: Tracking<System> = Tracking(System);

static CORPUS: &[u8] = include_bytes!("../../../tiny/sample.bin");

/// Cumulative heap bytes requested so far (allocation activity, for WORK).
#[no_mangle]
pub extern "C" fn cm_heap_bytes() -> u64 {
    cm_telemetry::allocated()
}

/// Peak wasm linear-memory size in 64 KiB pages (heap + stack + statics).
#[no_mangle]
pub extern "C" fn cm_mem_pages() -> u32 {
    core::arch::wasm32::memory_size(0) as u32
}

/// Compress the first `n` bytes of the embedded corpus; return the output length
/// (so the call can't be optimized away). The host runs this at two prefix
/// lengths and subtracts, cancelling the fixed one-time setup cost.
#[no_mangle]
pub extern "C" fn compress_prefix(n: u32) -> u32 {
    let n = (n as usize).min(CORPUS.len());
    cm::compress(&CORPUS[..n]).len() as u32
}
