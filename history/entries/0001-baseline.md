# Entry 0001 — SCORE 642822 (baseline)

| Field | Value |
|-------|-------|
| Date | 2026-06-14 |
| Author | @10d9e |
| Model | claude opus 4.8 |
| Git author | autoresearch \<autoresearch@example.com\> |
| Commit | `d12023b` |
| SCORE | 642822 |
| Δ vs previous record | — (initial baseline) |
| vs zstd -22 | +7.06% (smaller) |
| WORK | 77422350 |
| MEMCOST | 40081362 |
| Status | record |

## Approach

Initial lpaq-class context-mixing compressor shipped with the autoresearch
harness. Per-bit prediction from multi-order hashed context models (orders 0–6
+ word + sparse) with adaptive-rate counters, a learned match model, a
context-selected logistic mixer, a two-stage APM/SSE, an x86 BCJ filter, and a
binary arithmetic coder.

## Algorithm changes

```
(none — starting point)
```

## Eval snapshot

```
file                        orig      ours    ratio  vs zstd    vs xz  lossless
binary_mozilla.bin        393216    237934    1.653    -0.7%    -0.6%  OK
repetitive_nci.bin        393216     24669   15.940    +2.1%    -5.0%  OK
source_samba.bin          393216    188970    2.081    +4.1%    +2.4%  OK
struct_xml.bin            393216     15281   25.732    -3.5%   -10.8%  OK
text_dickens.bin          393216    101514    3.874   +19.2%   +18.2%  OK
text_reymont.bin          393216     74454    5.281   +19.7%   +18.1%  OK
--------------------------------------------------------------------------------
TOTAL                    2359296    642822    3.670
  vs zstd -22 total: 691652 bytes  ->  +7.06% (smaller, WIN)
  vs xz -9e   total: 682412 bytes  ->  +5.80% (smaller, WIN)

SCORE: 642822 (total compressed bytes; lower is better)
```
