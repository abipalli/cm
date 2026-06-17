//! WebAssembly measurement shim — lives OUTSIDE src/algorithm/, so a submission
//! cannot touch the measurement path. Embeds the fixed metric corpus and exposes
//! one export that compresses a prefix of it; the runtime's fuel meter counts the
//! executed operators. The algorithm itself stays completely uninstrumented.
static CORPUS: &[u8] = include_bytes!("../../../tiny/sample.bin");

/// Compress the first `n` bytes of the embedded corpus; return the output length
/// (so the call can't be optimized away). The host runs this at two prefix
/// lengths and subtracts, cancelling the fixed one-time setup cost.
#[no_mangle]
pub extern "C" fn compress_prefix(n: u32) -> u32 {
    let n = (n as usize).min(CORPUS.len());
    cm::compress(&CORPUS[..n]).len() as u32
}
