# cm — a context-mixing compressor (with an autoresearch harness)

A general lossless compressor that maximizes **compression ratio**. On the
bundled dev corpus it beats `zstd -22` and `xz -9e` in aggregate, with the
largest wins on natural-language text (~19% smaller than zstd).

It is built to be improved by automated agents: the algorithm lives behind a
fixed contract, and a frozen harness scores any candidate. See
[`AUTORESEARCH.md`](AUTORESEARCH.md) for the rules.

## Layout

```
src/algorithm/   EDITABLE — the compressor (model, coder, tables, filters)
src/harness/     frozen   — corpus loader + scoring
src/main.rs      frozen   — CLI
tests/           frozen   — losslessness gate (fuzzed, not corpus-tied)
corpus/          frozen   — fixed benchmark + baselines.tsv
scripts/         frozen   — guard.sh, evaluate.sh
```

## Usage

```
cargo build --release
./target/release/cm c file.in file.cm     # compress
./target/release/cm d file.cm file.out    # decompress
./target/release/cm eval corpus           # score against the corpus
```

Or grade a candidate end-to-end (guard + tests + score):

```
bash scripts/evaluate.sh
```

## Design (current)

lpaq-class context mixing: per-bit prediction from multi-order hashed context
models (orders 0–6 + word + sparse) with adaptive-rate counters, a learned
match model, a context-selected logistic mixer, a two-stage APM/SSE, an x86
BCJ filter, and a binary arithmetic coder. The objective is ratio only;
decompression is symmetric and slow by design.

## Improving it

Edit only `src/algorithm/`, run `bash scripts/evaluate.sh`, keep changes that
lower the SCORE. The biggest known lever is replacing the plain counters with
bit-history states + a StateMap (helps the repetitive-data cases). Details and
constraints are in `AUTORESEARCH.md`.
