//! Context-mixing predictor: multi-order adaptive counters, a learned match
//! model, a logistic mixer, and a two-stage APM/SSE.
//!
//! THIS IS THE PRIMARY EDITABLE SURFACE. Change models, add models, retune,
//! restructure — anything goes, provided `compress`/`decompress` remain exactly
//! lossless on all inputs and the predict/update sequence stays identical
//! between encode and decode.

use super::tables::{build, squash_d};

const NCTX: usize = 69; // orders + word/n-gram + strided-sparse + gap bigrams + text shape/layout
// Mixer input layout:
//   [0 .. NCTX)            direct adaptive counters
//   [SM_BASE .. SM_BASE+NCTX) bit-history StateMap predictions (one per context)
//   [MM_BASE .. MM_BASE+5)  five match models (order-6, -8, -10, -12, -14)
const SM_BASE: usize = NCTX;
const MM_BASE: usize = 2 * NCTX;
const NINPUT: usize = 2 * NCTX + 5;
const TBITS: u32 = 23;
const TSIZE: usize = 1 << TBITS;
const TMASK: u32 = (TSIZE as u32) - 1;
const MIXCTX: usize = 16384;
const NL1: usize = 5; // number of layer-1 specialist mixers
const MIX3CTX: usize = 8192; // order-2 specialist rows
const MIX4CTX: usize = 8192; // order-3 specialist rows
const MMBITS: u32 = 25;
const MMSIZE: usize = 1 << MMBITS;
const MMBITS2: u32 = 26;
const MMSIZE2: usize = 1 << MMBITS2;
const MMBITS3: u32 = 23;
const MMSIZE3: usize = 1 << MMBITS3;
const MMBITS4: u32 = 24;
const MMSIZE4: usize = 1 << MMBITS4;
const MMBITS5: u32 = 24;
const MMSIZE5: usize = 1 << MMBITS5;
const APM_S: usize = 33;
const CNT_LIMIT: i32 = 254;
const RATE_FLOOR: i32 = 40;

#[inline]
fn hashk(h: u32, x: u32) -> u32 {
    h.wrapping_add(x).wrapping_add(1).wrapping_mul(2654435761)
}

