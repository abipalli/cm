#!/usr/bin/env bash
# Evaluate one candidate: boundary guard -> correctness gate -> score.
# FROZEN — do not edit as part of autoresearch.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$PATH:/usr/bin"

echo "== boundary guard =="
bash scripts/guard.sh

echo "== correctness gate (round-trip tests) =="
cargo test --release --quiet 2>&1 | tail -n 5

echo "== build =="
cargo build --release --quiet

echo "== score =="
./target/release/cm eval corpus
