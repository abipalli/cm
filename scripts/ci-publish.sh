#!/usr/bin/env bash
# Scorekeeper — PUBLISH phase. FROZEN — do not edit as part of autoresearch.
#
# This phase holds the privileged push token (SCOREKEEPER_PAT) but NEVER builds
# or runs competitor code. It only applies the ledger files produced by the
# score phase and pushes them to main. Because no untrusted code runs here, the
# token cannot be exfiltrated by a malicious submission.
set -euo pipefail
cd "$(dirname "$0")/.."

IN_DIR="${IN_DIR:-ledger-in}"
if [[ ! -f "$IN_DIR/meta.env" ]]; then
  echo "publish: no ledger artifact; nothing to do"
  exit 0
fi

# shellcheck disable=SC1090,SC1091
source "$IN_DIR/meta.env"
if [[ "${RECORD:-0}" != "1" ]]; then
  echo "publish: score phase recorded nothing; nothing to publish"
  exit 0
fi

# Validate the handoff before trusting any of it (the score phase shares a
# filesystem with untrusted code; constrain what we will commit).
if [[ ! "${ENTRY_ID:-}" =~ ^[0-9]{4}$ ]]; then
  echo "publish: bad ENTRY_ID '${ENTRY_ID:-}'" >&2
  exit 1
fi
case "${ENTRY_FILE:-}" in
  ""|*..*|*/*) echo "publish: bad ENTRY_FILE '${ENTRY_FILE:-}'" >&2; exit 1 ;;
esac
if [[ ! -f "$IN_DIR/RESULTS.md" || ! -f "$IN_DIR/entries/$ENTRY_FILE" ]]; then
  echo "publish: missing ledger files in artifact" >&2
  exit 1
fi

# Apply only the ledger paths onto the current (fresh) checkout of main.
cp "$IN_DIR/RESULTS.md" RESULTS.md
mkdir -p history/entries
cp "$IN_DIR/entries/$ENTRY_FILE" "history/entries/$ENTRY_FILE"

git add RESULTS.md "history/entries/$ENTRY_FILE"
if git diff --staged --quiet; then
  echo "publish: no ledger changes to commit"
  exit 0
fi

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git commit -m "$(cat <<EOF
ci: record submission ${ENTRY_ID} [skip ci]

Authoritative ledger update from verified evaluate on main.
EOF
)"
git push origin HEAD:main

echo "publish: ledger committed and pushed (entry ${ENTRY_ID})"
