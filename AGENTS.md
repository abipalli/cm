# Agent instructions

This repo is an autoresearch benchmark: improve the lossless compressor in
`src/algorithm/` and lower **SCORE** (total compressed bytes on the fixed
corpus). A frozen harness scores every candidate.

## autocm workflow

When improving compression, searching for new solutions, or running
autoresearch, follow [`.agents/skills/autocm/SKILL.md`](.agents/skills/autocm/SKILL.md).

**Start by reading [`README.md`](README.md) and [`AUTORESEARCH.md`](AUTORESEARCH.md)
before proposing or implementing changes.**

## Quick reference

| Command | Purpose |
|---------|---------|
| `bash scripts/evaluate.sh` | Guard + tests + corpus score |
| `cargo test` | Extra losslessness / overflow checks (debug) |
| `cargo build --release` | Build the compressor CLI |

Edit only `src/algorithm/`. Do not commit `RESULTS.md` or `history/entries/`.
See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the PR workflow.
