//! Deterministic, tamper-proof memory-traffic meter for the cm wasm.
//!
//! WORK (the fuel meter) counts executed wasm operators, so a cache miss and an
//! L1 hit look identical — yet native wall-clock is dominated by cache/memory
//! latency. This meter measures that axis instead:
//!
//!   1. Instrument the wasm shim (walrus): inject a `mem.track(addr,size,rw)` host
//!      call before every linear-memory load/store. The shim lives outside
//!      `src/algorithm/`, so a submission cannot touch the measurement.
//!   2. Run `compress_prefix(FULL)` and `compress_prefix(HALF)` — each in a FRESH
//!      instance (cold linear memory / allocator, so the allocator's bookkeeping
//!      accesses are reproducible) — feeding every access through a fixed 3-level
//!      LRU cache model.
//!   3. Report MEMCOST = (full − half) weighted miss penalty: the steady-state
//!      memory cost of the extra `FULL−HALF` bytes, with one-time setup cancelled.
//!
//! Determinism: the wasm operator stream is reproducible, the instrumenter is a
//! pure function of the input module, and the cache model is integer-only — so
//! the number is identical across runs and machines given the pinned toolchain.
//!
//! Caveat: wasm linear-memory layout differs from a native allocator, so absolute
//! misses are not native cachegrind; but the access *pattern* (locality, reuse)
//! is identical, so MEMCOST is a faithful relative/ranking metric.
//!
//! FROZEN — not part of the editable algorithm surface.

use anyhow::Result;
use walrus::ir::*;
use walrus::{FunctionId, FunctionKind, LocalFunction, LocalId, ValType};
use wasmtime::{Caller, Config, Engine, Linker};

// Measurement window over the HIGH-ENTROPY stream (compress_prefix_he): high
// entropy fills the context tables and grows the CTW, so the active working set
// — and thus the cache traffic — is representative rather than the tiny repetitive
// corpus's near-resident footprint. The (full−half) subtraction cancels one-time
// setup. Lower MEMCOST = friendlier to cache/memory.
const FULL: u32 = 8192;
const HALF: u32 = 4096;
const EXPORT: &str = "compress_prefix_he";

// Fixed cache model (a representative desktop hierarchy). Frozen for determinism.
const LINE: usize = 64;
const L1_BYTES: usize = 32 * 1024;
const L1_WAYS: usize = 8;
const L2_BYTES: usize = 1024 * 1024;
const L2_WAYS: usize = 8;
const L3_BYTES: usize = 32 * 1024 * 1024;
const L3_WAYS: usize = 16;
// Miss penalties (cycles) at each level; an L3 miss goes to DRAM.
const PEN_L1: u64 = 14;
const PEN_L2: u64 = 40;
const PEN_L3: u64 = 200;

// ---------- set-associative LRU cache ----------

struct Cache {
    set_mask: u64,
    ways: usize,
    tags: Vec<u64>, // u64::MAX = empty
    age: Vec<u64>,
}
impl Cache {
    fn new(size: usize, ways: usize) -> Self {
        let sets = size / (ways * LINE);
        assert!(sets.is_power_of_two(), "cache sets must be a power of two");
        Cache {
            set_mask: sets as u64 - 1,
            ways,
            tags: vec![u64::MAX; sets * ways],
            age: vec![0; sets * ways],
        }
    }
    #[inline]
    fn hit(&mut self, line: u64, clock: u64) -> bool {
        let base = ((line & self.set_mask) as usize) * self.ways;
        let ways = &mut self.tags[base..base + self.ways];
        let ages = &mut self.age[base..base + self.ways];
        for w in 0..self.ways {
            if ways[w] == line {
                ages[w] = clock;
                return true;
            }
        }
        let mut victim = 0;
        for w in 1..self.ways {
            if ages[w] < ages[victim] {
                victim = w;
            }
        }
        ways[victim] = line;
        ages[victim] = clock;
        false
    }
}

