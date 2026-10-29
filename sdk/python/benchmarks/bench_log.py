"""Latency of ``run.log()`` per call, as seen by the training loop.

    uv run python benchmarks/bench_log.py

Target (docs/REQUIREMENTS.md): p99 < 10 µs with up to 20 scalars, no network on the call.
Runs in offline mode so only the SDK is measured; the background thread still batches and
writes the local log while this runs.
"""

from __future__ import annotations

import os
import tempfile
import time

os.environ.setdefault("LOSSCAPE_SILENT", "1")

import losscape  # noqa: E402


def measure(run: losscape.Run, metrics: dict[str, float], calls: int) -> list[int]:
    timings = [0] * calls
    clock = time.perf_counter_ns
    for i in range(calls):
        metrics["train/loss"] = 1.0 / (i + 1)
        start = clock()
        run.log(metrics)
        timings[i] = clock() - start
    return timings


def report(label: str, timings: list[int]) -> None:
    timings = sorted(timings)
    pct = lambda p: timings[min(len(timings) - 1, int(len(timings) * p))] / 1000  # noqa: E731
    total_s = sum(timings) / 1e9
    print(
        f"{label:>12}: p50 {pct(0.50):6.2f} µs  p99 {pct(0.99):6.2f} µs  "
        f"p99.9 {pct(0.999):7.2f} µs  max {timings[-1] / 1000:8.1f} µs  "
        f"({len(timings) / total_s:,.0f} calls/s)"
    )


def main() -> None:
    calls = 200_000
    with tempfile.TemporaryDirectory() as root:
        run = losscape.init(project="bench", mode="offline", dir=root)
        for n in (1, 5, 20):
            metrics = {f"train/metric_{i:02d}": float(i) for i in range(n - 1)}
            metrics["train/loss"] = 0.0
            measure(run, metrics, 20_000)  # warm up
            report(f"{n} metrics", measure(run, metrics, calls))
        run.finish()


if __name__ == "__main__":
    main()
