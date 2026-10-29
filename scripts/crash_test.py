#!/usr/bin/env python3
"""Kill the server (SIGKILL) during a load test, restart it, and check that every point is
stored exactly once.

    cargo build --release -p losscape-server -p losscape-loadgen
    python3 scripts/crash_test.py

Needs the dev databases (make dev-db), a .env at the repo root, and LOSSCAPE_API_KEY set to
an API key from the web UI (Settings > API keys). Standard library only.
"""

import argparse
import base64
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LISTEN = "127.0.0.1:18081"


def load_env() -> dict[str, str]:
    env = {}
    for line in (ROOT / ".env").read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            key, value = line.split("=", 1)
            env[key.strip()] = value.strip()
    return {**env, **os.environ}


def start_server(env: dict[str, str], log) -> subprocess.Popen:
    server = subprocess.Popen(
        [ROOT / "target/release/losscape-server"],
        cwd=ROOT,
        env={**env, "LOSSCAPE_LISTEN": LISTEN},
        stdout=log,
        stderr=log,
    )
    deadline = time.time() + 30
    while time.time() < deadline:
        try:
            urllib.request.urlopen(f"http://{LISTEN}/readyz", timeout=1)
            return server
        except OSError:
            time.sleep(0.2)
    server.kill()
    sys.exit("server did not become ready")


def clickhouse(env: dict[str, str], sql: str) -> str:
    req = urllib.request.Request(
        f"{env['CLICKHOUSE_URL']}/?database={env.get('CLICKHOUSE_DATABASE', 'losscape')}",
        data=sql.encode(),
    )
    token = f"{env.get('CLICKHOUSE_USER', 'default')}:{env.get('CLICKHOUSE_PASSWORD', '')}"
    req.add_header("Authorization", "Basic " + base64.b64encode(token.encode()).decode())
    return urllib.request.urlopen(req, timeout=60).read().decode()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--runs", type=int, default=8)
    parser.add_argument("--metrics", type=int, default=16)
    parser.add_argument("--steps", type=int, default=20_000)
    parser.add_argument("--kill-after", type=float, default=2.0, help="seconds into the load")
    parser.add_argument("--down-for", type=float, default=3.0, help="seconds the server is down")
    args = parser.parse_args()

    env = load_env()
    if not env.get("LOSSCAPE_API_KEY", "").startswith("df_"):
        sys.exit("set LOSSCAPE_API_KEY to an API key from the web UI (Settings > API keys)")
    workdir = Path(tempfile.mkdtemp(prefix="losscape-crash-"))
    report_path = workdir / "report.json"
    log = (workdir / "server.log").open("ab")
    print(f"logs: {workdir}")

    server = start_server(env, log)
    loadgen = subprocess.Popen(
        [
            ROOT / "target/release/losscape-loadgen",
            "--server",
            f"http://{LISTEN}",
            "--project",
            f"crashtest-{int(time.time())}",
            "--runs",
            str(args.runs),
            "--metrics",
            str(args.metrics),
            "--steps",
            str(args.steps),
            "--report",
            report_path,
        ],
        cwd=ROOT,
        env=env,
    )

    time.sleep(args.kill_after)
    if loadgen.poll() is not None:
        sys.exit("load finished before the kill; raise --steps or lower --kill-after")
    server.send_signal(signal.SIGKILL)
    server.wait()
    print(f"server killed at {args.kill_after}s, restarting in {args.down_for}s")
    time.sleep(args.down_for)
    server = start_server(env, log)

    loadgen.wait()
    server.send_signal(signal.SIGTERM)
    server.wait()
    if loadgen.returncode != 0:
        sys.exit(f"loadgen failed with exit code {loadgen.returncode}")

    report = json.loads(report_path.read_text())
    expected = report["points_per_run"]
    ids = ",".join(f"toUUID('{run_id}')" for run_id in report["run_ids"])
    rows = clickhouse(
        env,
        f"""SELECT toString(run_id), count(), uniqExact(key, step)
            FROM metrics WHERE run_id IN ({ids}) GROUP BY run_id FORMAT TSV""",
    ).split("\n")
    stored = {}
    for row in filter(None, rows):
        run_id, raw, unique = row.split("\t")
        stored[run_id.replace("-", "")] = (int(raw), int(unique))

    print(
        f"retries during the test: {report['retries']}, duplicates reported: {report['duplicates']}"
    )
    failed = False
    for run_id in report["run_ids"]:
        raw, unique = stored.get(run_id, (0, 0))
        missing = expected - unique
        extra = raw - unique
        status = "ok" if missing == 0 else "MISSING POINTS"
        failed |= missing != 0
        print(
            f"  run {run_id}: {unique}/{expected} unique points, {extra} duplicate rows ({status})"
        )
    if failed:
        sys.exit("FAIL: points were lost")
    print("PASS: every point stored; duplicate rows (if any) collapse in ReplacingMergeTree")


if __name__ == "__main__":
    main()
