//! Dynamic Markov Coding (Cormack & Horspool, 1987) as one extra mixer input.
//!
//! DMC is a bitwise, variable-order Markov predictor. It keeps a state graph in
//! which each state has two outgoing transitions (for bit 0 and bit 1) carrying
//! visit counts; the next-bit probability is the smoothed ratio of those counts.
//! The graph *grows by cloning*: when a transition is both well-established and
//! leads to a state that is also reached from elsewhere, the target is cloned so
//! the path gets its own specialized statistics. This lets the model discover
//! high-order contexts on its own — orthogonal signal the fixed hashed-context
//! bank does not capture.
//!
//! Why it is a sustainable addition here: it is a self-contained, well-specified
//! classical model with a hard node cap (bounded memory; growth simply freezes
//! when full), pure integer arithmetic (deterministic — encode and decode evolve
//! the identical graph, so the codec stays exactly lossless), and it is wired as
//! a down-weightable mixer input, so if it fails to help the mixer ignores it.

const MAXN: usize = 1 << 22; // node cap (~64 MB at 16 B/node); freeze growth when hit

struct Node {
    nx: [u32; 2], // next state for an observed 0 / 1
    c: [u32; 2],  // visit counts for 0 / 1
}

pub struct Dmc {
    nodes: Vec<Node>,
    n: u32,    // number of allocated nodes
    curr: u32, // current state
    t1: u32,   // min count on the taken edge before it may clone its target
    t2: u32,   // min residual count on the target (reached from elsewhere too)
}

impl Dmc {
    /// `t1`/`t2` are the clone thresholds: small values clone aggressively (fast,
    /// high-order specialization); larger values stay lower-order longer (slower,
    /// more stable). Running two instances at different speeds is complementary.
    pub fn new(t1: u32, t2: u32) -> Self {
        // Initial graph: a depth-8 binary tree over the bits of one byte, so the
        // model starts as a plain order-0 byte model; leaves loop back to the root
        // to begin the next byte. index(k,p) = (2^k - 1) + p for prefix p of k bits.
        let mut nodes: Vec<Node> = Vec::with_capacity(512);
        for k in 0..8u32 {
            let width = 1u32 << k;
            for p in 0..width {
                let (n0, n1) = if k < 7 {
                    let base_k1 = (1u32 << (k + 1)) - 1;
                    (base_k1 + p * 2, base_k1 + p * 2 + 1)
                } else {
                    (0, 0) // byte complete -> back to root
                };
                nodes.push(Node { nx: [n0, n1], c: [0, 0] });
            }
        }
        let n = nodes.len() as u32; // 255
        Dmc { nodes, n, curr: 0, t1, t2 }
    }

    /// Smoothed P(bit=1) at the current state, returned as a stretched logit.
    #[inline]
    pub fn predict(&self, stretch: &[i32]) -> i32 {
        let s = self.curr as usize;
        let c0 = self.nodes[s].c[0] as u64;
        let c1 = self.nodes[s].c[1] as u64;
        // Krichevsky–Trofimov style smoothing: (c1 + 0.5) / (c0 + c1 + 1).
        let mut p = (((2 * c1 + 1) * 2048) / (c0 + c1 + 1)) as i32;
        if p < 1 { p = 1; }
        if p > 4095 { p = 4095; }
        stretch[p as usize]
    }

    /// Observe `bit`: optionally clone the target, bump the edge count, advance.
    #[inline]
    pub fn update(&mut self, bit: i32) {
        let s = self.curr as usize;
        let b = bit as usize;
        let next = self.nodes[s].nx[b];
        let nu = next as usize;
        let edge = self.nodes[s].c[b];
        let tot = self.nodes[nu].c[0] + self.nodes[nu].c[1];

        let target = if edge >= self.t1 && tot >= edge + self.t2 && (self.n as usize) < MAXN {
            // Clone `next`: the new node inherits next's transitions and a share of
            // its counts proportional to how much of next's traffic came via `edge`.
            let nx0 = self.nodes[nu].nx[0];
            let nx1 = self.nodes[nu].nx[1];
            let mc0 = (self.nodes[nu].c[0] as u64 * edge as u64 / tot as u64) as u32;
            let mc1 = (self.nodes[nu].c[1] as u64 * edge as u64 / tot as u64) as u32;
            self.nodes[nu].c[0] -= mc0;
            self.nodes[nu].c[1] -= mc1;
            let m = self.n;
            self.nodes.push(Node { nx: [nx0, nx1], c: [mc0, mc1] });
            self.nodes[s].nx[b] = m;
            self.n += 1;
            m
        } else {
            next
        };

        self.nodes[s].c[b] += 1;
        self.curr = target;
    }
}
