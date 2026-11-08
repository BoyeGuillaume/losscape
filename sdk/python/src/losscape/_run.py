"""Runs: ``init``, ``Run.log``, ``Run.finish`` and friends."""

from __future__ import annotations

import atexit
import json
import math
import os
import platform
import re
import sys
import threading
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from losscape import _native
from losscape.utils import random_name

_PROJECT_RE = re.compile(r"^[A-Za-z0-9._-]{1,100}$")
_MODES = ("online", "offline", "disabled")


class _Process:
    """Updated after fork, so a run created in the parent knows it can't log in a child."""

    pid = os.getpid()


def _after_fork_in_child() -> None:
    _Process.pid = os.getpid()


if hasattr(os, "register_at_fork"):
    os.register_at_fork(after_in_child=_after_fork_in_child)


def _say(message: str) -> None:
    if not os.environ.get("LOSSCAPE_SILENT"):
        print(f"losscape: {message}", file=sys.stderr, flush=True)


def _jsonable(value: Any) -> Any:
    """Plain JSON types; numpy/torch scalars and arrays via .item()/.tolist(); NaN/Inf
    become None; anything else becomes its str()."""
    if value is None or isinstance(value, (bool, int, str)):
        return value
    if isinstance(value, float):
        return value if math.isfinite(value) else None
    if isinstance(value, Mapping):
        return {str(k): _jsonable(v) for k, v in value.items()}
    if isinstance(value, (list, tuple, set, frozenset)):
        return [_jsonable(v) for v in value]
    if isinstance(value, Path):
        return str(value)
    for method in ("item", "tolist"):  # numpy / torch
        convert = getattr(value, method, None)
        if callable(convert):
            try:
                return _jsonable(convert())
            except (TypeError, ValueError):
                pass
    return str(value)


def _to_json(update: Mapping[str, Any]) -> str:
    return json.dumps(_jsonable(update))


class _SyncedDict(dict):
    """A dict whose changes are sent to the run; keys are also readable as attributes."""

    def __init__(self, run: Run, initial: Mapping[str, Any], send) -> None:
        super().__init__(initial)
        object.__setattr__(self, "_run", run)
        object.__setattr__(self, "_send", send)

    def __getattr__(self, name: str) -> Any:
        try:
            return self[name]
        except KeyError:
            raise AttributeError(name) from None

    def __setattr__(self, name: str, value: Any) -> None:
        self[name] = value

    def __setitem__(self, key: str, value: Any) -> None:
        super().__setitem__(key, value)
        self._send({key: value})

    def update(self, other: Mapping[str, Any] | None = None, **kwargs: Any) -> None:
        changes = {**(dict(other) if other else {}), **kwargs}
        super().update(changes)
        self._send(changes)

    def setdefault(self, key: str, default: Any = None) -> Any:
        if key not in self:
            self[key] = default
        return self[key]


class Config(_SyncedDict):
    """Hyperparameters of a run: ``run.config.lr`` or ``run.config["lr"]``."""


class Summary(_SyncedDict):
    """Final values of a run. Every logged metric's last value is added automatically on
    the server; set extra values here (``run.summary["best_acc"] = 0.93``)."""


