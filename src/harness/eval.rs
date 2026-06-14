//! Evaluation + scoring. FROZEN — do not edit as part of autoresearch.
//!
//! Score = total compressed bytes over the corpus (LOWER IS BETTER),
//! gated on exact lossless round-trip for every file. Any round-trip failure
//! makes the candidate INVALID regardless of size.

use crate::algorithm::{compress, decompress};
use crate::harness::corpus;

pub fn run(dir: &str) -> i32 {
    let entries = corpus::load(dir);
    if entries.is_empty() {
        eprintln!("no corpus files found in {dir}");
        return 2;
    }

    let mut tot_orig = 0u64;
    let mut tot_ours = 0u64;
    let mut tot_zstd = 0u64;
    let mut tot_xz = 0u64;
    let mut all_lossless = true;
    let mut have_zstd = true;
    let mut have_xz = true;

    println!(
        "{:<22}{:>10}{:>10}{:>9}{:>9}{:>9}  {}",
        "file", "orig", "ours", "ratio", "vs zstd", "vs xz", "lossless"
    );
    for e in &entries {
        let c = compress(&e.data);
        let d = decompress(&c);
        let lossless = d == e.data;
        if !lossless {
            all_lossless = false;
        }
        let ours = c.len() as u64;
        let orig = e.data.len() as u64;
        tot_orig += orig;
        tot_ours += ours;

        let ratio = orig as f64 / ours as f64;
        let vs_zstd = match e.zstd22 {
            Some(z) => {
                tot_zstd += z;
                format!("{:+.1}%", 100.0 * (z as f64 - ours as f64) / z as f64)
            }
            None => {
                have_zstd = false;
                "-".to_string()
            }
        };
        let vs_xz = match e.xz9e {
            Some(x) => {
                tot_xz += x;
                format!("{:+.1}%", 100.0 * (x as f64 - ours as f64) / x as f64)
            }
            None => {
                have_xz = false;
                "-".to_string()
            }
        };
        println!(
            "{:<22}{:>10}{:>10}{:>9.3}{:>9}{:>9}  {}",
            e.name,
            orig,
            ours,
            ratio,
            vs_zstd,
            vs_xz,
            if lossless { "OK" } else { "FAIL!" }
        );
    }

    println!("{}", "-".repeat(80));
    let overall = tot_orig as f64 / tot_ours as f64;
    println!(
        "{:<22}{:>10}{:>10}{:>9.3}",
        "TOTAL", tot_orig, tot_ours, overall
    );
    if have_zstd && tot_zstd > 0 {
        println!(
            "  vs zstd -22 total: {} bytes  ->  {:+.2}% ({})",
            tot_zstd,
            100.0 * (tot_zstd as f64 - tot_ours as f64) / tot_zstd as f64,
            if tot_ours < tot_zstd { "smaller, WIN" } else { "larger" }
        );
    }
    if have_xz && tot_xz > 0 {
        println!(
            "  vs xz -9e   total: {} bytes  ->  {:+.2}% ({})",
            tot_xz,
            100.0 * (tot_xz as f64 - tot_ours as f64) / tot_xz as f64,
            if tot_ours < tot_xz { "smaller, WIN" } else { "larger" }
        );
    }

    if !all_lossless {
        println!("\nSCORE: INVALID (lossless round-trip failed on at least one file)");
        return 1;
    }
    println!("\nSCORE: {} (total compressed bytes; lower is better)", tot_ours);
    0
}
