"""Reading runs back from a server, for analysis in scripts and notebooks.

    import lsc

    api = lsc.Api()  # LOSSCAPE_SERVER and LOSSCAPE_API_KEY, as for lsc.init()
    runs = api.runs("mnist")
    best = max(runs, key=lambda r: r.summary.get("val/acc", 0))
    df = best.history(["train/loss", "val/acc"])  # pandas: pip install 'losscape-sdk[pandas]'
    folder = best.checkpoints[-1].download(files="model.pt")

Only the standard library is needed, except for ``to_pandas()`` and ``history()``.
"""

from __future__ import annotations

import csv
import io
import json
import os
import re
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Iterable, Iterator, Mapping
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Any, NamedTuple

from lsc import _native

__all__ = ["Api", "ApiError", "Checkpoint", "CheckpointFile", "Point", "Run", "Runs"]

#: Query strings are kept under this, as servers and proxies limit URLs.
_MAX_QUERY_BYTES = 6000
_CHECKPOINT_ID = re.compile(r"[0-9a-fA-F]{8}-?(?:[0-9a-fA-F]{4}-?){3}[0-9a-fA-F]{12}")
#: `<project>/<run>:<step, latest or best>[/<file>]`, as after `/ckpt/` in a link.
_NAMED_CHECKPOINT = re.compile(r"[^/:]+/[^/]+:[^/]+(/.*)?$")
_CHECKPOINT_LINK = re.compile(rf"/ckpt/({_CHECKPOINT_ID.pattern})(?:/|$)")


class ApiError(Exception):
    """The server refused a request, or couldn't be reached."""

    def __init__(self, message: str, status: int | None = None, code: str | None = None):
        super().__init__(message)
        self.status = status
        self.code = code


class Point(NamedTuple):
    """One logged value of a metric."""

    metric: str
    step: int
    time: datetime
    value: float


class CheckpointFile(NamedTuple):
    """A file of a checkpoint: its path in the checkpoint's folder, size in bytes and the
    BLAKE3 of its content (hex, as ``b3sum`` gives it), computed by the server as it arrived."""

    path: str
    size: int
    blake3: str


class Api:
    """A connection to a server, with an API key (web UI: Settings > API keys).

    ``server`` and ``api_key`` default to ``LOSSCAPE_SERVER`` (else http://localhost:8080)
    and ``LOSSCAPE_API_KEY``.
    """

    def __init__(
        self, server: str | None = None, api_key: str | None = None, *, timeout: float = 60
    ):
        api_key = api_key or os.environ.get("LOSSCAPE_API_KEY")
        if not api_key:
            raise ValueError("no API key: set LOSSCAPE_API_KEY or pass api_key=...")
        server = server or os.environ.get("LOSSCAPE_SERVER") or "http://localhost:8080"
        self.server = server.rstrip("/")
        self._key = api_key
        self.timeout = timeout

    def __repr__(self) -> str:
        return f"Api({self.server!r})"

    def projects(self) -> list[str]:
        """Names of the projects you can read."""
        return [p["name"] for p in self._json("/projects")]

    def runs(self, project: str, *, archived: bool = False) -> Runs:
        """Runs of a project, newest first (archived ones instead, if asked)."""
        page = self._json(
            f"/projects/{_quote(project)}/runs", {"archived": "true"} if archived else {}
        )
        return Runs(Run(self, project, r) for r in page["runs"])

    def run(self, run_id: str) -> Run:
        """One run, by id (as shown on its page)."""
        detail = self._json(f"/runs/{_quote(run_id)}")
        return Run(self, detail["project"], detail, detail=detail)

    def checkpoint(self, link: str) -> Checkpoint:
        """A checkpoint by its link, as copied from its run's Checkpoints tab:
        ``https://…/ckpt/<project>/<run>:<step>`` (or ``:latest``, ``:best``; the run by name,
        ``@alias`` or id), ``https://…/ckpt/<id>``, the part after ``/ckpt/``, or its id."""
        link = link.strip()
        found = _CHECKPOINT_LINK.search(link)
        if found or _CHECKPOINT_ID.fullmatch(link):
            checkpoint_id = found.group(1) if found else link
            return Checkpoint(self, self._json(f"/checkpoints/{checkpoint_id}"))
        if "/ckpt/" in link:
            named = link.split("/ckpt/", 1)[1]
        elif "://" in link:
            raise ValueError(f"{link!r}: not a checkpoint link (…/ckpt/…) or id")
        else:
            named = link
        if not _NAMED_CHECKPOINT.match(named):
            raise ValueError(f"{link!r}: not a checkpoint link or id")
        return Checkpoint(self, self._json("/checkpoints/resolve", {"ref": named}))

    # ---- HTTP

    def _open(
        self,
        path: str,
        params: Iterable[tuple[str, Any]] = (),
        *,
        method: str = "GET",
        body: Any = None,
        headers: Mapping[str, str] | None = None,
    ) -> Any:
        query = urllib.parse.urlencode(list(params))
        url = f"{self.server}/api/v0{path}" + (f"?{query}" if query else "")
        all_headers = {"Authorization": f"Bearer {self._key}", **(headers or {})}
        data = None
        if body is not None:
            data = json.dumps(body).encode()
            all_headers["Content-Type"] = "application/json"
        req = urllib.request.Request(url, data=data, headers=all_headers, method=method)
        try:
            return urllib.request.urlopen(req, timeout=self.timeout)  # noqa: S310 - http(s) only
        except urllib.error.HTTPError as err:
            try:
                body = json.loads(err.read())
                code, message = body.get("code"), body.get("message")
            except (ValueError, AttributeError):
                code, message = None, err.reason
            raise ApiError(f"{err.code} {message}", status=err.code, code=code) from None
        except urllib.error.URLError as err:
            raise ApiError(f"can't reach {self.server}: {err.reason}") from None

    def _json(self, path: str, params: Mapping[str, Any] | None = None) -> Any:
        with self._open(path, (params or {}).items()) as res:
            return json.load(res)