struct Sim {
    l1: Cache,
    l2: Cache,
    l3: Cache,
    clock: u64,
    accesses: u64,
    l1m: u64,
    l2m: u64,
    l3m: u64,
}
impl Sim {
    fn new() -> Self {
        Sim {
            l1: Cache::new(L1_BYTES, L1_WAYS),
            l2: Cache::new(L2_BYTES, L2_WAYS),
            l3: Cache::new(L3_BYTES, L3_WAYS),
            clock: 0,
            accesses: 0,
            l1m: 0,
            l2m: 0,
            l3m: 0,
        }
    }
    #[inline]
    fn touch(&mut self, line: u64) {
        self.clock += 1;
        let c = self.clock;
        if self.l1.hit(line, c) {
            return;
        }
        self.l1m += 1;
        if self.l2.hit(line, c) {
            return;
        }
        self.l2m += 1;
        if self.l3.hit(line, c) {
            return;
        }
        self.l3m += 1;
    }
    #[inline]
    fn access(&mut self, addr: u32, size: u32) {
        self.accesses += 1;
        let first = (addr as u64) / LINE as u64;
        let last = ((addr as u64) + size.max(1) as u64 - 1) / LINE as u64;
        let mut l = first;
        loop {
            self.touch(l);
            if l == last {
                break;
            }
            l += 1;
        }
    }
    fn memcost(&self) -> u64 {
        self.l1m * PEN_L1 + self.l2m * PEN_L2 + self.l3m * PEN_L3
    }
}

// ---------- wasm instrumentation ----------

fn load_size(k: &LoadKind) -> u32 {
    match k {
        LoadKind::I32 { .. } => 4,
        LoadKind::I64 { .. } => 8,
        LoadKind::F32 => 4,
        LoadKind::F64 => 8,
        LoadKind::V128 => 16,
        LoadKind::I32_8 { .. } | LoadKind::I64_8 { .. } => 1,
        LoadKind::I32_16 { .. } | LoadKind::I64_16 { .. } => 2,
        LoadKind::I64_32 { .. } => 4,
    }
}
fn store_info(k: &StoreKind) -> (u32, ValType) {
    match k {
        StoreKind::I32 { .. } => (4, ValType::I32),
        StoreKind::I64 { .. } => (8, ValType::I64),
        StoreKind::F32 => (4, ValType::F32),
        StoreKind::F64 => (8, ValType::F64),
        StoreKind::V128 => (16, ValType::V128),
        StoreKind::I32_8 { .. } => (1, ValType::I32),
        StoreKind::I32_16 { .. } => (2, ValType::I32),
        StoreKind::I64_8 { .. } => (1, ValType::I64),
        StoreKind::I64_16 { .. } => (2, ValType::I64),
        StoreKind::I64_32 { .. } => (4, ValType::I64),
    }
}

fn collect_seqs(f: &LocalFunction, seq: walrus::ir::InstrSeqId, out: &mut Vec<walrus::ir::InstrSeqId>) {
    out.push(seq);
    for (instr, _) in f.block(seq).instrs.iter() {
        match instr {
            Instr::Block(b) => collect_seqs(f, b.seq, out),
            Instr::Loop(l) => collect_seqs(f, l.seq, out),
            Instr::IfElse(ie) => {
                collect_seqs(f, ie.consequent, out);
                collect_seqs(f, ie.alternative, out);
            }
            _ => {}
        }
    }
}

struct Tmp {
    track: FunctionId,
    addr: LocalId,
    vi32: LocalId,
    vi64: LocalId,
    vf32: LocalId,
    vf64: LocalId,
}
impl Tmp {
    fn val_local(&self, t: ValType) -> LocalId {
        match t {
            ValType::I64 => self.vi64,
            ValType::F32 => self.vf32,
            ValType::F64 => self.vf64,
            _ => self.vi32,
        }
    }
}

fn loc() -> InstrLocId {
    InstrLocId::default()
}
fn ci32(v: i32) -> Instr {
    Instr::Const(Const { value: Value::I32(v) })
}

