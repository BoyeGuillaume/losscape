"""Against a running server: start one (make dev-db && make dev-server), then

    export LOSSCAPE_TEST_SERVER=http://localhost:8080
    export LOSSCAPE_API_KEY=df_...   # web UI: Settings > API keys
    uv run pytest tests/test_e2e.py

Points are checked directly in ClickHouse (CLICKHOUSE_URL / _USER / _PASSWORD / _DATABASE,
defaults match deploy/docker-compose.dev.yml).
"""

import base64
import json
import math
import os
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request

import pytest

import lsc

SERVER = os.environ.get("LOSSCAPE_TEST_SERVER")
pytestmark = pytest.mark.skipif(not SERVER, reason="set LOSSCAPE_TEST_SERVER to run")


@pytest.fixture(scope="module", autouse=True)
def project():
    """The SDK only logs to existing projects: creates "pytest-e2e" unless it exists."""
    req = urllib.request.Request(
        f"{SERVER}/api/v0/projects",
        data=b'{"name": "pytest-e2e"}',
        headers={
            "Authorization": f"Bearer {os.environ['LOSSCAPE_API_KEY']}",
            "Content-Type": "application/json",
        },
    )
    try:
        urllib.request.urlopen(req, timeout=10)
    except urllib.error.HTTPError as err:
        if err.code != 409:
            raise


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
        f"WHERE run_id = toUUID('{run_id}') AND NOT startsWith(key, 'system/') FORMAT TSV"
    )
    points, max_step = out.split("\t")
    return int(points), int(max_step)


def test_online_run_is_uploaded(tmp_path):
    run = lsc.init(project="pytest-e2e", mode="online", server=SERVER, dir=tmp_path)
    for i in range(2000):
        run.log({"loss": 1 / (i + 1), "acc": i / 2000})
    assert run.finish() == "uploaded"
    assert stored(run.id) == (4000, 1999)
    assert not run.dir.exists(), "local log deleted once uploaded"


def test_a_run_reads_back_as_logged(tmp_path):
    run = lsc.init(
        project="pytest-e2e",
        mode="online",
        server=SERVER,
        dir=tmp_path,
        config={"optimizer": {"lr": 3e-4}},
        system_metrics=False,
    )
    for i in range(500):
        run.log({"loss": 1 / (i + 1), "acc": i / 500})
    run.log({"loss": float("nan")})
    assert run.finish() == "uploaded"

    api = lsc.Api(SERVER)
    back = api.run(run.id)
    assert back.name == run.name and back.state == "finished"
    assert back.config == {"optimizer": {"lr": 3e-4}}
    assert any(r.id == run.id for r in api.runs("pytest-e2e"))
    points = list(back.scan_history("loss"))
    assert [p.step for p in points] == list(range(501))
    assert points[9].value == 0.1
    assert math.isnan(points[-1].value)
    assert back.history(min_step=10, max_step=19)["acc"].tolist() == [
        i / 500 for i in range(10, 20)
    ]


CHILD = """
import sys, time, lsc
run = lsc.init(project="pytest-e2e", mode="online", server=sys.argv[1], dir=sys.argv[2])
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
        [sys.executable, "-m", "lsc.cli", "sync", "--dir", str(tmp_path), "--server", SERVER],
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


CONSOLE_CHILD = r"""
import os, subprocess, sys, time, lsc
run = lsc.init(project="pytest-e2e", mode="online", server=sys.argv[1], dir=sys.argv[2])
print("hello from python")
print("isatty", sys.stdout.isatty())
for i in range(101):
    print(f"\rprogress {i:3d}%", end="", flush=True)
    time.sleep(0.005)
print()
print("\x1b[31mred\x1b[0m on stderr", file=sys.stderr)
sys.stdout.flush()
subprocess.run(["echo", "from a subprocess"])
os.write(1, b"raw descriptor write\n")
time.sleep(2.5)  # a few system metric samples
run.log({"loss": 1.0})
print("unflushed last line", end="")
outcome = run.finish()
sys.__stderr__.write(f"RESULT {run.id} {outcome}\n")
"""


def api(path: str):
    req = urllib.request.Request(
        f"{SERVER}/api/v0{path}",
        headers={"Authorization": f"Bearer {os.environ['LOSSCAPE_API_KEY']}"},
    )
    return json.loads(urllib.request.urlopen(req, timeout=10).read())


@pytest.mark.parametrize("terminal", [False, True], ids=["pipe", "terminal"])
def test_console_and_system_metrics_are_captured(tmp_path, terminal):
    env = {**os.environ, "LOSSCAPE_SYSTEM_INTERVAL": "1", "LOSSCAPE_SILENT": "1"}
    args = [sys.executable, "-c", CONSOLE_CHILD, SERVER, str(tmp_path)]
    if terminal:
        import pty

        master, slave = pty.openpty()
        child = subprocess.Popen(args, stdout=slave, stderr=subprocess.PIPE, env=env)
        os.close(slave)
        shown = b""
        while True:
            try:
                chunk = os.read(master, 65536)
            except OSError:  # EIO: the child closed the terminal
                break
            if not chunk:
                break
            shown += chunk
        os.close(master)
        stderr = child.stderr.read().decode()
        child.wait()
        shown = shown.decode()
    else:
        done = subprocess.run(args, capture_output=True, text=True, env=env)
        shown, stderr = done.stdout, done.stderr
    result = [line for line in stderr.splitlines() if line.startswith("RESULT")]
    assert result, stderr
    _, run_id, outcome = result[0].split()
    assert outcome == "uploaded"

    # Still shown where it would have gone without losscape.
    assert "hello from python" in shown and "raw descriptor write" in shown
    assert f"isatty {terminal}" in shown, "programs see the same kind of output"

    logs = api(f"/runs/{run_id}/logs?from=0&to=100")
    lines = [(line["stream"], line["text"]) for line in logs["lines"]]
    assert [t for _, t in lines] == [
        "hello from python",
        f"isatty {terminal}",
        "progress 100%",
        "\x1b[31mred\x1b[0m on stderr",
        "from a subprocess",
        "raw descriptor write",
        "unflushed last line",
    ], lines
    assert lines[3][0] == "stderr"

    detail = api(f"/runs/{run_id}")
    keys = {m["key"] for m in detail["metrics"]}
    assert {"system/cpu/util_pct", "system/memory/used_pct", "loss"} <= keys, keys
    assert not any(k.startswith("system/") for k in detail["summary"]), "kept out of summary"
    environment = detail["environment"]
    assert environment["python"]["executable"] == sys.executable
    assert any(p.startswith("pytest==") for p in environment["packages"])
    assert environment["cpu"]["threads"] >= 1
