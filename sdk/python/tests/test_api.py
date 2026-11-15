"""The query API (lsc.Api) against a fake server answering as the real one does."""

import json
import math
import subprocess
import sys
import threading
import urllib.parse
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest

from lsc import _native
from lsc.api import Api, ApiError, Point

KEY = "df_test"
RUN = "01a1a361-acce-72d1-ada3-ac673877effb"
POINTS = {
    "train/loss": [(0, 2.0), (1, 1.5), (2, float("nan")), (3, 1.0)],
    "val/acc": [(1, 0.5), (3, 0.7)],
    "system/cpu/util": [(0, 40.0)],
}
RUN_JSON = {
    "id": RUN,
    "name": "brave-falcon-1",
    "state": "finished",
    "group_name": "sweep",
    "tags": ["baseline"],
    "created_at_ms": 1_700_000_000_000,
    "finished_at_ms": 1_700_000_360_000,
    "last_step": 3,
    "config": {"optimizer": {"lr": 0.001, "name": "adam"}, "batch_size": 32},
    "summary": {"val/acc": 0.7},
}


def b3(data: bytes) -> str:
    h = _native.Blake3()
    h.update(data)
    return h.hexdigest()


CKPT = "01a1a361-acce-72d1-ada3-000000000001"
CKPT_FILES = {"model.pt": b"weights" * 1000, "tok/vocab.json": b"{}"}
CKPT_JSON = {
    "id": CKPT,
    "run": RUN,
    "step": 100,
    "state": "committed",
    "size": sum(map(len, CKPT_FILES.values())),
    "stored_size": 40,
    "file_count": 2,
    "created_at_ms": 1_700_000_000_000,
    "committed_at_ms": 1_700_000_001_000,
    "important": False,
    "keep": [],
    "expires_at_ms": None,
    "url": f"/ckpt/{CKPT}",
}
CKPT_DETAIL = {
    **CKPT_JSON,
    "files": [
        {"index": i, "path": p, "size": len(b), "blake3": b3(b)}
        for i, (p, b) in enumerate(CKPT_FILES.items())
    ],
}


