#!/usr/bin/env python3
"""Generate docs/data/leaderboard.json from the authoritative ledger.

Parses the RESULTS.md leaderboard table and enriches each row with the full
"Approach" narrative from its history/entries/ file. The static GitHub Pages
site (docs/) renders this JSON. Run from the repo root; safe to run anywhere.
"""
from __future__ import annotations

import json
import os
import re
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RESULTS = ROOT / "RESULTS.md"
BASELINES = ROOT / "corpus" / "baselines.tsv"
OUT = ROOT / "docs" / "data" / "leaderboard.json"

BASELINE_LABELS = {
    "zstd22": "zstd −22",
    "xz9e": "xz −9e",
    "brotli11": "brotli −11",
    "zpaq5": "zpaq m5",
    "lpaq1_9": "lpaq1 ×9",
}

ROW_RE = re.compile(r"^\|\s*\d{4}\s*\|")
LINK_RE = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
FIRST_INT_RE = re.compile(r"-?\d+")


def cells(line: str) -> list[str]:
    return [c.strip() for c in line.strip().strip("|").split("|")]


def parse_entry(entry_rel: str) -> dict:
    """Parse a history entry into its '##' sections plus metadata-table fields."""
    result: dict = {"sections": {}, "meta": {}}
    if not entry_rel:
        return result
    path = ROOT / entry_rel
    if not path.is_file():
        return result
    text = path.read_text(encoding="utf-8")

    current = None
    buf: list[str] = []
    for line in text.splitlines():
        if line.startswith("## "):
            if current is not None:
                result["sections"][current] = "\n".join(buf).strip()
            current = line[3:].strip().lower()
            buf = []
            continue
        if current is not None:
            buf.append(line)
        else:
            # metadata table rows: | Field | Value |
            m = re.match(r"^\|\s*([^|]+?)\s*\|\s*(.+?)\s*\|\s*$", line)
            if m and m.group(1).lower() not in ("field", "-------"):
                result["meta"][m.group(1).strip().lower()] = m.group(2).strip()
    if current is not None:
        result["sections"][current] = "\n".join(buf).strip()
    return result


def strip_code_fence(text: str) -> str:
    """Drop a single wrapping ``` fence so the raw snapshot can be shown in <pre>."""
    lines = text.splitlines()
    if lines and lines[0].startswith("```"):
        lines = lines[1:]
    if lines and lines[-1].strip().startswith("```"):
        lines = lines[:-1]
    return "\n".join(lines).strip()


def first_int(s: str) -> int | None:
    m = FIRST_INT_RE.search(s)
    return int(m.group()) if m else None


def parse_baselines() -> list[dict]:
    """Sum per-algorithm compressed bytes from corpus/baselines.tsv."""
    if not BASELINES.is_file():
        return []
    lines = BASELINES.read_text(encoding="utf-8").splitlines()
    if len(lines) < 2:
        return []
    headers = lines[0].split("\t")
    algo_cols = headers[2:]
    totals = [0] * len(algo_cols)
    for line in lines[1:]:
        parts = line.split("\t")
        for i in range(len(algo_cols)):
            idx = i + 2
            if idx < len(parts) and parts[idx].strip().isdigit():
                totals[i] += int(parts[idx])
    out = []
    for col, total in zip(algo_cols, totals):
        if total <= 0:
            continue
        out.append(
            {
                "id": col,
                "label": BASELINE_LABELS.get(col, col),
                "total": total,
            }
        )
    out.sort(key=lambda b: b["total"])
    return out


def main() -> int:
    repo = os.environ.get("GITHUB_REPOSITORY", "10d9e/cm")
    rows: list[dict] = []

    for raw in RESULTS.read_text(encoding="utf-8").splitlines():
        if not ROW_RE.match(raw):
            continue
        c = cells(raw)
        if len(c) < 9:
            continue
        entry_id, date, author, score, delta, vs_zstd, commit, entry, note = c[:9]
        link_m = LINK_RE.search(entry)
        entry_rel = link_m.group(1) if link_m else ""
        parsed = parse_entry(entry_rel)
        sections = parsed["sections"]
        meta = parsed["meta"]
        approach = sections.get("approach", "")
        rows.append(
            {
                "id": entry_id,
                "date": date,
                "author": author,
                "model": meta.get("model", ""),
                "score": first_int(score),
                "work": first_int(meta.get("work", "")),
                "memcost": first_int(meta.get("memcost", "")),
                "lines": first_int(meta.get("lines", "")),
                "heapPeak": first_int(meta.get("heap_peak", "")),
                "heapChurn": first_int(meta.get("heap_churn", "")),
                "delta": delta,
                "deltaValue": first_int(delta) if "baseline" not in delta else None,
                "vsZstd": vs_zstd,
                "commit": commit.strip("`"),
                "commitFull": meta.get("commit", ""),
                "status": meta.get("status", ""),
                "entryPath": entry_rel,
                "note": approach or note,
                "approach": approach or note,
                "iterationNotes": sections.get("iteration notes", ""),
                "algoChanges": strip_code_fence(sections.get("algorithm changes", "")),
                "evalSnapshot": strip_code_fence(sections.get("eval snapshot", "")),
                "isRecord": "record" in delta.lower(),
            }
        )

    scored = [r for r in rows if r["score"] is not None]
    baseline = scored[0]["score"] if scored else None
    # Rank by (SCORE asc, then WORK asc): byte score is dominant; WORK
    # (deterministic complexity) breaks exact byte-score ties only. A missing
    # WORK sorts as +infinity so it can never win a tie.
    record_row = (
        min(
            scored,
            key=lambda r: (
                r["score"],
                r["work"] if r["work"] is not None else float("inf"),
            ),
        )
        if scored
        else None
    )

    data = {
        "repo": repo,
        "generatedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "baseline": baseline,
        "baselines": parse_baselines(),
        "record": (
            {
                "id": record_row["id"],
                "score": record_row["score"],
                "author": record_row["author"],
            }
            if record_row
            else None
        ),
        "entries": rows,
    }

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {OUT.relative_to(ROOT)} ({len(rows)} entries)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
