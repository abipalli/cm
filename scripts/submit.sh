#!/usr/bin/env bash
# Submit a candidate: evaluate locally, ensure gh auth, push the branch, open a
# PR, and wait for CI to verify + auto-merge it to main.
#
# This wraps the contribution flow described in CONTRIBUTING.md so contributors
# don't have to remember the gh login, the required ## Model / ## Approach PR
# sections, or how merges land. The algorithm itself is graded by CI on GitHub —
# the local score here is just a pre-flight check.
#
# Usage:
#   bash scripts/submit.sh [options]
#
# Options:
#   --model   <name>   AI model used (e.g. "opus 4.8"). REQUIRED by Verify CI.
#                      Falls back to $CM_MODEL, then an interactive prompt.
#   --title   <text>   PR title. Defaults to the commit subject (or branch name).
#   --approach <text>  ## Approach body. Defaults to the commit message(s).
#   --notes   <text>   ## Iteration notes body. Optional.
#   --commit  <msg>    If the working tree has uncommitted src/algorithm/ changes,
#                      commit them with this message first.
#   --no-wait          Create the PR and exit without waiting for CI to merge.
#   --yes              Don't prompt for confirmation before pushing/creating.
#   -h, --help         Show this help.
set -euo pipefail
cd "$(dirname "$0")/.."

# ---- args ----------------------------------------------------------------
MODEL="${CM_MODEL:-}"
TITLE=""
APPROACH=""
NOTES=""
COMMIT_MSG=""
WAIT=1
ASSUME_YES=0

die()  { echo "submit: $*" >&2; exit 1; }
info() { echo "==> $*"; }

usage() { awk 'NR==1{next} /^#/{sub(/^# ?/,""); print; next} {exit}' "$0"; exit 0; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --model)    MODEL="${2:?--model needs a value}"; shift 2;;
    --title)    TITLE="${2:?--title needs a value}"; shift 2;;
    --approach) APPROACH="${2:?--approach needs a value}"; shift 2;;
    --notes)    NOTES="${2:?--notes needs a value}"; shift 2;;
    --commit)   COMMIT_MSG="${2:?--commit needs a value}"; shift 2;;
    --no-wait)  WAIT=0; shift;;
    --yes|-y)   ASSUME_YES=1; shift;;
    -h|--help)  usage;;
    *) die "unknown option: $1 (try --help)";;
  esac
done

confirm() {
  # confirm "prompt" -> returns 0 to proceed
  [[ "$ASSUME_YES" == 1 ]] && return 0
  [[ -t 0 ]] || die "non-interactive and --yes not given; refusing to $1"
  local reply
  read -r -p "$1 [y/N] " reply
  [[ "$reply" =~ ^[Yy]$ ]]
}

# ---- preconditions -------------------------------------------------------
command -v git >/dev/null || die "git not found"
command -v gh  >/dev/null || die "GitHub CLI (gh) not found — install from https://cli.github.com"
git rev-parse --git-dir >/dev/null 2>&1 || die "not a git repository"

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
case "$BRANCH" in
  main|master|HEAD)
    die "you are on '$BRANCH'. Create a feature branch first: git checkout -b improve/<name>";;
esac

# ---- gh auth -------------------------------------------------------------
info "checking GitHub authentication"
if ! gh auth status >/dev/null 2>&1; then
  echo "Not logged in to GitHub."
  if [[ -t 0 ]]; then
    info "launching 'gh auth login'"
    gh auth login || die "gh auth login failed"
    gh auth status >/dev/null 2>&1 || die "still not authenticated after login"
  else
    die "run 'gh auth login' first (no TTY to do it here)"
  fi
fi

# ---- commit any pending algorithm changes --------------------------------
# guard.sh (run by evaluate.sh) allows uncommitted src/algorithm/ edits, but
# those would not be in the pushed commit. Make sure the tree is committed.
if ! git diff --quiet || ! git diff --cached --quiet || \
   [[ -n "$(git ls-files --others --exclude-standard)" ]]; then
  # Anything outside src/algorithm/ is a boundary violation — fail early with a
  # clearer message than the CI guard would give.
  outside="$( { git diff --name-only HEAD; git ls-files --others --exclude-standard; } \
    | sort -u | grep -v '^src/algorithm/' || true)"
  [[ -n "$outside" ]] && die $'uncommitted changes outside src/algorithm/ (PRs may only touch the algorithm):\n'"$outside"

  echo "Uncommitted changes in src/algorithm/:"
  git status --short -- src/algorithm/
  if [[ -z "$COMMIT_MSG" ]]; then
    if [[ -t 0 ]]; then
      read -r -p "Commit message: " COMMIT_MSG
    fi
    [[ -n "$COMMIT_MSG" ]] || die "uncommitted changes; pass --commit \"<msg>\" or commit them yourself"
  fi
  info "committing src/algorithm/ changes"
  git add -A -- src/algorithm/
  git commit -m "$COMMIT_MSG"
fi

# Need something to submit.
git rev-parse --verify -q origin/main >/dev/null 2>&1 || git fetch origin main --quiet
if [[ -z "$(git rev-list origin/main..HEAD)" ]]; then
  die "no commits ahead of origin/main on '$BRANCH' — nothing to submit"
fi

# ---- local evaluation ----------------------------------------------------
info "evaluating candidate locally (guard + tests + score)"
eval_out="$(mktemp)"
if ! bash scripts/evaluate.sh | tee "$eval_out"; then
  die "local evaluation failed — fix before submitting"
