# RFC 0001 — Account for complexity and memory, not just bytes

**Status:** Draft / request-for-comment.
**Scope:** how submissions are judged (the `metrics/` meters, `record.sh`,
`build-leaderboard.py`, CI gates, `AUTORESEARCH.md`). Touches frozen paths on
purpose — this is an infra/policy proposal, **not** a competition entry, so it
intentionally fails the `src/algorithm/`-only boundary guard.

## TL;DR

1. WORK-as-tiebreaker has almost no optimization gradient — it bites only on an
   *exact* byte tie, so bots ignore complexity. (The current record spends ~4 GB
   freely; WORK never affected its ranking.)
2. Don't fold WORK into SCORE with a product or weighted sum (`b·g`, `b+λg`) on
   the leaderboard — bytes are a *quality* target, cost is a *cost*; combining
   needs an arbitrary, gameable exchange rate.
3. **Better: fold cost into WORK itself**, so a single `(SCORE asc, WORK asc)`
   ranking already prices compute *and* memory. This PR does that — WORK now
   counts heap allocation alongside executed operators, flowing through the
   existing WORK plumbing unchanged.
4. **But a key subtlety (measured below): WORK is *differenced* to cancel
   one-time setup, which also cancels the one-time table allocation.** So WORK
   captures allocation *churn*, not the table *footprint*. The footprint (the
   real "GBs for a few hundred KB" pain) needs a *non-differenced, full-scale*
   memory number — provided here as a separate meter, and best enforced as a
   hard budget.

## 1. The problem

SCORE (compressed bytes) is optimized well, but nothing constrains resources, so
the frontier drifted to correct-but-degenerate solutions: the current record
peaks at **~4.4 GB reserved memory** (and tens of minutes) to compress a 2.36 MB
corpus. A valid SCORE winner; a poor codec.

## 2. Why the current WORK tiebreaker doesn't change behavior

Ranking is `(SCORE asc, WORK asc)` with WORK breaking *exact* byte ties only.
Independent algorithm changes essentially never tie to the byte, so WORK has no
gradient and a rational optimizer spends it freely. (Its one active effect is to
reward output-neutral micro-optimization of an existing record — real but narrow.)

## 3. Why a combined SCORE formula is the wrong tool

`b·g` treats bytes and cost symmetrically (a 1 % cost cut "buys" a 1 % byte cut),
contradicting "bytes primary," and its optimum sits at a mediocre middle. `b+λg`
needs an arbitrary λ that bots park on and that you tune forever. The ECDSA `t·q`
analogy doesn't transfer: there both factors are pure costs; here one is the goal.

## 4. Implemented: fold cost into WORK; meter the footprint separately

Two complementary meters, both outside `src/algorithm/` (tamper-proof):

**(a) WORK now includes heap allocation** (`metrics/`). A reusable tracking
allocator (`metrics/telemetry`) meters heap bytes requested; the wasm shim runs
under it, and the host charges `HEAP_GAS_PER_BYTE` per allocated byte on top of
the executed-operator fuel — both **differenced** (full prefix − half prefix) so
the one-time Cm setup cancels. Result still prints as `WORK:`, so it flows
through `measure-complexity.sh → ci-score.sh → record.sh → build-leaderboard.py`
with no plumbing change. The host also reports **peak wasm linear-memory pages**
(heap + shadow stack + statics; wasm memory only grows, so its final size is the
peak).

**(b) Full-scale reserved-memory meter** (`metrics/mem`, `scripts/measure-memory.sh`).
The same tracking allocator wraps the *native* codec over the *whole* corpus and
reports `MEM:` = peak live reserved bytes. Deterministic (it sums requested byte
sizes, independent of OS/page size/RSS), reserved (a lazily-touched table counts
in full), full-scale (no wasm32 4 GiB ceiling).

## 5. Measured finding — why both meters are needed

Running the unified WORK meter on the 8 KB prefix:

```
full 8192B  fuel 17,464,329,637   heap 1,173,403,332 B
half 4096B  fuel  9,231,366,194   heap 1,173,229,764 B
peak linear memory: 1.17 GB (heap + stack + statics)
WORK: 8,233,137,011  (= 8,232,963,443 fuel + 1 × 173,568 heap-bytes)
```

The table allocation (~1.17 GB) is **one-time at `Cm::new`**, so it is identical
in the full and half runs and **cancels in the differenced WORK** — the heap term
contributes only 173 KB. So:

- **WORK** (differenced) prices *per-byte* compute + allocation *churn*. Good for
  rewarding leaner hot loops; blind to one-time footprint.
- **MEM** (`metrics/mem`, non-differenced, full scale) prices the *footprint*
  (4.36 GB here) — the thing that actually hurts.

They measure different costs; keep both.

## 6. Recommendation

- **Keep WORK = fuel + heap-allocation** (this PR) as the secondary ranking key —
  it now penalizes allocation churn that pure fuel missed, at no plumbing cost.
- **Add a full-scale memory budget** as a hard *validity gate* (like losslessness):
  `MEM ≤ MEM_MAX` ⇒ valid, else INVALID. This is what kills the GB-for-KB
  frontier, and it's the Hutter-Prize pattern (hard resource limits, then rank by
  size). `metrics/mem` provides the number; pick `MEM_MAX` from observed data.
- Optionally a `WORK_MAX` gate / wall-time cap for the slowest solutions.
- Tunables to decide: `HEAP_GAS_PER_BYTE` (currently 1) and `MEM_MAX`.

## 7. Measurement / gaming cautions

- **Prefix vs. full corpus.** WORK runs on a fixed prefix; SCORE on the full
  corpus. With table sizes that scale with input, the prefix under-represents the
  full-corpus footprint (1.17 GB prefix vs 4.36 GB full). A submission could also
  special-case the prefix. Footprint must be measured full-scale (it is, in `(b)`).
- **Keep ranking inputs deterministic.** Wasm fuel, heap-byte counts and the
  native reserved-byte count are reproducible across machines; native RSS and
  wall-clock are not — don't rank on those.

## 8. Suggested migration

1. Land these meters and report `WORK` (now incl. heap) and `MEM` next to each
   other (no ranking change).
2. Calibrate `MEM_MAX` (and `HEAP_GAS_PER_BYTE`) from the entry distribution.
3. Enforce `MEM ≤ MEM_MAX` as a validity gate in `evaluate.sh` / verify; keep
   `(SCORE asc, WORK asc)` among valid entries.
4. Document budgets in `AUTORESEARCH.md`.

## Appendix — files in this PR

- `metrics/telemetry/` — reusable tracking-allocator wrapper (heap volume + peak).
- `metrics/mem/` + `scripts/measure-memory.sh` — full-scale reserved-memory meter.
- `metrics/wasm/`, `metrics/host/` — WORK now folds in heap allocation and reports
  peak linear memory.
- `docs/proposals/0001-complexity-and-memory-scoring.md` — this document.

No `src/algorithm/` changes; no ranking code is altered (steps 2–4 are the
maintainers' call). For discussion.
