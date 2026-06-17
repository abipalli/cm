# Entry 0073 — SCORE 572423 (-154 (new record))

| Field | Value |
|-------|-------|
| Date | 2026-06-17 |
| Author | @abipalli |
| Model | opus 4.8 |
| Git author | unknown \<unknown\> |
| Commit | `09048d9` (09048d9ca1f8461d4c94a2988ea3b8f1c993cf5a) |
| SCORE | 572423 |
| Δ vs previous record | -154 (new record) |
| vs zstd -22 | +17.24% |
| WORK | 8037454358 |
| MEMCOST | 3114620224 |
| Status | record |

## Approach

Deepens the Context Tree Weighting model's context from 32 bits (4 bytes) to **48 bits (6 bytes)**. CTW performs exact Bayesian model averaging over every prunable context tree up to depth D; a deeper D lets its optimal weighting capture longer deterministic structure that the fixed-order context bank and the DMC models miss. The hashed node store keeps its 2^24-node cap, so peak RSS is unchanged at 4.52 GB (well within the verifier budget) and the model stays deterministic/lossless.
SCORE 572577 → 572423 (**−154**).

## Iteration notes

CTW depth has paid monotonically as it deepens (16→24→32→48), because its weighting gracefully backs off to shallower contexts where the deep ones are unseen — so extra depth never hurts and helps wherever long structure exists. Memory is depth-independent thanks to the node cap. All 9 round-trip tests pass.

## Algorithm changes

```
 src/algorithm/ctw.rs | 2 +-
 1 file changed, 1 insertion(+), 1 deletion(-)
```

## Eval snapshot

```
file                        orig      ours    ratio  vs zstd    vs xz  lossless
binary_mozilla.bin        393216    228222    1.723    +3.4%    +3.5%  OK
repetitive_nci.bin        393216     13367   29.417   +47.0%   +43.1%  OK
source_samba.bin          393216    175406    2.242   +10.9%    +9.4%  OK
struct_xml.bin            393216      7463   52.689   +49.5%   +45.9%  OK
text_dickens.bin          393216     90620    4.339   +27.9%   +27.0%  OK
text_reymont.bin          393216     57345    6.857   +38.2%   +36.9%  OK
--------------------------------------------------------------------------------
TOTAL                    2359296    572423    4.122
  vs zstd -22 total: 691699 bytes  ->  +17.24% (smaller, WIN)
  vs xz -9e   total: 682460 bytes  ->  +16.12% (smaller, WIN)

SCORE: 572423 (total compressed bytes; lower is better)
```
