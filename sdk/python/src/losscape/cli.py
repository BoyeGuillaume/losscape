"""``losscape`` command line."""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

from losscape import __version__, _native


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="losscape")
    parser.add_argument("--version", action="version", version=f"losscape {__version__}")
    commands = parser.add_subparsers(dest="command", required=True)

    sync = commands.add_parser(
        "sync",
        help="upload runs left in the local log (offline mode, server down, killed process)",
    )
    sync.add_argument("--dir", default=os.environ.get("LOSSCAPE_DIR", ".losscape"))
    sync.add_argument(
        "--server", default=os.environ.get("LOSSCAPE_SERVER", "http://localhost:8080")
    )
    sync.add_argument("--api-key", default=os.environ.get("LOSSCAPE_API_KEY"))

    args = parser.parse_args(argv)
    if args.command == "sync":
        return _sync(Path(args.dir), args.server, args.api_key)
    return 2


def _sync(root: Path, server: str, api_key: str | None) -> int:
    if not api_key:
        print("losscape sync: set LOSSCAPE_API_KEY or pass --api-key", file=sys.stderr)
        return 2
    results = _native.sync(root, server, api_key)
    if not results:
        print(f"nothing to sync in {root}")
        return 0
    failed = 0
    for run_id, status, message in results:
        suffix = f": {message}" if message else ""
        print(f"{run_id}  {status}{suffix}")
        failed += status == "failed"
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
