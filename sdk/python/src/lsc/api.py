"""Reading runs back from a server, for analysis in scripts and notebooks.

    import lsc

    api = lsc.Api()  # LOSSCAPE_SERVER and LOSSCAPE_API_KEY, as for lsc.init()
    runs = api.runs("mnist")
    best = max(runs, key=lambda r: r.summary.get("val/acc", 0))
    df = best.history(["train/loss", "val/acc"])  # pandas: pip install 'losscape-sdk[pandas]'

Only the standard library is needed, except for ``to_pandas()`` and ``history()``.
"""

from __future__ import annotations

import csv
import io
import json
import os
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Iterable, Iterator, Mapping
from datetime import datetime, timezone
from typing import Any, NamedTuple

__all__ = ["Api", "ApiError", "Point", "Run", "Runs"]

#: Query strings are kept under this, as servers and proxies limit URLs.
_MAX_QUERY_BYTES = 6000


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

    # ---- HTTP

    def _open(self, path: str, params: Iterable[tuple[str, Any]] = ()) -> Any:
        query = urllib.parse.urlencode(list(params))
        url = f"{self.server}/api/v0{path}" + (f"?{query}" if query else "")
        req = urllib.request.Request(url, headers={"Authorization": f"Bearer {self._key}"})
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
