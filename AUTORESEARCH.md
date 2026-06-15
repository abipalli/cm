# AUTORESEARCH — rules for improving this compressor

You are an automated research agent. Your job is to **improve the compression
ratio** of this codec by editing the algorithm, while a frozen harness measures
you. Read this whole file before changing anything.

## The objective

Minimize **SCORE = total compressed bytes** over the fixed corpus in `corpus/`,
as reported by the harness:

```
bash scripts/evaluate.sh
```

Lower SCORE is better. The current baseline numbers (smaller = we win) are in
`corpus/baselines.tsv` (zstd -22 and xz -9e). Beating them by as much as
possible is the goal.

## The one hard invariant (non-negotiable)

The codec must be **exactly lossless for every possible input**:

```
decompress(compress(x)) == x        for all x
```

This is enforced by `tests/roundtrip.rs`, which fuzzes synthetic and adversarial
inputs (empty, single byte, random, all-same, highly repetitive, BCJ-heavy).
A candidate that fails any round-trip is **INVALID** and scores nothing,
no matter how small its output.

## What you MAY edit

**Only files under `src/algorithm/`.** That is the entire mutable surface:

- `src/algorithm/model.rs`  — the context-mixing predictor (primary target)
- `src/algorithm/coder.rs`  — the arithmetic coder
- `src/algorithm/tables.rs` — logistic tables
- `src/algorithm/mod.rs`    — entry point + filters
- You may **add new files/modules** under `src/algorithm/`.

You may freely change models, add models, retune constants (table sizes, learning
rates, counter limits, mixer contexts), restructure the mixer, add SSE stages,
add match models, add format-specific models, etc.

### The two frozen signatures

Inside `src/algorithm/mod.rs`, these signatures must remain **character-for-character** intact (bodies are yours):

```rust
pub fn compress(input: &[u8]) -> Vec<u8>
pub fn decompress(input: &[u8]) -> Vec<u8>
```

## What you MUST NOT touch (frozen)

Everything else, including:

- `src/main.rs`, `src/lib.rs`
- `src/harness/**`  (corpus loader, scoring)
- `tests/**`        (the losslessness gate)
- `corpus/**`, `corpus/baselines.tsv`
- `scripts/**`
- `Cargo.toml`      (no new dependencies — std-only Rust)
- `AUTORESEARCH.md`

The boundary is enforced by `scripts/guard.sh` (local) and `scripts/guard-pr.sh`
(CI): only `src/algorithm/` may change in a submission. `RESULTS.md` and
`history/entries/` are updated exclusively by the Scorekeeper GitHub Action on
merge to `main`.

## Anti-cheat rules (these define "a real improvement")

Improvements must be **general compression**, not benchmark gaming:

1. **No embedding corpus data.** Do not bake corpus bytes, corpus hashes, or a
   dictionary derived from the corpus into the algorithm. No detecting specific
   corpus files and special-casing them.
2. **No side channels.** `compress`/`decompress` may use **only** their input
   argument. No reading files, no network, no clock, no environment, no
   process state. `decompress` must reconstruct purely from the compressed bytes.
3. **Determinism.** Encode and decode must execute the identical predict/update
   sequence; any model state must evolve identically on both sides. No
   nondeterminism (no unseeded RNG, no thread-timing-dependent behavior).
4. **Generality over the held-out tests.** The round-trip tests use data that is
   *not* in the corpus, on purpose. Your algorithm must work on arbitrary input,
   not just the scored files.

A change that lowers SCORE by violating these is not a result — it will be
rejected on review and may break on the hidden evaluation set.

## Per-iteration workflow

1. Edit only `src/algorithm/`.
2. Run the gate + scorer:
   ```
   bash scripts/evaluate.sh
   ```
   A candidate is **accepted** only if: the boundary guard passes, the build
   succeeds, all round-trip tests pass, and it prints a numeric `SCORE:`.
3. If the new SCORE is lower than your best, keep the change; otherwise revert
   (`git checkout -- src/algorithm`).
4. Open a pull request with **only** `src/algorithm/` changes. Put your approach
   in the PR description (`## Model`, `## Approach`, `## Iteration notes`) — CI copies that
   into `history/entries/` on merge. **Do not** commit `RESULTS.md` or ledger files.
5. Wait for the **Verify PR** GitHub Actions check (authoritative score). It
   auto-merges passing PRs to `main`; Scorekeeper then writes the history entry.
6. Occasionally run `cargo test` (debug build) — it additionally catches
   integer-overflow bugs that release mode silently wraps. Use `wrapping_*`
   ops anywhere values may overflow (hashes, the mixer, `c4`).

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the GitHub competition workflow.

## Leads, roughly by expected payoff

These are extensions of the existing design (it is currently an lpaq-class
compressor). The biggest known weakness is highly repetitive data
(`repetitive_nci`, `struct_xml`), where LZ-style methods still win.

- **Bit-history states + a StateMap** instead of the plain adaptive counters in
  `model.rs`. This is the single largest known win and directly addresses the
  nonstationary / repetitive-data weakness. (Replace the `cp`/`cn` counters with
  an 8-bit state per context whose probability comes from an adaptive map.)
- **A second match model** at a longer order (e.g. order-8) alongside the
  current order-6 one, to catch long repeats more reliably.
- **More context orders** (7, 8) and a richer **word / sparse model bank**.
- **Two-layer mixing**: a set of mixers selected by different contexts, combined
  by a second mixer.
- **A longer SSE/APM chain** with more diverse contexts.
- **Format-specific models** (text vs executable vs tabular), gated by a cheap
  detector — but keep them general, not corpus-specific.

Good luck. Make the number smaller.
