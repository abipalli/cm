# Submission history ledger

This directory is the repo's **memory**: a permanent, append-only record of how
each competitor arrived at their compressor changes and what score they achieved.

**Only CI may append entries.** The Scorekeeper GitHub Action runs on every push
to `main` that changes `src/algorithm/`, re-evaluates on GitHub Actions, and
commits new files here plus a row in `RESULTS.md`. Users cannot forge scores by
editing these files locally.

## Layout

```
history/
  README.md          this file
  entries/           one markdown file per CI-recorded submission
  TEMPLATE.md        reference format (not used directly by CI)
```

Each entry captures:

- **Who** submitted (GitHub author, commit)
- **Model** — AI model used (from the merged PR's `## Model` section)
- **What** changed (`git diff` summary of `src/algorithm/` vs parent commit)
- **Score** (CI-computed total compressed bytes) and delta vs the previous record
- **Approach** — copied from the merged PR's `## Approach` section
- **Eval snapshot** — full per-file output from the authoritative CI run

## For competitors

1. Edit only `src/algorithm/`.
2. Open a PR with a good **`## Model`**, **`## Approach`** (and optional **`## Iteration notes`**).
4. Pass the **Verify PR** check — it auto-merges to `main`.
5. **Scorekeeper** writes the entry automatically.

Do **not** commit files under `history/entries/` in your PR.

Ledger entries are append-only — never rewrite or delete past entries.
