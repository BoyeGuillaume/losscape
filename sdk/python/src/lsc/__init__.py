"""losscape: self-hosted ML experiment tracking.

import lsc

with lsc.init(project="mnist", config={"lr": 1e-3}) as run:
    for step in range(1000):
        run.log({"train/loss": loss})
"""

from __future__ import annotations

from typing import Any

from lsc import utils
from lsc._native import PROTOCOL_VERSION, __version__, new_run_id
from lsc._run import Config, Run, Summary, init

#: The run started last by :func:`init` (``None`` before).
run: Run | None = None


def log(data: dict[str, Any], step: int | None = None, commit: bool | None = None) -> None:
    """``run.log(...)`` on the current run."""
    if run is None:
        raise RuntimeError("call lsc.init() before lsc.log()")
    run.log(data, step=step, commit=commit)


def finish(exit_code: int = 0) -> str | None:
    """``run.finish(...)`` on the current run."""
    return run.finish(exit_code) if run is not None else None


__all__ = [
    "PROTOCOL_VERSION",
    "Config",
    "Run",
    "Summary",
    "__version__",
    "finish",
    "init",
    "log",
    "new_run_id",
    "run",
    "utils",
]