class FakeServer(BaseHTTPRequestHandler):
    requests: list[str] = []
    # What file downloads answer instead, by index (to corrupt one).
    served: dict[int, bytes] = {}
    deleted: list[str] = []

    def log_message(self, *args):
        pass

    def send(self, status, body, kind="application/json"):
        data = body if isinstance(body, bytes) else json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        url = urllib.parse.urlsplit(self.path)
        query = urllib.parse.parse_qs(url.query)
        FakeServer.requests.append(self.path)
        if self.headers.get("Authorization") != f"Bearer {KEY}":
            return self.send(
                401, {"code": "unauthorized", "message": "sign in, or send an API key"}
            )
        if url.path == "/api/v1/projects":
            return self.send(200, [{"name": "mnist", "role": "owner", "runs": 1}])
        if url.path == "/api/v1/projects/mnist/runs":
            archived = query.get("archived") == ["true"]
            return self.send(200, {"runs": [] if archived else [RUN_JSON], "next": None})
        if url.path in (f"/api/v1/runs/{RUN}", f"/api/v1/runs/{RUN.replace('-', '')}"):
            metrics = [{"key": k, "points": len(v)} for k, v in sorted(POINTS.items())]
            return self.send(
                200, {**RUN_JSON, "project": "mnist", "notes": "hi", "metrics": metrics}
            )
        if url.path == "/api/v1/projects/mnist/history":
            lo = int(query.get("min_step", ["0"])[0])
            hi = int(query.get("max_step", [str(2**64)])[0])
            lines = ['"run","run_id","metric","step","time_utc","value"']
            for key in sorted(query.get("key", [])):
                for step, value in POINTS.get(key, []):
                    if lo <= step <= hi:
                        text = "nan" if math.isnan(value) else repr(value)
                        time = f"2023-11-14 22:13:2{step}.000"
                        lines.append(f'"brave-falcon-1","{RUN}","{key}",{step},"{time}",{text}')
            return self.send(200, ("\n".join(lines) + "\n").encode(), "text/csv")
        if url.path == "/api/v1/checkpoints/resolve":
            FakeServer.resolved.append(query["ref"][0])
            return self.send(200, FakeServer.detail)
        if url.path == f"/api/v1/runs/{RUN.replace('-', '')}/checkpoints":
            return self.send(200, [CKPT_JSON])
        if url.path in (
            f"/api/v1/checkpoints/{CKPT}",
            f"/api/v1/checkpoints/{CKPT.replace('-', '')}",
        ):
            return self.send(200, FakeServer.detail)
        if url.path.startswith(f"/api/v1/checkpoints/{CKPT}/files/"):
            index = int(url.path.rsplit("/", 1)[1])
            body = FakeServer.served.get(index, list(CKPT_FILES.values())[index])
            ranged = self.headers.get("Range")
            if ranged:
                FakeServer.ranges.append(ranged)
                start = int(ranged.removeprefix("bytes=").rstrip("-"))
                return self.send(206, body[start:], "application/octet-stream")
            return self.send(200, body, "application/octet-stream")
        return self.send(404, {"code": "unknown_endpoint", "message": "no such endpoint"})

    def do_PATCH(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        FakeServer.patched.append((self.path, body))
        self.send(200, {**CKPT_JSON, "important": body["important"], "keep": ["important"]})

    def do_DELETE(self):
        FakeServer.deleted.append(self.path)
        self.send_response(204)
        self.end_headers()


@pytest.fixture
def api():
    server = ThreadingHTTPServer(("127.0.0.1", 0), FakeServer)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    FakeServer.requests = []
    FakeServer.served = {}
    FakeServer.deleted = []
    FakeServer.patched = []
    FakeServer.resolved = []
    FakeServer.ranges = []
    FakeServer.detail = CKPT_DETAIL
    yield Api(f"http://127.0.0.1:{server.server_address[1]}", KEY)
    server.shutdown()


def test_runs_and_their_details(api):
    assert api.projects() == ["mnist"]
    runs = api.runs("mnist")
    assert [r.name for r in runs] == ["brave-falcon-1"]
    run = runs[0]
    assert (run.state, run.group, run.tags) == ("finished", "sweep", ["baseline"])
    assert run.id == RUN.replace("-", ""), "the form lsc.init() gives"
    assert run.created_at == datetime(2023, 11, 14, 22, 13, 20, tzinfo=timezone.utc)
    assert run.config["optimizer"]["lr"] == 0.001
    assert run.summary == {"val/acc": 0.7}
    assert run.notes == "hi", "details are fetched when first needed"
    assert run.metrics["train/loss"] == 4
    assert api.runs("mnist", archived=True) == []
    assert api.run(RUN).project == "mnist"


def test_scan_history_streams_points(api):
    points = list(api.run(RUN).scan_history(["val/acc", "train/loss"]))
    assert points[0] == Point(
        "train/loss", 0, datetime(2023, 11, 14, 22, 13, 20, tzinfo=timezone.utc), 2.0
    )
    assert [(p.metric, p.step) for p in points] == [
        ("train/loss", 0),
        ("train/loss", 1),
        ("train/loss", 2),
        ("train/loss", 3),
        ("val/acc", 1),
        ("val/acc", 3),
    ]
    assert math.isnan(points[2].value), "NaN is kept as logged"
    some = list(api.run(RUN).scan_history("train/loss", min_step=1, max_step=2))
    assert [p.step for p in some] == [1, 2]


def test_history_as_a_table(api):
    pytest.importorskip("pandas")
    run = api.run(RUN)
    df = run.history()
    assert list(df.columns) == ["step", "train/loss", "val/acc"], "system metrics left out"
    assert df["step"].tolist() == [0, 1, 2, 3]
    assert df["val/acc"].isna().tolist() == [True, False, True, False]
    assert "system/cpu/util" in run.history(system=True).columns
    long = run.history("val/acc", wide=False)
    assert list(long.columns) == ["metric", "step", "time", "value"]
    assert long["value"].tolist() == [0.5, 0.7]


def test_runs_as_a_table(api):
    pytest.importorskip("pandas")
    df = api.runs("mnist").to_pandas()
    assert df.loc[0, "config.optimizer.lr"] == 0.001
    assert df.loc[0, "config.batch_size"] == 32
    assert df.loc[0, "summary.val/acc"] == 0.7


def test_many_metrics_are_asked_for_in_parts(api):
    keys = [f"layer{i:03}/" + "x" * 200 for i in range(100)]
    assert list(api.run(RUN).scan_history(keys)) == []
    asked = [r for r in FakeServer.requests if "/history" in r]
    assert len(asked) > 1
    assert all(len(r) < 8000 for r in asked)
    names = [k for r in asked for k in urllib.parse.parse_qs(urllib.parse.urlsplit(r).query)["key"]]
    assert names == sorted(keys), "each metric asked for once"


def test_errors(api):
    with pytest.raises(ApiError) as refused:
        Api(api.server, "df_wrong").runs("mnist")
    assert (refused.value.status, refused.value.code) == (401, "unauthorized")
    with pytest.raises(ApiError, match="can't reach"):
        Api("http://127.0.0.1:1", KEY).projects()
    with pytest.raises(ValueError, match="no API key"):
        Api("http://127.0.0.1:1", "")


def test_logging_doesnt_load_the_http_client():
    code = "import sys, lsc; print('urllib.request' in sys.modules); print(lsc.Api.__name__)"
    out = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, check=True)
    assert out.stdout.split() == ["False", "Api"]


