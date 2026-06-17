#!/usr/bin/env bash
# Deterministic full-scale reserved-memory metric (lower = leaner). Builds the
# tracking-allocator meter (metrics/mem) and prints MEM: peak live requested bytes
# while the real codec compresses the full corpus. See docs/proposals/0001-*.md.
# FROZEN — not part of the editable algorithm surface.
set -euo pipefail
cd "$(dirname "$0")/.."

( cd metrics/mem && cargo build --release --quiet )
./metrics/mem/target/release/cm-mem-meter corpus
