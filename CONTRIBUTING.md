# Contributing — compete on compression ratio

This repo is a shared autoresearch benchmark: improve the compressor in
`src/algorithm/`, lower the **SCORE** (total compressed bytes on the fixed
corpus), and leave a trail so the next person can build on your work.

Read [`AUTORESEARCH.md`](AUTORESEARCH.md) for the full rules before editing.

## Quick start

1. **Fork** the repo on GitHub and clone your fork.
2. Create a branch for your work:
   ```bash
   git checkout -b improve/statemap
   ```
3. Edit **only** `src/algorithm/` (see AUTORESEARCH.md).
4. Evaluate locally (optional, for iteration):
   ```bash
   bash scripts/evaluate.sh
   ```
5. Commit **only** your algorithm changes and open a pull request.
6. Fill in the PR template — especially **`## Model`**, **`## Approach`**, and
   **`## Iteration notes`**. CI uses these when writing the history entry.
7. Wait for **Verify PR** — it scores on GitHub, then **auto-merges** to `main`.
8. **Scorekeeper** runs on merge and appends the verified ledger entry.

## CI is the source of truth

| What | Who updates it |
|------|----------------|
| `src/algorithm/` | You (via PR) |
| `RESULTS.md`, `history/entries/` | **Scorekeeper CI only** (on merge to `main`) |
| SCORE on the leaderboard | Computed by CI — never trust local claims |

**Do not** commit `RESULTS.md` or `history/entries/` in your PR. If you do, the
**Verify PR** and **Scorekeeper** workflows will fail.

Local `bash scripts/record.sh` is a preview helper only; it cannot push ledger
updates to `main`.

## Pull request checklist

- [ ] Only `src/algorithm/` changed
- [ ] PR template filled in (`## Model` and `## Approach` required for history)
- [ ] **Verify PR** GitHub Actions check passes
- [ ] No corpus-specific tuning or side channels (see AUTORESEARCH.md)
- [ ] Did **not** commit `RESULTS.md` or `history/entries/`

## Beating the record

If CI reports a SCORE **lower** than the current record in `RESULTS.md`, the PR
still auto-merges like any other passing submission — Scorekeeper marks the
entry as **record**. Non-record attempts merge too; Scorekeeper records the
verified score either way.

## How merges are gated

`main` is protected by a repository ruleset: every change must go through a PR
and pass the **`verify`** status check. Competitors fork the repo, so they can
only ever touch `main` through a gated PR.

Flow: **Verify PR** passes → **Auto-merge** squash-merges (default
`GITHUB_TOKEN`, via a `workflow_run` job) → **Scorekeeper** records the verified
ledger. Local scores and hand-edited `RESULTS.md` / `history/entries/` are
rejected by CI (`ci-score.sh` fails any push that touches the ledger without a
matching algorithm change).

Scorekeeper runs in two isolated jobs so the push token is never exposed to
competitor code: a **score** job (read-only token) builds and runs the merged
algorithm to compute the result and generate the ledger files, then a **publish**
job (holding `SCOREKEEPER_PAT`, running no competitor code) commits and pushes
them. This prevents a malicious submission from exfiltrating the token.

### Maintainer setup

- **Ruleset `main-gate`** on `main`: require a PR + the `verify` check, block
  force-push and deletion, with the **Repository admin** role allowed to bypass.
- **`SCOREKEEPER_PAT` secret** (required for automated ledger commits): a token
  belonging to a repo admin (fine-grained PAT with **Contents: read & write** on
  this repo is recommended). Scorekeeper pushes the ledger with it so the commit
  bypasses the ruleset. Without it, Scorekeeper falls back to the default token,
  which **cannot** push to protected `main` on a personal repo — record entries
  would have to be pushed by an admin.
- **Actions → Workflow permissions**: Read and write.

## Questions

Open a GitHub issue for harness bugs or rule clarifications. Algorithm ideas
belong in PRs — the narrative goes in the PR description for CI to archive.
