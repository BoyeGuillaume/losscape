from os import PathLike

__version__: str
PROTOCOL_VERSION: int

def new_run_id() -> str: ...

class NativeRun:
    @property
    def id(self) -> str: ...
    @property
    def dir(self) -> str: ...
    @property
    def next_step(self) -> int | None: ...
    def log(self, data: dict, step: int) -> None: ...
    def update_summary(self, json: str) -> None: ...
    def update_config(self, json: str) -> None: ...
    def update_models(self, json: str) -> None: ...
    def finish(self, state: str, exit_code: int, timeout: float) -> tuple[str, str, int]: ...

def start_run(
    *,
    project: str,
    root: str | PathLike[str],
    server: str | None = None,
    api_key: str | None = None,
    run_id: str | None = None,
    resume: bool = False,
    name: str = "",
    group: str = "",
    tags: list[str] = ...,
    notes: str = "",
    config_json: str = "",
    python_version: str = "",
    sdk_version: str = "",
    console: bool = False,
    system_interval: float | None = None,
    python_path: list[str] = ...,
    details_json: str = "",
    git_diff: bool = True,
) -> NativeRun: ...
def sync(root: str | PathLike[str], server: str, api_key: str) -> list[tuple[str, str, str]]: ...
