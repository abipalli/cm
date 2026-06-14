//! Losslessness gate. FROZEN — do not edit as part of autoresearch.
//!
//! These tests deliberately use synthetic / adversarial inputs (NOT the corpus)
//! so that a candidate cannot pass by overfitting to the benchmark. Any change
//! to the algorithm must keep `decompress(compress(x)) == x` for ALL of these.

use cm::{compress, decompress};

fn rt(data: &[u8]) {
    let c = compress(data);
    let d = decompress(&c);
    assert!(
        d == data,
        "round-trip failed: len={} (got {} bytes back)",
        data.len(),
        d.len()
    );
}

// Simple deterministic PRNG so tests need no dependencies.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| (self.next() & 0xff) as u8).collect()
    }
}

#[test]
fn empty() {
    rt(&[]);
}

#[test]
fn single_bytes() {
    for b in [0u8, 1, 0x7f, 0x80, 0xE8, 0xE9, 0xff] {
        rt(&[b]);
    }
}

#[test]
fn tiny_lengths() {
    let v: Vec<u8> = (0..40u8).collect();
    for n in 0..=v.len() {
        rt(&v[..n]);
    }
}

#[test]
fn all_same() {
    for b in [0u8, 0x41, 0xff] {
        rt(&vec![b; 100_000]);
    }
}

#[test]
fn random_incompressible() {
    let mut r = Rng(0xDEAD_BEEF);
    for n in [1usize, 7, 256, 4096, 200_000] {
        rt(&r.bytes(n));
    }
}

#[test]
fn highly_repetitive() {
    let pat = b"the quick brown fox 0123456789 ";
    let mut v = Vec::new();
    while v.len() < 300_000 {
        v.extend_from_slice(pat);
    }
    rt(&v);
}

#[test]
fn text_like() {
    let mut v = Vec::new();
    let words = ["alpha ", "beta ", "gamma ", "delta. ", "epsilon\n"];
    let mut r = Rng(42);
    while v.len() < 150_000 {
        v.extend_from_slice(words[(r.next() as usize) % words.len()].as_bytes());
    }
    rt(&v);
}

#[test]
fn structured_e8e9_heavy() {
    // Many 0xE8/0xE9 bytes to exercise the BCJ filter path.
    let mut r = Rng(7);
    let mut v = vec![b'M', b'Z'];
    for _ in 0..50_000 {
        v.push(0xE8);
        v.extend_from_slice(&r.bytes(4));
    }
    rt(&v);
}

#[test]
fn compresses_redundant_input() {
    // Sanity: compressible data must actually shrink.
    let data = vec![0u8; 50_000];
    assert!(compress(&data).len() < data.len() / 10);
}
