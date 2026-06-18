#!/usr/bin/env bash
# Deterministic distinct-cache-lines metric (LINES), companion to MEMCOST: same
# high-entropy input + init-free differencing, but counts distinct 64B lines
# touched (associativity-free) instead of a modeled cache penalty. FROZEN.
set -euo pipefail
cd "$(dirname "$0")/.."
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
( cd metrics/wasm && RUSTFLAGS="" cargo build --release --quiet --target wasm32-unknown-unknown )
( cd metrics/lines && cargo build --release --quiet )
WASM=metrics/wasm/target/wasm32-unknown-unknown/release/cm_wasm_meter.wasm
./metrics/lines/target/release/cm-lines-meter "$WASM"
