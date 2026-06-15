---
name: autocm
description: >-
  Improve the cm context-mixing compressor by lowering SCORE on the fixed corpus.
  Use when improving compression, searching for new algorithm ideas, running
  autoresearch, competing on the leaderboard, or when the user mentions autocm.
---

# autocm — autoresearch for the cm compressor

Portable agent skill for any coding agent (Cursor, Claude Code, Codex, Copilot,
Gemini, etc.). Invoke by name or follow [`AGENTS.md`](../../AGENTS.md) at the
repo root.

You are an automated research agent. Your job is to **lower SCORE** (total
compressed bytes on the fixed corpus) by editing the algorithm, while a frozen
harness measures you.

## Start here (required)

Before changing anything or proposing ideas, read these files in order:

1. [`README.md`](../../README.md) — project layout, usage, current design
2. [`AUTORESEARCH.md`](../../AUTORESEARCH.md) — objective, invariants, edit
   boundaries, anti-cheat rules, workflow, and research leads

Treat `AUTORESEARCH.md` as the authoritative rulebook. Do not violate its
constraints.

## Orient on prior work

After reading the above, scan what has already been tried:

- [`RESULTS.md`](../../RESULTS.md) — current record and score history
- [`history/entries/`](../../history/entries/) — per-submission approaches and diffs
- [`src/algorithm/`](../../src/algorithm/) — current implementation (primary target:
  `model.rs`)

Use history to avoid repeating failed ideas and to build on what worked.

## Search for new solutions

Work from the leads in `AUTORESEARCH.md`, prioritizing the highest-payoff gaps
(e.g. bit-history states + StateMap for repetitive data). Then explore
adjacent ideas that stay within the rules:

- New or richer context models (orders, word/sparse banks, format detectors)
- Additional match models at longer orders
- Deeper mixing (two-layer mixers, longer SSE/APM chains)
- Better counter/state machinery and learning rates

Every candidate must be **general compression** — no corpus-specific tuning,
side channels, or nondeterminism.

## Iteration loop

1. Edit **only** `src/algorithm/` (signatures of `compress`/`decompress` in
   `mod.rs` stay character-for-character intact).
2. Evaluate locally:
   ```bash
   bash scripts/evaluate.sh
   ```
3. Accept only if: guard passes, build succeeds, round-trip tests pass, and a
   numeric `SCORE:` is printed.
4. If SCORE improved, keep the change; otherwise revert
   (`git checkout -- src/algorithm/`).
5. Repeat until you have a defensible improvement or exhaust the current lead.

For PR workflow and CI rules, see [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

## Output expectations

When reporting progress, include:

- **SCORE** before and after (lower is better)
- **Model** — which AI model assisted the work (e.g. opus 4.8, codex 5.5)
- **Approach** — what changed and why it should help
- **Iteration notes** — what you tried, what failed, what to try next
- Confirmation that only `src/algorithm/` was edited and losslessness holds

Make the number smaller.
