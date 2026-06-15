# Results log

Leaderboard of recorded submissions. Full narratives live in
[`history/entries/`](history/entries/).

| # | date | author | SCORE | Δ vs record | vs zstd-22 | commit | entry | note |
|---|------|--------|-------|-------------|------------|--------|-------|------|
| 0001 | 2026-06-14 | @10d9e | 642822 | — (baseline) | +7.06% | `d12023b` | [0001](history/entries/0001-baseline.md) | lpaq-class: orders 0-6 + word + sparse, match model, 2x APM, BCJ |
| 0002 | 2026-06-14 | @10d9e | 639105 | -3717 (new record) | +7.60% | `e838d6b` | [0002](history/entries/0002--10d9e.md) | 1. **Second match model at order-8.** Alongside the existing order-6 match model… |
| 0003 | 2026-06-14 | @10d9e | 637956 | -1149 (new record) | +7.76% | `3f837de` | [0003](history/entries/0003--10d9e.md) | Longer deterministic contexts continue to help the mixer on structured and textu… |
| 0004 | 2026-06-15 | @10d9e | 636158 | -1798 (new record) | +8.02% | `731096d` | [0004](history/entries/0004--10d9e.md) | - Add order-10, order-12, and order-14 match models to catch longer deterministi… |
| 0005 | 2026-06-15 | @10d9e | 628826 | -7332 (new record) | +9.08% | `019c128` | [0005](history/entries/0005--10d9e.md) | Adds three general-purpose shape/layout context models to the existing context m… |
| 0006 | 2026-06-15 | @10d9e | 614363 | -14463 (new record) | +11.17% | `847678f` | [0006](history/entries/0006--10d9e.md) | Adds an adaptive bit-history `StateMap` per context model and indexes each State… |
| 0007 | 2026-06-15 | @10d9e | 610511 | -3852 (new record) | +11.73% | `d8a8cd9` | [0007](history/entries/0007--10d9e.md) | Retunes three online-learning adaptation-rate constants — no new models, no co… |
| 0008 | 2026-06-15 | @10d9e | 606779 | -3732 (new record) | +12.27% | `03e1d79` | [0008](history/entries/0008--10d9e.md) | Extends the context-model bank from 17 to 23 models — all general-purpose, no … |
| 0009 | 2026-06-15 | @10d9e | 605962 | -817 (new record) | +12.40% | `8a1b5e6` | [0009](history/entries/0009--10d9e.md) | Adds word-level n-gram context models, targeting natural-language text where the… |
| 0010 | 2026-06-15 | @10d9e | 595819 | -10143 (new record) | +13.86% | `defe1d9` | [0010](history/entries/0010--10d9e.md) | Replaces the single context-selected logistic mixer with a two-layer mixing netw… |
| 0011 | 2026-06-15 | @10d9e | 594283 | -1536 (new record) | +14.08% | `c3774ba` | [0011](history/entries/0011--10d9e.md) | Adds a third APM/SSE calibration stage after the existing two, keyed on a *dense… |
| 0012 | 2026-06-15 | @10d9e | 588570 | -5713 (new record) | +14.91% | `f60bc60` | [0012](history/entries/0012--10d9e.md) | Expands the context-model bank from 26 to 47 models, all general-purpose, exploi… |
| 0013 | 2026-06-15 | @10d9e | 588120 | -450 (new record) | +14.97% | `7ef74d8` | [0013](history/entries/0013--10d9e.md) | Adds eight gap-bigram context models to the bank (26 -> ... -> now extended): th… |
| 0014 | 2026-06-15 | @10d9e | 587905 | -215 (new record) | +15.01% | `f323fca` | [0014](history/entries/0014--10d9e.md) | Re-tunes two online-learning constants that were last set at entry 0007, when th… |
| 0015 | 2026-06-15 | @10d9e | 586819 | -1086 (new record) | +15.16% | `5c18fb8` | [0015](history/entries/0015--10d9e.md) | Doubles each context model's hash table from 2^22 to 2^23 slots. With the contex… |
| 0016 | 2026-06-15 | @10d9e | 585739 | -1080 (new record) | +15.32% | `d7d4fec` | [0016](history/entries/0016--10d9e.md) | Two related SSE/APM improvements: 1. **Fourth APM/SSE stage keyed on match lengt… |
| 0017 | 2026-06-15 | @10d9e | 585226 | -513 (new record) | +15.39% | `31d60b0` | [0017](history/entries/0017--10d9e.md) | Adds a second layer-2 combiner and averages it with the existing one in the logi… |
| 0018 | 2026-06-15 | @10d9e | 584982 | -244 (new record) | +15.43% | `50f1f5e` | [0018](history/entries/0018--10d9e.md) | Re-tunes the layer-2 combiner learning rate from 4/65536 to 12/65536. The rate w… |
| 0019 | 2026-06-15 | @10d9e | 584723 | -259 (new record) | +15.47% | `5611fb9` | [0019](history/entries/0019--10d9e.md) | Expands the layer-2 ensemble from two combiners to four, averaged in the logit d… |
| 0020 | 2026-06-15 | @10d9e | 584276 | -447 (new record) | +15.53% | `bf5b353` | [0020](history/entries/0020--10d9e.md) | Adds eleven 4-sample strided context models — each hashes bytes at pos-k, pos-… |
| 0021 | 2026-06-15 | @10d9e | 583905 | -371 (new record) | +15.58% | `56ef71a` | [0021](history/entries/0021--10d9e.md) | Adds word-level n-gram/skip-gram context models, targeting natural-language text… |
| 0022 | 2026-06-15 | @10d9e | 583868 | -37 (new record) | +15.59% | `91f5665` | [0022](history/entries/0022--10d9e.md) | Extends the layer-2 ensemble from four combiners to five, averaged in the logit … |
| 0023 | 2026-06-15 | @10d9e | 583253 | -615 (new record) | +15.68% | `a5ff3e6` | [0023](history/entries/0023--10d9e.md) | Adds a sixth layer-1 specialist mixer selected by the current match state — th… |
| 0024 | 2026-06-15 | @10d9e | 583001 | -252 (new record) | +15.71% | `1ce805f` | [0024](history/entries/0024--10d9e.md) | Adds a seventh layer-1 specialist mixer selected by the byte column since the la… |
| 0025 | 2026-06-15 | @10d9e | 582758 | -243 (new record) | +15.75% | `1696f2b` | [0025](history/entries/0025--10d9e.md) | Adds an eighth layer-1 specialist mixer selected by a hash of the last four byte… |
| 0026 | 2026-06-15 | @10d9e | 582663 | -95 (new record) | +15.76% | `b0d13d3` | [0026](history/entries/0026--10d9e.md) | Adds a ninth layer-1 specialist mixer selected by a hash of the last six bytes (… |
| 0027 | 2026-06-15 | @10d9e | 582587 | -76 (new record) | +15.77% | `9330f7c` | [0027](history/entries/0027--10d9e.md) | Re-tunes the layer-1 specialist mixers' weight-update rate from 14/65536 to 12/6… |
| 0028 | 2026-06-15 | @10d9e | 582351 | -236 (new record) | +15.81% | `3f5917e` | [0028](history/entries/0028--10d9e.md) | Adds a tenth layer-1 specialist mixer selected by a stride-2 sparse context — … |
| 0029 | 2026-06-15 | @10d9e | 582052 | -299 (new record) | +15.85% | `a052d1e` | [0029](history/entries/0029--10d9e.md) | Adds an eleventh layer-1 specialist mixer selected by a stride-3 sparse context … |
| 0030 | 2026-06-15 | @10d9e | 581078 | -974 (new record) | +15.99% | `c292dc5` | [0030](history/entries/0030--10d9e.md) | Adds a new model family: 2D / 'byte-above' modelling, which predicts from the by… |
| 0031 | 2026-06-15 | @10d9e | 579415 | -1663 (new record) | +16.23% | `722ed67` | [0031](history/entries/0031--10d9e.md) | Adds a new model family: indirect context models. For each of the order-1..4 con… |
| 0032 | 2026-06-15 | @10d9e | 579224 | -191 (new record) | +16.26% | `5f3154f` | [0032](history/entries/0032--10d9e.md) | Extends the 2D / byte-above model family with two more contexts that read the up… |
| 0033 | 2026-06-15 | @10d9e | 579171 | -53 (new record) | +16.27% | `3a282e6` | [0033](history/entries/0033--10d9e.md) | Extends the indirect-model family to the word level: a hash table records the re… |
| 0034 | 2026-06-15 | @10d9e | 579101 | -70 (new record) | +16.28% | `562dd16` | [0034](history/entries/0034--10d9e.md) | Adds a run-length context: the last byte combined with the length of its current… |
| 0035 | 2026-06-15 | @10d9e | 578791 | -310 (new record) | +16.32% | `6236ca9` | [0035](history/entries/0035--10d9e.md) | Adds a sixth match model anchored on just the last 4 bytes. The existing match m… |
| 0036 | 2026-06-15 | @10d9e | 578673 | -118 (new record) | +16.34% | `ec9792e` | [0036](history/entries/0036--10d9e.md) | Retunes the short match model added in the previous entry from an order-4 anchor… |
| 0037 | 2026-06-15 | @10d9e | 578672 | -1 (new record) | +16.34% | `aa300ac` | [0037](history/entries/0037--10d9e.md) | Memory optimization. The six match-model hash tables were sized at 2^23..2^26 en… |
| 0038 | 2026-06-15 | @10d9e | 578672 | 0 (tie) | +16.34% | `de442dd` | [0038](history/entries/0038--10d9e.md) | Memory optimization with provably identical output. The order-0 context model ha… |
| 0039 | 2026-06-15 | @10d9e | 578672 | 0 (tie) | +16.34% | `c71ba05` | [0039](history/entries/0039--10d9e.md) | Memory optimization, provably identical output. The direct-counter probability t… |
| 0040 | 2026-06-15 | @10d9e | 578552 | -120 (new record) | +16.36% | `e0ecc8f` | [0040](history/entries/0040--10d9e.md) | Adds a new model family: a nesting model that tracks the stack of currently-open… |

**Current record: 578552** (@10d9e, entry 0040)

Ledger updates are **CI-only** — see [`.github/workflows/scorekeeper.yml`](.github/workflows/scorekeeper.yml).
