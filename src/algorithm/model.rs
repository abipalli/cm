//! Context-mixing predictor: multi-order adaptive counters, a learned match
//! model, a logistic mixer, and a two-stage APM/SSE.
//!
//! THIS IS THE PRIMARY EDITABLE SURFACE. Change models, add models, retune,
//! restructure — anything goes, provided `compress`/`decompress` remain exactly
//! lossless on all inputs and the predict/update sequence stays identical
//! between encode and decode.

use super::lstm::Lstm;
use super::tables::{build, build16, squash16_d, squash_d};

const NCTX: usize = 99; // orders + word/n-gram + sparse + 2D + record + indirect + run + nest + nibble + text shape/layout
// Mixer input layout:
//   [0 .. NCTX)            direct adaptive counters
//   [SM_BASE .. SM_BASE+NCTX) bit-history StateMap predictions (one per context)
//   [MM_BASE .. MM_BASE+5)  five match models (order-6, -8, -10, -12, -14)
const SM_BASE: usize = NCTX;
const MM_BASE: usize = 2 * NCTX;
const LSTM_IN: usize = 2 * NCTX + 6; // recurrent-mixer prediction (one extra input)
const NINPUT: usize = 2 * NCTX + 7;
const TBITS: u32 = 23; // default per-model context-table size (2^TBITS slots)
const MIXCTX: usize = 16384;
const NL1: usize = 27; // number of layer-1 specialist mixers
const L1LR: i32 = 8; // layer-1 specialist learning rate
const L2LR: i32 = 10; // layer-2 combiner learning rate
const MIX3CTX: usize = 8192; // order-2 specialist rows
const MIX4CTX: usize = 8192; // order-3 specialist rows
const FBITS: u32 = 23; // indirect order-3/-4/-5/-6 follow-history hash table bits
const WAYS: usize = 8; // set-associative ways for context tables
const WAYS_LOG: u32 = 3; // log2(WAYS)
const FSIZE: usize = 1 << FBITS;
const MMBITS: u32 = 23;
const MMSIZE: usize = 1 << MMBITS;
const MMBITS2: u32 = 23;
const MMSIZE2: usize = 1 << MMBITS2;
const MMBITS3: u32 = 22;
const MMSIZE3: usize = 1 << MMBITS3;
const MMBITS4: u32 = 23;
const MMSIZE4: usize = 1 << MMBITS4;
const MMBITS5: u32 = 23;
const MMSIZE5: usize = 1 << MMBITS5;
const MMBITS6: u32 = 23; // order-4 (short) match model
const MMSIZE6: usize = 1 << MMBITS6;
const APM_S: usize = 33;
const CNT_LIMIT: i32 = 254;
const RATE_FLOOR: i32 = 40;

#[inline]
fn hashk(h: u32, x: u32) -> u32 {
    h.wrapping_add(x).wrapping_add(1).wrapping_mul(2654435761)
}

/// Pack the letter/digit/space/other class (2 bits each) of the four bytes in
/// `c4` into an 8-bit signature (0..255).
#[inline]
fn cls4(c4: u32) -> u32 {
    let cl = |b: u32| -> u32 {
        let b = b & 0xff;
        if (b >= 97 && b <= 122) || (b >= 65 && b <= 90) {
            1
        } else if b >= 48 && b <= 57 {
            2
        } else if b == 32 || b == 9 || b == 10 || b == 13 {
            3
        } else {
            0
        }
    };
    cl(c4) | (cl(c4 >> 8) << 2) | (cl(c4 >> 16) << 4) | (cl(c4 >> 24) << 6)
}

/// Nonstationary bit-history state transition. The state byte packs two bounded
/// counts (n0 in the high nibble, n1 in the low nibble, each 0..15). On each
/// observed bit the matching count is incremented and the opposite count is
/// reset to a small floor (3), which strongly emphasises recent statistics — an
/// aggressive recency bias that lets the StateMap track nonstationary data well.
#[inline]
fn next_state(s: u8, bit: i32) -> u8 {
    // Asymmetric encoding: with the reset-recency rule the minority count is
    // always small, so pack (sign, big=majority 0..31, small=minority 0..3) to
    // distinguish run lengths up to 31 instead of the nibble's 15.
    let sign = (s >> 7) & 1;
    let big = ((s >> 2) & 31) as i32;
    let small = (s & 3) as i32;
    let (mut n0, mut n1) = if sign == 0 { (big, small) } else { (small, big) };
    if bit != 0 {
        n1 += 1;
        if n0 > 3 { n0 = 3; }
    } else {
        n0 += 1;
        if n1 > 3 { n1 = 3; }
    }
    let (ns, nb, nsm) = if n1 > n0 { (1, n1, n0) } else { (0, n0, n1) };
    let nb = if nb > 31 { 31 } else { nb };
    let nsm = if nsm > 3 { 3 } else { nsm };
    ((ns << 7) | (nb << 2) | nsm) as u8
}

struct Apm {
    t: Vec<u16>,
    idx: usize,
}

impl Apm {
    fn new(n: usize, squash: &[i32]) -> Self {
        let mut t = vec![0u16; n * APM_S];
        for c in 0..n {
            for j in 0..APM_S {
                t[c * APM_S + j] = (squash_d(squash, (j as i32 - 16) * 128) * 16) as u16;
            }
        }
        Apm { t, idx: 0 }
    }

    /// 16-bit SSE/APM step: takes a 16-bit probability `p`, stretches it via the
    /// 16-bit table, interpolates the (16-bit) calibration table, and returns a
    /// 16-bit probability. Keeps the whole final chain at 16 bits so confident
    /// predictions are not re-quantized to the 12-bit 1/4096 grid.
    #[inline]
    fn apply16(&mut self, stretch16: &[i32], ctx: usize, p: i32) -> i32 {
        let s = stretch16[p as usize] + 2048; // 0..4095
        let w = s & 127;
        let j = (s >> 7) as usize;
        self.idx = ctx * APM_S + j;
        let lo = self.t[self.idx] as i32;
        let hi = self.t[self.idx + 1] as i32;
        let mut pp = (lo * (128 - w) + hi * w) >> 7;
        if pp < 1 { pp = 1; }
        if pp > 65534 { pp = 65534; }
        pp
    }

    #[inline]
    fn update(&mut self, bit: i32) {
        let g = (bit << 16) + (bit << 4) - bit - bit;
        let a = self.t[self.idx] as i32;
        let b = self.t[self.idx + 1] as i32;
        self.t[self.idx] = (a + ((g - a) >> 7)) as u16;
        self.t[self.idx + 1] = (b + ((g - b) >> 7)) as u16;
    }
}

/// A context-selected logistic mixer. Holds `nctx` weight rows of `n` inputs;
/// each step selects one row by context, dot-products it with the stretched
/// inputs to produce a logit, and trains that row online toward the observed
/// bit. Used both as the layer-1 specialists and the layer-2 combiner.
struct Mixer {
    n: usize,
    nctx: usize,
    w: Vec<i32>,
    ctx: usize,
    pr: i32,
    lr: i32,
}

impl Mixer {
    fn new(n: usize, nctx: usize, lr: i32) -> Self {
        Mixer {
            n,
            nctx,
            w: vec![(1 << 16) / n as i32; n * nctx],
            ctx: 0,
            pr: 2048,
            lr,
        }
    }

    /// Mix `inputs` under the given context; returns the clamped logit (the
    /// stretched prediction) and caches the squashed probability for `update`.
    #[inline]
    fn mix(&mut self, inputs: &[i32], squash: &[i32], ctx: usize) -> i32 {
        let ctx = ctx & (self.nctx - 1);
        self.ctx = ctx;
        let base = ctx * self.n;
        let mut dot: i64 = 0;
        for i in 0..self.n {
            dot += self.w[base + i] as i64 * inputs[i] as i64;
        }
        let mut d = (dot >> 16) as i32;
        if d > 2047 { d = 2047; }
        if d < -2047 { d = -2047; }
        let mut p = squash_d(squash, d);
        if p < 1 { p = 1; }
        if p > 4094 { p = 4094; }
        self.pr = p;
        d
    }

    /// Train the selected row toward `bit` using this mixer's own prediction.
    #[inline]
    fn update(&mut self, bit: i32, inputs: &[i32]) {
        let err = (bit << 12) - self.pr;
        let base = self.ctx * self.n;
        for i in 0..self.n {
            let delta = (inputs[i] * err * self.lr) >> 16;
            self.w[base + i] = self.w[base + i].wrapping_add(delta);
        }
    }
}

pub struct Cm {
    stretch: Vec<i32>,
    squash: Vec<i32>,
    stretch16: Vec<i32>,
    squash16: Vec<i32>,
    cp: Vec<Vec<i16>>, // probabilities stored as (prob - 2048); zero-init so only
                       // touched table pages are committed (lazy RSS)
    cn: Vec<Vec<u8>>,  // [NCTX][TSIZE] observation counts
    st: Vec<Vec<u8>>,  // [NCTX][TSIZE] bit-history state per context slot
    ck: Vec<Vec<u8>>,  // [NCTX][TSIZE] checksum byte per slot (assoc models only; else empty)
    sm: Vec<Vec<u32>>, // [NCTX][256*8] StateMap: (state | bitpos<<8) -> (prob22<<10 | count)
    sm_idx: [usize; NCTX],
    tmask: [u32; NCTX], // per-model context-table index mask
    assoc: [bool; NCTX], // model uses 2-way set-associative buckets
    bmask: [u32; NCTX],  // bucket-index mask for associative models (2^(tb-1) - 1)
    cshift: [u32; NCTX], // checksum shift for associative models (tb - 1)
    rate_tab: [i32; 256],
    ctxhash: [u32; NCTX],
    idx: [usize; NCTX],
    mix_in: [i32; NINPUT],
    l1: Vec<Mixer>,   // layer-1 specialist mixers (different selection contexts)
    l2: Mixer,        // layer-2 combiner over the layer-1 logits (last-byte ctx)
    l2b: Mixer,       // second layer-2 combiner (bit-position ctx)
    l2c: Mixer,       // third layer-2 combiner (match-state ctx)
    l2d: Mixer,       // fourth layer-2 combiner (2nd-to-last-byte ctx)
    l2e: Mixer,       // fifth layer-2 combiner (word ctx)
    l2f: Mixer,       // sixth layer-2 combiner (high-nibble / opcode-class ctx)
    l2g: Mixer,       // seventh layer-2 combiner (char-class / text-mode ctx)
    l2h: Mixer,       // eighth layer-2 combiner (nesting-state ctx)
    l2i: Mixer,       // ninth layer-2 combiner (byte-above / 2D ctx)
    l2j: Mixer,       // tenth layer-2 combiner (byte-delta / numeric ctx)
    l2_in: [i32; NL1],
    buf: Vec<u8>,
    bufmask: u32,
    pos: u32,
    mmtab: Vec<u32>,
    matchptr: u32,
    matchlen: i32,
    predicted_byte: i32,
    mm_sm: [u16; 80],
    mm_used: bool,
    mm_idx: usize,
    mmtab2: Vec<u32>,
    matchptr2: u32,
    matchlen2: i32,
    predicted_byte2: i32,
    mm_sm2: [u16; 80],
    mm_used2: bool,
    mm_idx2: usize,
    mmtab3: Vec<u32>,
    matchptr3: u32,
    matchlen3: i32,
    predicted_byte3: i32,
    mm_sm3: [u16; 184],
    mm_used3: bool,
    mm_idx3: usize,
    mmtab4: Vec<u32>,
    matchptr4: u32,
    matchlen4: i32,
    predicted_byte4: i32,
    mm_sm4: [u16; 160],
    mm_used4: bool,
    mm_idx4: usize,
    mmtab5: Vec<u32>,
    matchptr5: u32,
    matchlen5: i32,
    predicted_byte5: i32,
    mm_sm5: [u16; 160],
    mm_used5: bool,
    mm_idx5: usize,
    mmtab6: Vec<u32>,
    matchptr6: u32,
    matchlen6: i32,
    predicted_byte6: i32,
    mm_sm6: [u16; 80],
    mm_used6: bool,
    mm_idx6: usize,
    apm1: Apm,
    apm2: Apm,
    apm3: Apm,
    apm4: Apm,
    lstm: Lstm,
    c0: i32,
    bitcount: i32,
    c4: u32,
    wordhash: u32,
    prevword: u32,
    prevword2: u32,
    prevword3: u32,
    c1: i32,
    run_len: u32, // length of the current run of identical bytes
    nest_stack: [u8; 64], // stack of currently-open bracket chars (source nesting)
    nest_depth: usize,
    col: u32,
    line_start: u32,
    prev_line_start: u32,
    prev2_line_start: u32,
    rpos: [u32; 256], // last position each byte value occurred (record detector)
    rlen: u32,        // current dominant recurrence period (record length)
    rcount: i32,      // confidence in rlen (Boyer-Moore majority vote)
    above_byte: u32,  // byte directly above (same column, previous line); 256 if none
    ind_pred: u32,    // most recent byte that followed the current order-2 context
    follow1: Vec<u32>, // [256] packed recent bytes that followed each order-1 ctx
    follow2: Vec<u32>, // [65536] packed recent bytes that followed each order-2 ctx
    follow3: Vec<u32>, // [FSIZE] hashed: bytes that followed each order-3 ctx
    follow4: Vec<u32>, // [FSIZE] hashed: bytes that followed each order-4 ctx
    follow5: Vec<u32>, // [FSIZE] hashed: bytes that followed each order-5 ctx
    follow6: Vec<u32>, // [FSIZE] hashed: bytes that followed each order-6 ctx
    followhn: Vec<u32>, // [FSIZE] hashed: bytes that followed each high-nibble ctx
    followd: Vec<u32>, // [FSIZE] hashed: bytes that followed each byte-delta ctx
    followc: Vec<u32>, // [256] bytes that followed each char-class ctx
    followln: Vec<u32>, // [FSIZE] hashed: bytes that followed each low-nibble ctx
    followg: Vec<u32>, // [65536] bytes that followed each gap-bigram ctx
    follows2: Vec<u32>, // [65536] bytes that followed each stride-2 ctx
    follows3: Vec<u32>, // [65536] bytes that followed each stride-3 ctx
    followg2: Vec<u32>, // [65536] bytes that followed each wide-gap ctx
    followw: Vec<u32>, // [65536] hashed: bytes that followed each word prefix
}

