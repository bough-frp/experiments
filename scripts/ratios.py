#!/usr/bin/env python3
"""Ratios of Criterion benchmarks to their baselines, with intervals.

A research note quotes only a variant's ratio to the baseline measured in
the same run, never an absolute time. Criterion reports each benchmark's
own interval, not a ratio's, so this bootstraps one: it resamples each
benchmark's per-sample mean times (Criterion's `new/sample.json`),
takes the ratio of the resampled means, and reports the median and the
95% percentile interval of that ratio.

A benchmark ID is `<group>/<variant>/<param>`, or `<group>/<variant>`.
Each variant is compared with `<group>/baseline/<param>` (or
`<group>/baseline`). Run it from the repository root after
`cargo bench --bench <name>-wallclock`:

    python3 scripts/ratios.py [GROUP-PREFIX ...]

Standard library only. The seed is fixed, so a re-run over the same
samples prints the same intervals.
"""

import json
import random
import sys
from pathlib import Path

ROOT = Path("target/criterion")
RESAMPLES = 10_000
SEED = 20260928


def benchmarks():
    """Every benchmark Criterion has a sample for, keyed by its ID."""
    found = {}
    for bench in ROOT.rglob("new/benchmark.json"):
        meta = json.loads(bench.read_text())
        sample = json.loads((bench.parent / "sample.json").read_text())
        per_iter = [t / n for t, n in zip(sample["times"], sample["iters"])]
        found[meta["full_id"]] = per_iter
    return found


def baseline_of(bench_id):
    parts = bench_id.split("/")
    if len(parts) == 3:
        return f"{parts[0]}/baseline/{parts[2]}"
    if len(parts) == 2:
        return f"{parts[0]}/baseline"
    return None


def mean(xs):
    return sum(xs) / len(xs)


def ratio_interval(variant, baseline, rng):
    ratios = []
    for _ in range(RESAMPLES):
        v = mean(rng.choices(variant, k=len(variant)))
        b = mean(rng.choices(baseline, k=len(baseline)))
        ratios.append(v / b)
    ratios.sort()
    return (
        ratios[len(ratios) // 2],
        ratios[int(0.025 * len(ratios))],
        ratios[int(0.975 * len(ratios)) - 1],
    )


def main(prefixes):
    found = benchmarks()
    rng = random.Random(SEED)
    rows = []
    for bench_id in sorted(found):
        if prefixes and not any(bench_id.startswith(p) for p in prefixes):
            continue
        base = baseline_of(bench_id)
        if base is None or base == bench_id or base not in found:
            continue
        mid, lo, hi = ratio_interval(found[bench_id], found[base], rng)
        rows.append((bench_id, base, mid, lo, hi, mean(found[base])))
    if not rows:
        print("no variant with a baseline found", file=sys.stderr)
        return 1
    width = max(len(r[0]) for r in rows)
    print(f"{'benchmark':<{width}}  ratio   95% interval       baseline mean")
    for bench_id, _, mid, lo, hi, base_ns in rows:
        print(f"{bench_id:<{width}}  {mid:6.3f}  [{lo:6.3f}, {hi:6.3f}]  {base_ns:12.1f} ns")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
