//! Context Tree Weighting (Willems, Shtarkov & Tjalkens, 1995) as one extra
//! mixer input.
//!
//! CTW is the information-theoretic reference method for sequential binary
//! prediction: it performs exact Bayesian model averaging over *every* prunable
//! context tree up to a fixed depth D, with a Krichevsky–Trofimov estimator at
//! each node, achieving the minimax redundancy bound for tree sources. That is a
//! fundamentally different mixing law from this codec's logistic/geometric mixer,
//! so it contributes orthogonal signal.
//!
//! Standard recursion (per node s over its context's sub-block):
//!     Pw(s) = 1/2 · Pe(s) + 1/2 · Pw(s0) · Pw(s1)        (internal)
//!     Pw(s) = Pe(s)                                       (depth-D leaf)
//! Probabilities are kept in the log domain (they underflow over a corpus).
//!
//! Sustainable here: deterministic f64 (encoder and decoder run the identical
//! recursion → exactly lossless), a bounded hashed node store, and a
//! down-weightable mixer input (the mixer ignores it if it does not help).

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Pass-through hasher for the already-well-mixed `u64` node keys. `key()` below
/// applies a strong avalanche mix, so re-hashing with the default SipHash is pure
/// wasted work; this hasher just returns the key. HashMap semantics are unchanged
/// (lookups still match by exact key), so predictions are byte-for-byte identical
/// — only far cheaper. Keys are always written via `write_u64`.
#[derive(Default)]
struct IdHasher(u64);
impl Hasher for IdHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        // Not used for u64 keys; provide a correct fallback just in case.
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ b as u64;
        }
    }
}
type NodeMap = HashMap<u64, Node, BuildHasherDefault<IdHasher>>;

const DEPTH: usize = 48; // context depth in bits (6 bytes)
const MAXNODES: usize = 1 << 24; // node-store cap (~0.8 GB); freeze growth when hit
                                 // so adversarial/random inputs cannot OOM the verifier
const LN_HALF: f64 = -0.693_147_180_559_945_3; // ln(1/2)

#[derive(Clone, Copy)]
struct Node {
    n: [u32; 2], // KT counts for bit 0 / 1
    lpe: f64,    // ln Pe(s)  — log estimator block probability
    lw: f64,     // ln Pw(s)  — log weighted block probability
}

impl Node {
    #[inline]
    fn empty() -> Node {
        Node { n: [0, 0], lpe: 0.0, lw: 0.0 } // empty block: Pe = Pw = 1 -> ln = 0
    }
    /// KT predictive P(next bit = 1) = (n1 + 1/2) / (n0 + n1 + 1).
    #[inline]
    fn kt_p1(&self) -> f64 {
        (self.n[1] as f64 + 0.5) / (self.n[0] as f64 + self.n[1] as f64 + 1.0)
    }
}

#[inline]
fn ln_add(a: f64, b: f64) -> f64 {
    // log(e^a + e^b), numerically stable.
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    hi + (lo - hi).exp().ln_1p()
}

pub struct Ctw {
    nodes: NodeMap,
    hist: u64, // bit history, most-recent bit in bit 0
    // Nodes read by `predict`, stashed so the immediately-following `update` (same
    // bit, same history, no intervening writes) reuses them instead of re-fetching
    // every path/sibling node from the map. predict and update are always paired
    // (predict first), so these are fresh when update runs.
    spath: [Node; DEPTH + 1], // path node at each depth 0..=DEPTH
    ssib: [f64; DEPTH],       // sibling lw used at depth d (d in 0..DEPTH)
    skey: [u64; DEPTH + 1],   // map key per depth, computed once in predict and reused by update
    ln_half: f64,             // ln(0.5), computed once; reused for the KT update when n0==n1
    ln2: f64,                 // ln(2), computed once; the value ln_add adds when its args are equal
    // Until the node store hits its cap, the path's *present* nodes form a prefix
    // {0..=frontier} of the depth axis (a context can only have been inserted if all
    // its suffixes were), so `predict` probes top-down only until the first absent
    // depth: the deeper tail is provably empty and contributes pred = 0.5 with no
    // map probes. Once an insert is refused the prefix invariant can break (a parent
    // may be absent while a child exists), so this flips and `predict` reverts to the
    // full leaf→root walk — byte-identical to the unskipped recursion on every input.
    capped: bool,
}

