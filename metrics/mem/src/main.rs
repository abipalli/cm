//! Full-scale reserved-memory meter. Runs the codec over the whole corpus under
//! the shared tracking allocator; reports MEM = peak live reserved bytes (the
//! largest single-file footprint, since each file uses a fresh model).

use cm_telemetry::Tracking;
use std::alloc::System;

#[global_allocator]
static ALLOC: Tracking<System> = Tracking(System);

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "corpus".to_string());
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("cannot read corpus dir: {dir}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "bin").unwrap_or(false))
        .collect();
    files.sort();
    for f in &files {
        let data = std::fs::read(f).expect("read corpus file");
        std::hint::black_box(cm::compress(&data));
    }
    println!(
        "MEM: {} (peak live reserved bytes over the full corpus; lower is leaner)",
        cm_telemetry::peak()
    );
}