class Run:
    """A training run. Create it with :func:`losscape.init`."""

    def __init__(
        self,
        native: Any,
        *,
        project: str,
        name: str,
        config: Mapping[str, Any],
        mode: str,
        start_step: int = 0,
    ) -> None:
        self._native = native
        self._mode = mode
        self._pid = os.getpid()
        self._step = start_step
        self._finished = False
        self._warned_fork = False
        self.id: str = native.id if native is not None else _native.new_run_id()
        self.name = name
        self.project = project
        self.config = Config(self, config, self._send_config)
        self.summary = Summary(self, {}, self._send_summary)

    def __repr__(self) -> str:
        return f"<losscape.Run {self.name} ({self.id}) project={self.project} mode={self._mode}>"

    @property
    def step(self) -> int:
        """Step the next ``log()`` without an explicit ``step`` will use."""
        return self._step

    @property
    def dir(self) -> Path | None:
        """Local log of this run, until it is uploaded."""
        return Path(self._native.dir) if self._native is not None else None

    def log(
        self, data: Mapping[str, Any], step: int | None = None, commit: bool | None = None
    ) -> None:
        """Log numbers at a step. Nested dicts are flattened with ``/``.

        Without ``step``, uses :attr:`step` and then advances it, unless ``commit=False``
        (more values for the same step follow). With ``step``, logs there and continues
        after it. Never blocks on the network.
        """
        native = self._native
        if native is None or self._finished:
            return
        if self._pid != _Process.pid:
            self._warn_fork()
            return
        if step is None:
            step = self._step
            if commit is not False:
                self._step = step + 1
        else:
            if step < 0:
                raise ValueError("step must be >= 0")
            self._step = step if commit is False else step + 1
        if type(data) is not dict:
            if not isinstance(data, Mapping):
                raise TypeError(f"log() takes a dict of metrics, got {type(data).__name__}")
            data = dict(data)
        native.log(data, step)

    def finish(
        self, exit_code: int = 0, *, state: str | None = None, timeout: float | None = None
    ) -> str:
        """End the run: flush everything, wait for the upload (up to ``timeout`` seconds,
        default ``LOSSCAPE_FINISH_TIMEOUT`` or 60). Whatever isn't uploaded by then stays
        in the local log for ``losscape sync``. Returns "uploaded", "saved_locally",
        "failed" or "disabled"."""
        if self._finished:
            return "finished"
        self._finished = True
        _active_runs.discard(self)
        if self._native is None:
            return "disabled"
        if self._pid != _Process.pid:
            return "failed"
        if state is None:
            state = "finished" if exit_code == 0 else "failed"
        if timeout is None:
            timeout = float(os.environ.get("LOSSCAPE_FINISH_TIMEOUT", "60"))
        # Output still in Python's buffers goes into the captured console.
        for stream in (sys.stdout, sys.stderr):
            try:
                stream.flush()
            except (AttributeError, OSError, ValueError):
                pass

        outcome, message, pending = self._native.finish(state, exit_code, timeout)
        if outcome == "uploaded":
            _say(f"run {self.name} uploaded")
        elif outcome == "saved_locally":
            if message != "offline mode":
                message = f"{pending} batch(es) not uploaded yet ({message})"
            _say(f"run {self.name}: {message}; saved in {self._native.dir}.")
            _say("upload it with `losscape sync`")
        else:
            _say(f"run {self.name}: upload failed: {message}. Data kept in {self._native.dir}.")
        return outcome

    def __enter__(self) -> Run:
        return self

    def __exit__(self, exc_type, exc, tb) -> bool:
        if exc_type is None:
            self.finish()
        elif issubclass(exc_type, KeyboardInterrupt):
            self.finish(exit_code=130, state="killed")
        else:
            self.finish(exit_code=1)
        return False

    def _send_config(self, update: Mapping[str, Any]) -> None:
        if self._native is not None and not self._finished and self._pid == _Process.pid:
            self._native.update_config(_to_json(update))

    def _send_summary(self, update: Mapping[str, Any]) -> None:
        if self._native is not None and not self._finished and self._pid == _Process.pid:
            self._native.update_summary(_to_json(update))

    def _warn_fork(self) -> None:
        if not self._warned_fork:
            self._warned_fork = True
            _say(
                f"run {self.name} was created in process {self._pid}; logging from a forked "
                f"child ({_Process.pid}) is ignored. Log from the parent, or call "
                "losscape.init() in the child."
            )


_active_runs: set[Run] = set()
_lock = threading.Lock()


class _Exit:
    """How the interpreter is exiting, recorded by the excepthook for the atexit handler."""

    exit_code = 0
    state = "finished"


def _excepthook(exc_type, exc, tb) -> None:
    if issubclass(exc_type, KeyboardInterrupt):
        _Exit.exit_code, _Exit.state = 130, "killed"
    else:
        _Exit.exit_code, _Exit.state = 1, "failed"
    _previous_excepthook(exc_type, exc, tb)


_previous_excepthook = sys.excepthook
sys.excepthook = _excepthook


@atexit.register
def _finish_at_exit() -> None:
    for run in list(_active_runs):
        if run._pid == _Process.pid:
            run.finish(exit_code=_Exit.exit_code, state=_Exit.state)


def _flag(value: bool | None, env: str, default: bool) -> bool:
    if value is not None:
        return value
    raw = os.environ.get(env, "").strip().lower()
    if raw in ("0", "false", "off", "no"):
        return False
    if raw in ("1", "true", "on", "yes"):
        return True
    return default


def _console_setting(console: bool | None) -> tuple[bool, str | None]:
    """Whether to capture the console, and why not. Off by default where it can't work
    well: Windows (no file-descriptor capture yet) and Jupyter (the kernel captures the
    descriptors itself)."""
    default_off = None
    if os.name == "nt":
        default_off = "windows"
    elif "ipykernel" in sys.modules:
        default_off = "notebook"
    on = _flag(console, "LOSSCAPE_CONSOLE", default_off is None)
    if on:
        return True, None
    if console is False:
        return False, "console=False"
    if not _flag(None, "LOSSCAPE_CONSOLE", True):
        return False, "LOSSCAPE_CONSOLE"
    return False, default_off