/// Nonstationary bit-history state transition. The state byte packs two bounded
/// counts (n0 in the high nibble, n1 in the low nibble, each 0..15). On each
/// observed bit the matching count is incremented and the opposite count is
/// discounted toward 2, which emphasises recent statistics — the classic
/// recency bias that lets the StateMap track nonstationary / repetitive data.
#[inline]
fn next_state(s: u8, bit: i32) -> u8 {
    let mut n0 = (s >> 4) as i32;
    let mut n1 = (s & 15) as i32;
    if bit != 0 {
        n1 += 1;
        if n0 > 2 { n0 = 2 + ((n0 - 2) >> 1); }
    } else {
        n0 += 1;
        if n1 > 2 { n1 = 2 + ((n1 - 2) >> 1); }
    }
    if n0 > 15 { n0 = 15; }
    if n1 > 15 { n1 = 15; }
    ((n0 << 4) | n1) as u8
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

    #[inline]
    fn apply(&mut self, stretch: &[i32], ctx: usize, p: i32) -> i32 {
        let s = stretch[p as usize] + 2048; // 0..4095
        let w = s & 127;
        let j = (s >> 7) as usize; // 0..31
        self.idx = ctx * APM_S + j;
        let lo = self.t[self.idx] as i32;
        let hi = self.t[self.idx + 1] as i32;
        let mut pp = (lo * (128 - w) + hi * w) >> 11;
        if pp < 1 { pp = 1; }
        if pp > 4094 { pp = 4094; }
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
    cp: Vec<Vec<u16>>, // [NCTX][TSIZE] probabilities 0..4095
    cn: Vec<Vec<u8>>,  // [NCTX][TSIZE] observation counts
    st: Vec<Vec<u8>>,  // [NCTX][TSIZE] bit-history state per context slot
    sm: Vec<Vec<u32>>, // [NCTX][256*8] StateMap: (state | bitpos<<8) -> (prob22<<10 | count)
    sm_idx: [usize; NCTX],
    rate_tab: [i32; 256],
    ctxhash: [u32; NCTX],
    idx: [usize; NCTX],
    mix_in: [i32; NINPUT],
    l1: Vec<Mixer>,   // layer-1 specialist mixers (different selection contexts)
    l2: Mixer,        // layer-2 combiner over the layer-1 logits (last-byte ctx)
    l2b: Mixer,       // second layer-2 combiner (bit-position ctx)
    l2c: Mixer,       // third layer-2 combiner (match-state ctx)
    l2d: Mixer,       // fourth layer-2 combiner (2nd-to-last-byte ctx)
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
    apm1: Apm,
    apm2: Apm,
    apm3: Apm,
    apm4: Apm,
    c0: i32,
    bitcount: i32,
    c4: u32,
    wordhash: u32,
    prevword: u32,
    prevword2: u32,
    prevword3: u32,
    c1: i32,
    col: u32,
}

impl Cm {
    pub fn new(expected_len: usize) -> Self {
        let (stretch, squash) = build();
        let mut rate_tab = [0i32; 256];
        for n in 0..256 {
            let mut r = 4096 / (n as i32 + 2);
            if r < RATE_FLOOR { r = RATE_FLOOR; }
            rate_tab[n] = r;
        }
        let cp = (0..NCTX).map(|_| vec![2048u16; TSIZE]).collect();
        let cn = (0..NCTX).map(|_| vec![0u8; TSIZE]).collect();
        let st = (0..NCTX).map(|_| vec![0u8; TSIZE]).collect();
        let sm = (0..NCTX).map(|_| vec![1u32 << 31; 256 * 8]).collect();
        let l1 = vec![
            Mixer::new(NINPUT, MIXCTX, 14),
            Mixer::new(NINPUT, 256, 14),
            Mixer::new(NINPUT, 256, 14),
            Mixer::new(NINPUT, MIX3CTX, 14),
            Mixer::new(NINPUT, MIX4CTX, 14),
        ];
        let l2 = Mixer::new(NL1, 256, 12);
        let l2b = Mixer::new(NL1, 256, 12);
        let l2c = Mixer::new(NL1, 256, 12);
        let l2d = Mixer::new(NL1, 256, 12);

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
            cp,
            cn,
            st,
            sm,
            sm_idx: [0; NCTX],
            rate_tab,
            ctxhash: [0; NCTX],
            idx: [0; NCTX],
            mix_in: [0; NINPUT],
            l1,
            l2,
            l2b,
            l2c,
            l2d,
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
            apm1,
            apm2,
            apm3,
            apm4,
            c0: 1,
            bitcount: 0,
            c4: 0,
            wordhash: 0,
            prevword: 0,
            prevword2: 0,
            prevword3: 0,
            c1: 0,
            col: 0,
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
    }

    #[inline]
    pub fn predict(&mut self) -> i32 {
        for i in 0..NCTX {
            let ix = (self.ctxhash[i].wrapping_mul(769).wrapping_add(self.c0 as u32) & TMASK) as usize;
            self.idx[i] = ix;
            self.mix_in[i] = self.stretch[self.cp[i][ix] as usize];
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
        // Two layer-2 combiners over the layer-1 logits — one keyed on the last
        // byte, one on the within-byte bit position — averaged in the logit domain.
        let d2a = self.l2.mix(&self.l2_in, &self.squash, self.c1 as usize);
        let d2b = self.l2b.mix(&self.l2_in, &self.squash, self.c0 as usize);
        let l2cctx = ((self.matchlen.min(15) as usize) << 2)
            | (if self.matchlen3 > 0 { 2 } else { 0 })
            | (if self.matchlen4 > 0 { 1 } else { 0 });
        let d2c = self.l2c.mix(&self.l2_in, &self.squash, l2cctx);
        let d2d = self.l2d.mix(&self.l2_in, &self.squash, ((self.c4 >> 8) & 0xff) as usize);
        let mut p = squash_d(&self.squash, (d2a + d2b + d2c + d2d) >> 2);
        if p < 1 { p = 1; }
        if p > 4094 { p = 4094; }

        let a1ctx = ((self.c1 | (if self.matchlen > 0 { 256 } else { 0 })) as usize) & 1023;
        let a1 = self.apm1.apply(&self.stretch, a1ctx, p);
        p = (p + a1) >> 1;
        if p < 1 { p = 1; }
        if p > 4094 { p = 4094; }
        let a2 = self.apm2.apply(&self.stretch, (self.c4 & 0x3fff) as usize, p);
        p = (p + a2) >> 1;
        if p < 1 { p = 1; }
        if p > 4094 { p = 4094; }
        let a3ctx = (self.c0 as usize)
            | (if self.matchlen > 0 { 256 } else { 0 })
            | (if self.matchlen3 > 0 { 512 } else { 0 });
        let a3 = self.apm3.apply(&self.stretch, a3ctx, p);
        p = (p + a3) >> 1;
        if p < 1 { p = 1; }
        if p > 4094 { p = 4094; }
        // Match-length SSE: calibrate by how long the current order-6 match runs.
        let a4ctx = ((self.matchlen as usize) & 0xff)
            | (if self.matchlen3 > 0 { 256 } else { 0 })
            | (if self.matchlen4 > 0 { 512 } else { 0 });
        let a4 = self.apm4.apply(&self.stretch, a4ctx, p);
        p = (3 * p + a4) >> 2;
        if p < 1 { p = 1; }
        if p > 4094 { p = 4094; }
        p
    }

    #[inline]
    pub fn update(&mut self, bit: i32, _p: i32) {
        let t = if bit != 0 { 4095 } else { 0 };
        self.apm1.update(bit);
        self.apm2.update(bit);
        self.apm3.update(bit);
        self.apm4.update(bit);
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
        self.l1[0].update(bit, &self.mix_in);
        self.l1[1].update(bit, &self.mix_in);
        self.l1[2].update(bit, &self.mix_in);
        self.l1[3].update(bit, &self.mix_in);
        self.l1[4].update(bit, &self.mix_in);
        self.l2.update(bit, &self.l2_in);
        self.l2b.update(bit, &self.l2_in);
        self.l2c.update(bit, &self.l2_in);
        self.l2d.update(bit, &self.l2_in);
        for i in 0..NCTX {
            let ix = self.idx[i];
            let n = self.cn[i][ix] as i32;
            let pr = self.cp[i][ix] as i32;
            self.cp[i][ix] = (pr + (((t - pr) * self.rate_tab[n as usize]) >> 12)) as u16;
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
            let newcnt = if cnt < 255 { cnt + 1 } else { 255 };
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
            let bp = (self.pos & self.bufmask) as usize;
            self.buf[bp] = byte;
            self.pos += 1;
            self.c4 = (self.c4 << 8) | byte as u32;
            self.c1 = byte as i32;
            if byte == b'\n' || byte == b'\r' {
                self.col = 0;
            } else if self.col < 255 {
                self.col += 1;
            }
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
            self.c0 = 1;
            self.bitcount = 0;
            self.byte_start();
        }
    }
}
