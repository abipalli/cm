# Entry 0056 — SCORE 573376 (-165 (new record))

| Field | Value |
|-------|-------|
| Date | 2026-06-16 |
| Author | @abipalli |
| Model | opus 4.8 |
| Git author | unknown \<unknown\> |
| Commit | `6491617` (6491617b9a25e96fb4465045d2976f774e1ac902) |
| SCORE | 573376 |
| Δ vs previous record | -165 (new record) |
| vs zstd -22 | +17.11% |
| WORK | 15600842519 |
| MEMCOST | 3171752404 |
| Status | record |

## Approach

Size context tables to input length (recover the CI-blocked table-growth win)
The 4-way associative context tables were fixed at 2^22 slots regardless of
input size. On the 384 KB corpus files each model's index space (context x the
in-byte c0 partial-byte selector) reaches ~3M distinct slots against 4M slots —
a ~75% load factor whose collisions/evictions cost real bits. Growing the tables
is the single biggest known lever, but a uniform increase OOM-kills the verifier:
the parallel round-trip tests each commit the full tables.
Resolve the tension by sizing the high-cardinality tables to the input length —
the standard "more data needs a bigger model" policy (cf. zstd window-log, lpaq
memory option). Inputs >= 256 KB get one extra table bit (2^22 -> 2^23 for the
associative models); smaller inputs are unchanged. This is general (any large
input benefits; no corpus detection) and deterministic (the decoder knows the
length from the header, so encode/decode size tables identically).
Effect: the parallel test suite is memory-unchanged (its only >=256 KB input is
the periodic repetitive test, which touches ~tens of distinct contexts and so
commits almost nothing), while the corpus files get a halved load factor.
Single-threaded eval peak RSS measured at 4.48 GB (< the 7 GB runner limit);
+2 bits would ~double table memory and risk OOM, so the growth is capped at +1.
SCORE 573541 -> 573376 (-165), improving five of six files. All round-trip
tests pass; the change is a table-sizing constant, no new state or arithmetic.

## Iteration notes

Tried first and reverted: a layer-3 learned mixer replacing the equal-average of the ten layer-2 combiners (last-byte ctx, LR 6) regressed +710 — averaging already-strong, correlated combiners is hard to beat and per-context weights overfit at 384 KB. This table-sizing win is orthogonal to the 4-way associativity (#0053) and 16-bit precision (#0055) records and stacks on top of both. Peak RSS measured 4.48 GB single-threaded at 2^23; +2 bits (~8 GB) would OOM the 7 GB verifier, hence the +1 cap. The 256 KB gate keeps parallel-test memory unchanged: the only round-trip test >=256 KB is the 300 KB periodic case, which touches ~tens of distinct contexts and commits almost nothing.

## Algorithm changes

```
 src/algorithm/model.rs | 13 ++++++++++---
 1 file changed, 10 insertions(+), 3 deletions(-)
```

## Eval snapshot

```
file                        orig      ours    ratio  vs zstd    vs xz  lossless
binary_mozilla.bin        393216    228192    1.723    +3.4%    +3.5%  OK
repetitive_nci.bin        393216     13545   29.030   +46.3%   +42.4%  OK
source_samba.bin          393216    175518    2.240   +10.9%    +9.3%  OK
struct_xml.bin            393216      7533   52.199   +49.0%   +45.4%  OK
text_dickens.bin          393216     90881    4.327   +27.7%   +26.8%  OK
text_reymont.bin          393216     57707    6.814   +37.8%   +36.5%  OK
--------------------------------------------------------------------------------
TOTAL                    2359296    573376    4.115
  vs zstd -22 total: 691699 bytes  ->  +17.11% (smaller, WIN)
  vs xz -9e   total: 682460 bytes  ->  +15.98% (smaller, WIN)

SCORE: 573376 (total compressed bytes; lower is better)
```