impl Ctw {
    pub fn new() -> Self {
        Ctw {
            nodes: NodeMap::with_capacity_and_hasher(1 << 20, BuildHasherDefault::default()),
            hist: 0,
            spath: [Node::empty(); DEPTH + 1],
            ssib: [0.0; DEPTH],
            skey: [0; DEPTH + 1],
            // Same value `0.5_f64.ln()` would yield in the hot loop, computed once.
            ln_half: 0.5_f64.ln(),
            // ln(2) == `1.0_f64.ln_1p()`, the value `ln_add(a, a)` adds; computed once.
            ln2: 1.0_f64.ln_1p(),
            capped: false,
        }
    }

    #[inline]
    fn key(depth: usize, ctx: u64) -> u64 {
        // Distinct key per (depth, context bits). Mix so depths don't collide.
        let x = ctx | ((depth as u64) << 56);
        let mut h = x.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 29;
        h
    }
    #[inline]
    fn get(&self, depth: usize, ctx: u64) -> Node {
        *self.nodes.get(&Self::key(depth, ctx)).unwrap_or(&Node::empty())
    }

    #[inline]
    fn ctx_at(&self, d: usize, mask_d: u64) -> u64 {
        if d == DEPTH { self.hist & mask_d } else { self.hist & ((1u64 << d) - 1) }
    }

    /// One bottom-up CTW mixing step at depth `d`: blend node `nd`'s KT estimate with
    /// the deeper prediction `pred` by the CTW weight, using the on-path child lw
    /// `onpath_lw` and the sibling lw `sib_lw`. Returns the updated prediction.
    #[inline]
    fn step(pred: f64, nd: &Node, onpath_lw: f64, sib_lw: f64) -> f64 {
        let lpc = onpath_lw + sib_lw; // ln Pw(s0)Pw(s1)
        // alpha = Pe / (Pe + Pc) = 1 / (1 + e^{lpc - lpe}). When the exponent is
        // exactly 0 (an empty node with empty children, e.g. the deep unseen tail)
        // e^0 == 1, so alpha is exactly 0.5 — skip the expensive transcendental.
        let arg = lpc - nd.lpe;
        let alpha = if arg == 0.0 { 0.5 } else { 1.0 / (1.0 + arg.exp()) };
        alpha * nd.kt_p1() + (1.0 - alpha) * pred
    }

    /// CTW predictive P(bit = 1) at the current context, as a stretched logit.
    #[inline]
    pub fn predict(&mut self, stretch: &[i32]) -> i32 {
        let mask_d = if DEPTH >= 64 { u64::MAX } else { (1u64 << DEPTH) - 1 };
        let pred = if !self.capped {
            // Fast path: probe the path top-down only until the first absent depth.
            // Pre-cap, present nodes are a prefix {0..=frontier}; everything deeper is
            // empty and leaves pred at 0.5, so it needs no probe.
            let mut frontier: isize = -1;
            let mut absent = DEPTH + 1; // first depth with no node (exclusive top of prefix)
            for d in 0..=DEPTH {
                let k = Self::key(d, self.ctx_at(d, mask_d));
                self.skey[d] = k;
                match self.nodes.get(&k) {
                    Some(node) => {
                        self.spath[d] = *node;
                        frontier = d as isize;
                    }
                    None => {
                        absent = d;
                        break;
                    }
                }
            }
            // Absent tail: empty nodes, absent siblings. Fill keys (for update's
            // inserts) and the empty stash so update reproduces the full recursion.
            for d in absent..=DEPTH {
                self.skey[d] = Self::key(d, self.ctx_at(d, mask_d));
                self.spath[d] = Node::empty();
            }
            for d in absent..DEPTH {
                self.ssib[d] = 0.0;
            }
            // Seed the recursion. If the leaf is present we start from its KT estimate
            // (and it is the on-path child of depth DEPTH-1); otherwise the empty tail
            // pins pred at 0.5 down to the frontier, whose on-path child is empty.
            let (mut pred, mut onpath_lw, top) = if frontier == DEPTH as isize {
                let leaf = self.spath[DEPTH];
                (leaf.kt_p1(), leaf.lw, DEPTH as isize - 1)
            } else {
                (0.5, 0.0, frontier)
            };
            let mut d = top;
            while d >= 0 {
                let di = d as usize;
                let sib_ctx = (self.hist & ((1u64 << (di + 1)) - 1)) ^ (1u64 << di);
                let sib_lw = self.get(di + 1, sib_ctx).lw; // sibling may be present
                self.ssib[di] = sib_lw;
                let nd = self.spath[di];
                pred = Self::step(pred, &nd, onpath_lw, sib_lw);
                onpath_lw = nd.lw;
                d -= 1;
            }
            pred
        } else {
            // Post-cap fallback: full leaf→root walk (the prefix invariant can break
            // once growth is frozen, so probe every depth).
            let leaf = self.get(DEPTH, self.hist & mask_d);
            self.spath[DEPTH] = leaf;
            self.skey[DEPTH] = Self::key(DEPTH, self.hist & mask_d);
            let mut pred = leaf.kt_p1();
            let mut onpath = leaf;
            for d in (0..DEPTH).rev() {
                let ctx_d = self.hist & ((1u64 << d) - 1);
                let k = Self::key(d, ctx_d);
                let nd = self.get(d, ctx_d);
                let sib_ctx = (self.hist & ((1u64 << (d + 1)) - 1)) ^ (1u64 << d);
                let sib = self.get(d + 1, sib_ctx);
                self.spath[d] = nd;
                self.skey[d] = k;
                self.ssib[d] = sib.lw;
                pred = Self::step(pred, &nd, onpath.lw, sib.lw);
                onpath = nd;
            }
            pred
        };
        let mut p = (pred * 4096.0) as i32;
        if p < 1 { p = 1; }
        if p > 4095 { p = 4095; }
        stretch[p as usize]
    }