impl Cm {
    pub fn new(expected_len: usize) -> Self {
        let (stretch, squash) = build();
        let (stretch16, squash16) = build16();
        let mut rate_tab = [0i32; 256];
        for n in 0..256 {
            let mut r = 4096 / (n as i32 + 2);
            if r < RATE_FLOOR { r = RATE_FLOOR; }
            rate_tab[n] = r;
        }
        // Per-model context-table sizes. All models use the full 2^TBITS table
        // except order-0 (ctxhash[0] == 0), whose index is just the partial-byte
        // c0 (<=255); a 512-slot table is byte-for-byte identical there, saving
        // ~32 MB at zero cost to compression.
        // 2-way set-associative tables for the high-cardinality models (the dense
        // orders, sparse/stride banks, and indirect families). Each context maps
        // to a 2-slot bucket distinguished by an 8-bit checksum; colliding contexts
        // occupy separate warm slots instead of polluting one, recovering most of
        // the loss that a 4x-bigger direct table would. Because associativity
        // resolves collisions, the associative data tables can be SMALLER (2^22)
        // than the old direct ones (2^23) and still beat them — so this is also
        // net memory-negative. Low-cardinality models (order-0/1/2, bytes-2-3,
        // char-class) gain nothing and stay direct-mapped.
        let mut assoc = [false; NCTX];
        for i in 0..NCTX {
            let small = i == 0 || i == 1 || i == 2 || i == 8 || i == 93;
            if !small {
                assoc[i] = true;
            }
        }
        // Size the high-cardinality context tables to the input length: a larger
        // input touches more distinct contexts, so a bigger table keeps the hash
        // load factor (and the associative-bucket eviction rate) low. General
        // policy — bigger input, bigger model — not tied to any specific data.
        // Capped at +1 bit so peak resident memory stays bounded for the verifier
        // (single-threaded eval at 2^23 assoc is ~5 GB; +2 would risk OOM).
        let grow: u32 = if expected_len >= 262_144 { 1 } else { 0 };
        let mut tb = [TBITS; NCTX];
        tb[0] = 9;
        for i in 0..NCTX {
            if assoc[i] {
                tb[i] = 22 + grow; // 2^22 slots = 2^19 buckets x 8 ways (×2 for large inputs)
            }
        }
        let mut tmask = [0u32; NCTX];
        for i in 0..NCTX {
            tmask[i] = (1u32 << tb[i]) - 1;
        }
        let mut bmask = [0u32; NCTX];
        let mut cshift = [0u32; NCTX];
        for i in 0..NCTX {
            if assoc[i] {
                bmask[i] = (1u32 << (tb[i] - WAYS_LOG)) - 1;
                cshift[i] = tb[i] - WAYS_LOG;
            }
        }
        let cp: Vec<Vec<i16>> = (0..NCTX).map(|i| vec![0i16; 1usize << tb[i]]).collect();
        let cn: Vec<Vec<u8>> = (0..NCTX).map(|i| vec![0u8; 1usize << tb[i]]).collect();
        let st: Vec<Vec<u8>> = (0..NCTX).map(|i| vec![0u8; 1usize << tb[i]]).collect();
        let ck: Vec<Vec<u8>> = (0..NCTX)
            .map(|i| if assoc[i] { vec![0u8; 1usize << tb[i]] } else { Vec::new() })
            .collect();
        let sm = (0..NCTX).map(|_| vec![1u32 << 31; 256 * 8]).collect();
        let q = L1LR;
        let l1 = vec![
            Mixer::new(NINPUT, MIXCTX, q),
            Mixer::new(NINPUT, 256, q),
            Mixer::new(NINPUT, 256, q),
            Mixer::new(NINPUT, MIX3CTX, q),
            Mixer::new(NINPUT, MIX4CTX, q),
            Mixer::new(NINPUT, 64, q),
            Mixer::new(NINPUT, 4096, q),
            Mixer::new(NINPUT, 8192, q),
            Mixer::new(NINPUT, 8192, q),
            Mixer::new(NINPUT, 4096, q),
            Mixer::new(NINPUT, 4096, q),
            Mixer::new(NINPUT, 512, q),
            Mixer::new(NINPUT, 256, q),
            Mixer::new(NINPUT, 1024, q),
            Mixer::new(NINPUT, 4096, q),
            Mixer::new(NINPUT, 256, q),
            Mixer::new(NINPUT, 256, q),
            Mixer::new(NINPUT, 64, q),  // run-length regime selector
            Mixer::new(NINPUT, 32, q),  // gradient / delta-sign selector
            Mixer::new(NINPUT, 16, q),  // periodic / record selector
            Mixer::new(NINPUT, 64, q),  // above-char-class + nest selector
            Mixer::new(NINPUT, 32, q),  // gradient-magnitude selector
            Mixer::new(NINPUT, 32, q),  // vertical-repeat + match selector
            Mixer::new(NINPUT, 256, q), // bit-position + match-state selector
            Mixer::new(NINPUT, 64, q),  // opcode-trigram (3 high nibbles) selector
            Mixer::new(NINPUT, 64, q),  // delta sign+magnitude selector
            Mixer::new(NINPUT, 64, q),  // column-bucket + char-class selector
        ];
        let l2 = Mixer::new(NL1, 256, L2LR);
        let l2b = Mixer::new(NL1, 256, L2LR);
        let l2c = Mixer::new(NL1, 256, L2LR);
        let l2d = Mixer::new(NL1, 256, L2LR);
        let l2e = Mixer::new(NL1, 256, L2LR);
        let l2f = Mixer::new(NL1, 256, L2LR);
        let l2g = Mixer::new(NL1, 256, L2LR);
        let l2h = Mixer::new(NL1, 256, L2LR);
        let l2i = Mixer::new(NL1, 512, L2LR);
        let l2j = Mixer::new(NL1, 256, L2LR);

        let mut bufsize: u32 = 1;
        while (bufsize as usize) < expected_len + 16 && bufsize < (1 << 27) {
            bufsize <<= 1;
        }
        if bufsize < (1 << 16) { bufsize = 1 << 16; }

        let apm1 = Apm::new(1024, &squash);
        let apm2 = Apm::new(16384, &squash);
        let apm3 = Apm::new(1024, &squash);
        let apm4 = Apm::new(1024, &squash);

        Cm {
            stretch,
            squash,
            stretch16,
            squash16,
            cp,
            cn,
            st,
            ck,
            sm,
            sm_idx: [0; NCTX],
            tmask,
            assoc,
            bmask,
            cshift,
            rate_tab,
            ctxhash: [0; NCTX],
            idx: [0; NCTX],
            mix_in: [0; NINPUT],
            l1,
            l2,
            l2b,
            l2c,
            l2d,
            l2e,
            l2f,
            l2g,
            l2h,
            l2i,
            l2j,
            l2_in: [0; NL1],
            buf: vec![0u8; bufsize as usize],
            bufmask: bufsize - 1,
            pos: 0,
            mmtab: vec![0u32; MMSIZE],
            matchptr: 0,
            matchlen: 0,
            predicted_byte: -1,
            mm_sm: [2048; 80],
            mm_used: false,
            mm_idx: 0,
            mmtab2: vec![0u32; MMSIZE2],
            matchptr2: 0,
            matchlen2: 0,
            predicted_byte2: -1,
            mm_sm2: [2048; 80],
            mm_used2: false,
            mm_idx2: 0,
            mmtab3: vec![0u32; MMSIZE3],
            matchptr3: 0,
            matchlen3: 0,
            predicted_byte3: -1,
            mm_sm3: [2048; 184],
            mm_used3: false,
            mm_idx3: 0,
            mmtab4: vec![0u32; MMSIZE4],
            matchptr4: 0,
            matchlen4: 0,
            predicted_byte4: -1,
            mm_sm4: [2048; 160],
            mm_used4: false,
            mm_idx4: 0,
            mmtab5: vec![0u32; MMSIZE5],
            matchptr5: 0,
            matchlen5: 0,
            predicted_byte5: -1,
            mm_sm5: [2048; 160],
            mm_used5: false,
            mm_idx5: 0,
            mmtab6: vec![0u32; MMSIZE6],
            matchptr6: 0,
            matchlen6: 0,
            predicted_byte6: -1,
            mm_sm6: [2048; 80],
            mm_used6: false,
            mm_idx6: 0,
            apm1,
            apm2,
            apm3,
            apm4,
            lstm: Lstm::new(),
            c0: 1,
            bitcount: 0,
            c4: 0,
            wordhash: 0,
            prevword: 0,
            prevword2: 0,
            prevword3: 0,
            c1: 0,
            run_len: 1,
            nest_stack: [0u8; 64],
            nest_depth: 0,
            col: 0,
            line_start: 0,
            prev_line_start: 0,
            prev2_line_start: 0,
            rpos: [0; 256],
            rlen: 0,
            rcount: 0,
            above_byte: 256,
            ind_pred: 0,
            follow1: vec![0u32; 256],
            follow2: vec![0u32; 65536],
            follow3: vec![0u32; FSIZE],
            follow4: vec![0u32; FSIZE],
            follow5: vec![0u32; FSIZE],
            follow6: vec![0u32; FSIZE],
            followhn: vec![0u32; FSIZE],
            followd: vec![0u32; FSIZE],
            followc: vec![0u32; 256],
            followln: vec![0u32; FSIZE],
            followg: vec![0u32; 65536],
            follows2: vec![0u32; 65536],
            follows3: vec![0u32; 65536],
            followg2: vec![0u32; 65536],
            followw: vec![0u32; 65536],
        }
    }

    #[inline]
    fn b(&self, p: u32) -> u8 {
        self.buf[(p & self.bufmask) as usize]
    }

