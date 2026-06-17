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

const DEPTH: usize = 32; // context depth in bits (4 bytes)
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
}

impl Ctw {
    pub fn new() -> Self {
        Ctw {
            nodes: NodeMap::with_capacity_and_hasher(1 << 20, BuildHasherDefault::default()),
            hist: 0,
            spath: [Node::empty(); DEPTH + 1],
            ssib: [0.0; DEPTH],
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

    /// CTW predictive P(bit = 1) at the current context, as a stretched logit.
    #[inline]
    pub fn predict(&mut self, stretch: &[i32]) -> i32 {
        // Walk up from the depth-D leaf to the root, mixing each node's estimator
        // prediction with the deeper prediction by its CTW weight. Each node read
        // is stashed for the matching `update` to reuse.
        let mask_d = if DEPTH >= 64 { u64::MAX } else { (1u64 << DEPTH) - 1 };
        let leaf = self.get(DEPTH, self.hist & mask_d);
        self.spath[DEPTH] = leaf;
        let mut pred = leaf.kt_p1();
        // The on-path child at depth d+1 is exactly the node fetched as `nd` in the
        // previous (deeper) iteration, so carry it forward instead of re-fetching:
        // halves the path lookups. `onpath` starts as the depth-D leaf.
        let mut onpath = leaf;
        for d in (0..DEPTH).rev() {
            let ctx_d = self.hist & ((1u64 << d) - 1).max(0); // last d bits (0 when d==0)
            let nd = self.get(d, ctx_d);
            // children at depth d+1 split on bit d of history
            let sib_ctx = (self.hist & ((1u64 << (d + 1)) - 1)) ^ (1u64 << d);
            let sib = self.get(d + 1, sib_ctx);
            self.spath[d] = nd;
            self.ssib[d] = sib.lw;
            let lpc = onpath.lw + sib.lw; // ln Pw(s0)Pw(s1)
            // alpha = Pe / (Pe + Pc) = 1 / (1 + e^{lpc - lpe})
            let alpha = 1.0 / (1.0 + (lpc - nd.lpe).exp());
            pred = alpha * nd.kt_p1() + (1.0 - alpha) * pred;
            onpath = nd; // becomes the on-path child for the next, shallower depth
        }
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
            let ctx = if d == 0 { 0 } else { self.hist & ((1u64 << d) - 1) };
            // Reuse the node `predict` already read for this depth (same history,
            // no writes since) instead of re-fetching it from the map.
            let mut nd = self.spath[d];
            // KT probability of the observed bit, then count it.
            let denom = nd.n[0] as f64 + nd.n[1] as f64 + 1.0;
            let p_obs = (nd.n[b] as f64 + 0.5) / denom;
            nd.lpe += p_obs.ln();
            nd.n[b] += 1;
            if d == DEPTH {
                nd.lw = nd.lpe; // leaf
                child_lw = nd.lw;
            } else {
                // sibling (off-path child) lw, unchanged this step — also stashed
                // by predict.
                let sib_lw = self.ssib[d];
                nd.lw = ln_add(LN_HALF + nd.lpe, LN_HALF + child_lw + sib_lw);
                child_lw = nd.lw;
            }
            let k = Self::key(d, ctx);
            // Bounded memory: only grow the store until the cap; past it, refresh
            // existing nodes but stop adding new contexts (they stay empty).
            if self.nodes.len() < MAXNODES || self.nodes.contains_key(&k) {
                self.nodes.insert(k, nd);
            }
        }
        self.hist = (self.hist << 1) | (b as u64);
    }
}