fn instrument(module: &mut walrus::Module) {
    let ty = module.types.add(&[ValType::I32, ValType::I32, ValType::I32], &[]);
    let (track, _) = module.add_import_func("mem", "track", ty);
    let t = Tmp {
        track,
        addr: module.locals.add(ValType::I32),
        vi32: module.locals.add(ValType::I32),
        vi64: module.locals.add(ValType::I64),
        vf32: module.locals.add(ValType::F32),
        vf64: module.locals.add(ValType::F64),
    };

    let local_ids: Vec<FunctionId> = module
        .funcs
        .iter()
        .filter(|f| matches!(f.kind, FunctionKind::Local(_)))
        .map(|f| f.id())
        .collect();

    for fid in local_ids {
        let f = match &mut module.funcs.get_mut(fid).kind {
            FunctionKind::Local(lf) => lf,
            _ => continue,
        };
        let mut seqs = Vec::new();
        collect_seqs(f, f.entry_block(), &mut seqs);
        for sid in seqs {
            let old = std::mem::take(&mut f.block_mut(sid).instrs);
            let mut new: Vec<(Instr, InstrLocId)> = Vec::with_capacity(old.len() + 8);
            for (instr, il) in old {
                match &instr {
                    // stack: [addr] -> tee (keep addr), call track(addr+off, sz, 0)
                    Instr::Load(l) => {
                        let sz = load_size(&l.kind) as i32;
                        let off = l.arg.offset as i32;
                        new.push((Instr::LocalTee(LocalTee { local: t.addr }), loc()));
                        new.push((Instr::LocalGet(LocalGet { local: t.addr }), loc()));
                        new.push((ci32(off), loc()));
                        new.push((Instr::Binop(Binop { op: BinaryOp::I32Add }), loc()));
                        new.push((ci32(sz), loc()));
                        new.push((ci32(0), loc()));
                        new.push((Instr::Call(Call { func: t.track }), loc()));
                        new.push((instr, il));
                    }
                    // stack: [addr, val] -> stash val, tee addr, call track, restore val
                    Instr::Store(s) => {
                        let (sz, vt) = store_info(&s.kind);
                        let off = s.arg.offset as i32;
                        let vl = t.val_local(vt);
                        new.push((Instr::LocalSet(LocalSet { local: vl }), loc()));
                        new.push((Instr::LocalTee(LocalTee { local: t.addr }), loc()));
                        new.push((Instr::LocalGet(LocalGet { local: t.addr }), loc()));
                        new.push((ci32(off), loc()));
                        new.push((Instr::Binop(Binop { op: BinaryOp::I32Add }), loc()));
                        new.push((ci32(sz as i32), loc()));
                        new.push((ci32(1), loc()));
                        new.push((Instr::Call(Call { func: t.track }), loc()));
                        new.push((Instr::LocalGet(LocalGet { local: vl }), loc()));
                        new.push((instr, il));
                    }
                    _ => new.push((instr, il)),
                }
            }
            f.block_mut(sid).instrs = new;
        }
    }
}

fn main() -> Result<()> {
    let path = std::env::args().nth(1).expect("usage: cm-mem-meter <module.wasm>");

    let mut module = walrus::Module::from_file(&path)?;
    instrument(&mut module);
    let bytes = module.emit_wasm();

    let engine = Engine::new(&Config::new())?;
    let m = wasmtime::Module::from_binary(&engine, &bytes)?;
    let mut linker = Linker::new(&engine);
    linker.func_wrap(
        "mem",
        "track",
        |mut c: Caller<Sim>, addr: i32, size: i32, _rw: i32| {
            c.data_mut().access(addr as u32, size as u32);
        },
    )?;

    // Fresh instance per measurement → cold allocator → reproducible accesses.
    let measure = |prefix: u32| -> Result<Sim> {
        let mut store = wasmtime::Store::new(&engine, Sim::new());
        let inst = linker.instantiate(&mut store, &m)?;
        let f = inst.get_typed_func::<u32, u32>(&mut store, EXPORT)?;
        f.call(&mut store, prefix)?;
        Ok(store.into_data())
    };

    let full = measure(FULL)?;
    let half = measure(HALF)?;
    let cost = full.memcost() as i64 - half.memcost() as i64;

    println!(
        "full {}B: accesses {}, miss L1 {} L2 {} L3/DRAM {}, memcost {}",
        FULL, full.accesses, full.l1m, full.l2m, full.l3m, full.memcost()
    );
    println!(
        "half {}B: accesses {}, miss L1 {} L2 {} L3/DRAM {}, memcost {}",
        HALF, half.accesses, half.l1m, half.l2m, half.l3m, half.memcost()
    );
    println!(
        "MEMCOST: {} (deterministic, init-free weighted cache-miss penalty for {} bytes; lower is friendlier to memory)",
        cost,
        FULL - HALF
    );
    Ok(())
}
