import math
import os
import random
import re
import subprocess
import sys
import threading

import pytest

import losscape
from losscape.utils import random_name


def test_version_and_ids():
    assert losscape.__version__
    assert losscape.PROTOCOL_VERSION == 0
    ids = [losscape.new_run_id() for _ in range(100)]
    assert len(set(ids)) == 100
    assert ids == sorted(ids), "UUIDv7 ids sort by creation time"


def test_random_name():
    assert re.fullmatch(r"[a-z]+-[a-z]+-\d{1,2}", random_name())
    assert random_name(seed=1) == random_name(seed=1)
    random.seed(0)
    first = random_name()
    random.seed(0)
    assert random_name() != first or random_name() != first, "not tied to the global seed"


def test_steps_follow_wandb_semantics(offline):
    run = losscape.init(project="t")
    run.log({"a": 1})
    run.log({"a": 2})
    assert run.step == 2
    run.log({"b": 1}, commit=False)
    run.log({"c": 1})
    assert run.step == 3, "commit=False keeps the step open"
    run.log({"a": 3}, step=10)
    assert run.step == 11
    run.log({"a": 4})
    assert run.step == 12
    with pytest.raises(ValueError):
        run.log({"a": 1}, step=-1)
    assert run.finish() == "saved_locally"


def test_logs_numbers_of_any_kind(offline):
    class Scalar:  # like numpy / torch scalars
        def __float__(self):
            return 0.5

    run = losscape.init(project="t")
    run.log({"int": 3, "float": 0.1, "bool": True, "nan": math.nan, "custom": Scalar()})
    run.log({"train": {"loss": 0.1, "inner": {"x": 1}}})
    with pytest.raises(TypeError, match="not a number"):
        run.log({"ok": 1.0, "text": "hello"})
    with pytest.raises(TypeError, match="metric names must be str"):
        run.log({1: 1.0})
    with pytest.raises(TypeError, match="dict of metrics"):
        run.log([1, 2])
    run.finish()


def test_config_and_summary(offline):
    run = losscape.init(project="t", config={"lr": 1e-3, "layers": [64, 64]})
    assert run.config.lr == 1e-3
    run.config.batch_size = 32
    run.config.update({"optimizer": "adam"}, dropout=0.1)
    assert run.config["batch_size"] == 32 and run.config.dropout == 0.1
    run.summary["best"] = math.inf  # not valid JSON: sent as null
    run.summary.update(epoch=3)
    with pytest.raises(AttributeError):
        run.config.missing  # noqa: B018
    run.finish()


def test_offline_files_and_module_api(offline):
    run = losscape.init(project="t")
    assert losscape.run is run
    losscape.log({"loss": 1.0})
    assert run.dir is not None and run.dir.parent.parent == offline
    assert losscape.finish() == "saved_locally"
    files = sorted(p.name for p in run.dir.iterdir())
    assert "init.pb" in files and "finish.pb" in files
    assert any(f.startswith("seg-") for f in files)
    assert run.finish() == "finished", "second finish is a no-op"


def test_disabled_mode_writes_nothing(tmp_path, monkeypatch):
    monkeypatch.setenv("LOSSCAPE_DIR", str(tmp_path))
    run = losscape.init(project="t", mode="disabled")
    run.log({"loss": 1.0})
    assert run.finish() == "disabled"
    assert list(tmp_path.iterdir()) == []


def test_invalid_settings(offline):
    with pytest.raises(ValueError, match="project"):
        losscape.init(project="has space")
    with pytest.raises(ValueError, match="mode"):
        losscape.init(project="t", mode="sometimes")


def test_online_needs_api_key(monkeypatch):
    monkeypatch.delenv("LOSSCAPE_API_KEY", raising=False)
    with pytest.raises(ValueError, match="API key"):
        losscape.init(project="t", mode="online")


def test_context_manager_records_failure(offline):
    with pytest.raises(RuntimeError):
        with losscape.init(project="t") as run:
            run.log({"loss": 1.0})
            raise RuntimeError("boom")
    assert run._finished