class Run:
    """A run as stored on the server. ``config`` and ``summary`` are plain dicts."""

    def __init__(self, api: Api, project: str, data: dict[str, Any], detail: dict | None = None):
        self._api = api
        self._detail = detail
        self.project = project
        # As lsc.init() gives them (no dashes), so that ids from both compare equal.
        self.id: str = data["id"].replace("-", "")
        self.name: str = data["name"]
        self.state: str = data["state"]
        self.group: str | None = data.get("group_name")
        self.tags: list[str] = list(data.get("tags") or [])
        self.config: dict[str, Any] = data.get("config") or {}
        self.summary: dict[str, Any] = data.get("summary") or {}
        self.last_step: int | None = data.get("last_step")
        self.created_at = _from_ms(data["created_at_ms"])
        self.finished_at = _from_ms(data.get("finished_at_ms"))

    def __repr__(self) -> str:
        return f"<Run {self.name} ({self.id}) {self.state}>"

    def _details(self) -> dict[str, Any]:
        if self._detail is None:
            self._detail = self._api._json(f"/runs/{_quote(self.id)}")
        return self._detail

    @property
    def notes(self) -> str:
        return self._details().get("notes") or ""

    @property
    def checkpoints(self) -> list[Checkpoint]:
        """Checkpoints uploaded for this run, by step."""
        listed = self._api._json(f"/runs/{_quote(self.id)}/checkpoints")
        return [Checkpoint(self._api, c) for c in listed]

    def checkpoint(self, step: int) -> Checkpoint:
        """The checkpoint at ``step``. Raises ``KeyError`` if there's none."""
        for c in self.checkpoints:
            if c.step == step:
                return c
        raise KeyError(f"run {self.name} has no checkpoint at step {step}")

    @property
    def metrics(self) -> dict[str, int]:
        """Logged metrics, with how many points each has."""
        return {m["key"]: m["points"] for m in self._details().get("metrics", [])}

    def _keys(self, keys: str | Iterable[str] | None, system: bool) -> list[str]:
        if keys is None:
            return [k for k in self.metrics if system or not k.startswith("system/")]
        return [keys] if isinstance(keys, str) else list(keys)

    def scan_history(
        self,
        keys: str | Iterable[str] | None = None,
        *,
        min_step: int | None = None,
        max_step: int | None = None,
        system: bool = False,
    ) -> Iterator[Point]:
        """Every logged point, by metric then step, as it downloads.

        ``keys``: metric names (default: all of them, system metrics only if ``system``).
        ``min_step`` and ``max_step`` are both included.
        """
        steps = [("min_step", min_step), ("max_step", max_step)]
        steps = [(k, v) for k, v in steps if v is not None]
        for chunk in _chunks(sorted(set(self._keys(keys, system)))):
            params = [("run", self.id), *(("key", k) for k in chunk), *steps]
            with self._api._open(f"/projects/{_quote(self.project)}/history", params) as res:
                rows = csv.reader(io.TextIOWrapper(res, encoding="utf-8", newline=""))
                next(rows, None)  # header: run, run_id, metric, step, time_utc, value
                for _run, _id, metric, step, time, value in rows:
                    yield Point(metric, int(step), _from_utc(time), float(value))

    def history(
        self,
        keys: str | Iterable[str] | None = None,
        *,
        min_step: int | None = None,
        max_step: int | None = None,
        system: bool = False,
        wide: bool = True,
    ) -> Any:
        """Every logged point as a pandas DataFrame: a ``step`` column and one column per
        metric (``wide``), or one row per point with ``metric``, ``step``, ``time`` and
        ``value``. Steps a metric wasn't logged at are NaN. Arguments as for
        :meth:`scan_history`.
        """
        pd = _pandas()
        long = pd.DataFrame(
            list(self.scan_history(keys, min_step=min_step, max_step=max_step, system=system)),
            columns=list(Point._fields),
        )
        if not wide:
            return long
        table = long.pivot(index="step", columns="metric", values="value")
        table.columns.name = None
        return table.reset_index()


