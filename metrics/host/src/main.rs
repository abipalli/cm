//! Deterministic, tamper-proof complexity meter. Loads the wasm shim and reports
//! WORK = fuel(full prefix) − fuel(half prefix), i.e. the executed-operator count
//! attributable to compressing the extra bytes — with the one-time Cm setup/table
//! init cancelled out. Lives outside src/algorithm/, so submissions can't alter it.
use wasmtime::{Config, Engine, Instance, Module, Store};

const FULL: u32 = 8192; // whole embedded corpus
const HALF: u32 = 4096; // same table-size regime, so setup cost cancels

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: cm-fuel-meter <module.wasm>");
    let wasm = std::fs::read(&path).expect("read wasm");

    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).expect("engine");
    let module = Module::from_binary(&engine, &wasm).expect("parse wasm");

    let mut store = Store::new(&engine, ());
    store.set_fuel(1_000_000_000_000_000).expect("set fuel");
    let instance = Instance::new(&mut store, &module, &[]).expect("instantiate");
    let f = instance
        .get_typed_func::<u32, u32>(&mut store, "compress_prefix")
        .expect("get compress_prefix");

    let b1 = store.get_fuel().unwrap();
    let out_full = f.call(&mut store, FULL).expect("call full");
    let a1 = store.get_fuel().unwrap();
    let fuel_full = b1 - a1;

    let out_half = f.call(&mut store, HALF).expect("call half");
    let a2 = store.get_fuel().unwrap();
    let fuel_half = a1 - a2;

    let work = fuel_full - fuel_half;
    println!("full {}B -> {}B (fuel {})", FULL, out_full, fuel_full);
    println!("half {}B -> {}B (fuel {})", HALF, out_half, fuel_half);
    println!(
        "WORK: {} (deterministic, init-free wasm operators for {} bytes; lower is faster)",
        work,
        FULL - HALF
    );
}
