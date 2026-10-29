"""Against a running server: start one (make dev-db && make dev-server), then

    export LOSSCAPE_TEST_SERVER=http://localhost:8080 LOSSCAPE_API_KEY=...
    uv run pytest tests/test_e2e.py

Points are checked directly in ClickHouse (CLICKHOUSE_URL / _USER / _PASSWORD / _DATABASE,
defaults match deploy/docker-compose.dev.yml).
"""

import base64
import os
import signal
import subprocess
import sys
import time
import urllib.request

import pytest

import losscape

SERVER = os.environ.get("LOSSCAPE_TEST_SERVER")
pytestmark = pytest.mark.skipif(not SERVER, reason="set LOSSCAPE_TEST_SERVER to run")


def clickhouse(sql: str) -> str:
    url = os.environ.get("CLICKHOUSE_URL", "http://localhost:8123")
    database = os.environ.get("CLICKHOUSE_DATABASE", "losscape")
    req = urllib.request.Request(f"{url}/?database={database}", data=sql.encode())
    user = os.environ.get("CLICKHOUSE_USER", "losscape")
    password = os.environ.get("CLICKHOUSE_PASSWORD", "losscape")
    token = base64.b64encode(f"{user}:{password}".encode()).decode()
    req.add_header("Authorization", f"Basic {token}")
    return urllib.request.urlopen(req, timeout=30).read().decode().strip()


def stored(run_id: str) -> tuple[int, int]:
    """(distinct (key, step) points, max step) of a run."""
    out = clickhouse(
        "SELECT uniqExact(key, step), max(step) FROM metrics "
        f"WHERE run_id = toUUID('{run_id}') FORMAT TSV"
    )
    points, max_step = out.split("\t")
    return int(points), int(max_step)


def test_online_run_is_uploaded(tmp_path):
    run = losscape.init(project="pytest-e2e", mode="online", server=SERVER, dir=tmp_path)
    for i in range(2000):
        run.log({"loss": 1 / (i + 1), "acc": i / 2000})
    assert run.finish() == "uploaded"
    assert stored(run.id) == (4000, 1999)
    assert not run.dir.exists(), "local log deleted once uploaded"


CHILD = """
import sys, time, losscape
run = losscape.init(project="pytest-e2e", mode="online", server=sys.argv[1], dir=sys.argv[2])
print(run.id, flush=True)
start = time.perf_counter()
step = 0
while True:
    run.log({"loss": 1 / (step + 1), "acc": step / 1e6}, step=step)
    if step % 50 == 0:
        print(step, step / (time.perf_counter() - start), flush=True)
    step += 1
    time.sleep(0.0005)
"""


def test_killed_process_loses_at_most_a_moment(tmp_path):
    child = subprocess.Popen(
        [sys.executable, "-c", CHILD, SERVER, str(tmp_path)],
        stdout=subprocess.PIPE,
        text=True,
    )
    run_id = child.stdout.readline().strip()
    deadline = time.time() + 2.0
    last_step, rate = 0, 0.0
    while time.time() < deadline:
        step, rate = child.stdout.readline().split()
        last_step, rate = int(step), float(rate)
    child.send_signal(signal.SIGKILL)
    child.wait()

    sync = subprocess.run(
        [sys.executable, "-m", "losscape.cli", "sync", "--dir", str(tmp_path), "--server", SERVER],
        capture_output=True,
        text=True,
    )
    assert sync.returncode == 0, sync.stdout + sync.stderr
    assert f"{run_id}  uploaded" in sync.stdout

    points, max_step = stored(run_id)
    assert points == 2 * (max_step + 1), "no gaps: every step up to the last stored one"
    lost_seconds = max(0, last_step - max_step) / rate
    assert lost_seconds <= 1.0, f"lost {lost_seconds:.2f}s of logging"
    lost_ms = lost_seconds * 1000
    print(f"\nkilled at step >= {last_step}, stored up to {max_step} ({lost_ms:.0f} ms lost)")
