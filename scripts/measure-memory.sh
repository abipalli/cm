#!/usr/bin/env bash
# Peak-memory metric (lower = leaner) — REFERENCE/STOPGAP for the scoring RFC in
# docs/proposals/0001-complexity-and-memory-scoring.md.
#
# Reports MEMORY: the maximum resident set size (bytes) observed while compressing
# each corpus file, taken as the max over files (each file compresses with a fresh
# model, so the peak is the largest single-file run). This is the resource the
# current WORK (wasm fuel) metric does NOT capture: a model can be cheap in
# executed-operator count yet allocate gigabytes of context tables. The recent
# top-of-leaderboard solutions peak at multiple GB for a ~384 KB file, which is
# the cost this metric makes visible.
#
# NOTE on determinism: native RSS depends on the allocator, page size and OS, so
# this is a quick, human-meaningful stopgap, NOT a cross-machine-reproducible
# ranking input. The RFC recommends the durable version — have the existing wasm
# fuel meter (metrics/) also report peak wasm linear-memory pages, which is
# deterministic and tamper-proof exactly like WORK. See the proposal.
#
# Lives outside src/algorithm/ so a submission cannot tamper with the measurement.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ ! -x ./target/release/cm ]]; then
  echo "== build ==" >&2
  cargo build --release --quiet
fi

# Pick the platform's peak-RSS reporter.
peak_rss_bytes() { # usage: peak_rss_bytes <infile>
  local infile="$1" line kb
  case "$(uname -s)" in
    Linux)
      # GNU time: "Maximum resident set size (kbytes): N"
      line="$(/usr/bin/time -v ./target/release/cm c "$infile" /dev/null 2>&1 1>/dev/null \
              | grep -i 'maximum resident set size' || true)"
      kb="$(printf '%s\n' "$line" | grep -oE '[0-9]+' | tail -1)"
      [[ -n "$kb" ]] && echo $(( kb * 1024 )) || echo 0
      ;;
    Darwin)
      # BSD time -l: "  N  maximum resident set size" (already in bytes)
      line="$(/usr/bin/time -l ./target/release/cm c "$infile" /dev/null 2>&1 1>/dev/null \
              | grep -i 'maximum resident set size' || true)"
      printf '%s\n' "$line" | grep -oE '[0-9]+' | head -1
      ;;
    *) echo "measure-memory: unsupported OS $(uname -s)" >&2; exit 2 ;;
  esac
}

max=0
for f in corpus/*.bin; do
  [[ -e "$f" ]] || continue
  b="$(peak_rss_bytes "$f")"
  printf '  %-24s %10d bytes\n' "${f##*/}" "${b:-0}" >&2
  (( ${b:-0} > max )) && max="$b"
done

echo "MEMORY: ${max} (peak resident bytes; lower is leaner)"
