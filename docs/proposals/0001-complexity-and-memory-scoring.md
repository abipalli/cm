# RFC 0001 — Scoring for complexity and memory, not just bytes

**Status:** Draft / request-for-comment (maintainer discussion)
**Scope:** how submissions are judged (`record.sh`, `build-leaderboard.py`, the
CI gates, `AUTORESEARCH.md`). Touches frozen paths on purpose — this is an
infra/policy proposal, **not** a competition entry, so it intentionally fails the
`src/algorithm/`-only boundary guard.

## TL;DR

1. WORK-as-tiebreaker exerts **almost no optimization pressure** — it only bites
   on an *exact* byte tie, which essentially never happens between independent
   algorithm changes. Bots therefore ignore complexity entirely.
2. **Do not fold WORK into SCORE with a product or weighted sum** (`b·g`, `b+λg`).
   `b` is a *quality* target and `g`/memory are *costs*; combining them needs an
   arbitrary, gameable exchange rate and optimizes the wrong point.
3. **Recommended:** keep **SCORE primary**, and make complexity a **hard validity
   budget** (like the losslessness gate) — the Hutter-Prize model. Submissions
   over the budget are *invalid*, not penalized.
4. **The biggest gap: WORK (wasm fuel) does not measure memory.** The "8 GB for a
   few hundred KB" pain is a *memory* problem; fuel is a *time* proxy and is
   nearly blind to it. Add an explicit, deterministic **peak-memory** metric and
   budget — that, more than WORK, is what fixes the complaint.

## 1. The problem, concretely

The objective is raw compressed bytes (SCORE), and bots optimize it well. But
nothing constrains *resources*, so the frontier has drifted to solutions that are
correct yet practically degenerate. The current record (`#0073`, SCORE 572,423)
compresses the 2.36 MB corpus at roughly **8 GB peak RSS and tens of minutes of
wall-clock**, i.e. multiple GB to handle a ~384 KB file. That is a legitimate
SCORE winner under today's rules and a poor *codec*.

This is the right instinct to fix. The questions are (a) what to measure and
(b) how it enters the ranking.

## 2. Why the current WORK tiebreaker doesn't change behavior

Ranking is `(SCORE asc, WORK asc)` with WORK breaking **exact** byte ties only.
Independent algorithm changes essentially never tie to the byte, so WORK is
almost never decisive — it has **no gradient**. A rational optimizer spends WORK
freely (as the current record does). The tiebreaker *looks* like it accounts for
complexity but does not.

Observed side effect: because a byte-tie *does* let WORK win, the only thing the
tiebreaker actively rewards is **output-neutral micro-optimization** of an
existing record (same bytes, fewer ops). That is real but narrow work; minting
"records" for it is a policy choice worth making deliberately.

## 3. Why a combined SCORE formula is the wrong tool

- **`b · g` (or any product).** Treats bytes and cost symmetrically — a 1 % WORK
  cut "buys" a 1 % byte cut anywhere — which contradicts "bytes are primary."
  Its minimum sits at a mediocre middle (decent ratio, medium speed), not at
  "best ratio that is also reasonable." The ECDSA `t·q` analogy doesn't transfer:
  there *both* factors are pure costs minimized jointly; here one factor is the
  goal.
- **`b + λ·g` (weighted sum).** Requires an arbitrary exchange rate λ (how many
  gas = one byte?). Bots will park exactly at the λ knee, and λ becomes a
  perpetual tuning/gaming surface.

Bytes are the goal; complexity is a budget. Encode that asymmetry directly.

## 4. Recommendation: hard budgets + SCORE-primary (the Hutter-Prize model)

This benchmark is a mini-Hutter-Prize, and Hutter already solved this: impose
**hard resource limits**, then rank purely by compressed size.

**Validity gates** (binary — a violation makes the submission INVALID, exactly
like a failed round-trip):

- lossless round-trip (already enforced)
- `peak_memory ≤ MEM_MAX`
- `WORK ≤ WORK_MAX` (deterministic fuel; a proxy for time)

**Ranking among valid submissions:**

- primary: minimize SCORE (compressed bytes)
- tiebreaker: minimize WORK (keep it — harmless on exact ties)

Why this beats a combined formula:

