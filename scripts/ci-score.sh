#!/usr/bin/env bash
# Scorekeeper — SCORE phase. FROZEN — do not edit as part of autoresearch.
#
# This phase BUILDS AND RUNS the (untrusted) merged competitor code to compute
# the authoritative score and generate the ledger files. It therefore runs with
# a read-only token and NO privileged secret — so a malicious submission cannot
# exfiltrate a push-capable credential. The generated ledger files are emitted
# to $OUT_DIR and handed to the separate (privileged) publish phase.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT_DIR="${OUT_DIR:-ledger-out}"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"
printf 'RECORD=0\n' > "$OUT_DIR/meta.env"

commit_msg="${GITHUB_EVENT_HEAD_COMMIT_MESSAGE:-$(git log -1 --format=%B)}"
if [[ "$commit_msg" == *"[skip ci]"* ]]; then
  echo "scorekeeper: skipping bot ledger commit"
  exit 0
fi

if ! git rev-parse HEAD~1 >/dev/null 2>&1; then
  echo "scorekeeper: no parent commit; nothing to compare"
  exit 0
fi

algo_changed="$(git diff --name-only HEAD~1 HEAD -- src/algorithm/ || true)"
ledger_changed="$(git diff --name-only HEAD~1 HEAD -- RESULTS.md history/entries/ || true)"

if [[ -n "$ledger_changed" && -z "$algo_changed" ]]; then
  echo "INTEGRITY VIOLATION: RESULTS.md or history/entries/ changed without an algorithm update." >&2
  echo "Only CI may update the ledger (commits tagged [skip ci])." >&2
  exit 1
fi
if [[ -n "$ledger_changed" && -n "$algo_changed" ]]; then
  echo "INTEGRITY VIOLATION: do not commit RESULTS.md or history/entries/ in your PR." >&2
  echo "CI records the verified score after merge." >&2
  exit 1
fi
if [[ -z "$algo_changed" ]]; then
  echo "scorekeeper: no algorithm changes on main; nothing to record"
  exit 0
fi

echo "== algorithm changed =="
printf '  %s\n' $algo_changed

echo "== evaluate (authoritative score; runs untrusted competitor code) =="
bash scripts/evaluate.sh --no-guard

# Deterministic complexity metric (best-effort; never blocks recording). Runs the
# merged code through the wasm fuel meter that lives outside src/algorithm/.
work=""
echo "== complexity metric (wasm fuel) =="
if work_out="$(bash scripts/measure-complexity.sh 2>&1)"; then
  echo "$work_out"
  work="$(printf '%s\n' "$work_out" | sed -n 's/^WORK: \([0-9][0-9]*\).*/\1/p' | tail -1)"
else
  echo "scorekeeper: complexity metric unavailable; recording without WORK"
fi

# Memory-traffic metric (best-effort; never blocks recording). Instruments the
# wasm loads/stores and runs the deterministic access trace through a fixed cache
# model — captures the cache/latency cost that WORK (operator count) cannot see.
memcost=""
echo "== memory-traffic metric (wasm cache model) =="
if mem_out="$(bash scripts/measure-memcost.sh 2>&1)"; then
  echo "$mem_out"
  memcost="$(printf '%s\n' "$mem_out" | sed -n 's/^MEMCOST: \([0-9][0-9]*\).*/\1/p' | tail -1)"
else
  echo "scorekeeper: memory-traffic metric unavailable; recording without MEMCOST"
fi

# Distinct-cache-lines metric (best-effort; never blocks recording). Companion to
# MEMCOST: counts distinct 64B lines touched on the same init-free differencing.
lines=""
echo "== distinct-cache-lines metric (LINES) =="
if out=$(bash scripts/measure-lines.sh 2>&1); then
  echo "$out"
  lines="$(printf '%s\n' "$out" | sed -n 's/^LINES: \([0-9]*\).*/\1/p' | tail -1)"
else
  echo "scorekeeper: LINES unavailable; recording without LINES"
fi

