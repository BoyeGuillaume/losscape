"""losscape: self-hosted ML experiment tracking.

import lsc

with lsc.init(project="mnist", config={"lr": 1e-3}) as run:
    for step in range(1000):
        run.log({"train/loss": loss})

Reading runs back: ``lsc.Api().runs("mnist")`` (see :mod:`lsc.api`).
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

from lsc import utils
from lsc._native import PROTOCOL_VERSION, __version__, new_run_id
from lsc._run import Config, Run, Summary, init

if TYPE_CHECKING:
    from lsc.api import Api

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


def __getattr__(name: str) -> Any:
    # Loaded when first used: training scripts that only log don't pay for an HTTP client.
    if name == "Api":
        from lsc.api import Api

        return Api
    raise AttributeError(f"module 'lsc' has no attribute {name!r}")


__all__ = [
    "PROTOCOL_VERSION",
    "Api",
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
