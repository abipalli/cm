//! Logistic transform tables (stretch / squash).
//!
//! Part of the EDITABLE algorithm. You may change how these are built or
//! replace them entirely, as long as `compress`/`decompress` stay lossless.

/// Build (stretch, squash) lookup tables.
/// squash(d) = 4096 / (1 + e^(-d/256)), d in [-2047, 2047] -> p in [0, 4095].
/// stretch is its inverse.
pub fn build() -> (Vec<i32>, Vec<i32>) {
    let mut squash = vec![0i32; 4096];
    for i in 0..4096 {
        let d = (i as f64 - 2048.0) / 256.0;
        let v = 4096.0 / (1.0 + (-d).exp());
        let mut iv = (v + 0.5) as i32;
        if iv < 0 { iv = 0; }
        if iv > 4095 { iv = 4095; }
        squash[i] = iv;
    }
    let mut stretch = vec![0i32; 4096];
    let mut pi = 0usize;
    let mut d = -2047i32;
    while d <= 2047 {
        let p = squash_d(&squash, d);
        while pi <= p as usize {
            stretch[pi] = d;
            pi += 1;
        }
        d += 1;
    }
    while pi < 4096 {
        stretch[pi] = 2047;
        pi += 1;
    }
    (stretch, squash)
}

#[inline]
pub fn squash_d(squash: &[i32], d: i32) -> i32 {
    if d >= 2047 { return 4095; }
    if d <= -2047 { return 0; }
    squash[(d + 2048) as usize]
}
