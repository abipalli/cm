#!/usr/bin/env bash
# Deterministic, tamper-proof memory-traffic metric (lower = friendlier to cache).
#
# Companion to measure-complexity.sh (WORK). Where WORK counts executed wasm
# operators — blind to cache behaviour — this instruments the wasm shim's
# loads/stores and runs the deterministic access trace through a fixed cache
# model, printing MEMCOST: the init-free weighted cache-miss penalty. Both the
# shim and the meter live OUTSIDE src/algorithm/, so a submission cannot alter
# the measurement; the wasm is built for the fixed wasm32 target, so the number
# is reproducible across machines given a pinned toolchain + walrus/wasmtime.
#
# FROZEN — not part of the editable algorithm surface.
set -euo pipefail
cd "$(dirname "$0")/.."

rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
( cd metrics/wasm && RUSTFLAGS="" cargo build --release --quiet --target wasm32-unknown-unknown )
( cd metrics/memmeter && cargo build --release --quiet )

WASM=metrics/wasm/target/wasm32-unknown-unknown/release/cm_wasm_meter.wasm
./metrics/memmeter/target/release/cm-mem-meter "$WASM"
