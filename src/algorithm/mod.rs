//! Algorithm entry point.
//!
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ FROZEN CONTRACT — do NOT change these two signatures:                 │
//! │     pub fn compress(input: &[u8]) -> Vec<u8>                          │
//! │     pub fn decompress(input: &[u8]) -> Vec<u8>                        │
//! │ The bodies, and everything in this directory, are yours to improve.  │
//! │ Invariant: decompress(compress(x)) == x for EVERY possible x.        │
//! └─────────────────────────────────────────────────────────────────────┘

mod coder;
mod ctw;
mod dmc;
mod model;
mod tables;

use coder::{Decoder, Encoder};
use model::Cm;

/// Compress `input` into a self-describing byte stream.
pub fn compress(input: &[u8]) -> Vec<u8> {
    let n = input.len();
    let mut data = input.to_vec();
    let flag = want_e8e9(&data);
    if flag {
        e8e9(&mut data, true);
    }

    let mut out = Vec::with_capacity(n / 2 + 16);
    out.extend_from_slice(&(n as u64).to_le_bytes());
    out.push(flag as u8);

    let mut cm = Cm::new(n.min((1 << 27) - 1));
    cm.byte_start();
    let mut enc = Encoder::new();
    for &ch in &data {
        for b in (0..8).rev() {
            let bit = ((ch >> b) & 1) as i32;
            let p = cm.predict();
            enc.encode(p, bit);
            cm.update(bit, p);
        }
    }
    out.extend_from_slice(&enc.finish());
    out
}

/// Decompress a stream produced by `compress`.
pub fn decompress(input: &[u8]) -> Vec<u8> {
    if input.len() < 9 {
        return Vec::new();
    }
    let mut lenb = [0u8; 8];
    lenb.copy_from_slice(&input[0..8]);
    let n = u64::from_le_bytes(lenb) as usize;
    let flag = input[8] != 0;

    let mut cm = Cm::new(n.min((1 << 27) - 1));
    cm.byte_start();
    let mut dec = Decoder::new(&input[9..]);
    let mut data = vec![0u8; n];
    for k in 0..n {
        let mut byte = 0i32;
        for _ in 0..8 {
            let p = cm.predict();
            let bit = dec.decode(p);
            cm.update(bit, p);
            byte = (byte << 1) | bit;
        }
        data[k] = byte as u8;
    }
    if flag {
        e8e9(&mut data, false);
    }
    data
}

/// Reversible x86 BCJ filter: rewrite E8/E9 relative operands to/from absolute.
fn e8e9(b: &mut [u8], enc: bool) {
    let n = b.len();
    if n < 5 {
        return;
    }
    let mut i = 0usize;
    while i + 4 < n {
        if b[i] == 0xE8 || b[i] == 0xE9 {
            let v = b[i + 1] as i32
                | (b[i + 2] as i32) << 8
                | (b[i + 3] as i32) << 16
                | (b[i + 4] as i32) << 24;
            let p = (i as i32) + 1 + 4;
            let nv = if enc { v.wrapping_add(p) } else { v.wrapping_sub(p) };
            b[i + 1] = nv as u8;
            b[i + 2] = (nv >> 8) as u8;
            b[i + 3] = (nv >> 16) as u8;
            b[i + 4] = (nv >> 24) as u8;
            i += 5;
        } else {
            i += 1;
        }
    }
}

fn want_e8e9(b: &[u8]) -> bool {
    let n = b.len();
    if n >= 4 && b[0] == b'M' && b[1] == b'Z' {
        return true;
    }
    if n >= 4 && b[0] == 0x7f && b[1] == b'E' && b[2] == b'L' && b[3] == b'F' {
        return true;
    }
    let lim = n.min(1 << 20);
    let mut cnt = 0usize;
    for i in 0..lim {
        if b[i] == 0xE8 || b[i] == 0xE9 {
            cnt += 1;
        }
    }
    cnt * 200 > lim
}
