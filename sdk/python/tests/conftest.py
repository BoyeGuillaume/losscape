import os

import pytest

os.environ["LOSSCAPE_SILENT"] = "1"


@pytest.fixture
def offline(tmp_path, monkeypatch):
    """Runs default to offline mode in a temporary directory."""
    monkeypatch.setenv("LOSSCAPE_MODE", "offline")
    monkeypatch.setenv("LOSSCAPE_DIR", str(tmp_path))
    return tmp_path