    pub fn byte_start(&mut self) {
        let c4 = self.c4;
        self.ctxhash[0] = 0;
        self.ctxhash[1] = hashk(0x100, c4 & 0xff);
        self.ctxhash[2] = hashk(0x200, c4 & 0xffff);
        self.ctxhash[3] = hashk(0x300, c4 & 0xffffff);
        self.ctxhash[4] = hashk(0x400, c4);
        self.ctxhash[5] = hashk(0x500, c4.wrapping_mul(0x9E37_79B1) ^ ((self.c1 as u32) << 3));
        self.ctxhash[6] = if self.pos >= 6 {
            hashk(
                0x600,
                c4.wrapping_mul(2654435761)
                    ^ ((self.b(self.pos - 5) as u32) << 7)
                    ^ ((self.b(self.pos - 6) as u32) << 15),
            )
        } else {
            hashk(0x600, c4)
        };
        self.ctxhash[7] = if self.wordhash != 0 { hashk(0x700, self.wordhash) } else { 0 };
        self.ctxhash[8] = hashk(0x800, ((c4 >> 8) & 0xff) | (((c4 >> 16) & 0xff) << 8));
        self.ctxhash[9] = if self.pos >= 7 {
            hashk(
                0x900,
                c4.wrapping_mul(0x9E37_79B1)
                    ^ ((self.b(self.pos - 5) as u32).wrapping_mul(0x85eb_ca6b))
                    ^ ((self.b(self.pos - 6) as u32).wrapping_mul(0xc2b2_ae35))
                    ^ ((self.b(self.pos - 7) as u32).wrapping_mul(0x27d4_eb2f)),
            )
        } else {
            hashk(0x900, c4)
        };
        self.ctxhash[10] = if self.pos >= 8 {
            hashk(
                0xA00,
                c4.wrapping_mul(0x85eb_ca6b)
                    ^ ((self.b(self.pos - 5) as u32).wrapping_mul(0xc2b2_ae35))
                    ^ ((self.b(self.pos - 6) as u32).wrapping_mul(0x27d4_eb2f))
                    ^ ((self.b(self.pos - 7) as u32).wrapping_mul(0x1656_67b1))
                    ^ ((self.b(self.pos - 8) as u32).wrapping_mul(0x9E37_79B1)),
            )
        } else {
            hashk(0xA00, c4)
        };
        self.ctxhash[11] = if self.pos >= 9 {
            hashk(
                0xB00,
                c4.wrapping_mul(0xc2b2_ae35)
                    ^ ((self.b(self.pos - 5) as u32).wrapping_mul(0x27d4_eb2f))
                    ^ ((self.b(self.pos - 6) as u32).wrapping_mul(0x1656_67b1))
                    ^ ((self.b(self.pos - 7) as u32).wrapping_mul(0x85eb_ca6b))
                    ^ ((self.b(self.pos - 8) as u32).wrapping_mul(0x9E37_79B1))
                    ^ ((self.b(self.pos - 9) as u32).wrapping_mul(0xff51_afd7)),
            )
        } else {
            hashk(0xB00, c4)
        };
        self.ctxhash[12] = if self.wordhash != 0 {
            hashk(0xC00, self.wordhash ^ ((self.c1 as u32) << 8))
        } else {
            let b1 = c4 & 0xff;
            let b2 = (c4 >> 8) & 0xff;
            let b3 = (c4 >> 16) & 0xff;
            hashk(0xC00, b1 | (b2 << 8) | ((b3 & 0x1f) << 16))
        };
        self.ctxhash[13] = if self.wordhash != 0 {
            hashk(0xD00, self.wordhash.wrapping_mul(0x85eb_ca6b) ^ (c4 & 0xffff))
        } else {
            let mut h = 0u32;
            let mut x = c4;
            for _ in 0..4 {
                let b = (x & 0xff) as u8;
                let class = if (b >= b'a' && b <= b'z') || (b >= b'A' && b <= b'Z') {
                    1
                } else if b >= b'0' && b <= b'9' {
                    2
                } else if b == b' ' || b == b'\n' || b == b'\t' || b == b'\r' {
                    3
                } else {
                    4
                };
                h = (h << 3) | class;
                x >>= 8;
            }
            hashk(0xD00, h ^ (c4 & 0xff))
        };
        self.ctxhash[14] = if self.wordhash != 0 {
            let folded = (c4 & 0xdfdf_dfdf).wrapping_mul(0x27d4_eb2f);
            hashk(0xE00, self.wordhash.wrapping_mul(0xc2b2_ae35) ^ folded)
        } else {
            let b1 = c4 & 0xff;
            let b2 = (c4 >> 8) & 0xff;
            let b3 = (c4 >> 16) & 0xff;
            let b4 = (c4 >> 24) & 0xff;
            hashk(
                0xE00,
                b1.wrapping_mul(3)
                    ^ b2.wrapping_mul(5)
                    ^ b3.wrapping_mul(7)
                    ^ b4.wrapping_mul(11),
            )
        };
        self.ctxhash[15] = hashk(0xF00, (self.col.min(255) << 16) ^ (c4 & 0xffff));
        let b1 = (c4 & 0xff) as u8;
        let class = if (b1 >= b'a' && b1 <= b'z') || (b1 >= b'A' && b1 <= b'Z') {
            1
        } else if b1 >= b'0' && b1 <= b'9' {
            2
        } else if b1 == b' ' || b1 == b'\n' || b1 == b'\t' || b1 == b'\r' {
            3
        } else {
            4
        };
        self.ctxhash[16] = hashk(
            0x1000,
            ((self.col & 63) << 8) ^ class ^ self.wordhash.wrapping_mul(0x9e37_79b1),
        );
        // order-5: the four bytes in c4 plus the byte at pos-5.
        self.ctxhash[17] = if self.pos >= 5 {
            hashk(
                0x1100,
                c4.wrapping_mul(0x2545_f491)
                    ^ ((self.b(self.pos - 5) as u32).wrapping_mul(0x9e37_79b1)),
            )
        } else {
            hashk(0x1100, c4)
        };
        // order-11: c4 plus bytes pos-5..pos-11.
        self.ctxhash[18] = if self.pos >= 11 {
            hashk(
                0x1200,
                c4.wrapping_mul(0x9e37_79b1)
                    ^ ((self.b(self.pos - 5) as u32).wrapping_mul(0x85eb_ca6b))
                    ^ ((self.b(self.pos - 6) as u32).wrapping_mul(0xc2b2_ae35))
                    ^ ((self.b(self.pos - 7) as u32).wrapping_mul(0x27d4_eb2f))
                    ^ ((self.b(self.pos - 8) as u32).wrapping_mul(0x1656_67b1))
                    ^ ((self.b(self.pos - 9) as u32).wrapping_mul(0xff51_afd7))
                    ^ ((self.b(self.pos - 10) as u32).wrapping_mul(0xc4ce_b9fe))
                    ^ ((self.b(self.pos - 11) as u32).wrapping_mul(0x2545_f491)),
            )
        } else {
            hashk(0x1200, c4)
        };
        // stride-2 sparse: bytes at pos-2, pos-4, pos-6 (skips every other byte).
        self.ctxhash[19] = if self.pos >= 6 {
            hashk(
                0x1300,
                (self.b(self.pos - 2) as u32)
                    | ((self.b(self.pos - 4) as u32) << 8)
                    | ((self.b(self.pos - 6) as u32) << 16),
            )
        } else {
            hashk(0x1300, c4)
        };
        // gap bigram: bytes at pos-1 and pos-3 (skips pos-2).
        self.ctxhash[20] = if self.pos >= 3 {
            hashk(
                0x1400,
                (c4 & 0xff) | ((self.b(self.pos - 3) as u32) << 8),
            )
        } else {
            hashk(0x1400, c4)
        };
        // gap bigram: bytes at pos-1 and pos-4 (skips pos-2, pos-3).
        self.ctxhash[21] = if self.pos >= 4 {
            hashk(
                0x1500,
                (c4 & 0xff) | ((self.b(self.pos - 4) as u32) << 8),
            )
        } else {
            hashk(0x1500, c4)
        };
        // gap bigram: bytes at pos-1 and pos-5.
        self.ctxhash[22] = if self.pos >= 5 {
            hashk(
                0x1600,
                (c4 & 0xff) | ((self.b(self.pos - 5) as u32) << 8),
            )
        } else {
            hashk(0x1600, c4)
        };
        // word bigram: previous completed word + the word currently being typed.
        self.ctxhash[23] = if self.prevword != 0 {
            hashk(
                0x1700,
                self.prevword
                    .wrapping_mul(0x9e37_79b1)
                    .wrapping_add(self.wordhash.wrapping_mul(0x85eb_ca6b)),
            )
        } else {
            0
        };
        // previous word + recent literal bytes: models the gap/punctuation that
        // follows a word and the run-up into the next one.
        self.ctxhash[24] = if self.prevword != 0 {
            hashk(
                0x1800,
                self.prevword.wrapping_mul(0xc2b2_ae35) ^ (c4 & 0xffff),
            )
        } else {
            0
        };
        // word trigram: the two preceding words plus the word being typed.
        self.ctxhash[25] = if self.prevword2 != 0 {
            hashk(
                0x1900,
                self.prevword2
                    .wrapping_mul(0x27d4_eb2f)
                    .wrapping_add(self.prevword.wrapping_mul(0x9e37_79b1))
                    .wrapping_add(self.wordhash.wrapping_mul(0x85eb_ca6b)),
            )
        } else {
            0
        };
        // order-10: c4 plus bytes pos-5..pos-10 (fills the gap below order-11).
        self.ctxhash[26] = if self.pos >= 10 {
            hashk(
                0x1A00,
                c4.wrapping_mul(0x2545_f491)
                    ^ ((self.b(self.pos - 5) as u32).wrapping_mul(0x85eb_ca6b))
                    ^ ((self.b(self.pos - 6) as u32).wrapping_mul(0xc2b2_ae35))
                    ^ ((self.b(self.pos - 7) as u32).wrapping_mul(0x27d4_eb2f))
                    ^ ((self.b(self.pos - 8) as u32).wrapping_mul(0x1656_67b1))
                    ^ ((self.b(self.pos - 9) as u32).wrapping_mul(0xff51_afd7))
                    ^ ((self.b(self.pos - 10) as u32).wrapping_mul(0xc4ce_b9fe)),
            )
        } else {
            hashk(0x1A00, c4)
        };
        // order-13: extends the direct-context ladder past order-11.
        self.ctxhash[27] = if self.pos >= 13 {
            hashk(
                0x1B00,
                c4.wrapping_mul(0xc2b2_ae35)
                    ^ ((self.b(self.pos - 5) as u32).wrapping_mul(0x85eb_ca6b))
                    ^ ((self.b(self.pos - 6) as u32).wrapping_mul(0xc2b2_ae35))
                    ^ ((self.b(self.pos - 7) as u32).wrapping_mul(0x27d4_eb2f))
                    ^ ((self.b(self.pos - 8) as u32).wrapping_mul(0x1656_67b1))
                    ^ ((self.b(self.pos - 9) as u32).wrapping_mul(0xff51_afd7))
                    ^ ((self.b(self.pos - 10) as u32).wrapping_mul(0xc4ce_b9fe))
                    ^ ((self.b(self.pos - 11) as u32).wrapping_mul(0x2545_f491))
                    ^ ((self.b(self.pos - 12) as u32).wrapping_mul(0x9e37_79b9))
                    ^ ((self.b(self.pos - 13) as u32).wrapping_mul(0x7f4a_7c15)),
            )
        } else {
            hashk(0x1B00, c4)
        };
        // order-16: a very long deterministic context for structured / repetitive
        // data, beyond what the match models alone cover.
        self.ctxhash[28] = if self.pos >= 16 {
            let mut h = c4.wrapping_mul(0x9e37_79b1);
            let mut k: u32 = 5;
            let mults: [u32; 12] = [
                0x85eb_ca6b, 0xc2b2_ae35, 0x27d4_eb2f, 0x1656_67b1, 0xff51_afd7,
                0xc4ce_b9fe, 0x2545_f491, 0x9e37_79b9, 0x7f4a_7c15, 0x94d0_49bb,
                0xd6e8_feb8, 0xa548_1ad7,
            ];
            while k <= 16 {
                h ^= (self.b(self.pos - k) as u32).wrapping_mul(mults[(k - 5) as usize]);
                k += 1;
            }
            hashk(0x1C00, h)
        } else {
            hashk(0x1C00, c4)
        };
        // stride-3 sparse: bytes at pos-3, pos-6, pos-9 (columnar / record-aligned).
        self.ctxhash[29] = if self.pos >= 9 {
            hashk(
                0x1D00,
                (self.b(self.pos - 3) as u32)
                    | ((self.b(self.pos - 6) as u32) << 8)
                    | ((self.b(self.pos - 9) as u32) << 16),
            )
        } else {
            hashk(0x1D00, c4)
        };
        // stride-4 sparse: bytes at pos-4, pos-8, pos-12 (wider record alignment).
        self.ctxhash[30] = if self.pos >= 12 {
            hashk(
                0x1E00,
                (self.b(self.pos - 4) as u32)
                    | ((self.b(self.pos - 8) as u32) << 8)
                    | ((self.b(self.pos - 12) as u32) << 16),
            )
        } else {
            hashk(0x1E00, c4)
        };
        // stride-5 sparse: bytes at pos-5, pos-10, pos-15.
        self.ctxhash[31] = if self.pos >= 15 {
            hashk(
                0x1F00,
                (self.b(self.pos - 5) as u32)
                    | ((self.b(self.pos - 10) as u32) << 8)
                    | ((self.b(self.pos - 15) as u32) << 16),
            )
        } else {
            hashk(0x1F00, c4)
        };
        // stride-6 sparse: bytes at pos-6, pos-12, pos-18.
        self.ctxhash[32] = if self.pos >= 18 {
            hashk(
                0x2000,
                (self.b(self.pos - 6) as u32)
                    | ((self.b(self.pos - 12) as u32) << 8)
                    | ((self.b(self.pos - 18) as u32) << 16),
            )
        } else {
            hashk(0x2000, c4)
        };
        // stride-7 sparse: bytes at pos-7, pos-14, pos-21.
        self.ctxhash[33] = if self.pos >= 21 {
            hashk(
                0x2100,
                (self.b(self.pos - 7) as u32)
                    | ((self.b(self.pos - 14) as u32) << 8)
                    | ((self.b(self.pos - 21) as u32) << 16),
            )
        } else {
            hashk(0x2100, c4)
        };
        // stride-8 sparse: bytes at pos-8, pos-16, pos-24.
        self.ctxhash[34] = if self.pos >= 24 {
            hashk(
                0x2200,
                (self.b(self.pos - 8) as u32)
                    | ((self.b(self.pos - 16) as u32) << 8)
                    | ((self.b(self.pos - 24) as u32) << 16),
            )
        } else {
            hashk(0x2200, c4)
        };
        // stride-9..16 sparse: three samples at each stride (record alignments).
        for (slot, stride, tag) in [
            (35usize, 9u32, 0x2300u32), (36, 10, 0x2400), (37, 11, 0x2500), (38, 12, 0x2600),
            (39, 13, 0x2700), (40, 14, 0x2800), (41, 15, 0x2900), (42, 16, 0x2A00),
            (43, 17, 0x2B00), (44, 18, 0x2C00), (45, 19, 0x2D00), (46, 20, 0x2E00),
        ] {
            self.ctxhash[slot] = if self.pos >= stride * 3 {
                hashk(
                    tag,
                    (self.b(self.pos - stride) as u32)
                        | ((self.b(self.pos - stride * 2) as u32) << 8)
                        | ((self.b(self.pos - stride * 3) as u32) << 16),
                )
            } else {
                hashk(tag, c4)
            };
        }
        // gap bigrams: last byte paired with one byte at distance k = 6..13.
        for (slot, k, tag) in [
            (47usize, 6u32, 0x3000u32), (48, 7, 0x3100), (49, 8, 0x3200), (50, 9, 0x3300),
            (51, 10, 0x3400), (52, 11, 0x3500), (53, 12, 0x3600), (54, 13, 0x3700),
        ] {
            self.ctxhash[slot] = if self.pos >= k {
                hashk(tag, (c4 & 0xff) | ((self.b(self.pos - k) as u32) << 8))
            } else {
                hashk(tag, c4)
            };
        }
        // 4-sample strided contexts: bytes at pos-k,2k,3k,4k for k=2..8
        // (longer periodic / record context than the 3-sample strides).
        for (slot, k, tag) in [
            (55usize, 2u32, 0x4000u32), (56, 3, 0x4100), (57, 4, 0x4200),
            (58, 5, 0x4300), (59, 6, 0x4400), (60, 7, 0x4500), (61, 8, 0x4600),
            (62, 9, 0x4700), (63, 10, 0x4800), (64, 11, 0x4900), (65, 12, 0x4A00),
        ] {
            self.ctxhash[slot] = if self.pos >= k * 4 {
                hashk(
                    tag,
                    (self.b(self.pos - k) as u32)
                        ^ (self.b(self.pos - k * 2) as u32).wrapping_mul(0x85eb_ca6b)
                        ^ (self.b(self.pos - k * 3) as u32).wrapping_mul(0xc2b2_ae35)
                        ^ (self.b(self.pos - k * 4) as u32).wrapping_mul(0x27d4_eb2f),
                )
            } else {
                hashk(tag, c4)
            };
        }
        // word 4-gram: three preceding words plus the word currently being typed.
        self.ctxhash[66] = if self.prevword3 != 0 {
            hashk(
                0x6000,
                self.prevword3
                    .wrapping_mul(0x1656_67b1)
                    .wrapping_add(self.prevword2.wrapping_mul(0x27d4_eb2f))
                    .wrapping_add(self.prevword.wrapping_mul(0x9e37_79b1))
                    .wrapping_add(self.wordhash.wrapping_mul(0x85eb_ca6b)),
            )
        } else {
            0
        };
        // word skip-gram: the word two back paired with the word being typed
        // (skips the immediately preceding word).
        self.ctxhash[67] = if self.prevword2 != 0 {
            hashk(
                0x6100,
                self.prevword2
                    .wrapping_mul(0xc2b2_ae35)
                    .wrapping_add(self.wordhash.wrapping_mul(0x9e37_79b1)),
            )
        } else {
            0
        };
        // word skip-gram: the word three back paired with the word being typed.
        self.ctxhash[68] = if self.prevword3 != 0 {
            hashk(
                0x6200,
                self.prevword3
                    .wrapping_mul(0x1656_67b1)
                    .wrapping_add(self.wordhash.wrapping_mul(0x9e37_79b1)),
            )
        } else {
            0
        };
        // 2D / "byte above" model: the byte at the same column in the previous
        // line. Powerful for aligned source code, text and tabular structure.
        let col2d = self.pos.wrapping_sub(self.line_start);
        let above_pos = self.prev_line_start.wrapping_add(col2d);
        let have_above = self.prev_line_start != 0 && above_pos < self.line_start;
        let above = if have_above { self.b(above_pos) as u32 } else { 0 };
        self.above_byte = if have_above { above } else { 256 };
        // byte above + current column
        self.ctxhash[69] = if have_above {
            hashk(0x6300, above | (col2d.min(1023) << 9))
        } else {
            0
        };
        // byte above + byte to the left (2D neighbourhood)
        self.ctxhash[70] = if have_above {
            hashk(0x6400, above | ((self.c1 as u32) << 8))
        } else {
            0
        };
        // byte above + the byte above-and-left (diagonal), captures 2D runs
        self.ctxhash[71] = if have_above && above_pos > self.prev_line_start {
            let above_left = self.b(above_pos - 1) as u32;
            hashk(0x6500, above | (above_left << 8))
        } else {
            0
        };
        // byte two lines up (same column) + byte above: a vertical bigram that
        // captures repeated/aligned blocks spanning multiple lines.
        let above2_pos = self.prev2_line_start.wrapping_add(col2d);
        self.ctxhash[74] = if self.prev2_line_start != 0
            && above2_pos < self.prev_line_start
            && have_above
        {
            let above2 = self.b(above2_pos) as u32;
            hashk(0x6800, above | (above2 << 8))
        } else {
            0
        };
        // upper-forward: byte above + the byte above-right (the char that came
        // next on the previous line) — strong when the current line copies it.
        self.ctxhash[79] = if have_above && above_pos + 1 < self.line_start {
            let above_r = self.b(above_pos + 1) as u32;
            hashk(0x6D00, above | (above_r << 8))
        } else {
            0
        };
        // 3-wide horizontal window from the previous line (above-left/above/right)
        self.ctxhash[80] = if have_above
            && above_pos > self.prev_line_start
            && above_pos + 1 < self.line_start
        {
            let al = self.b(above_pos - 1) as u32;
            let ar = self.b(above_pos + 1) as u32;
            hashk(0x6E00, above ^ (al << 8) ^ (ar << 16))
        } else {
            0
        };
        // Record model: the byte one detected-period back (the "byte above" for
        // newline-free periodic data such as executables and tables).
        let r = self.rlen;
        let rec_ok = self.rcount > 8 && r >= 2 && r < self.pos;
        let recb = if rec_ok { self.b(self.pos - r) as u32 } else { 0 };
        self.ctxhash[72] = if rec_ok {
            hashk(0x6600, recb | ((self.c1 as u32) << 8))
        } else {
            0
        };
        // record byte + the byte just before it one period back (2-gram above)
        self.ctxhash[73] = if rec_ok && self.pos > r + 1 {
            let recb1 = self.b(self.pos - r - 1) as u32;
            hashk(0x6700, recb | (recb1 << 8))
        } else {
            0
        };
        // Indirect models: the current order-1 / order-2 context combined with
        // the recent history of bytes that have followed it. Captures higher-order
        // regularity ("what usually comes next here") that direct contexts miss.
        let f1 = self.follow1[(c4 & 0xff) as usize];
        self.ctxhash[75] = hashk(0x6900, (c4 & 0xff) ^ f1.wrapping_mul(0x9e37_79b1));
        let f2 = self.follow2[(c4 & 0xffff) as usize];
        self.ctxhash[76] = hashk(0x6A00, (c4 & 0xffff) ^ f2.wrapping_mul(0x85eb_ca6b));
        self.ind_pred = f2 & 0xff;
        let j3 = ((c4 & 0x00ff_ffff).wrapping_mul(0x9e37_79b1) >> (32 - FBITS)) as usize;
        self.ctxhash[77] = hashk(0x6B00, (c4 & 0x00ff_ffff) ^ self.follow3[j3].wrapping_mul(0xc2b2_ae35));
        let j4 = (c4.wrapping_mul(0x85eb_ca6b) >> (32 - FBITS)) as usize;
        self.ctxhash[78] = hashk(0x6C00, c4 ^ self.follow4[j4].wrapping_mul(0x27d4_eb2f));
        // Word-indirect: current word prefix + the bytes that have followed it.
        self.ctxhash[81] = if self.wordhash != 0 {
            let wk = (self.wordhash.wrapping_mul(0x9e37_79b1) >> 16) as usize;
            hashk(0x7000, self.wordhash ^ self.followw[wk].wrapping_mul(0xc2b2_ae35))
        } else {
            0
        };
        // Run model: last byte + the length of its current run (capped). Models
        // run continuation/termination (zero-runs in binary, repeated chars).
        self.ctxhash[82] = hashk(0x7100, (c4 & 0xff) | (self.run_len.min(255) << 8));
        // Nesting model: predict from bracket-nesting depth and the enclosing
        // bracket — captures the ()[]{} structure pervasive in source code.
        let last_open = if self.nest_depth > 0 {
            self.nest_stack[self.nest_depth - 1] as u32
        } else {
            0
        };
        self.ctxhash[83] = hashk(0x7200, (self.nest_depth as u32 & 31) | ((c4 & 0xff) << 5));
        self.ctxhash[84] = hashk(0x7300, last_open | ((c4 & 0xff) << 8));
        // enclosing bracket + nesting depth + order-2 context (finer structure)
        self.ctxhash[85] = hashk(
            0x7400,
            last_open
                .wrapping_mul(0x9e37_79b1)
                ^ ((self.nest_depth as u32 & 31) << 16)
                ^ (c4 & 0xffff),
        );
        // High-nibble (opcode-class) context: the top nibble of the last 5 bytes,
        // ignoring low-bit operand noise — targets executable/binary structure.
        let hn = (c4 & 0xf0f0_f0f0)
            ^ if self.pos >= 5 { ((self.b(self.pos - 5) as u32) & 0xf0) << 24 } else { 0 };
        self.ctxhash[86] = hashk(0x7700, hn);
        // longer high-nibble context (last 8 bytes' top nibbles), order-8-coarse.
        self.ctxhash[87] = if self.pos >= 8 {
            hashk(
                0x7800,
                hn.wrapping_mul(0x9e37_79b1)
                    ^ ((self.b(self.pos - 6) as u32 & 0xf0) << 4)
                    ^ ((self.b(self.pos - 7) as u32 & 0xf0) << 12)
                    ^ ((self.b(self.pos - 8) as u32 & 0xf0) << 20),
            )
        } else {
            hashk(0x7800, hn)
        };
        // byte-delta context: differences between consecutive recent bytes —
        // captures gradients/patterns in numeric and tabular data.
        let d1 = (c4 & 0xff).wrapping_sub((c4 >> 8) & 0xff) & 0xff;
        let d2 = ((c4 >> 8) & 0xff).wrapping_sub((c4 >> 16) & 0xff) & 0xff;
        let d3 = ((c4 >> 16) & 0xff).wrapping_sub((c4 >> 24) & 0xff) & 0xff;
        self.ctxhash[88] = hashk(0x7900, d1 | (d2 << 8) | (d3 << 16));
        // Indirect order-5 / order-6 models: the longer base context combined
        // with the recent history of bytes that have followed it. Extends the
        // order-1..4 indirect family to deterministic longer-range structure
        // (helps executable/source repeats the direct long orders miss).
        if self.pos >= 5 {
            let m5 = c4.wrapping_mul(0x9e37_79b1)
                ^ (self.b(self.pos - 5) as u32).wrapping_mul(0x85eb_ca6b);
            let k5 = (m5 >> (32 - FBITS)) as usize;
            self.ctxhash[89] = hashk(0x7A00, m5 ^ self.follow5[k5].wrapping_mul(0xc2b2_ae35));
        } else {
            self.ctxhash[89] = 0;
        }
        if self.pos >= 6 {
            let m6 = c4.wrapping_mul(0x85eb_ca6b)
                ^ (self.b(self.pos - 5) as u32).wrapping_mul(0xc2b2_ae35)
                ^ (self.b(self.pos - 6) as u32).wrapping_mul(0x27d4_eb2f);
            let k6 = (m6 >> (32 - FBITS)) as usize;
            self.ctxhash[90] = hashk(0x7B00, m6 ^ self.follow6[k6].wrapping_mul(0x9e37_79b1));
        } else {
            self.ctxhash[90] = 0;
        }
        // High-nibble indirect: the opcode-class pattern (high nibble of the last
        // four bytes) combined with the recent history of bytes that followed it.
        // Merges the high-nibble and indirect families to capture "what operand
        // byte usually follows this instruction-class pattern" in executables.
        let hnm = (c4 & 0xf0f0_f0f0).wrapping_mul(0x9e37_79b1);
        let khn = (hnm >> (32 - FBITS)) as usize;
        self.ctxhash[91] = hashk(0x7C00, hnm ^ self.followhn[khn].wrapping_mul(0xc2b2_ae35));
        // Byte-delta indirect: the consecutive-difference pattern combined with
        // the recent history of bytes that followed it (numeric/tabular regimes).
        let dm = (d1 | (d2 << 8) | (d3 << 16)).wrapping_mul(0x85eb_ca6b);
        let kd = (dm >> (32 - FBITS)) as usize;
        self.ctxhash[92] = hashk(0x7D00, dm ^ self.followd[kd].wrapping_mul(0x27d4_eb2f));
        // Char-class indirect: the letter/digit/space/other pattern of the last
        // four bytes combined with the bytes that have followed it (text regimes).
        let cck = cls4(c4);
        self.ctxhash[93] = hashk(0x7E00, cck ^ self.followc[cck as usize].wrapping_mul(0x9e37_79b1));
        // Low-nibble indirect: the low nibbles of the last four bytes (operand /
        // register pattern in code) combined with the bytes that followed it.
        let lnm = (c4 & 0x0f0f_0f0f).wrapping_mul(0xc2b2_ae35);
        let kln = (lnm >> (32 - FBITS)) as usize;
        self.ctxhash[94] = hashk(0x7F00, lnm ^ self.followln[kln].wrapping_mul(0x85eb_ca6b));
        // Gap-bigram indirect: the (last byte, byte three back) sparse pair plus
        // the bytes that followed it.
        let gk = if self.pos >= 3 {
            (c4 & 0xff) | ((self.b(self.pos - 3) as u32) << 8)
        } else {
            c4 & 0xffff
        };
        self.ctxhash[95] = hashk(0x8000, gk ^ self.followg[gk as usize].wrapping_mul(0x27d4_eb2f));
        // Stride-2 indirect: the (pos-2, pos-4) interleaved pair plus its history.
        let sk = if self.pos >= 4 {
            (self.b(self.pos - 2) as u32) | ((self.b(self.pos - 4) as u32) << 8)
        } else {
            c4 & 0xffff
        };
        self.ctxhash[96] = hashk(0x8100, sk ^ self.follows2[sk as usize].wrapping_mul(0x9e37_79b1));
        // Stride-3 indirect: the (pos-3, pos-6) pair plus its follow history.
        let s3k = if self.pos >= 6 {
            (self.b(self.pos - 3) as u32) | ((self.b(self.pos - 6) as u32) << 8)
        } else {
            c4 & 0xffff
        };
        self.ctxhash[97] = hashk(0x8200, s3k ^ self.follows3[s3k as usize].wrapping_mul(0x85eb_ca6b));
        // Wide-gap indirect: the (last byte, byte five back) sparse pair plus its
        // follow history (longer-range sparse structure).
        let g2k = if self.pos >= 5 {
            (c4 & 0xff) | ((self.b(self.pos - 5) as u32) << 8)
        } else {
            c4 & 0xffff
        };
        self.ctxhash[98] = hashk(0x8300, g2k ^ self.followg2[g2k as usize].wrapping_mul(0xc2b2_ae35));
    }

