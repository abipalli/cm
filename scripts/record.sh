#!/usr/bin/env bash
# Record a valid submission: append history entry + leaderboard row.
# FROZEN — do not edit as part of autoresearch.
#
# Usage:
#   bash scripts/record.sh --author @handle --model "codex 5.5" --note "what you changed and why"
#   bash scripts/record.sh --ci --author @handle --model "..." --note "..." --diff-base HEAD~1
#
# --ci: skip guard (GitHub Actions scorekeeper only). Never use locally to commit ledger.
set -euo pipefail
cd "$(dirname "$0")/.."

author=""
model=""
note=""
attempts=""
work=""
memcost=""
ci_mode=0
diff_base="HEAD"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --author) author="${2:-}"; shift 2 ;;
    --model) model="${2:-}"; shift 2 ;;
    --note) note="${2:-}"; shift 2 ;;
    --attempts) attempts="${2:-}"; shift 2 ;;
    --work) work="${2:-}"; shift 2 ;;
    --memcost) memcost="${2:-}"; shift 2 ;;
    --ci) ci_mode=1; shift ;;
    --diff-base) diff_base="${2:-}"; shift 2 ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *) echo "record.sh: unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$note" ]]; then
  echo "record.sh: --note is required (describe your approach)" >&2
  exit 2
fi

if (( ci_mode )) && [[ -z "$model" ]]; then
  echo "record.sh: --model is required in CI mode (name the AI model used)" >&2
  exit 2
fi

if [[ -z "$author" ]]; then
  author="$(git config --get github.user 2>/dev/null || true)"
fi
if [[ -z "$author" ]]; then
  author="$(git config user.name 2>/dev/null || echo unknown)"
fi

if (( ! ci_mode )); then
  echo "== boundary guard =="
  bash scripts/guard.sh
fi

if [[ ! -x ./target/release/cm ]]; then
  echo "== build =="
  cargo build --release --quiet
fi

echo "== score (for snapshot) =="
eval_out="$(./target/release/cm eval corpus 2>&1)" || {
  echo "$eval_out"
  echo "record.sh: eval failed — candidate is INVALID, not recorded" >&2
  exit 1
}
echo "$eval_out"

if echo "$eval_out" | grep -q 'SCORE: INVALID'; then
  echo "record.sh: lossless check failed — not recorded" >&2
  exit 1
fi

score="$(echo "$eval_out" | sed -n 's/^SCORE: \([0-9][0-9]*\).*/\1/p' | tail -1)"
if [[ -z "$score" ]]; then
  echo "record.sh: could not parse SCORE from eval output" >&2
  exit 1
fi

vs_zstd="$(echo "$eval_out" | sed -n 's/.*vs zstd -22 total:.*->  \([+-][0-9.]*%\).*/\1/p' | tail -1)"
[[ -z "$vs_zstd" ]] && vs_zstd="—"

commit="$(git rev-parse --short HEAD)"
commit_full="$(git rev-parse HEAD)"
git_name="$(git config user.name 2>/dev/null || echo unknown)"
git_email="$(git config user.email 2>/dev/null || echo unknown)"
date_iso="$(date +%Y-%m-%d)"

diff_stat="$(git diff --stat "$diff_base" HEAD -- src/algorithm/ 2>/dev/null || true)"
if [[ -z "$diff_stat" ]]; then
  diff_stat="(no algorithm diff between ${diff_base} and HEAD)"
fi

