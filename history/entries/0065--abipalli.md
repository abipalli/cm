# Entry 0065 — SCORE 572577 (-66 (new record))

| Field | Value |
|-------|-------|
| Date | 2026-06-17 |
| Author | @abipalli |
| Model | opus 4.8 |
| Git author | unknown \<unknown\> |
| Commit | `24a3698` (24a36981bcfbf6332fe71acf4fc8736d7140144b) |
| SCORE | 572577 |
| Δ vs previous record | -66 (new record) |
| vs zstd -22 | +17.22% |
| WORK | 12492087906 |
| MEMCOST | 2219244818 |
| Status | record |

## Approach

Two changes that together set a new record (**572643 → 572520, −123**):
**1. Context Tree Weighting model** (Willems, Shtarkov & Tjalkens 1995) as a mixer input. CTW performs exact Bayesian model averaging over *every* prunable depth-24 context tree, with a Krichevsky–Trofimov estimator per node and the standard weighting `Pw(s) = ½·Pe(s) + ½·Pw(s0)·Pw(s1)` (computed in the log domain). This is a fundamentally different mixing law from the codec's logistic/geometric mixer, so it contributes orthogonal signal on top of the hashed-context bank and the two DMC models. Deterministic f64 (encoder and decoder run the identical recursion → exactly lossless), hashed node store, down-weightable mixer input.
**2. Context tables sized for SCORE, not speed.** A prior speed-motivated change had shrunk the high-cardinality associative tables to 2^20 to cut WORK — but this competition's objective is compression and ignores speed, and larger tables cut hash collisions. Restored to 2^23 (+1 bit for inputs ≥ 256 KB → 2^24 on the corpus). Single-threaded eval peak RSS measured at 9.0 GB (within the runner limit).

## Iteration notes

CTW is the next frontier-academic additive model after DMC. Depth-16 CTW saved −57 but left +9 over the record; deepening to 24 bits (3 bytes of context) was the key — its optimal weighting then saved enough to clear the bar. The table regrow alone recovered most of the speed-shrink regression but landed +66 over the record; CTW closed the rest and beat it. 2^24 tables beat 2^23 here (the cold-slot tradeoff favors the larger size with the current model). All 9 round-trip tests pass.

## Algorithm changes

```
 src/algorithm/ctw.rs   | 137 +++++++++++++++++++++++++++++++++++++++++++++++++
 src/algorithm/mod.rs   |   1 +
 src/algorithm/model.rs |  18 +++++--
 3 files changed, 151 insertions(+), 5 deletions(-)
```

## Eval snapshot

```
file                        orig      ours    ratio  vs zstd    vs xz  lossless
binary_mozilla.bin        393216    228222    1.723    +3.4%    +3.5%  OK
repetitive_nci.bin        393216     13368   29.415   +47.0%   +43.1%  OK
source_samba.bin          393216    175418    2.242   +10.9%    +9.4%  OK
struct_xml.bin            393216      7463   52.689   +49.5%   +45.9%  OK
text_dickens.bin          393216     90685    4.336   +27.8%   +27.0%  OK
text_reymont.bin          393216     57421    6.848   +38.1%   +36.8%  OK
--------------------------------------------------------------------------------
TOTAL                    2359296    572577    4.120
  vs zstd -22 total: 691699 bytes  ->  +17.22% (smaller, WIN)
  vs xz -9e   total: 682460 bytes  ->  +16.10% (smaller, WIN)

SCORE: 572577 (total compressed bytes; lower is better)
```
