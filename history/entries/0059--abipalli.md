# Entry 0059 — SCORE 572643 (-126 (new record))

| Field | Value |
|-------|-------|
| Date | 2026-06-17 |
| Author | @abipalli |
| Model | opus 4.8 |
| Git author | unknown \<unknown\> |
| Commit | `1134861` (11348613a059c0bfa8865d1115b2021aaebbc8ba) |
| SCORE | 572643 |
| Δ vs previous record | -126 (new record) |
| vs zstd -22 | +17.21% |
| WORK | 16237821821 |
| MEMCOST | 1791892226 |
| Status | record |

## Approach

Adds a **second DMC instance at a conservative clone threshold**, complementing the existing fast-cloning DMC.
The merged DMC clones aggressively (thresholds 2/2): it reaches high order quickly but its deep states stay count-starved. A second DMC that clones conservatively (thresholds 8/8) keeps a lower-order, better-populated view of the same bit stream. The two speeds are complementary and the mixer learns to blend them. `Dmc::new(t1, t2)` now takes the clone thresholds as parameters, so the instances differ only in cloning speed.
Both are down-weightable mixer inputs (no regression risk), pure integer arithmetic (deterministic → exactly lossless), each capped at 2^22 nodes (~64 MB).
SCORE 572769 → 572643 (**−126**).

## Iteration notes

Multi-speed DMC is the classic way to extend a single DMC: paq-class compressors run several with different reset/clone behavior. Threshold 8/8 was chosen for the slow instance (4× the fast one); it adds orthogonal signal on top of the fast DMC and the recently-added indirect-on-transform contexts. All 9 round-trip tests pass.

## Algorithm changes

```
 src/algorithm/dmc.rs   | 13 ++++++++-----
 src/algorithm/model.rs | 11 ++++++++---
 2 files changed, 16 insertions(+), 8 deletions(-)
```

## Eval snapshot

```
file                        orig      ours    ratio  vs zstd    vs xz  lossless
binary_mozilla.bin        393216    228161    1.723    +3.5%    +3.5%  OK
repetitive_nci.bin        393216     13354   29.446   +47.0%   +43.2%  OK
source_samba.bin          393216    175389    2.242   +11.0%    +9.4%  OK
struct_xml.bin            393216      7458   52.724   +49.5%   +45.9%  OK
text_dickens.bin          393216     90767    4.332   +27.8%   +26.9%  OK
text_reymont.bin          393216     57514    6.837   +38.0%   +36.7%  OK
--------------------------------------------------------------------------------
TOTAL                    2359296    572643    4.120
  vs zstd -22 total: 691699 bytes  ->  +17.21% (smaller, WIN)
  vs xz -9e   total: 682460 bytes  ->  +16.09% (smaller, WIN)

SCORE: 572643 (total compressed bytes; lower is better)
```
