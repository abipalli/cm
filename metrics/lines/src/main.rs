//! Distinct cache-lines touched, via wasm load/store instrumentation.
//!
//! Rewrites the meter wasm so every memory load/store first calls an imported
//! `bench.track(addr, size)`; runs it under wasmtime; counts distinct 64-byte
//! lines. Deterministic, and immune to allocation-source gaming — it counts
//! touched memory regardless of heap/static/stack origin (unlike the GlobalAlloc
//! meters). Uses the SAME high-entropy stream + (full−half) init-free
//! differencing as MEMCOST, so LINES is directly comparable to MEMCOST.

use std::collections::HashSet;
use walrus::ir::{BinaryOp, Binop, Call, Const, Instr, InstrSeqId, LoadKind, LocalGet, LocalSet,
    LocalTee, StoreKind, Value};
use walrus::{FunctionId, FunctionKind, LocalId, Module, ValType};

const LINE: u64 = 64;

fn load_size(k: LoadKind) -> i32 {
    use LoadKind::*;
    match k {
        I32 { .. } | F32 => 4,
        I64 { .. } | F64 => 8,
        V128 => 16,
        I32_8 { .. } | I64_8 { .. } => 1,
        I32_16 { .. } | I64_16 { .. } => 2,
        I64_32 { .. } => 4,
    }
}

fn store_info(k: StoreKind) -> (i32, ValType) {
    use StoreKind::*;
    match k {
        I32 { .. } => (4, ValType::I32),
        I64 { .. } => (8, ValType::I64),
        F32 => (4, ValType::F32),
        F64 => (8, ValType::F64),
        V128 => (16, ValType::V128),
        I32_8 { .. } => (1, ValType::I32),
        I32_16 { .. } => (2, ValType::I32),
        I64_8 { .. } => (1, ValType::I64),
        I64_16 { .. } => (2, ValType::I64),
        I64_32 { .. } => (4, ValType::I64),
    }
}

/// Instructions that record the effective address (top-of-stack base + offset)
/// of an access of `size` bytes, leaving the stack unchanged.
fn track_seq(addr: LocalId, off: u32, size: i32, track: FunctionId) -> Vec<Instr> {
    vec![
        Instr::LocalTee(LocalTee { local: addr }),
        Instr::LocalGet(LocalGet { local: addr }),
        Instr::Const(Const { value: Value::I32(off as i32) }),
        Instr::Binop(Binop { op: BinaryOp::I32Add }),
        Instr::Const(Const { value: Value::I32(size) }),
        Instr::Call(Call { func: track }),
    ]
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: cm-lines-meter <module.wasm>");
    let mut m = Module::from_file(&path).expect("parse wasm");

    let ty = m.types.add(&[ValType::I32, ValType::I32], &[]);
    let (track, _) = m.add_import_func("bench", "track", ty);
    let addr = m.locals.add(ValType::I32);
    let vloc = |m: &mut Module, t: ValType| m.locals.add(t);
    let (vi32, vi64, vf32, vf64) = (
        vloc(&mut m, ValType::I32),
        vloc(&mut m, ValType::I64),
        vloc(&mut m, ValType::F32),
        vloc(&mut m, ValType::F64),
    );

    let fids: Vec<FunctionId> = m
        .funcs
        .iter_local()
        .map(|(id, _)| id)
        .collect();

    for fid in fids {
        let lf = match &mut m.funcs.get_mut(fid).kind {
            FunctionKind::Local(lf) => lf,
            _ => continue,
        };
        // gather all instruction sequences (recurse through control flow)
        let mut seqs: Vec<InstrSeqId> = vec![lf.entry_block()];
        let mut i = 0;
        while i < seqs.len() {
            let sid = seqs[i];
            i += 1;
            for (instr, _) in lf.block(sid).instrs.iter() {
                match instr {
                    Instr::Block(b) => seqs.push(b.seq),
                    Instr::Loop(l) => seqs.push(l.seq),
                    Instr::IfElse(ie) => {
                        seqs.push(ie.consequent);
                        seqs.push(ie.alternative);
                    }
                    _ => {}
                }
            }
        }
        for sid in seqs {
            let old = std::mem::take(&mut lf.block_mut(sid).instrs);
            let mut out = Vec::with_capacity(old.len());
            for (instr, loc) in old {
                match &instr {
                    Instr::Load(ld) => {
                        let sz = load_size(ld.kind);
                        for ins in track_seq(addr, ld.arg.offset, sz, track) {
                            out.push((ins, loc));
                        }
                        out.push((instr, loc));
                    }
                    Instr::Store(st) => {
                        let (sz, vt) = store_info(st.kind);
                        let v = match vt {
                            ValType::I32 => vi32,
                            ValType::I64 => vi64,
                            ValType::F32 => vf32,
                            ValType::F64 => vf64,
                            _ => {
                                out.push((instr, loc));
                                continue;
                            }
                        };
                        out.push((Instr::LocalSet(LocalSet { local: v }), loc));
                        for ins in track_seq(addr, st.arg.offset, sz, track) {
                            out.push((ins, loc));
                        }
                        out.push((Instr::LocalGet(LocalGet { local: v }), loc));
                        out.push((instr, loc));
                    }
                    _ => out.push((instr, loc)),
                }
            }
            lf.block_mut(sid).instrs = out;
        }
    }

    let bytes = m.emit_wasm();
    run(&bytes);
}

const FULL: u32 = 8192;
const HALF: u32 = 4096;
const EXPORT: &str = "compress_prefix_he";

fn run(wasm: &[u8]) {
    use wasmtime::*;
    let engine = Engine::default();
    let module = Module::from_binary(&engine, wasm).expect("parse instrumented");
    let count = |prefix: u32| -> u64 {
        let mut store = Store::new(&engine, HashSet::<u64>::new());
        let mut linker = Linker::new(&engine);
        linker
            .func_wrap("bench", "track", |mut caller: Caller<'_, HashSet<u64>>, addr: i32, size: i32| {
                let a = addr as u64;
                let lo = a / LINE;
                let hi = (a + size as u64 - 1) / LINE;
                let set = caller.data_mut();
                for line in lo..=hi { set.insert(line); }
            })
            .unwrap();
        let instance = linker.instantiate(&mut store, &module).expect("instantiate");
        let f = instance.get_typed_func::<u32, u32>(&mut store, EXPORT).expect(EXPORT);
        f.call(&mut store, prefix).expect("call");
        store.data().len() as u64
    };
    let full = count(FULL);
    let half = count(HALF);
    assert_eq!(full, count(FULL), "non-deterministic line count (full)");
    assert_eq!(half, count(HALF), "non-deterministic line count (half)");
    let lines = full.saturating_sub(half);
    println!("full {}B: {} distinct 64B lines", FULL, full);
    println!("half {}B: {} distinct 64B lines", HALF, half);
    println!("LINES: {} (deterministic, init-free distinct 64B lines touched for {} bytes; counts heap+static+stack; lower is friendlier to memory)", lines, FULL - HALF);
}
