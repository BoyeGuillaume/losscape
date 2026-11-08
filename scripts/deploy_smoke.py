#!/usr/bin/env python3
"""Checks that a fresh losscape deployment works, end to end.

    python3 scripts/deploy_smoke.py http://localhost:8080
    # ... restart it, then check that nothing was lost:
    python3 scripts/deploy_smoke.py http://localhost:8080 --after-restart

Needs an empty server: it creates the first account. With the Python SDK importable, it also
logs a real run through it. Standard library otherwise; what it needs between the two steps
is kept in a state file (--state), including the account's password: delete it afterwards.
"""

import argparse
import http.cookiejar
import json
import os
import secrets
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

PROJECT = "smoke"
STEPS = 500


class Client:
    def __init__(self, base: str) -> None:
        self.base = base.rstrip("/")
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
        )

    def request(self, method: str, path: str, body=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(self.base + path, data=data, method=method)
        if data is not None:
            request.add_header("Content-Type", "application/json")
        try:
            with self.opener.open(request, timeout=30) as response:
                return response.status, response.read(), response.headers
        except urllib.error.HTTPError as err:
            return err.code, err.read(), err.headers

    def get(self, path: str):
        return self.request("GET", path)

    def json(self, method: str, path: str, body=None):
        status, raw, _ = self.request(method, path, body)
        if status >= 400:
            sys.exit(f"FAILED: {method} {path}: {status} {raw[:300]!r}")
        return json.loads(raw) if raw else None


def check(what: str, ok: bool, detail: object = "") -> None:
    print(f"{'ok  ' if ok else 'FAIL'} {what}{f': {detail}' if detail != '' else ''}", flush=True)
    if not ok:
        sys.exit(1)


def wait_until_ready(client: Client, timeout: float = 120) -> None:
    deadline = time.monotonic() + timeout
    while True:
        try:
            if client.get("/readyz")[0] == 200:
                return
        except OSError:
            pass
        if time.monotonic() > deadline:
            sys.exit(f"FAILED: {client.base}/readyz not ready after {timeout} s")
        time.sleep(1)


def first_start(client: Client, state: Path) -> None:
    check("healthz", client.get("/healthz")[0] == 200)

    status, raw, headers = client.get("/")
    check("the web UI is served", status == 200 and b"<html" in raw.lower(), f"{len(raw)} bytes")
    policy = headers.get("content-security-policy", "")
    check("with its Content-Security-Policy", "script-src" in policy)
    status, raw, _ = client.get("/p/any/runs")
    check("client-side routes load the app", status == 200 and b"<html" in raw.lower())
    status, raw, _ = client.get("/THIRD-PARTY-NOTICES.txt")
    check("license notices are served", status == 200 and b"zxcvbn" in raw, f"{len(raw)} bytes")
    status, raw, _ = client.get("/THIRD-PARTY-NOTICES-SERVER.txt")
    check("so are the server's", status == 200 and b"tokio" in raw, f"{len(raw)} bytes")
    for part in ("server", "web"):
        listed = client.json("GET", f"/third-party-{part}.json")["packages"]
        check(f"the About page has the {part}'s libraries", len(listed) > 0, f"{len(listed)}")

    settings = client.json("GET", "/api/v0/auth/settings")
    check("the server is empty", settings["first_user"] is True, settings)
    check("it asks for the setup token", settings["setup_token_required"] is True)
    refused, _, _ = client.request(
        "POST", "/api/v0/auth/register", {"username": "first-comer", "password": "a password"}
    )
    check("whoever comes first doesn't become administrator", refused == 403, refused)
    password = secrets.token_urlsafe(24)
    me = client.json(
        "POST",
        "/api/v0/auth/register",
        {
            "username": "smoke-admin",
            "password": password,
            "setup_token": os.environ.get("LOSSCAPE_SETUP_TOKEN"),
        },
    )
    check("the first account is administrator", me["is_admin"] is True)
    client.json("POST", "/api/v0/projects", {"name": PROJECT})
    key = client.json("POST", "/api/v0/me/api-keys", {"name": "smoke"})["secret"]
    check("project and API key created", key.startswith("df_"))

    run_id = log_a_run(client.base, key)
    state.write_text(json.dumps({"password": password, "key": key, "run": run_id}))
    os.chmod(state, 0o600)
    verify(client, run_id)


def log_a_run(base: str, key: str) -> str | None:
    try:
        import lsc
    except ImportError:
        print("     no SDK here: skipping the run")
        return None
    run = lsc.init(
        project=PROJECT,
        name="smoke",
        server=base,
        api_key=key,
        config={"lr": 0.001, "model": {"depth": 3}},
    )
    print("printed by the run, for its console")
    for step in range(STEPS):
        run.log({"train/loss": 1 / (step + 1), "val/acc": step / STEPS}, step=step)
    outcome = run.finish()
    check("the SDK uploaded a run", outcome == "uploaded", outcome)
    return run.id


def verify(client: Client, run_id: str | None) -> None:
    if run_id is None:
        return
    runs = client.json("GET", f"/api/v0/projects/{PROJECT}/runs?limit=10")["runs"]
    found = next((r for r in runs if r["id"].replace("-", "") == run_id.replace("-", "")), None)
    check("the run is listed", found is not None)
    check("finished", found["state"] == "finished", found["state"])
    check("every step is there", found["last_step"] == STEPS - 1, found["last_step"])
    check("with its config", found["config"]["model"]["depth"] == 3)
    logs = client.json("GET", f"/api/v0/runs/{found['id']}/logs?tail=50")
    check("its console too", any("printed by the run" in line["text"] for line in logs["lines"]))
    # The points themselves live in ClickHouse.
    metrics = client.json("POST", f"/api/v0/projects/{PROJECT}/metrics", {"runs": [found["id"]]})
    loss = next(m for m in metrics if m["key"] == "train/loss")
    check("its points are in ClickHouse", loss["max_step"] == STEPS - 1, loss["max_step"])


def after_restart(client: Client, state: Path) -> None:
    saved = json.loads(state.read_text())
    client.json(
        "POST", "/api/v0/auth/login", {"username": "smoke-admin", "password": saved["password"]}
    )
    check("the account survived", True)
    projects = client.json("GET", "/api/v0/projects")
    check("the project survived", any(p["name"] == PROJECT for p in projects))
    verify(client, saved["run"])


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("url", nargs="?", default="http://localhost:8080")
    parser.add_argument("--after-restart", action="store_true")
    parser.add_argument("--state", type=Path, default=Path("deploy-smoke-state.json"))
    args = parser.parse_args()

    client = Client(args.url)
    wait_until_ready(client)
    check("readyz: both databases answer", True)
    if args.after_restart:
        after_restart(client, args.state)
    else:
        first_start(client, args.state)
    print("all good")


if __name__ == "__main__":
    main()