- **No arbitrary λ.** The budget is one honest policy decision ("what envelope is
  realistic?"), not a continuous knob to tune and game.
- **Kills the degenerate frontier by fiat.** An 8 GB / 30-minute submission is
  simply invalid; no need to price it.
- **Matches the real problem.** Compression is always resource-bounded; "best
  ratio within a fixed budget" is the actual engineering target and a clean,
  generalizable autoresearch goal.
- **Keeps SCORE a clean scalar** the optimizer already understands.

Set the budgets to a defensible envelope (suggestion: `MEM_MAX ≈ 2 GB`,
`WORK_MAX ≈ a small multiple of xz -9e / zstd -22`). Tighten over time.

### Optional: two leaderboards

If the unbounded "how far can ratio go, cost be damned" frontier is itself worth
keeping for research, run two boards: **Open** (no budget) and **Budgeted**
(within `MEM_MAX`/`WORK_MAX`). The Budgeted board is the one that generalizes to
a practical autoresearch framework; the Open board preserves the pure-ratio race.

## 5. The key technical gap: WORK ≠ memory

`measure-complexity.sh` counts *executed wasm operators*. That is a good **time**
proxy but is **nearly blind to memory**: a model can touch a few operators while
allocating gigabytes of hash tables (precisely what oversized context tables do),
or be operator-heavy yet tiny in memory. So **adding or weighting WORK alone will
not fix the "8 GB" complaint** — the dominant pain is memory, and it needs its own
metric.

There is already an *accidental* memory gate: oversized submissions OOM-kill the
verifier (exit 143 → CI fails). That is opaque (the optimizer just dies, with no
published ceiling) and runner-dependent. **Make it explicit and reported.**

### How to measure memory deterministically

Mirror the WORK design. The existing wasm fuel meter (`metrics/`) already runs the
codec under `wasmtime` for a deterministic operator count; have it **also report
the peak wasm linear-memory high-water mark** (`memory.size` max, or
`Store`/`Memory` growth). That is deterministic, cross-machine reproducible, and
tamper-proof — the same properties that make WORK trustworthy — and it lives
outside `src/algorithm/`.

`scripts/measure-memory.sh` (in this PR) is a **stopgap**: it reports native peak
RSS via `/usr/bin/time`. It is human-meaningful and works today, but native RSS is
allocator/OS-dependent, so it should not be a *ranking* input — use it to pick
budgets and to sanity-check, then graduate to the deterministic wasm-memory meter
for enforcement.

## 6. Measurement / gaming cautions

- **Prefix vs. full corpus.** WORK is measured on a *fixed corpus prefix* while
  SCORE uses the full corpus. A submission can special-case: cheap on the prefix,
  expensive on the rest. Measure resource metrics on the **same data** as SCORE
  (or a representative sample), and watch for prefix overfitting.
- **Fuel ≠ wall-clock.** "Lower fuel is faster" holds on average but breaks for
  memory-/cache-bound code (few ops, many cache misses). Pair fuel with the memory
  budget; keep fuel deterministic for ranking but report wall-clock for humans.
- **Keep ranking inputs deterministic.** Wasm fuel and wasm peak-memory are
  reproducible across machines; native RSS and wall-clock are not. Only
  deterministic quantities should affect the leaderboard order.

## 7. Suggested migration

1. **Observe.** Land a `peak_memory` field (this PR's script as a stopgap; the
   wasm-memory meter as the durable version). Report it in the ledger/leaderboard
   next to WORK. *(No ranking change yet.)*
2. **Calibrate.** Read the distribution across existing entries; pick `MEM_MAX`
   (and re-confirm `WORK_MAX`) at a defensible practical envelope.
3. **Enforce.** Add the budgets as **validity gates** in `evaluate.sh` / the
   verify workflow (INVALID on violation). Keep ranking `(SCORE asc, WORK asc)`
   among valid entries. Optionally split Open vs. Budgeted boards.
4. **Document** the budgets and rationale in `AUTORESEARCH.md`.

## Appendix — files in this PR

- `scripts/measure-memory.sh` — reference/stopgap peak-RSS measurement.
- `docs/proposals/0001-complexity-and-memory-scoring.md` — this document.

No `src/algorithm/` changes; no ranking code is altered yet (steps 1–4 above are
the maintainers' call). This PR is for discussion.