# Next entry number from existing files.
next=1
for f in history/entries/*.md; do
  [[ -e "$f" ]] || continue
  n="${f##*/}"
  n="${n%%-*}"
  n="${n#0}"
  n="${n#0}"
  n="${n#0}"
  if [[ "$n" =~ ^[0-9]+$ ]] && (( 10#$n >= next )); then
    next=$((10#$n + 1))
  fi
done
entry_id="$(printf '%04d' "$next")"

# Previous record = the row minimising (SCORE asc, then WORK asc): byte score is
# dominant, and on an exact SCORE tie the lowest WORK (deterministic complexity)
# holds the record. So the incumbent is the lowest-WORK row *among* the min-SCORE
# rows — not merely the first one — which matters once several entries share the
# record SCORE. A missing WORK counts as +infinity. We remember the incumbent's
# entry id to look up its WORK in the decision block below.
INF=9000000000000000000 # > any real WORK; stands in for "no WORK measured"
prev_score=""
prev_wv=""
prev_id=""
while IFS= read -r line; do
  # Data rows look like: | 0001 | date | @author | 642822 | ...
  case "$line" in
    "| "[0-9]*) ;;
    *) continue ;;
  esac
  s="$(echo "$line" | awk -F'|' '{gsub(/ /,"",$5); print $5}')"
  [[ "$s" =~ ^[0-9]+$ ]] || continue
  id="$(echo "$line" | awk -F'|' '{gsub(/ /,"",$2); print $2}')"
  # This row's WORK from its history entry (empty -> +infinity for ranking).
  rw=""
  rf="$(ls history/entries/${id}*.md 2>/dev/null | head -1 || true)"
  [[ -n "$rf" ]] && rw="$(sed -n 's/^| WORK | \([0-9][0-9]*\) |.*/\1/p' "$rf" | tail -1)"
  wv="${rw:-$INF}"
  if [[ -z "$prev_score" ]] || (( s < prev_score )) \
     || { (( s == prev_score )) && (( wv < prev_wv )); }; then
    prev_score="$s"
    prev_wv="$wv"
    prev_id="$id"
  fi
done < RESULTS.md

if [[ -n "$prev_score" ]]; then
  delta=$((score - prev_score))
  if (( delta < 0 )); then
    delta_str="${delta} (new record)"
    status="record"
  elif (( delta == 0 )); then
    # Exact byte-score tie: the lower-WORK (deterministic complexity) submission
    # takes the record. A missing incumbent WORK counts as +infinity (so a
    # measured challenger wins); a missing challenger WORK cannot claim the
    # tie-win (it stays a plain tie). Byte score is otherwise always dominant.
    prev_work=""
    if [[ -n "$prev_id" ]]; then
      prev_entry="$(ls history/entries/${prev_id}*.md 2>/dev/null | head -1 || true)"
      if [[ -n "$prev_entry" ]]; then
        prev_work="$(sed -n 's/^| WORK | \([0-9][0-9]*\) |.*/\1/p' "$prev_entry" | tail -1)"
      fi
    fi
    if [[ -n "$work" ]] && { [[ -z "$prev_work" ]] || (( work < prev_work )); }; then
      if [[ -n "$prev_work" ]]; then
        delta_str="0 bytes, -$((prev_work - work)) WORK (new record)"
      else
        delta_str="0 bytes, lower WORK (new record)"
      fi
      status="record"
    else
      delta_str="0 (tie)"
      status="attempt"
    fi
  else
    delta_str="+${delta}"
    status="attempt"
  fi
else
  delta_str="— (first entry)"
  status="record"
fi

slug="$(echo "$author" | tr '[:upper:]' '[:lower:]' | tr -cd 'a-z0-9@._-' | tr '@.' '-')"
slug="${slug:-unknown}"
entry_file="history/entries/${entry_id}-${slug}.md"

mkdir -p history/entries

{
  echo "# Entry ${entry_id} — SCORE ${score} (${delta_str})"
  echo
  echo "| Field | Value |"
  echo "|-------|-------|"
  echo "| Date | ${date_iso} |"
  echo "| Author | ${author} |"
  if [[ -n "$model" ]]; then
    echo "| Model | ${model} |"
  fi
  echo "| Git author | ${git_name} \<${git_email}\> |"
  echo "| Commit | \`${commit}\` (${commit_full}) |"
  echo "| SCORE | ${score} |"
  echo "| Δ vs previous record | ${delta_str} |"
  echo "| vs zstd -22 | ${vs_zstd} |"
  if [[ -n "$work" ]]; then
    echo "| WORK | ${work} |"
  fi
  if [[ -n "$memcost" ]]; then
    echo "| MEMCOST | ${memcost} |"
  fi
  echo "| Status | ${status} |"
  echo
  echo "## Approach"
  echo
  echo "$note"
  echo
  if [[ -n "$attempts" ]]; then
    echo "## Iteration notes"
    echo
    echo "$attempts"
    echo
  fi
  echo "## Algorithm changes"
  echo
  echo '```'
  echo "$diff_stat"
  echo '```'
  echo
  echo "## Eval snapshot"
  echo
  echo '```'
  echo "$eval_out"
  echo '```'
} > "$entry_file"

# Leaderboard row
short_note="$(echo "$note" | tr '\n' ' ' | sed 's/  */ /g' | cut -c1-80)"
if [[ ${#note} -gt 80 ]]; then
  short_note="${short_note}…"
fi

row="| ${entry_id} | ${date_iso} | ${author} | ${score} | ${delta_str} | ${vs_zstd} | \`${commit}\` | [${entry_id}](history/entries/${entry_id}-${slug}.md) | ${short_note} |"

# Insert the row immediately after the last existing table line (so it lands
# inside the markdown table, not after the footer text).
tmp_results="$(mktemp)"
awk -v row="$row" '
  { lines[NR] = $0; if ($0 ~ /^\|/) last = NR }
  END {
    for (i = 1; i <= NR; i++) {
      print lines[i]
      if (i == last) print row
    }
  }
' RESULTS.md > "$tmp_results" && mv "$tmp_results" RESULTS.md

if [[ "$status" == "record" ]]; then
  if grep -q '^\*\*Current record:' RESULTS.md; then
    sed -i.bak "s/^\*\*Current record:.*/\*\*Current record: ${score}\*\* (${author}, entry ${entry_id})/" RESULTS.md
    rm -f RESULTS.md.bak
  fi
fi

echo
echo "Recorded entry ${entry_id} (${status}): SCORE ${score}"
echo "  history: ${entry_file}"
echo "  leaderboard: RESULTS.md"
