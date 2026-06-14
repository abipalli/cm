//! Corpus + baseline loading. FROZEN — do not edit as part of autoresearch.

use std::fs;
use std::path::Path;

pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
    pub zstd22: Option<u64>,
    pub xz9e: Option<u64>,
}

/// Load every `*.bin` file in `dir`, sorted by name, attaching baseline sizes
/// from `dir/baselines.tsv` when present.
pub fn load(dir: &str) -> Vec<Entry> {
    let baselines = load_baselines(dir);
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|_| panic!("cannot read corpus dir {dir}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "bin").unwrap_or(false))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let data = fs::read(&p).unwrap();
            let (zstd22, xz9e) = baselines.get(&name).copied().unwrap_or((None, None));
            Entry { name, data, zstd22, xz9e }
        })
        .collect()
}

fn load_baselines(dir: &str) -> std::collections::HashMap<String, (Option<u64>, Option<u64>)> {
    let mut m = std::collections::HashMap::new();
    let path = Path::new(dir).join("baselines.tsv");
    if let Ok(text) = fs::read_to_string(path) {
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() >= 4 {
                let z = f[2].parse::<u64>().ok();
                let x = f[3].parse::<u64>().ok();
                m.insert(f[0].to_string(), (z, x));
            }
        }
    }
    m
}