# Peak reserved-heap metric (best-effort; never blocks recording). Full-corpus
# peak live reserved heap under a tracking allocator.
heap_peak=""
echo "== peak reserved-heap metric (HEAP_PEAK) =="
if out=$(bash scripts/measure-heappeak.sh 2>&1); then
  echo "$out"
  heap_peak="$(printf '%s\n' "$out" | sed -n 's/^HEAP_PEAK: \([0-9]*\).*/\1/p' | tail -1)"
else
  echo "scorekeeper: HEAP_PEAK unavailable; recording without HEAP_PEAK"
fi

# Init-free heap-churn metric (best-effort; never blocks recording). Differences
# requested heap bytes between FULL and HALF prefixes via the heap-feature shim.
heap_churn=""
echo "== init-free heap-churn metric (HEAP_CHURN) =="
if out=$(bash scripts/measure-heapchurn.sh 2>&1); then
  echo "$out"
  heap_churn="$(printf '%s\n' "$out" | sed -n 's/^HEAP_CHURN: \([0-9]*\).*/\1/p' | tail -1)"
else
  echo "scorekeeper: HEAP_CHURN unavailable; recording without HEAP_CHURN"
fi

# PR metadata. The token here is the read-only default GITHUB_TOKEN.
author="@${GITHUB_ACTOR:-unknown}"
model=""
note=""
attempts=""
pr_body=""
if [[ -n "${GITHUB_REPOSITORY:-}" && -n "${GITHUB_SHA:-}" ]]; then
  pr_body="$(gh api "repos/${GITHUB_REPOSITORY}/commits/${GITHUB_SHA}/pulls" \
    --jq '.[0].body // empty' 2>/dev/null || true)"
  pr_author="$(gh api "repos/${GITHUB_REPOSITORY}/commits/${GITHUB_SHA}/pulls" \
    --jq '.[0].user.login // empty' 2>/dev/null || true)"
  [[ -n "$pr_author" ]] && author="@${pr_author}"
fi
if [[ -n "$pr_body" ]]; then
  model="$(bash scripts/ci-parse-pr-body.sh Model "$pr_body" || true)"
  note="$(bash scripts/ci-parse-pr-body.sh Approach "$pr_body" || true)"
  attempts="$(bash scripts/ci-parse-pr-body.sh "Iteration notes" "$pr_body" || true)"
fi
if [[ -z "$model" ]]; then
  echo "scorekeeper: missing required ## Model section in PR description" >&2
  exit 1
fi
[[ -z "$note" ]] && note="$(git log -1 --format=%B | sed '/^$/d' | head -5)"
[[ -z "$note" ]] && note="Algorithm update merged to main (no PR description captured)."

record_args=(--ci --author "$author" --model "$model" --note "$note" --diff-base HEAD~1)
[[ -n "$attempts" ]] && record_args+=(--attempts "$attempts")
[[ -n "$work" ]] && record_args+=(--work "$work")
[[ -n "$memcost" ]] && record_args+=(--memcost "$memcost")
[[ -n "$lines" ]] && record_args+=(--lines "$lines")
[[ -n "$heap_peak" ]] && record_args+=(--heap-peak "$heap_peak")
[[ -n "$heap_churn" ]] && record_args+=(--heap-churn "$heap_churn")

echo "== record submission (generate ledger files) =="
rec_out="$(bash scripts/record.sh "${record_args[@]}")"
echo "$rec_out"

entry_file="$(printf '%s\n' "$rec_out" | sed -n 's/^  history: //p' | tail -1)"
if [[ -z "$entry_file" || ! -f "$entry_file" ]]; then
  echo "scorekeeper: record.sh produced no entry; nothing to publish" >&2
  exit 1
fi
entry_base="${entry_file##*/}"
entry_id="${entry_base%%-*}"

# Stage outputs for the privileged publish phase (only the ledger paths).
cp RESULTS.md "$OUT_DIR/RESULTS.md"
mkdir -p "$OUT_DIR/entries"
cp "$entry_file" "$OUT_DIR/entries/$entry_base"
cat > "$OUT_DIR/meta.env" <<EOF
RECORD=1
ENTRY_ID=$entry_id
ENTRY_FILE=$entry_base
EOF

echo "scorekeeper(score): prepared entry $entry_id for publish"
