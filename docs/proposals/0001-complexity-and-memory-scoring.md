# RFC 0001 — Account for complexity and memory, not just bytes

**Status:** Draft / request-for-comment.
**Scope:** how submissions are judged — the `metrics/` meters, `record.sh`,
`ci-score.sh`, `build-leaderboard.py`, and a proposed `MEM_MAX` validity gate.
This is an infra/policy proposal; it touches frozen paths on purpose, so it
fails the `src/algorithm/`-only boundary guard by design and is admin-merged.
No `src/algorithm/` change.

This PR also **ships** the observational half of the proposal: the `LINES`,
`HEAP_PEAK`, and `HEAP_CHURN` meters/columns (alongside the existing `WORK` and
`MEMCOST`). The `MEM_MAX` gate (§5) remains a proposal — nothing here changes the
ranking.

## TL;DR

1. **WORK-as-tiebreaker has no optimization gradient** — it bites only on an
   *exact* byte tie, so bots ignore complexity and the frontier spends memory
   freely (§2).
2. **Don't fold cost into SCORE** (`b·g`, `b+λg`): bytes are a *quality* target,
   cost is a *cost*; combining needs an arbitrary, gameable exchange rate (§3).
3. **The cost that dominates wall-clock is memory traffic** — random scatter
   across multi-GB tables — which executed-operator fuel is blind to: a cache
   miss and an L1 hit are the same one operator. Measured: a 64× larger table
   moves traffic 4.1× but `WORK` only +0.07 % (§4.1).
4. **Track that traffic directly.** `MEMCOST` (a modeled cache-miss penalty,
   already on main) and `LINES` (distinct cache lines touched, this PR) both
   price it; `HEAP_PEAK`/`HEAP_CHURN` price footprint and allocation. All are
   **non-scoring diagnostics** today — observe before deciding (§4.2–§4.4).
5. **Propose a `MEM_MAX` validity gate** (native peak RSS ≤ the verifier's RAM)
   so a submission can't OOM the judge — a hard budget, *not* folded into SCORE
   (§5).

## 1. The problem

SCORE (compressed bytes) is optimized well, but nothing constrains resources, so
the frontier drifted toward correct-but-degenerate solutions: the current record
peaks at multiple GB of reserved memory (and tens of minutes) to compress a
2.36 MB corpus. A valid SCORE winner; a poor codec.

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
The fix is not to combine bytes and cost, but to (a) make the *cost* meters
measure real cost, and (b) cap the genuinely pathological via a hard validity
budget — keeping the two-key sort.

## 4. Pricing real cost: memory traffic

### 4.1 Why fuel can't see it — measured

Two builds, identical 8 KB input, identical operator count, only the
context-table size changed:

| context tables | distinct 64B lines touched | bytes touched | WORK (fuel) |
|---|---:|---:|---:|
| 2¹⁶ slots | 10,390,194 | 664,972,416 | 8,249,510,050 |
| 2²² slots | 42,968,754 | 2,750,000,256 | 8,255,239,687 |

A **64× larger table → 4.1× more memory traffic, but WORK moves +0.07 %.** Fuel
is flat because the operator stream is unchanged; only the *addresses* moved.
Wall-clock tracks the traffic, not the fuel — so a cost metric must price the
traffic. (Illustrative run; raw output in `0001-lines-evidence.txt`.)

### 4.2 The meters — what each measures

All live outside `src/algorithm/` (a submission cannot alter them) and are
deterministic given the pinned toolchain. Measured on this PR's base (`9e24b54`):

| meter | this base | role | what it captures |
|---|---:|---|---|
| `WORK` | 7,639,867,643 | **ranking** (tiebreak) | init-free executed wasm operators (fuel) |
| `MEMCOST` | 2,451,899,718 | diagnostic | init-free weighted miss penalty, 3-level LRU cache model |
| `LINES` | 853,928 | diagnostic | init-free distinct 64B lines touched |
| `HEAP_PEAK` | 4,361,191,836 | diagnostic | native peak reserved heap over the full corpus |
| `HEAP_CHURN` | 173,568 | diagnostic | init-free steady-state heap bytes requested |

`MEMCOST` and `LINES` run the **same** high-entropy input (`compress_prefix_he`)
with the **same** `(full − half)` differencing, so they are directly comparable.

### 4.3 MEMCOST and LINES — track both, let the data decide

`MEMCOST` models the cost of misses through a fixed L1/L2/L3 hierarchy; `LINES`
counts distinct lines touched with no cache geometry at all. They are
complementary, and the reason to ship both is empirical: over a backfill of the
existing entries, **do they rank submissions the same?**

- If they agree, `MEMCOST` is validated and `LINES` is a cheap robustness
  cross-check (associativity-free, so nothing in a fixed cache geometry can be
  overfit — the only way to lower `LINES` is to genuinely touch fewer lines).
- If they diverge, the divergence is exactly where the cache model's assumptions
  bite, and worth understanding before either is used to rank.

Neither is wired into ranking here. This is the "observe" phase (§8).

### 4.4 Allocation is inert — measured

`HEAP_CHURN` is **173,568 B** — the codec is allocation-free in steady state
(the multi-GB tables are one-time at `Cm::new` and cancel in the differencing).
So allocation is not a useful cost axis to price; `HEAP_PEAK` (the ~4.36 GB
footprint) is the part that actually hurts, and it motivates the gate below.

## 5. Proposed: a `MEM_MAX` footprint gate

A hard **validity gate** (Hutter-Prize style), not folded into SCORE:

- **`MEM_MAX` = the verifier machine's RAM** (e.g. 16 GiB on GitHub-hosted
  `ubuntu-latest`). The gate's only job is "a submission must not OOM the judge,"
  so it is pinned to the judge's memory, not a hand-picked budget. Over-budget ⇒
  **invalid**, converting today's cryptic `exit 143` into an explicit verdict.