@pytest.mark.skipif(not hasattr(os, "fork"), reason="needs fork")
@pytest.mark.filterwarnings("ignore:This process .* is multi-threaded")
def test_forked_child_does_not_touch_the_parent_run(offline):
    run = losscape.init(project="t")
    run.log({"before": 1})
    pid = os.fork()
    if pid == 0:  # child: logging is ignored, nothing hangs
        run.log({"in_child": 1})
        os._exit(0 if run.finish() == "failed" else 1)
    _, status = os.waitpid(pid, 0)
    assert os.WEXITSTATUS(status) == 0
    run.log({"after": 1})
    assert run.finish() == "saved_locally"


def test_unfinished_run_is_finished_at_exit(tmp_path):
    script = (
        "import losscape\n"
        f"run = losscape.init(project='t', mode='offline', dir=r'{tmp_path}')\n"
        "run.log({'loss': 1.0})\n"
        "print(run.dir)\n"
    )
    out = subprocess.run([sys.executable, "-c", script], capture_output=True, text=True, check=True)
    run_dir = out.stdout.strip()
    assert os.path.exists(os.path.join(run_dir, "finish.pb"))


def test_sync_cli_with_nothing_to_sync(tmp_path):
    from losscape.cli import main

    assert main(["sync", "--dir", str(tmp_path), "--api-key", "x"]) == 0


def test_console_setting_says_why_capture_is_off(monkeypatch):
    from losscape._run import _console_setting

    monkeypatch.delenv("LOSSCAPE_CONSOLE", raising=False)
    monkeypatch.delitem(sys.modules, "ipykernel", raising=False)
    assert _console_setting(None) == (os.name != "nt", "windows" if os.name == "nt" else None)
    assert _console_setting(False) == (False, "console=False")
    monkeypatch.setenv("LOSSCAPE_CONSOLE", "0")
    assert _console_setting(None) == (False, "LOSSCAPE_CONSOLE")
    assert _console_setting(True) == (True, None)
    monkeypatch.delenv("LOSSCAPE_CONSOLE")
    monkeypatch.setitem(sys.modules, "ipykernel", object())
    if os.name != "nt":
        assert _console_setting(None) == (False, "notebook")


def test_abort_from_the_ui_interrupts_the_script(monkeypatch):
    from losscape import _run

    class Native:
        abort_requested = True

    interrupts = []
    monkeypatch.setattr(_run, "_interrupt_main", lambda: interrupts.append(True))
    run = _run.Run.__new__(_run.Run)
    run._native, run._pid, run._finished, run._aborted, run.name = (
        Native(),
        os.getpid(),
        False,
        False,
        "r",
    )
    finished = threading.Event()

    def stop(*_args, **_kwargs):
        run._finished = True
        finished.set()

    monkeypatch.setattr(_run, "ABORT_GRACE_SECONDS", 0)
    monkeypatch.setattr(_run.Run, "finish", stop)
    monkeypatch.setattr(_run.os, "kill", lambda *_: None)
    _run.Run._watch_abort(run)
    assert interrupts == [True]
    assert run._aborted
    assert finished.is_set(), "still running after the grace period: ended anyway"


def test_background_script_ignores_interrupts_but_honors_an_abort():
    script = """
import os, signal, time
from losscape import _run

signal.signal(signal.SIGINT, signal.SIG_IGN)  # as inherited by `python train.py &`
class R: _aborted = False
_run._honor_abort_when_ignoring_interrupts(R)
os.kill(os.getpid(), signal.SIGINT)
time.sleep(0.05)
print("still running")
R._aborted = True
try:
    os.kill(os.getpid(), signal.SIGINT)
    time.sleep(5)
except KeyboardInterrupt:
    print("interrupted")
"""
    out = subprocess.run([sys.executable, "-c", script], capture_output=True, text=True, check=True)
    assert out.stdout.split("\n")[:2] == ["still running", "interrupted"]


def test_describe_model_does_not_advance_the_step(tmp_path):
    from test_model import Linear

    run = losscape.init(project="p", mode="offline", dir=tmp_path, console=False)
    run.log({"loss": 1.0})
    assert run.step == 1
    run.describe_model(Linear(3))
    run.describe_model(Linear(3), name="head", path="ckpt/best.pt")
    assert run.step == 1
    with pytest.raises(TypeError, match="torch.nn.Module"):
        run.describe_model("a model")
    with pytest.raises(ValueError, match="name"):
        run.describe_model(Linear(3), name="")
    assert run.finish() in ("saved_locally", "uploaded")