    /// Observe `bit`: update KT counts, Pe and Pw along the path (leaf → root),
    /// then shift it into the history.
    #[inline]
    pub fn update(&mut self, bit: i32) {
        let b = bit as usize;
        // Recompute each path node's lw from the bottom up; the on-path child's lw
        // is the value we just updated, the sibling's is unchanged.
        let mut child_lw = 0.0f64; // lw of the (updated) deeper path node; leaf has no children below
        for d in (0..=DEPTH).rev() {
            // Reuse the node `predict` already read for this depth (same history,
            // no writes since) instead of re-fetching it from the map.
            let mut nd = self.spath[d];
            // KT probability of the observed bit, then count it. When the two counts
            // are equal (always so on a node's first touch, n0==n1==0) the KT
            // probability is exactly (k+0.5)/(2k+1) = 0.5 for either bit, so its log
            // is the precomputed ln(0.5) — skip the transcendental. Bit-identical.
            if nd.n[0] == nd.n[1] {
                nd.lpe += self.ln_half;
            } else {
                let denom = nd.n[0] as f64 + nd.n[1] as f64 + 1.0;
                let p_obs = (nd.n[b] as f64 + 0.5) / denom;
                nd.lpe += p_obs.ln();
            }
            nd.n[b] += 1;
            if d == DEPTH {
                nd.lw = nd.lpe; // leaf
                child_lw = nd.lw;
            } else {
                // sibling (off-path child) lw, unchanged this step — also stashed
                // by predict.
                let sib_lw = self.ssib[d];
                let a = LN_HALF + nd.lpe;
                let b = LN_HALF + child_lw + sib_lw;
                // ln_add(a, b) with a == b reduces to a + ln(2) (its `exp(0)` term is
                // exactly 1) — common for the deep first-touch chain, where both sides
                // are 2·ln(0.5). Skip the exp/ln_1p; bit-identical to the full call.
                nd.lw = if a == b { a + self.ln2 } else { ln_add(a, b) };
                child_lw = nd.lw;
            }
            // Key was computed and stashed by `predict` for this same depth/history.
            let k = self.skey[d];
            // Bounded memory: only grow the store until the cap; past it, refresh
            // existing nodes but stop adding new contexts (they stay empty). The first
            // refused insert can leave a present node with an absent parent, breaking
            // the prefix invariant, so disable predict's top-down skip from here on.
            if self.nodes.len() < MAXNODES || self.nodes.contains_key(&k) {
                self.nodes.insert(k, nd);
            } else {
                self.capped = true;
            }
        }
        self.hist = (self.hist << 1) | (b as u64);
    }
}