    #[inline]
    pub fn predict(&mut self) -> i32 {
        for i in 0..NCTX {
            let h = self.ctxhash[i].wrapping_mul(769).wrapping_add(self.c0 as u32);
            let ix = if self.assoc[i] {
                // 4-way set-associative: a context maps to a 4-slot bucket; pick the
                // way whose checksum matches, else evict the lowest-count way. Keeps
                // colliding contexts in separate warm slots without a bigger table.
                let base = ((h & self.bmask[i]) << WAYS_LOG) as usize;
                let chk = (h >> self.cshift[i]) as u8;
                let mut sel = usize::MAX;
                let mut w = base;
                let mut lo = self.cn[i][base];
                for k in 0..WAYS {
                    let s = base + k;
                    if self.ck[i][s] == chk {
                        sel = s;
                        break;
                    }
                    if self.cn[i][s] < lo {
                        lo = self.cn[i][s];
                        w = s;
                    }
                }
                if sel != usize::MAX {
                    sel
                } else {
                    self.ck[i][w] = chk;
                    self.cp[i][w] = 0;
                    self.cn[i][w] = 0;
                    self.st[i][w] = 0;
                    w
                }
            } else {
                (h & self.tmask[i]) as usize
            };
            self.idx[i] = ix;
            self.mix_in[i] = self.stretch[(self.cp[i][ix] as i32 + 2048) as usize];
            let mi = (self.st[i][ix] as usize) | ((self.bitcount as usize) << 8);
            self.sm_idx[i] = mi;
            let smp = (self.sm[i][mi] >> 20) as usize;
            self.mix_in[SM_BASE + i] = self.stretch[smp];
        }
        self.mm_used = false;
        self.mix_in[MM_BASE] = 0;
        if self.matchlen > 0 && self.predicted_byte >= 0 {
            let sofar = self.c0 - (1 << self.bitcount);
            if sofar == (self.predicted_byte >> (8 - self.bitcount)) {
                let expected_bit = (self.predicted_byte >> (7 - self.bitcount)) & 1;
                let li = if self.matchlen > 32 { 32 } else { self.matchlen };
                self.mm_idx = ((li << 1) | expected_bit) as usize;
                self.mix_in[MM_BASE] = self.stretch[self.mm_sm[self.mm_idx] as usize];
                self.mm_used = true;
            } else {
                self.matchlen = 0;
            }
        }
        self.mm_used2 = false;
        self.mix_in[MM_BASE + 1] = 0;
        if self.matchlen2 > 0 && self.predicted_byte2 >= 0 {
            let sofar = self.c0 - (1 << self.bitcount);
            if sofar == (self.predicted_byte2 >> (8 - self.bitcount)) {
                let expected_bit = (self.predicted_byte2 >> (7 - self.bitcount)) & 1;
                let li = if self.matchlen2 > 32 { 32 } else { self.matchlen2 };
                self.mm_idx2 = ((li << 1) | expected_bit) as usize;
                self.mix_in[MM_BASE + 1] = self.stretch[self.mm_sm2[self.mm_idx2] as usize];
                self.mm_used2 = true;
            } else {
                self.matchlen2 = 0;
            }
        }
        self.mm_used3 = false;
        self.mix_in[MM_BASE + 2] = 0;
        if self.matchlen3 > 0 && self.predicted_byte3 >= 0 {
            let sofar = self.c0 - (1 << self.bitcount);
            if sofar == (self.predicted_byte3 >> (8 - self.bitcount)) {
                let expected_bit = (self.predicted_byte3 >> (7 - self.bitcount)) & 1;
                let li = if self.matchlen3 > 84 { 84 } else { self.matchlen3 };
                self.mm_idx3 = ((li << 1) | expected_bit) as usize;
                self.mix_in[MM_BASE + 2] = self.stretch[self.mm_sm3[self.mm_idx3] as usize];
                self.mm_used3 = true;
            } else {
                self.matchlen3 = 0;
            }
        }
        self.mm_used4 = false;
        self.mix_in[MM_BASE + 3] = 0;
        if self.matchlen4 > 0 && self.predicted_byte4 >= 0 {
            let sofar = self.c0 - (1 << self.bitcount);
            if sofar == (self.predicted_byte4 >> (8 - self.bitcount)) {
                let expected_bit = (self.predicted_byte4 >> (7 - self.bitcount)) & 1;
                let li = if self.matchlen4 > 72 { 72 } else { self.matchlen4 };
                self.mm_idx4 = ((li << 1) | expected_bit) as usize;
                self.mix_in[MM_BASE + 3] = self.stretch[self.mm_sm4[self.mm_idx4] as usize];
                self.mm_used4 = true;
            } else {
                self.matchlen4 = 0;
            }
        }
        self.mm_used5 = false;
        self.mix_in[MM_BASE + 4] = 0;
        if self.matchlen5 > 0 && self.predicted_byte5 >= 0 {
            let sofar = self.c0 - (1 << self.bitcount);
            if sofar == (self.predicted_byte5 >> (8 - self.bitcount)) {
                let expected_bit = (self.predicted_byte5 >> (7 - self.bitcount)) & 1;
                let li = if self.matchlen5 > 72 { 72 } else { self.matchlen5 };
                self.mm_idx5 = ((li << 1) | expected_bit) as usize;
                self.mix_in[MM_BASE + 4] = self.stretch[self.mm_sm5[self.mm_idx5] as usize];
                self.mm_used5 = true;
            } else {
                self.matchlen5 = 0;
            }
        }
        self.mm_used6 = false;
        self.mix_in[MM_BASE + 5] = 0;
        if self.matchlen6 > 0 && self.predicted_byte6 >= 0 {
            let sofar = self.c0 - (1 << self.bitcount);
            if sofar == (self.predicted_byte6 >> (8 - self.bitcount)) {
                let expected_bit = (self.predicted_byte6 >> (7 - self.bitcount)) & 1;
                let li = if self.matchlen6 > 32 { 32 } else { self.matchlen6 };
                self.mm_idx6 = ((li << 1) | expected_bit) as usize;
                self.mix_in[MM_BASE + 5] = self.stretch[self.mm_sm6[self.mm_idx6] as usize];
                self.mm_used6 = true;
            } else {
                self.matchlen6 = 0;
            }
        }
        // Recurrent (reservoir) bit predictor — one extra mixer input.
        self.mix_in[LSTM_IN] = self.lstm.predict();
        // Layer-1 specialist mixers, each selected by a different context:
        //   m0 — the proven last-byte + match-activity context (full resolution)
        //   m1 — the within-byte partial-byte context (order-0 bit position)
        //   m2 — the second-to-last byte (an order-2-distance specialist)
        let ctx0 = (((if self.matchlen4 > 0 { 1 } else { 0 }) << 13)
            | ((if self.matchlen3 > 72 { 1 } else { 0 }) << 12)
            | ((if self.matchlen3 > 52 { 1 } else { 0 }) << 11)
            | ((if self.matchlen3 > 0 { 1 } else { 0 }) << 10)
            | ((if self.matchlen2 > 0 { 1 } else { 0 }) << 9)
            | ((if self.matchlen > 0 { 1 } else { 0 }) << 8)
            | self.c1) as usize;
        let ctx1 = self.c0 as usize;
        let ctx2 = ((self.c4 >> 8) & 0xff) as usize;
        let ctx3 = (self.c4 & 0xffff) as usize;
        let ctx4 = ((self.c4 & 0xffffff).wrapping_mul(0x9e37_79b1) >> 13) as usize;
        self.l2_in[0] = self.l1[0].mix(&self.mix_in, &self.squash, ctx0);
        self.l2_in[1] = self.l1[1].mix(&self.mix_in, &self.squash, ctx1);
        self.l2_in[2] = self.l1[2].mix(&self.mix_in, &self.squash, ctx2);
        self.l2_in[3] = self.l1[3].mix(&self.mix_in, &self.squash, ctx3);
        self.l2_in[4] = self.l1[4].mix(&self.mix_in, &self.squash, ctx4);
        let ctx5 = ((self.matchlen.min(15) as usize) << 2)
            | (if self.matchlen3 > 0 { 2 } else { 0 })
            | (if self.matchlen4 > 0 { 1 } else { 0 });
        self.l2_in[5] = self.l1[5].mix(&self.mix_in, &self.squash, ctx5);
        let ctx6 = (((self.col & 63) << 6) | (self.c1 as u32 & 63)) as usize;
        self.l2_in[6] = self.l1[6].mix(&self.mix_in, &self.squash, ctx6);
        let ctx7 = (self.c4.wrapping_mul(0x9e37_79b1) >> 19) as usize;
        self.l2_in[7] = self.l1[7].mix(&self.mix_in, &self.squash, ctx7);
        let ctx8 = if self.pos >= 6 {
            (self.c4
                .wrapping_mul(0x85eb_ca6b)
                ^ (self.b(self.pos - 5) as u32).wrapping_mul(0xc2b2_ae35)
                ^ (self.b(self.pos - 6) as u32).wrapping_mul(0x27d4_eb2f))
                as usize
        } else {
            self.c4 as usize
        };
        self.l2_in[8] = self.l1[8].mix(&self.mix_in, &self.squash, ctx8);
        // stride-2 sparse selector: bytes at pos-2 and pos-4 (interleaved structure).
        let ctx9 = if self.pos >= 4 {
            (self.b(self.pos - 2) as usize) | ((self.b(self.pos - 4) as usize) << 8)
        } else {
            self.c1 as usize
        };
        self.l2_in[9] = self.l1[9].mix(&self.mix_in, &self.squash, ctx9);
        // stride-3 sparse selector: bytes at pos-3 and pos-6.
        let ctx10 = if self.pos >= 6 {
            (self.b(self.pos - 3) as usize) | ((self.b(self.pos - 6) as usize) << 8)
        } else {
            self.c1 as usize
        };
        self.l2_in[10] = self.l1[10].mix(&self.mix_in, &self.squash, ctx10);
        // byte-above selector (2D structure): specialise on the char one line up.
        let ctx11 = (self.above_byte as usize) | ((self.c1 as usize & 1) << 9);
        self.l2_in[11] = self.l1[11].mix(&self.mix_in, &self.squash, ctx11);
        // specialise on the order-2 indirect prediction (the byte that most
        // recently followed this 2-byte context).
        self.l2_in[12] = self.l1[12].mix(&self.mix_in, &self.squash, self.ind_pred as usize);
        // nest-state selector: specialise on the enclosing bracket + nesting depth.
        let nestsel = if self.nest_depth > 0 {
            (self.nest_stack[self.nest_depth - 1] as usize) | ((self.nest_depth & 3) << 8)
        } else {
            0
        };
        self.l2_in[13] = self.l1[13].mix(&self.mix_in, &self.squash, nestsel);
        // high-nibble (opcode-class) selector.
        let hnsel = ((self.c4 & 0xf0f0_f0f0).wrapping_mul(0x9e37_79b1) >> 20) as usize;
        self.l2_in[14] = self.l1[14].mix(&self.mix_in, &self.squash, hnsel);
        // character-class selector (letter/digit/space/other of last 4 bytes) —
        // a coarse semantic text-mode grouping (analogous to the high-nibble one).
        let cls = |b: u32| -> usize {
            let b = b & 0xff;
            if (b >= 97 && b <= 122) || (b >= 65 && b <= 90) {
                1
            } else if b >= 48 && b <= 57 {
                2
            } else if b == 32 || b == 9 || b == 10 || b == 13 {
                3
            } else {
                0
            }
        };
        let ccsel = cls(self.c4)
            | (cls(self.c4 >> 8) << 2)
            | (cls(self.c4 >> 16) << 4)
            | (cls(self.c4 >> 24) << 6);
        self.l2_in[15] = self.l1[15].mix(&self.mix_in, &self.squash, ccsel);
        // combined mode selector: last byte's high nibble + char-class of the
        // last two bytes (a richer visual+semantic mode than either alone).
        let modesel = ((self.c4 & 0xf0) >> 4) as usize
            | (cls(self.c4) << 4)
            | (cls(self.c4 >> 8) << 6);
        self.l2_in[16] = self.l1[16].mix(&self.mix_in, &self.squash, modesel);
        // run-length regime selector: bucket the current run length with the
        // class of the last byte — distinguishes "in a long run" from "varying".
        let runb = {
            let r = self.run_len;
            if r <= 1 { 0 } else if r == 2 { 1 } else if r <= 4 { 2 }
            else if r <= 8 { 3 } else if r <= 16 { 4 } else if r <= 64 { 5 }
            else if r <= 256 { 6 } else { 7 }
        };
        let runsel = runb | (cls(self.c4) << 3);
        self.l2_in[17] = self.l1[17].mix(&self.mix_in, &self.squash, runsel);
        // gradient / delta-sign selector: coarse sign (zero/up/down) of the last
        // three consecutive byte differences — a "numeric trend" mode.
        let dsign = |a: u32, b: u32| -> usize {
            let d = (a & 0xff).wrapping_sub(b & 0xff) & 0xff;
            if d == 0 { 0 } else if d < 128 { 1 } else { 2 }
        };
        let gradsel = dsign(self.c4, self.c4 >> 8)
            + 3 * dsign(self.c4 >> 8, self.c4 >> 16)
            + 9 * dsign(self.c4 >> 16, self.c4 >> 24);
        self.l2_in[18] = self.l1[18].mix(&self.mix_in, &self.squash, gradsel);
        // periodic / record selector: when the period detector is confident,
        // specialise on the coarse value of the byte one period back.
        let rgl = self.rlen;
        let rec_ok_l = self.rcount > 8 && rgl >= 2 && rgl < self.pos;
        let recsel = if rec_ok_l {
            1 + ((self.b(self.pos - rgl) as usize) >> 5)
        } else {
            0
        };
        self.l2_in[19] = self.l1[19].mix(&self.mix_in, &self.squash, recsel);
        // above-char-class + nesting selector: a 2D / structural mode keyed on
        // the class of the char one line up and the current bracket depth.
        let aboveclass = if self.above_byte > 255 { 4 } else { cls(self.above_byte) };
        let abovesel = aboveclass | ((self.nest_depth & 7) << 3);
        self.l2_in[20] = self.l1[20].mix(&self.mix_in, &self.squash, abovesel);
        // gradient-magnitude selector: bucket the magnitude of the last byte
        // difference (flat / small / medium / large) with the last-byte class —
        // a smooth-vs-noisy numeric mode, distinct from the delta-sign selector.
        let dmag = {
            let d = (self.c4 & 0xff).wrapping_sub((self.c4 >> 8) & 0xff) & 0xff;
            let m = if d >= 128 { 256 - d } else { d };
            if m == 0 { 0 } else if m <= 2 { 1 } else if m <= 8 { 2 }
            else if m <= 32 { 3 } else { 4 }
        };
        let gmagsel = dmag | (cls(self.c4) << 3);
        self.l2_in[21] = self.l1[21].mix(&self.mix_in, &self.squash, gmagsel);
        // vertical-repeat selector: whether the byte one line up equals the last
        // byte, combined with match activity and the last-byte class.
        let vrep = if self.above_byte <= 255 && self.above_byte == self.c1 as u32 { 1 } else { 0 };
        let vrepsel = vrep
            | ((if self.matchlen > 0 { 1 } else { 0 }) << 1)
            | (cls(self.c4) << 2);
        self.l2_in[22] = self.l1[22].mix(&self.mix_in, &self.squash, vrepsel);
        // bit-position + match-state selector: the within-byte bit position
        // combined with whether a match is currently active.
        let bmsel = (self.c0 as usize & 0x7f)
            | (if self.matchlen > 0 { 128 } else { 0 });
        self.l2_in[23] = self.l1[23].mix(&self.mix_in, &self.squash, bmsel);
        // opcode-trigram selector: the high nibbles of the last three bytes — a
        // coarse instruction-class trigram (binary), distinct from the existing
        // single-nibble selector.
        let optri = (((self.c4 >> 4) & 0xf)
            | (((self.c4 >> 12) & 0xf) << 2)
            | (((self.c4 >> 20) & 0xf) << 4)) as usize & 63;
        self.l2_in[24] = self.l1[24].mix(&self.mix_in, &self.squash, optri);
        // delta sign+magnitude selector: the last byte difference bucketed by
        // both sign and coarse magnitude (numeric trend, finer than sign alone).
        let dsm = {
            let d = (self.c4 & 0xff).wrapping_sub((self.c4 >> 8) & 0xff) & 0xff;
            let neg = d >= 128;
            let m = if neg { 256 - d } else { d };
            let mb = if m == 0 { 0 } else if m <= 4 { 1 } else if m <= 32 { 2 } else { 3 };
            (mb | (if neg { 4 } else { 0 })) as usize
        };
        let dsmsel = dsm | (cls(self.c4) << 3);
        self.l2_in[25] = self.l1[25].mix(&self.mix_in, &self.squash, dsmsel);
        // column-bucket + char-class selector: a layout / text-shape mode keyed
        // on coarse column position and the last-byte class.
        let colb = {
            let c = self.col;
            if c == 0 { 0 } else if c < 4 { 1 } else if c < 8 { 2 }
            else if c < 16 { 3 } else if c < 32 { 4 } else if c < 64 { 5 }
            else if c < 128 { 6 } else { 7 }
        };
        let wlsel = colb | (cls(self.c4) << 3);
        self.l2_in[26] = self.l1[26].mix(&self.mix_in, &self.squash, wlsel);
        // Two layer-2 combiners over the layer-1 logits — one keyed on the last
        // byte, one on the within-byte bit position — averaged in the logit domain.
        let d2a = self.l2.mix(&self.l2_in, &self.squash, self.c1 as usize);
        let d2b = self.l2b.mix(&self.l2_in, &self.squash, self.c0 as usize);
        let l2cctx = ((self.matchlen.min(15) as usize) << 2)
            | (if self.matchlen3 > 0 { 2 } else { 0 })
            | (if self.matchlen4 > 0 { 1 } else { 0 });
        let d2c = self.l2c.mix(&self.l2_in, &self.squash, l2cctx);
        let d2d = self.l2d.mix(&self.l2_in, &self.squash, ((self.c4 >> 8) & 0xff) as usize);
        let l2ectx = if self.wordhash != 0 {
            (self.wordhash.wrapping_mul(0x9e37_79b1) >> 24) as usize
        } else {
            self.c1 as usize
        };
        let d2e = self.l2e.mix(&self.l2_in, &self.squash, l2ectx);
        let l2fctx = ((self.c4 & 0xf0f0_f0f0).wrapping_mul(0x9e37_79b1) >> 24) as usize;
        let d2f = self.l2f.mix(&self.l2_in, &self.squash, l2fctx);
        let l2gctx = cls(self.c4)
            | (cls(self.c4 >> 8) << 2)
            | (cls(self.c4 >> 16) << 4)
            | (cls(self.c4 >> 24) << 6);
        let d2g = self.l2g.mix(&self.l2_in, &self.squash, l2gctx);
        let l2hctx = if self.nest_depth > 0 {
            (self.nest_stack[self.nest_depth - 1] as usize) | ((self.nest_depth & 3) << 8)
        } else {
            0
        };
        let d2h = self.l2h.mix(&self.l2_in, &self.squash, l2hctx);
        let l2ictx = (self.above_byte as usize) | ((self.c1 as usize & 1) << 9);
        let d2i = self.l2i.mix(&self.l2_in, &self.squash, l2ictx);
        // numeric-regime combiner: keyed on the byte-delta pattern of the last
        // three differences (captures gradient/tabular regimes).
        let l2jctx = (dsmsel & 0xff)
            | ((dsign(self.c4 >> 8, self.c4 >> 16)) << 6);
        let d2j = self.l2j.mix(&self.l2_in, &self.squash, l2jctx & 0xff);
        // Squash the combined logit straight to 16-bit and run the whole SSE/APM
        // chain at 16-bit precision (the calibration tables are ~16-bit), so no
        // stage re-quantizes the probability to the 12-bit 1/4096 grid.
        let mut p = squash16_d(
            &self.squash16,
            (d2a + d2b + d2c + d2d + d2e + d2f + d2g + d2h + d2i + d2j) / 10,
        );
        if p < 1 { p = 1; }
        if p > 65534 { p = 65534; }

        let a1ctx = ((self.c1 | (if self.matchlen > 0 { 256 } else { 0 })) as usize) & 1023;
        let a1 = self.apm1.apply16(&self.stretch16, a1ctx, p);
        p = (p + a1) >> 1;
        if p < 1 { p = 1; }
        if p > 65534 { p = 65534; }
        let a2 = self.apm2.apply16(&self.stretch16, (self.c4 & 0x3fff) as usize, p);
        p = (p + a2) >> 1;
        if p < 1 { p = 1; }
        if p > 65534 { p = 65534; }
        let a3ctx = (self.c0 as usize)
            | (if self.matchlen > 0 { 256 } else { 0 })
            | (if self.matchlen3 > 0 { 512 } else { 0 });
        let a3 = self.apm3.apply16(&self.stretch16, a3ctx, p);
        p = (p + a3) >> 1;
        if p < 1 { p = 1; }
        if p > 65534 { p = 65534; }
        // Match-length SSE: calibrate by how long the current order-6 match runs.
        let a4ctx = ((self.matchlen as usize) & 0xff)
            | (if self.matchlen3 > 0 { 256 } else { 0 })
            | (if self.matchlen4 > 0 { 512 } else { 0 });
        let a4 = self.apm4.apply16(&self.stretch16, a4ctx, p);
        p = (3 * p + a4) >> 2;
        if p < 1 { p = 1; }
        if p > 65534 { p = 65534; }
        p
    }