- **Measured as native peak RSS**, not a wasm or heap-only number: RSS is
  source-agnostic (counts `static`/BSS + stack + heap), so the static-table
  bypass that defeats a heap-only meter (`HEAP_PEAK`) cannot dodge it. RSS is
  non-deterministic, but determinism only matters for *ranking*; a pass/fail OOM
  gate with multi-GB margin tolerates jitter. Never rank on RSS.
- Ranking among valid entries stays `(SCORE asc, WORK asc)`.

## 6. Anti-gaming

- **Meters are outside `src/algorithm/`** (`guard.sh` boundary), so a submission
  cannot touch the measurement path.
- **No self-allocator.** This PR's heap meters install a `#[global_allocator]`;
  `guard.sh`/`guard-pr.sh` now reject a `#[global_allocator]` inside
  `src/algorithm/`, which would otherwise shadow the metering allocators and
  zero out `HEAP_PEAK`/`HEAP_CHURN`.
- **Non-perturbing.** Heap tracking is behind an off-by-default `--features heap`
  shim build; the default `cm_wasm_meter.wasm` that `WORK`/`MEMCOST` consume is
  byte-identical, so adding the heap meters cannot move their numbers.
- **Differencing + fixed input.** `MEMCOST`/`LINES`/`HEAP_CHURN` subtract a
  half-prefix run to cancel one-time setup, over a fixed embedded stream — no
  submission-controlled meter input.

## 7. Determinism

Every ranked or reported number is a pure function of the emitted wasm (or the
requested-byte trace), independent of host CPU/OS/page-size/RSS, given the pinned
`wasm32-unknown-unknown` toolchain and a pinned `wasmtime`/`walrus`. The only
non-deterministic quantity discussed is native RSS, used solely for the proposed
*validity* gate (§5), never for ranking.

## 8. Migration

1. **Observe (this PR).** Report `MEMCOST`, `LINES`, `HEAP_PEAK`, `HEAP_CHURN`
   next to `WORK`; backfill across existing entries. No ranking change.
2. **Calibrate.** From the backfill, decide whether `MEMCOST` and `LINES` agree,
   and pick `MEM_MAX` from the entry distribution / the verifier's RAM.
3. **Gate.** Enforce `MEM_MAX` (native peak RSS) as a validity gate in the
   verify path; keep `(SCORE asc, WORK asc)` among valid entries.
4. **Document** the budgets in `AUTORESEARCH.md`.

## Appendix

- §4.1 evidence: `docs/proposals/0001-lines-evidence.txt`.
- Meters: `metrics/{host,memmeter,lines,heappeak,heapchurn}`, measured via
  `scripts/measure-{complexity,memcost,lines,heappeak,heapchurn}.sh`.
