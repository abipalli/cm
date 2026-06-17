//! Deterministic complexity metric runner.
//!
//! Compresses each file in a (small, fixed) corpus and reports OPS — the count
//! of hot-path work units (mixer dot/update terms + context-model accesses)
//! accumulated by the `metrics`-feature counter. OPS is a bit-exact, machine-
//! independent function of the input and the algorithm, so it tracks algorithmic
//! complexity / compute-regressions far more reliably than a wall clock.
//!
//! Note: OPS measures *compute*, not cache/memory traffic, so it does not by
//! itself predict wall-time on this memory-bound codec — it tracks whether a
//! change adds or removes work.

#[cfg(feature = "metrics")]
use crate::algorithm::compress;
use crate::harness::corpus;

pub fn run(dir: &str) -> i32 {
    let entries = corpus::load(dir);
    if entries.is_empty() {
        eprintln!("no .bin files found in {dir}");
        return 2;
    }

    #[cfg(not(feature = "metrics"))]
    {
        eprintln!(
            "metrics counter is compiled out — rebuild with:\n  \
             cargo build --release --features metrics"
        );
        2
    }

    #[cfg(feature = "metrics")]
    {
        use crate::algorithm::metrics;
        let mut tot_bytes = 0u64;
        let mut tot_ops = 0u64;
        println!("{:<22}{:>12}{:>18}{:>11}", "file", "bytes", "ops", "ops/byte");
        for e in &entries {
            metrics::reset();
            let _ = compress(&e.data);
            let ops = metrics::get();
            let b = e.data.len() as u64;
            tot_bytes += b;
            tot_ops += ops;
            println!(
                "{:<22}{:>12}{:>18}{:>11}",
                e.name,
                b,
                ops,
                ops / b.max(1)
            );
        }
        println!("{}", "-".repeat(63));
        println!(
            "{:<22}{:>12}{:>18}{:>11}",
            "TOTAL",
            tot_bytes,
            tot_ops,
            tot_ops / tot_bytes.max(1)
        );
        println!("\nOPS: {} (deterministic work units; lower is faster)", tot_ops);
        0
    }
}
