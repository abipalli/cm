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

**Current record: 587905** (@10d9e, entry 0014)

Ledger updates are **CI-only** — see [`.github/workflows/scorekeeper.yml`](.github/workflows/scorekeeper.yml).