fi
SCORE="$(grep -oE 'SCORE:[[:space:]]*[0-9]+' "$eval_out" | grep -oE '[0-9]+' | tail -1 || true)"
[[ -n "$SCORE" ]] && info "local SCORE: $SCORE (CI will recompute the authoritative score)"

# ---- gather PR metadata --------------------------------------------------
if [[ -z "$MODEL" ]]; then
  if [[ -t 0 ]]; then
    read -r -p "AI model used (e.g. 'opus 4.8', 'codex 5.5'): " MODEL
  fi
  [[ -n "$MODEL" ]] || die "a model is required (Verify CI rejects PRs without a ## Model section)"
fi

# Defaults derived from the commits being submitted.
commit_subjects="$(git log --reverse --format='%s' origin/main..HEAD)"
commit_bodies="$(git log --reverse --format='%s%n%b' origin/main..HEAD)"
[[ -z "$TITLE" ]]    && TITLE="$(echo "$commit_subjects" | head -1)"
[[ -z "$TITLE" ]]    && TITLE="$BRANCH"
[[ -z "$APPROACH" ]] && APPROACH="$commit_bodies"

# ---- build PR body (Model + Approach are what CI/history need) -----------
body_file="$(mktemp)"
{
  echo "## Model"
  echo
  echo "$MODEL"
  echo
  echo "## Approach"
  echo
  echo "$APPROACH"
  if [[ -n "$NOTES" ]]; then
    echo
    echo "## Iteration notes"
    echo
    echo "$NOTES"
  fi
  echo
  echo "## Validation"
  echo
  echo '`bash scripts/evaluate.sh` passed locally; only `src/algorithm/` changed.'
  [[ -n "$SCORE" ]] && echo "Local SCORE: \`$SCORE\` (CI recomputes the trusted score)."
} > "$body_file"

echo
echo "Branch:  $BRANCH"
echo "Title:   $TITLE"
echo "Model:   $MODEL"
echo "----- PR body -----"; cat "$body_file"; echo "-------------------"
confirm "push '$BRANCH' and open a PR?" || die "aborted"

# ---- push ----------------------------------------------------------------
info "pushing $BRANCH"
git push -u origin "$BRANCH"

# ---- create or reuse PR --------------------------------------------------
PR="$(gh pr list --head "$BRANCH" --state open --json number --jq '.[0].number // empty')"
if [[ -n "$PR" ]]; then
  info "updating existing PR #$PR"
  gh pr edit "$PR" --title "$TITLE" --body-file "$body_file" >/dev/null
  # A body edit alone does not re-run Verify (it reads the event payload), so
  # bounce the PR to fire a fresh pull_request event with the new body.
  gh pr close "$PR" >/dev/null && gh pr reopen "$PR" >/dev/null
else
  url="$(gh pr create --base main --head "$BRANCH" --title "$TITLE" --body-file "$body_file")"
  PR="$(basename "$url")"
  info "opened PR #$PR — $url"
fi

if [[ "$WAIT" == 0 ]]; then
  info "PR #$PR created; --no-wait set, not waiting for CI."
  exit 0
fi

# ---- wait for Verify, then for Auto-merge to land it ---------------------
# gh pr checks --watch exits immediately with "no checks reported" if the
# workflow has not registered yet, so first poll until a check appears.
info "waiting for Verify CI to start on PR #$PR"
checks_deadline=$(( $(date +%s) + 120 ))
until gh pr checks "$PR" >/dev/null 2>&1; do
  [[ "$(date +%s)" -gt "$checks_deadline" ]] && die "no CI checks appeared for PR #$PR after 2m"
  sleep 5
done
info "watching Verify CI on PR #$PR"
if ! gh pr checks "$PR" --watch --fail-fast; then
  echo
  echo "Verify failed. Recent logs:" >&2
  gh pr checks "$PR" >&2 || true
  die "CI verification failed for PR #$PR — see the run above"
fi
info "Verify passed; waiting for Auto-merge to land it on main"

# Auto-merge runs as a separate workflow_run, so poll PR state until MERGED.
deadline=$(( $(date +%s) + 600 ))
while :; do
  state="$(gh pr view "$PR" --json state --jq .state)"
  case "$state" in
    MERGED) break;;
    CLOSED) die "PR #$PR was closed without merging";;
  esac
  [[ "$(date +%s)" -gt "$deadline" ]] && die "timed out waiting for PR #$PR to merge (still $state)"
  sleep 6
done

merge_sha="$(gh pr view "$PR" --json mergeCommit --jq '.mergeCommit.oid')"
info "PR #$PR merged to main ($merge_sha) ✅"

# Best-effort: surface the authoritative score once Scorekeeper records it.
info "waiting for Scorekeeper to record the verified score (best effort)…"
score_deadline=$(( $(date +%s) + 180 ))
while [[ "$(date +%s)" -lt "$score_deadline" ]]; do
  git fetch origin main --quiet || true
  if git show origin/main:RESULTS.md 2>/dev/null | grep -q "Current record:"; then
    record_line="$(git show origin/main:RESULTS.md | grep 'Current record:' | head -1)"
    latest_row="$(git show origin/main:RESULTS.md | grep -E '^\| [0-9]+ \|' | tail -1)"
    # Only stop once the ledger reflects our merge commit.
    if echo "$latest_row" | grep -q "${merge_sha:0:7}"; then
      echo
      echo "Recorded: $latest_row"
      echo "$record_line"
      break
    fi
  fi
  sleep 6
done

echo
info "Done. PR #$PR is merged to main. You can switch back: git checkout main && git pull"
