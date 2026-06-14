#!/usr/bin/env bash
# Boundary guard: fail if anything outside src/algorithm/ (or RESULTS.md) was
# changed relative to the committed baseline, or if the frozen contract
# signatures were altered. FROZEN — do not edit as part of autoresearch.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! git rev-parse --git-dir >/dev/null 2>&1; then
  echo "guard: not a git repo; run 'git init && git add -A && git commit -m base' first" >&2
  exit 2
fi

mapfile -t changed < <( { git diff --name-only HEAD; git ls-files --others --exclude-standard; } | sort -u )

violations=()
for f in "${changed[@]}"; do
  [[ -z "$f" ]] && continue
  case "$f" in
    src/algorithm/*) ;;
    RESULTS.md) ;;
    *) violations+=("$f") ;;
  esac
done

if (( ${#violations[@]} )); then
  echo "BOUNDARY VIOLATION — these frozen files were modified:"
  printf '  %s\n' "${violations[@]}"
  echo "Only src/algorithm/ (and RESULTS.md) may change."
  exit 1
fi

# Frozen contract signatures must remain intact.
if ! grep -q 'pub fn compress(input: &\[u8\]) -> Vec<u8>' src/algorithm/mod.rs \
  || ! grep -q 'pub fn decompress(input: &\[u8\]) -> Vec<u8>' src/algorithm/mod.rs; then
  echo "BOUNDARY VIOLATION — frozen compress/decompress signatures were changed."
  exit 1
fi

echo "boundary OK (only src/algorithm/ changed; contract intact)"