    #[inline]
    pub fn update(&mut self, bit: i32, _p: i32) {
        let t = if bit != 0 { 4095 } else { 0 };
        self.apm1.update(bit);
        self.apm2.update(bit);
        self.apm3.update(bit);
        self.apm4.update(bit);
        self.lstm.update(bit);
        if self.mm_used {
            let v = self.mm_sm[self.mm_idx] as i32;
            self.mm_sm[self.mm_idx] = (v + (((if bit != 0 { 4095 } else { 0 }) - v) >> 6)) as u16;
        }
        if self.mm_used2 {
            let v = self.mm_sm2[self.mm_idx2] as i32;
            self.mm_sm2[self.mm_idx2] = (v + (((if bit != 0 { 4095 } else { 0 }) - v) >> 6)) as u16;
        }
        if self.mm_used3 {
            let v = self.mm_sm3[self.mm_idx3] as i32;
            self.mm_sm3[self.mm_idx3] = (v + (((if bit != 0 { 4095 } else { 0 }) - v) >> 5)) as u16;
        }
        if self.mm_used4 {
            let v = self.mm_sm4[self.mm_idx4] as i32;
            self.mm_sm4[self.mm_idx4] = (v + (((if bit != 0 { 4095 } else { 0 }) - v) >> 1)) as u16;
        }
        if self.mm_used5 {
            let v = self.mm_sm5[self.mm_idx5] as i32;
            self.mm_sm5[self.mm_idx5] = (v + (((if bit != 0 { 4095 } else { 0 }) - v) >> 5)) as u16;
        }
        if self.mm_used6 {
            let v = self.mm_sm6[self.mm_idx6] as i32;
            self.mm_sm6[self.mm_idx6] = (v + (((if bit != 0 { 4095 } else { 0 }) - v) >> 5)) as u16;
        }
        self.l1[0].update(bit, &self.mix_in);
        self.l1[1].update(bit, &self.mix_in);
        self.l1[2].update(bit, &self.mix_in);
        self.l1[3].update(bit, &self.mix_in);
        self.l1[4].update(bit, &self.mix_in);
        self.l1[5].update(bit, &self.mix_in);
        self.l1[6].update(bit, &self.mix_in);
        self.l1[7].update(bit, &self.mix_in);
        self.l1[8].update(bit, &self.mix_in);
        self.l1[9].update(bit, &self.mix_in);
        self.l1[10].update(bit, &self.mix_in);
        self.l1[11].update(bit, &self.mix_in);
        self.l1[12].update(bit, &self.mix_in);
        self.l1[13].update(bit, &self.mix_in);
        self.l1[14].update(bit, &self.mix_in);
        self.l1[15].update(bit, &self.mix_in);
        self.l1[16].update(bit, &self.mix_in);
        self.l1[17].update(bit, &self.mix_in);
        self.l1[18].update(bit, &self.mix_in);
        self.l1[19].update(bit, &self.mix_in);
        self.l1[20].update(bit, &self.mix_in);
        self.l1[21].update(bit, &self.mix_in);
        self.l1[22].update(bit, &self.mix_in);
        self.l1[23].update(bit, &self.mix_in);
        self.l1[24].update(bit, &self.mix_in);
        self.l1[25].update(bit, &self.mix_in);
        self.l1[26].update(bit, &self.mix_in);
        self.l2.update(bit, &self.l2_in);
        self.l2b.update(bit, &self.l2_in);
        self.l2c.update(bit, &self.l2_in);
        self.l2d.update(bit, &self.l2_in);
        self.l2e.update(bit, &self.l2_in);
        self.l2f.update(bit, &self.l2_in);
        self.l2g.update(bit, &self.l2_in);
        self.l2h.update(bit, &self.l2_in);
        self.l2i.update(bit, &self.l2_in);
        self.l2j.update(bit, &self.l2_in);
        for i in 0..NCTX {
            let ix = self.idx[i];
            let n = self.cn[i][ix] as i32;
            let pr = self.cp[i][ix] as i32 + 2048;
            self.cp[i][ix] = ((pr + (((t - pr) * self.rate_tab[n as usize]) >> 12)) - 2048) as i16;
            if n < CNT_LIMIT {
                self.cn[i][ix] = (n + 1) as u8;
            }
            // StateMap: adapt prob for the observed bit-history state, then
            // advance that state. prob is 22-bit fixed point in the high bits,
            // an adaptation count (capped at 255) in the low 10 bits.
            let s = self.sm_idx[i];
            let entry = self.sm[i][s];
            let cnt = (entry & 1023) as i32;
            let p22 = (entry >> 10) as i32;
            let newp = p22 + (((bit << 22) - p22) / (cnt + 2));
            let newcnt = if cnt < 511 { cnt + 1 } else { 511 };
            self.sm[i][s] = ((newp as u32) << 10) | (newcnt as u32);
            self.st[i][ix] = next_state(s as u8, bit);
        }
        self.c0 = (self.c0 << 1) | bit;
        self.bitcount += 1;
        if self.bitcount == 8 {
            let byte = (self.c0 & 0xff) as u8;
            if self.matchlen > 0 {
                if (self.predicted_byte & 0xff) as u8 == byte {
                    self.matchptr += 1;
                    if self.matchlen < 0x3ff { self.matchlen += 1; }
                } else {
                    self.matchlen = 0;
                }
            }
            if self.matchlen2 > 0 {
                if (self.predicted_byte2 & 0xff) as u8 == byte {
                    self.matchptr2 += 1;
                    if self.matchlen2 < 0x3ff { self.matchlen2 += 1; }
                } else {
                    self.matchlen2 = 0;
                }
            }
            if self.matchlen3 > 0 {
                if (self.predicted_byte3 & 0xff) as u8 == byte {
                    self.matchptr3 += 1;
                    if self.matchlen3 < 0x3ff { self.matchlen3 += 1; }
                } else {
                    self.matchlen3 = 0;
                }
            }
            if self.matchlen4 > 0 {
                if (self.predicted_byte4 & 0xff) as u8 == byte {
                    self.matchptr4 += 1;
                    if self.matchlen4 < 0x3ff { self.matchlen4 += 1; }
                } else {
                    self.matchlen4 = 0;
                }
            }
            if self.matchlen5 > 0 {
                if (self.predicted_byte5 & 0xff) as u8 == byte {
                    self.matchptr5 += 1;
                    if self.matchlen5 < 0x3ff { self.matchlen5 += 1; }
                } else {
                    self.matchlen5 = 0;
                }
            }
            if self.matchlen6 > 0 {
                if (self.predicted_byte6 & 0xff) as u8 == byte {
                    self.matchptr6 += 1;
                    if self.matchlen6 < 0x3ff { self.matchlen6 += 1; }
                } else {
                    self.matchlen6 = 0;
                }
            }
            // Indirect model: record that `byte` followed the order-1 / order-2
            // context that preceded it (c4 still holds the pre-`byte` history).
            let ic1 = (self.c4 & 0xff) as usize;
            self.follow1[ic1] = (self.follow1[ic1] << 8) | byte as u32;
            let ic2 = (self.c4 & 0xffff) as usize;
            self.follow2[ic2] = (self.follow2[ic2] << 8) | byte as u32;
            let ic3 = ((self.c4 & 0x00ff_ffff).wrapping_mul(0x9e37_79b1) >> (32 - FBITS)) as usize;
            self.follow3[ic3] = (self.follow3[ic3] << 8) | byte as u32;
            let ic4 = (self.c4.wrapping_mul(0x85eb_ca6b) >> (32 - FBITS)) as usize;
            self.follow4[ic4] = (self.follow4[ic4] << 8) | byte as u32;
            if self.pos >= 5 {
                let m5 = self.c4.wrapping_mul(0x9e37_79b1)
                    ^ (self.b(self.pos - 5) as u32).wrapping_mul(0x85eb_ca6b);
                let k5 = (m5 >> (32 - FBITS)) as usize;
                self.follow5[k5] = (self.follow5[k5] << 8) | byte as u32;
            }
            if self.pos >= 6 {
                let m6 = self.c4.wrapping_mul(0x85eb_ca6b)
                    ^ (self.b(self.pos - 5) as u32).wrapping_mul(0xc2b2_ae35)
                    ^ (self.b(self.pos - 6) as u32).wrapping_mul(0x27d4_eb2f);
                let k6 = (m6 >> (32 - FBITS)) as usize;
                self.follow6[k6] = (self.follow6[k6] << 8) | byte as u32;
            }
            {
                let hnm = (self.c4 & 0xf0f0_f0f0).wrapping_mul(0x9e37_79b1);
                let khn = (hnm >> (32 - FBITS)) as usize;
                self.followhn[khn] = (self.followhn[khn] << 8) | byte as u32;
                let dd1 = (self.c4 & 0xff).wrapping_sub((self.c4 >> 8) & 0xff) & 0xff;
                let dd2 = ((self.c4 >> 8) & 0xff).wrapping_sub((self.c4 >> 16) & 0xff) & 0xff;
                let dd3 = ((self.c4 >> 16) & 0xff).wrapping_sub((self.c4 >> 24) & 0xff) & 0xff;
                let dm = (dd1 | (dd2 << 8) | (dd3 << 16)).wrapping_mul(0x85eb_ca6b);
                let kd = (dm >> (32 - FBITS)) as usize;
                self.followd[kd] = (self.followd[kd] << 8) | byte as u32;
                let cck = cls4(self.c4) as usize;
                self.followc[cck] = (self.followc[cck] << 8) | byte as u32;
                let lnm = (self.c4 & 0x0f0f_0f0f).wrapping_mul(0xc2b2_ae35);
                let kln = (lnm >> (32 - FBITS)) as usize;
                self.followln[kln] = (self.followln[kln] << 8) | byte as u32;
                let gk = if self.pos >= 3 {
                    (self.c4 & 0xff) | ((self.b(self.pos - 3) as u32) << 8)
                } else {
                    self.c4 & 0xffff
                } as usize;
                self.followg[gk] = (self.followg[gk] << 8) | byte as u32;
                let sk = if self.pos >= 4 {
                    (self.b(self.pos - 2) as u32) | ((self.b(self.pos - 4) as u32) << 8)
                } else {
                    self.c4 & 0xffff
                } as usize;
                self.follows2[sk] = (self.follows2[sk] << 8) | byte as u32;
                let s3k = if self.pos >= 6 {
                    (self.b(self.pos - 3) as u32) | ((self.b(self.pos - 6) as u32) << 8)
                } else {
                    self.c4 & 0xffff
                } as usize;
                self.follows3[s3k] = (self.follows3[s3k] << 8) | byte as u32;
                let g2k = if self.pos >= 5 {
                    (self.c4 & 0xff) | ((self.b(self.pos - 5) as u32) << 8)
                } else {
                    self.c4 & 0xffff
                } as usize;
                self.followg2[g2k] = (self.followg2[g2k] << 8) | byte as u32;
            }
            let bp = (self.pos & self.bufmask) as usize;
            self.buf[bp] = byte;
            self.pos += 1;
            self.c4 = (self.c4 << 8) | byte as u32;
            if byte as i32 == self.c1 {
                if self.run_len < 65535 { self.run_len += 1; }
            } else {
                self.run_len = 1;
            }
            self.c1 = byte as i32;
            // Nesting model: track the stack of open brackets (source structure).
            match byte {
                b'(' | b'[' | b'{' => {
                    if self.nest_depth < 64 {
                        self.nest_stack[self.nest_depth] = byte;
                        self.nest_depth += 1;
                    }
                }
                b')' | b']' | b'}' => {
                    if self.nest_depth > 0 {
                        self.nest_depth -= 1;
                    }
                }
                _ => {}
            }
            if byte == b'\n' || byte == b'\r' {
                self.col = 0;
            } else if self.col < 255 {
                self.col += 1;
            }
            if byte == b'\n' {
                self.prev2_line_start = self.prev_line_start;
                self.prev_line_start = self.line_start;
                self.line_start = self.pos;
            }
            // Record-length detector: majority-vote the distance between repeats
            // of each byte value, yielding the dominant period for data (binary /
            // tabular) that has no newline structure.
            let bi = byte as usize;
            let d = self.pos - self.rpos[bi];
            self.rpos[bi] = self.pos;
            if d == self.rlen {
                if self.rcount < 1024 { self.rcount += 1; }
            } else if self.rcount > 0 {
                self.rcount -= 1;
            } else {
                self.rlen = d;
                self.rcount = 1;
            }
            // Word-indirect: record that `byte` followed the current word prefix.
            let wk = (self.wordhash.wrapping_mul(0x9e37_79b1) >> 16) as usize;
            self.followw[wk] = (self.followw[wk] << 8) | byte as u32;
            if (byte >= b'a' && byte <= b'z') || (byte >= b'A' && byte <= b'Z')
                || (byte >= b'0' && byte <= b'9')
            {
                self.wordhash = hashk(self.wordhash, (byte | 0x20) as u32);
            } else {
                // Word boundary: shift the just-finished word into the word history.
                if self.wordhash != 0 {
                    self.prevword3 = self.prevword2;
                    self.prevword2 = self.prevword;
                    self.prevword = self.wordhash;
                }
                self.wordhash = 0;
            }
            if self.pos >= 6 {
                let h = (self.c4.wrapping_mul(2654435761)
                    .wrapping_add((self.b(self.pos - 5) as u32).wrapping_mul(0x85eb_ca6b))
                    .wrapping_add((self.b(self.pos - 6) as u32).wrapping_mul(0xc2b2_ae35)))
                    >> (32 - MMBITS);
                let cand = self.mmtab[h as usize];
                self.mmtab[h as usize] = self.pos;
                if self.matchlen == 0 && cand > 0 && cand < self.pos {
                    self.matchptr = cand;
                    let mut l: i32 = 0;
                    while l < 0x3ff
                        && cand > l as u32
                        && self.pos > (l as u32 + 1)
                        && self.b(cand - 1 - l as u32) == self.b(self.pos - 1 - l as u32)
                    {
                        l += 1;
                    }
                    self.matchlen = if l > 0 { l } else { 1 };
                }
            }
            if self.pos >= 8 {
                let h2 = (self.c4.wrapping_mul(2654435761)
                    .wrapping_add((self.b(self.pos - 5) as u32).wrapping_mul(0x85eb_ca6b))
                    .wrapping_add((self.b(self.pos - 6) as u32).wrapping_mul(0xc2b2_ae35))
                    .wrapping_add((self.b(self.pos - 7) as u32).wrapping_mul(0x27d4_eb2f))
                    .wrapping_add((self.b(self.pos - 8) as u32).wrapping_mul(0x1656_67b1)))
                    >> (32 - MMBITS2);
                let cand = self.mmtab2[h2 as usize];
                self.mmtab2[h2 as usize] = self.pos;
                if self.matchlen2 == 0 && cand > 0 && cand < self.pos {
                    self.matchptr2 = cand;
                    let mut l: i32 = 0;
                    while l < 0x3ff
                        && cand > l as u32
                        && self.pos > (l as u32 + 1)
                        && self.b(cand - 1 - l as u32) == self.b(self.pos - 1 - l as u32)
                    {
                        l += 1;
                    }
                    // Long-match specialist: only engage on genuinely long repeats.
                    self.matchlen2 = if l >= 8 { l } else { 0 };
                }
            }
            if self.pos >= 10 {
                let h3 = (self.c4.wrapping_mul(2654435761)
                    .wrapping_add((self.b(self.pos - 5) as u32).wrapping_mul(0x85eb_ca6b))
                    .wrapping_add((self.b(self.pos - 6) as u32).wrapping_mul(0xc2b2_ae35))
                    .wrapping_add((self.b(self.pos - 7) as u32).wrapping_mul(0x27d4_eb2f))
                    .wrapping_add((self.b(self.pos - 8) as u32).wrapping_mul(0x1656_67b1))
                    .wrapping_add((self.b(self.pos - 9) as u32).wrapping_mul(0xff51_afd7))
                    .wrapping_add((self.b(self.pos - 10) as u32).wrapping_mul(0xc4ce_b9fe)))
                    >> (32 - MMBITS3);
                let cand = self.mmtab3[h3 as usize];
                self.mmtab3[h3 as usize] = self.pos;
                if self.matchlen3 == 0 && cand > 0 && cand < self.pos {
                    self.matchptr3 = cand;
                    let mut l: i32 = 0;
                    while l < 0x3ff
                        && cand > l as u32
                        && self.pos > (l as u32 + 1)
                        && self.b(cand - 1 - l as u32) == self.b(self.pos - 1 - l as u32)
                    {
                        l += 1;
                    }
                    self.matchlen3 = if l >= 10 { l } else { 0 };
                }
            }
            if self.pos >= 12 {
                let h4 = (self.c4.wrapping_mul(2654435761)
                    .wrapping_add((self.b(self.pos - 5) as u32).wrapping_mul(0x85eb_ca6b))
                    .wrapping_add((self.b(self.pos - 6) as u32).wrapping_mul(0xc2b2_ae35))
                    .wrapping_add((self.b(self.pos - 7) as u32).wrapping_mul(0x27d4_eb2f))
                    .wrapping_add((self.b(self.pos - 8) as u32).wrapping_mul(0x1656_67b1))
                    .wrapping_add((self.b(self.pos - 9) as u32).wrapping_mul(0xff51_afd7))
                    .wrapping_add((self.b(self.pos - 10) as u32).wrapping_mul(0xc4ce_b9fe))
                    .wrapping_add((self.b(self.pos - 11) as u32).wrapping_mul(0x52dc_e729))
                    .wrapping_add((self.b(self.pos - 12) as u32).wrapping_mul(0x9e37_79b9)))
                    >> (32 - MMBITS4);
                let cand = self.mmtab4[h4 as usize];
                self.mmtab4[h4 as usize] = self.pos;
                if self.matchlen4 == 0 && cand > 0 && cand < self.pos {
                    self.matchptr4 = cand;
                    let mut l: i32 = 0;
                    while l < 0x3ff
                        && cand > l as u32
                        && self.pos > (l as u32 + 1)
                        && self.b(cand - 1 - l as u32) == self.b(self.pos - 1 - l as u32)
                    {
                        l += 1;
                    }
                    self.matchlen4 = if l >= 12 { l } else { 0 };
                }
            }
            if self.pos >= 14 {
                let h5 = (self.c4.wrapping_mul(2654435761)
                    .wrapping_add((self.b(self.pos - 5) as u32).wrapping_mul(0x85eb_ca6b))
                    .wrapping_add((self.b(self.pos - 6) as u32).wrapping_mul(0xc2b2_ae35))
                    .wrapping_add((self.b(self.pos - 7) as u32).wrapping_mul(0x27d4_eb2f))
                    .wrapping_add((self.b(self.pos - 8) as u32).wrapping_mul(0x1656_67b1))
                    .wrapping_add((self.b(self.pos - 9) as u32).wrapping_mul(0xff51_afd7))
                    .wrapping_add((self.b(self.pos - 10) as u32).wrapping_mul(0xc4ce_b9fe))
                    .wrapping_add((self.b(self.pos - 11) as u32).wrapping_mul(0x52dc_e729))
                    .wrapping_add((self.b(self.pos - 12) as u32).wrapping_mul(0x9e37_79b9))
                    .wrapping_add((self.b(self.pos - 13) as u32).wrapping_mul(0x7f4a_7c15))
                    .wrapping_add((self.b(self.pos - 14) as u32).wrapping_mul(0x94d0_49bb)))
                    >> (32 - MMBITS5);
                let cand = self.mmtab5[h5 as usize];
                self.mmtab5[h5 as usize] = self.pos;
                if self.matchlen5 == 0 && cand > 0 && cand < self.pos {
                    self.matchptr5 = cand;
                    let mut l: i32 = 0;
                    while l < 0x3ff
                        && cand > l as u32
                        && self.pos > (l as u32 + 1)
                        && self.b(cand - 1 - l as u32) == self.b(self.pos - 1 - l as u32)
                    {
                        l += 1;
                    }
                    self.matchlen5 = if l >= 14 { l } else { 0 };
                }
            }
            // order-2 (short) match model: anchored on just the last 2 bytes,
            // catches short repeats far earlier than the order-6+ models.
            if self.pos >= 2 {
                let h6 = ((self.c4 & 0x0000_ffff).wrapping_mul(2654435761)) >> (32 - MMBITS6);
                let cand = self.mmtab6[h6 as usize];
                self.mmtab6[h6 as usize] = self.pos;
                if self.matchlen6 == 0 && cand > 0 && cand < self.pos {
                    self.matchptr6 = cand;
                    let mut l: i32 = 0;
                    while l < 0x3ff
                        && cand > l as u32
                        && self.pos > (l as u32 + 1)
                        && self.b(cand - 1 - l as u32) == self.b(self.pos - 1 - l as u32)
                    {
                        l += 1;
                    }
                    self.matchlen6 = if l >= 2 { l } else { 0 };
                }
            }
            self.predicted_byte = if self.matchlen > 0 && self.matchptr < self.pos {
                self.b(self.matchptr) as i32
            } else {
                -1
            };
            self.predicted_byte2 = if self.matchlen2 > 0 && self.matchptr2 < self.pos {
                self.b(self.matchptr2) as i32
            } else {
                -1
            };
            self.predicted_byte3 = if self.matchlen3 > 0 && self.matchptr3 < self.pos {
                self.b(self.matchptr3) as i32
            } else {
                -1
            };
            self.predicted_byte4 = if self.matchlen4 > 0 && self.matchptr4 < self.pos {
                self.b(self.matchptr4) as i32
            } else {
                -1
            };
            self.predicted_byte5 = if self.matchlen5 > 0 && self.matchptr5 < self.pos {
                self.b(self.matchptr5) as i32
            } else {
                -1
            };
            self.predicted_byte6 = if self.matchlen6 > 0 && self.matchptr6 < self.pos {
                self.b(self.matchptr6) as i32
            } else {
                -1
            };
            self.c0 = 1;
            self.bitcount = 0;
            self.byte_start();
        }
    }
}
