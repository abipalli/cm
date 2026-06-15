# Results log

Leaderboard of recorded submissions. Full narratives live in
[`history/entries/`](history/entries/).

| # | date | author | SCORE | Δ vs record | vs zstd-22 | commit | entry | note |
|---|------|--------|-------|-------------|------------|--------|-------|------|
| 0001 | 2026-06-14 | @10d9e | 642822 | — (baseline) | +7.06% | `d12023b` | [0001](history/entries/0001-baseline.md) | lpaq-class: orders 0-6 + word + sparse, match model, 2x APM, BCJ |
| 0002 | 2026-06-14 | @10d9e | 639105 | -3717 (new record) | +7.60% | `e838d6b` | [0002](history/entries/0002--10d9e.md) | 1. **Second match model at order-8.** Alongside the existing order-6 match model… |
| 0003 | 2026-06-14 | @10d9e | 637956 | -1149 (new record) | +7.76% | `3f837de` | [0003](history/entries/0003--10d9e.md) | Longer deterministic contexts continue to help the mixer on structured and textu… |
| 0004 | 2026-06-15 | @10d9e | 636158 | -1798 (new record) | +8.02% | `731096d` | [0004](history/entries/0004--10d9e.md) | - Add order-10, order-12, and order-14 match models to catch longer deterministi… |

**Current record: 636158** (@10d9e, entry 0004)

Ledger updates are **CI-only** — see [`.github/workflows/scorekeeper.yml`](.github/workflows/scorekeeper.yml).
