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

/// Build the 16-bit-resolution logistic tables for the final SSE/APM chain +
/// coder: (stretch16[0..65536] -> logit, squash16[logit+2048] -> 0..65535 prob).
/// Same logistic as `build`, but the probability is kept at 16 bits so confident
/// predictions are not capped at the 12-bit 1/4096 resolution.
pub fn build16() -> (Vec<i32>, Vec<i32>) {
    let mut squash16 = vec![0i32; 4096];
    for i in 0..4096 {
        let d = (i as f64 - 2048.0) / 256.0;
        let v = 65536.0 / (1.0 + (-d).exp());
        let mut iv = (v + 0.5) as i32;
        if iv < 1 { iv = 1; }
        if iv > 65534 { iv = 65534; }
        squash16[i] = iv;
    }
    let mut stretch16 = vec![0i32; 65536];
    let mut pi = 0usize;
    let mut d = -2047i32;
    while d <= 2047 {
        let p = squash16_d(&squash16, d);
        while pi <= p as usize {
            stretch16[pi] = d;
            pi += 1;
        }
        d += 1;
    }
    while pi < 65536 {
        stretch16[pi] = 2047;
        pi += 1;
    }
    (stretch16, squash16)
}

#[inline]
pub fn squash16_d(squash16: &[i32], d: i32) -> i32 {
    if d >= 2047 { return 65534; }
    if d <= -2047 { return 1; }
    squash16[(d + 2048) as usize]
}