class Checkpoint:
    """A checkpoint of a run: the files saved at a step, as uploaded with
    :meth:`lsc.Run.log_checkpoint`. ``size`` is in bytes (``stored_size``: compressed).

    ``url`` is its link by name, through the server (``…/ckpt/<project>/<run>:<step>``): the
    whole checkpoint as a tar archive, and ``<url>/<path>`` one file (with an API key:
    ``Authorization: Bearer …``); ``permanent_url`` is by id. ``important`` ones are never
    deleted by the project's retention; ``expires_at`` is when retention deletes it, if it
    does; ``best`` says it's its run's best. ``digest`` is the BLAKE3 of its files listed as
    ``b3sum`` lists them (``<hash>  <path>``, by path): it changes with any of them.
    ``corrupt`` says what storage gave back changed, if it ever did (downloads then fail rather
    than give it)."""

    def __init__(self, api: Api, data: dict[str, Any]):
        self._api = api
        self._update(data)
        self._files: list[dict[str, Any]] | None = data.get("files")

    def _update(self, data: dict[str, Any]) -> None:
        self.id: str = data["id"]
        self.run_id: str = data["run"].replace("-", "")
        self.step: int = data["step"]
        self.size: int = data["size"]
        self.stored_size: int = data["stored_size"]
        self.file_count: int = data["file_count"]
        self.created_at = _from_ms(data["created_at_ms"])
        self.important: bool = data.get("important", False)
        self.expires_at = _from_ms(data.get("expires_at_ms"))
        self.permanent_url: str = self._api.server + data.get("permanent_url", f"/ckpt/{self.id}")
        self.url: str = self._api.server + data.get("url", f"/ckpt/{self.id}")
        self.best: bool = data.get("best", False)
        self.digest: str | None = data.get("digest")
        self.corrupt: str | None = data.get("corrupt")

    def set_important(self, important: bool = True) -> None:
        """Marks it important (never deleted by retention), or not (needs edit rights)."""
        with self._api._open(
            f"/checkpoints/{_quote(self.id)}", method="PATCH", body={"important": important}
        ) as res:
            self._update(json.load(res))

    def __repr__(self) -> str:
        return f"<Checkpoint step {self.step} of run {self.run_id}, {self.file_count} files>"

    def _file_list(self) -> list[dict[str, Any]]:
        if self._files is None:
            self._files = self._api._json(f"/checkpoints/{_quote(self.id)}")["files"]
        return self._files

    @property
    def files(self) -> list[CheckpointFile]:
        return [CheckpointFile(f["path"], f["size"], f["blake3"]) for f in self._file_list()]

    def download(
        self, to: str | os.PathLike[str] | None = None, files: str | Iterable[str] | None = None
    ) -> Path:
        """Downloads the checkpoint's files (or only ``files``, paths as in :attr:`files`)
        into the folder ``to`` (default ``checkpoints/<run id>/<step>``), checking each
        against its BLAKE3. Returns the folder."""
        folder = Path(to) if to is not None else Path("checkpoints", self.run_id, str(self.step))
        wanted = None if files is None else {files} if isinstance(files, str) else set(files)
        chosen = [f for f in self._file_list() if wanted is None or f["path"] in wanted]
        missing = (wanted or set()) - {f["path"] for f in chosen}
        if missing:
            raise KeyError(f"not in this checkpoint: {', '.join(sorted(missing))}")
        for f in chosen:
            self._download_file(f, folder / _safe_path(f["path"]))
        return folder

    def _download_file(self, f: dict[str, Any], target: Path) -> None:
        target.parent.mkdir(parents=True, exist_ok=True)
        partial = target.with_name(target.name + ".part")
        digest = _native.Blake3()
        # What an interrupted download left is kept: the rest is asked for.
        have = partial.stat().st_size if partial.exists() else 0
        if have >= f["size"]:
            have = 0
        if have:
            with open(partial, "rb") as done:
                while chunk := done.read(1 << 20):
                    digest.update(chunk)
        headers = {"Range": f"bytes={have}-"} if have else None
        with self._api._open(
            f"/checkpoints/{_quote(self.id)}/files/{f['index']}", headers=headers
        ) as res:
            if have and res.status != 206:  # the whole file after all
                have, digest = 0, _native.Blake3()
            with open(partial, "ab" if have else "wb") as out:
                while chunk := res.read(1 << 20):
                    digest.update(chunk)
                    out.write(chunk)
        if digest.hexdigest() != f["blake3"]:
            partial.unlink()
            raise ApiError(f"{f['path']}: downloaded content doesn't match its BLAKE3")
        os.replace(partial, target)

    def delete(self) -> None:
        """Deletes the checkpoint from the server (needs edit rights on the project)."""
        self._api._open(f"/checkpoints/{_quote(self.id)}", method="DELETE").close()


