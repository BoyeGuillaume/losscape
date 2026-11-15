"""Checkpoint tests against S3: a local S3 mock (moto, run with uvx), a bucket in it, then the
server's and the client's checkpoint tests with their storage there. Needs the dev databases
(make dev-db) and uv.

    python3 scripts/test_s3.py      # or: make test-s3
"""

import os
import socket
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MOTO = "moto[server]==5.2.5"
BUCKET = "losscape-test"


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for(url: str, process: subprocess.Popen, seconds: float = 120) -> None:
    """The first start downloads moto, which can take a while."""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if process.poll() is not None:
            sys.exit("moto stopped before it was ready")
        try:
            urllib.request.urlopen(url, timeout=1).close()  # noqa: S310 - local
            return
        except OSError:
            time.sleep(0.5)
    sys.exit(f"moto didn't answer at {url}")


def main() -> int:
    port = free_port()
    endpoint = f"http://127.0.0.1:{port}"
    moto = subprocess.Popen(
        ["uvx", "--from", MOTO, "moto_server", "-H", "127.0.0.1", "-p", str(port)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        wait_for(endpoint, moto)
        req = urllib.request.Request(f"{endpoint}/{BUCKET}", method="PUT")
        urllib.request.urlopen(req, timeout=10).close()  # noqa: S310 - local
        env = {
            **os.environ,
            "LOSSCAPE_TEST_STORAGE": f"s3://{BUCKET}/tests",
            "AWS_ACCESS_KEY_ID": "test",
            "AWS_SECRET_ACCESS_KEY": "test",
            "AWS_REGION": "us-east-1",
            "AWS_ENDPOINT": endpoint,
            "AWS_ALLOW_HTTP": "true",
        }
        for args in [
            ["-p", "losscape-server", "--features", "testing", "--test", "checkpoints"],
            ["-p", "losscape-client", "--test", "e2e", "checkpoint"],
        ]:
            done = subprocess.run(["cargo", "test", *args], cwd=ROOT, env=env, check=False)
            if done.returncode:
                return done.returncode
        print(f"checkpoint tests passed against S3 (moto at {endpoint})")
        return 0
    finally:
        moto.terminate()
        moto.wait(timeout=10)


if __name__ == "__main__":
    sys.exit(main())
