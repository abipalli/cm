//! CLI. FROZEN — do not edit as part of autoresearch.
//!
//!   cm c <in> <out>     compress
//!   cm d <in> <out>     decompress
//!   cm eval [dir]       score against the corpus (default: ./corpus)

use std::process::exit;

use cm::algorithm::{compress, decompress};
use cm::harness::eval;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
    }
    match args[1].as_str() {
        "c" | "d" if args.len() == 4 => {
            let input = std::fs::read(&args[2]).expect("read input");
            let out = if args[1] == "c" {
                compress(&input)
            } else {
                decompress(&input)
            };
            std::fs::write(&args[3], out).expect("write output");
        }
        "eval" => {
            let dir = args.get(2).map(|s| s.as_str()).unwrap_or("corpus");
            exit(eval::run(dir));
        }
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!("usage:\n  cm c <in> <out>\n  cm d <in> <out>\n  cm eval [corpus_dir]");
    exit(2);
}
