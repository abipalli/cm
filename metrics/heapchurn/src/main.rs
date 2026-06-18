//! HEAP_CHURN: init-free heap bytes the codec requests in steady state. Runs
//! the heap-instrumented shim (build metrics/wasm with --features heap) at FULL
//! and HALF prefixes in fresh instances and differences cumulative heap bytes,
//! so the one-time table allocation cancels. Deterministic; non-scoring
//! diagnostic. Outside src/algorithm/. FROZEN.
use wasmtime::{Engine, Instance, Module, Store};
const FULL: u32 = 8192;
const HALF: u32 = 4096;
fn main() {
    let path = std::env::args().nth(1).expect("usage: cm-heapchurn-meter <module.wasm>");
    let wasm = std::fs::read(&path).expect("read wasm");
    let engine = Engine::default();
    let module = Module::from_binary(&engine, &wasm).expect("parse wasm");
    let measure = |prefix: u32| -> u64 {
        let mut store = Store::new(&engine, ());
        let instance = Instance::new(&mut store, &module, &[]).expect("instantiate");
        let compress = instance.get_typed_func::<u32, u32>(&mut store, "compress_prefix").expect("compress_prefix");
        let heap = instance.get_typed_func::<(), u64>(&mut store, "cm_heap_bytes").expect("cm_heap_bytes — build the shim with --features heap");
        let h0 = heap.call(&mut store, ()).unwrap();
        compress.call(&mut store, prefix).expect("call");
        let h1 = heap.call(&mut store, ()).unwrap();
        h1 - h0
    };
    let full = measure(FULL);
    let half = measure(HALF);
    let churn = full.saturating_sub(half);
    println!("full {}B: heap {} B", FULL, full);
    println!("half {}B: heap {} B", HALF, half);
    println!("HEAP_CHURN: {} (deterministic, init-free heap bytes requested for {} bytes; ~steady-state allocation; lower is leaner)", churn, FULL - HALF);
}
