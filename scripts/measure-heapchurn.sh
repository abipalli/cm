#!/usr/bin/env bash
# Deterministic init-free heap-churn metric (HEAP_CHURN). Builds the shim WITH
# the heap-tracking feature (a SEPARATE build, so the WORK/MEMCOST shim bytes
# are unaffected) and differences requested heap bytes. Non-scoring. FROZEN.
set -euo pipefail
cd "$(dirname "$0")/.."
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
( cd metrics/wasm && RUSTFLAGS="" cargo build --release --quiet --target wasm32-unknown-unknown --features heap )
( cd metrics/heapchurn && cargo build --release --quiet )
WASM=metrics/wasm/target/wasm32-unknown-unknown/release/cm_wasm_meter.wasm
./metrics/heapchurn/target/release/cm-heapchurn-meter "$WASM"