def _python_details() -> dict[str, Any]:
    """Details only Python knows. Frameworks are described only if already imported:
    importing (or initializing CUDA) here would slow down init and break fork safety."""
    details: dict[str, Any] = {
        "python": {
            "executable": sys.executable,
            "implementation": platform.python_implementation(),
        }
    }
    torch = sys.modules.get("torch")
    if torch is not None:
        version = getattr(torch, "version", None)
        details["torch"] = {
            "version": getattr(torch, "__version__", None),
            "cuda": getattr(version, "cuda", None),
        }
    return details


def init(
    project: str | None = None,
    *,
    name: str | None = None,
    config: Mapping[str, Any] | None = None,
    tags: list[str] | None = None,
    group: str | None = None,
    notes: str | None = None,
    id: str | None = None,  # noqa: A002 - matches the W&B API
    resume: bool = False,
    mode: str | None = None,
    dir: str | os.PathLike[str] | None = None,  # noqa: A002
    server: str | None = None,
    api_key: str | None = None,
    console: bool | None = None,
    system_metrics: bool | None = None,
) -> Run:
    """Start a run. Returns immediately; the server is contacted in the background.

    Settings fall back to environment variables: ``LOSSCAPE_PROJECT``,
    ``LOSSCAPE_MODE`` (online | offline | disabled), ``LOSSCAPE_DIR`` (local logs,
    default ``./.losscape``), ``LOSSCAPE_SERVER`` (default http://localhost:8080),
    ``LOSSCAPE_API_KEY``, ``LOSSCAPE_CONSOLE`` (capture stdout/stderr; default on,
    except on Windows and in Jupyter), ``LOSSCAPE_SYSTEM_METRICS`` (CPU, memory, disk,
    network, NVIDIA GPUs; default on) and ``LOSSCAPE_SYSTEM_INTERVAL`` (seconds, 5).
    """
    import losscape

    project = project or os.environ.get("LOSSCAPE_PROJECT") or "uncategorized"
    if not _PROJECT_RE.match(project):
        raise ValueError(f"project {project!r}: use 1-100 characters from [A-Za-z0-9._-]")
    mode = mode or os.environ.get("LOSSCAPE_MODE") or "online"
    if mode not in _MODES:
        raise ValueError(f"mode must be one of {', '.join(_MODES)}, got {mode!r}")
    name = name or random_name()
    config = dict(config or {})

    if mode == "disabled":
        run = Run(None, project=project, name=name, config=config, mode=mode)
        losscape.run = run
        return run

    server_args: dict[str, str] = {}
    if mode == "online":
        api_key = api_key or os.environ.get("LOSSCAPE_API_KEY")
        if not api_key:
            raise ValueError(
                "no API key: set LOSSCAPE_API_KEY or pass api_key=..., "
                "or use mode='offline' and upload later with `losscape sync`"
            )
        server_args = {
            "server": server or os.environ.get("LOSSCAPE_SERVER") or "http://localhost:8080",
            "api_key": api_key,
        }

    interval = float(os.environ.get("LOSSCAPE_SYSTEM_INTERVAL") or 5)
    capture_console, console_off = _console_setting(console)
    details = _python_details()
    if console_off:
        # Shown on the run's Logs tab; the core adds whether capture started.
        details["console"] = {"off": console_off}
    native = _native.start_run(
        project=project,
        root=Path(dir or os.environ.get("LOSSCAPE_DIR") or ".losscape"),
        run_id=id,
        resume=resume,
        name=name,
        group=group or "",
        tags=list(tags or []),
        notes=notes or "",
        config_json=_to_json(config) if config else "",
        python_version=platform.python_version(),
        sdk_version=_native.__version__,
        console=capture_console,
        system_interval=interval
        if _flag(system_metrics, "LOSSCAPE_SYSTEM_METRICS", True)
        else None,
        python_path=[p for p in sys.path if p],
        details_json=json.dumps(_jsonable(details)),
        **server_args,
    )
    start_step = native.next_step or 0
    run = Run(native, project=project, name=name, config=config, mode=mode, start_step=start_step)
    with _lock:
        _active_runs.add(run)
    losscape.run = run
    where = f"offline, saving to {native.dir}" if mode == "offline" else server_args["server"]
    resumed = f", resuming at step {start_step}" if resume else ""
    _say(f"run {name} ({run.id}) in project {project} -> {where}{resumed}")
    return run