def test_checkpoints_download_and_delete(api, tmp_path):
    (ckpt,) = api.run(RUN).checkpoints
    assert (ckpt.step, ckpt.file_count, ckpt.run_id) == (100, 2, RUN.replace("-", ""))
    assert [f.path for f in ckpt.files] == ["model.pt", "tok/vocab.json"]
    assert api.run(RUN).checkpoint(100).id == CKPT
    with pytest.raises(KeyError):
        api.run(RUN).checkpoint(5)

    folder = ckpt.download(tmp_path / "all")
    assert (folder / "model.pt").read_bytes() == CKPT_FILES["model.pt"]
    assert (folder / "tok" / "vocab.json").read_bytes() == b"{}"
    some = ckpt.download(tmp_path / "some", files="tok/vocab.json")
    assert [p.name for p in some.rglob("*") if p.is_file()] == ["vocab.json"]
    with pytest.raises(KeyError, match="nope"):
        ckpt.download(tmp_path / "x", files=["nope"])

    ckpt.delete()
    assert FakeServer.deleted == [f"/api/v1/checkpoints/{CKPT}"]


def test_checkpoint_downloads_are_checked(api, tmp_path):
    ckpt = api.run(RUN).checkpoints[0]
    FakeServer.served = {0: b"tampered"}
    with pytest.raises(ApiError, match="BLAKE3"):
        ckpt.download(tmp_path, files="model.pt")
    assert list(tmp_path.iterdir()) == [], "nothing left behind"

    for path in ["../escape", "/etc/passwd", "a/../../b"]:
        FakeServer.detail = {**CKPT_DETAIL, "files": [{**CKPT_DETAIL["files"][1], "path": path}]}
        with pytest.raises(ApiError, match="outside"):
            api.run(RUN).checkpoints[0].download(tmp_path / "in")
    assert not (tmp_path / "escape").exists()


def test_checkpoints_by_link_marked_important_and_resumed(api, tmp_path):
    for link in [
        f"{api.server}/ckpt/{CKPT}",
        f"{api.server}/ckpt/{CKPT}/model.pt",
        f"/ckpt/{CKPT}",
        CKPT,
        CKPT.replace("-", ""),
        f"{api.server}/ckpt/mnist/brave-falcon-1:100",
        f"{api.server}/ckpt/mnist/@best:latest/model.pt",
        "mnist/brave-falcon-1:best",
    ]:
        ckpt = api.checkpoint(link)
        assert (ckpt.id, ckpt.step) == (CKPT, 100), link
    assert FakeServer.resolved == [
        "mnist/brave-falcon-1:100",
        "mnist/@best:latest/model.pt",
        "mnist/brave-falcon-1:best",
    ], "names are resolved by the server"
    assert ckpt.url == f"{api.server}/ckpt/{CKPT}"
    assert not ckpt.important
    with pytest.raises(ValueError, match="not a checkpoint"):
        api.checkpoint("https://example.com/runs/1")

    ckpt.set_important()
    assert FakeServer.patched == [(f"/api/v1/checkpoints/{CKPT}", {"important": True})]
    assert ckpt.important

    # An interrupted download: the rest is asked for, and the whole is checked.
    weights = CKPT_FILES["model.pt"]
    (tmp_path / "model.pt.part").write_bytes(weights[:1234])
    ckpt.download(tmp_path, files="model.pt")
    assert (tmp_path / "model.pt").read_bytes() == weights
    assert FakeServer.ranges == ["bytes=1234-"]
    assert not (tmp_path / "model.pt.part").exists()