def _safe_path(path: str) -> Path:
    """A checkpoint file's path, refused if it could land outside the download folder (the
    server already refuses those)."""
    parts = PurePosixPath(path).parts
    if not parts or path.startswith("/") or any(p in ("", ".", "..") for p in parts):
        raise ApiError(f"refusing to write {path!r} outside the download folder")
    if any("\\" in p or ":" in p for p in parts) and os.name == "nt":
        raise ApiError(f"refusing to write {path!r}: not a valid file name here")
    return Path(*parts)


class Runs(list[Run]):
    """A list of runs, which can become a table."""

    def to_pandas(self) -> Any:
        """One row per run: id, name, state, group, tags and times, then ``config.*`` and
        ``summary.*`` columns (nested settings flattened: ``config.optimizer.lr``)."""
        pd = _pandas()
        rows = []
        for r in self:
            row: dict[str, Any] = {
                "id": r.id,
                "name": r.name,
                "state": r.state,
                "group": r.group,
                "tags": r.tags,
                "created_at": r.created_at,
                "finished_at": r.finished_at,
                "last_step": r.last_step,
            }
            row.update(_flatten("config", r.config))
            row.update(_flatten("summary", r.summary))
            rows.append(row)
        return pd.DataFrame(rows)


def _flatten(prefix: str, value: Any) -> dict[str, Any]:
    if isinstance(value, dict) and value:
        out: dict[str, Any] = {}
        for k, v in value.items():
            out.update(_flatten(f"{prefix}.{k}", v))
        return out
    return {prefix: value}


def _chunks(keys: list[str]) -> Iterator[list[str]]:
    """Metric names in groups whose query strings stay short enough for any server."""
    chunk: list[str] = []
    size = 0
    for key in keys:
        length = len(urllib.parse.quote(key)) + 5
        if chunk and size + length > _MAX_QUERY_BYTES:
            yield chunk
            chunk, size = [], 0
        chunk.append(key)
        size += length
    if chunk:
        yield chunk


def _quote(part: str) -> str:
    return urllib.parse.quote(part, safe="")


def _from_ms(ms: int | None) -> datetime | None:
    return None if ms is None else datetime.fromtimestamp(ms / 1000, tz=timezone.utc)


def _from_utc(text: str) -> datetime:
    return datetime.fromisoformat(text).replace(tzinfo=timezone.utc)


def _pandas() -> Any:
    try:
        import pandas
    except ImportError:
        raise ImportError("this needs pandas: pip install 'losscape-sdk[pandas]'") from None
    return pandas
