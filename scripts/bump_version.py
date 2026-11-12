#!/usr/bin/env python3
"""Set a new version everywhere it's written: the Rust workspace (server, SDK core, the SDK
package, which takes its version from it), the web app, and the examples of the docs.

    python3 scripts/bump_version.py 0.1.5            # or: make bump VERSION=0.1.5
    python3 scripts/bump_version.py 0.1.5 --commit   # and commit it

Changes nothing unless every file is as expected. Then prints the tags to push, which publish
the SDK on PyPI and the server's image (.github/workflows). Standard library only.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERSION = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")


def parts(version: str) -> tuple[int, ...]:
    return tuple(map(int, VERSION.match(version).groups()))


def current_version() -> str:
    """The workspace's: `version` under `[workspace.package]` of Cargo.toml."""
    text = (ROOT / "Cargo.toml").read_text()
    section = re.search(r"^\[workspace\.package\]\n(.*?)(?=^\[|\Z)", text, re.M | re.S)
    found = section and re.search(r'^version = "([^"]+)"', section.group(1), re.M)
    if not found:
        sys.exit("Cargo.toml: no version under [workspace.package]")
    return found.group(1)


def edits(old: str, new: str) -> dict[str, list[tuple[str, str, int]]]:
    """Per file: (text, its replacement, how many there must be)."""
    image = "madproton/losscape:"
    return {
        "Cargo.toml": [(f'version = "{old}"', f'version = "{new}"', 1)],
        "crates/losscape-py/Cargo.toml": [
            (
                f'losscape-client = {{ version = "{old}"',
                f'losscape-client = {{ version = "{new}"',
                1,
            )
        ],
        "web/package.json": [(f'"version": "{old}"', f'"version": "{new}"', 1)],
        "README.md": [(f"{image}{old}", f"{image}{new}", 1)],
        "deploy/env.example": [(f"{image}{old}", f"{image}{new}", 1)],
        "docs/DEPLOYMENT.md": [
            (f"{image}{old}", f"{image}{new}", 1),
            (f"server-v{old}", f"server-v{new}", 2),
            (f"sdk-v{old}", f"sdk-v{new}", 2),
        ],
    }


def bump_lockfile(text: str, old: str, new: str) -> str:
    """The web app's own version in package-lock.json, and nothing else: a dependency can have
    the same version."""
    lock = json.loads(text)
    if lock.get("version") != old or lock["packages"][""].get("version") != old:
        raise ValueError(f"web/package-lock.json: the app's version isn't {old}")
    # Its two entries come first, before any dependency's.
    out = text.replace(f'"version": "{old}"', f'"version": "{new}"', 2)
    expected = json.loads(text)
    expected["version"] = expected["packages"][""]["version"] = new
    if json.loads(out) != expected:
        raise ValueError("web/package-lock.json: the app's entries aren't the first ones")
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("version", help="the new version, like 0.1.5")
    parser.add_argument("--commit", action="store_true", help="commit the change")
    args = parser.parse_args()

    old, new = current_version(), args.version
    if not VERSION.match(new):
        sys.exit(f"{new}: use X.Y.Z, like 0.1.5")
    if not VERSION.match(old):
        sys.exit(f"Cargo.toml: {old} isn't X.Y.Z")
    if parts(new) <= parts(old):
        sys.exit(f"{new}: not after the current version, {old}")

    # Everything checked before anything is written.
    changed: dict[Path, str] = {}
    problems: list[str] = []
    for name, replacements in edits(old, new).items():
        path = ROOT / name
        text = path.read_text()
        for before, after, count in replacements:
            if text.count(before) != count:
                problems.append(
                    f"{name}: {count} × {before!r} expected, {text.count(before)} found"
                )
            text = text.replace(before, after)
        changed[path] = text
    lockfile = ROOT / "web/package-lock.json"
    try:
        changed[lockfile] = bump_lockfile(lockfile.read_text(), old, new)
    except ValueError as err:
        problems.append(str(err))
    if problems:
        print("Nothing changed:", *problems, sep="\n  ", file=sys.stderr)
        return 1

    for path, text in changed.items():
        path.write_text(text)
    # Cargo.lock: the workspace's crates take the new version.
    subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        cwd=ROOT,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    files = [str(p.relative_to(ROOT)) for p in changed] + ["Cargo.lock"]
    print(f"{old} -> {new}:", *files, sep="\n  ")

    if args.commit:
        subprocess.run(["git", "add", "--", *files], cwd=ROOT, check=True)
        subprocess.run(
            ["git", "commit", "-q", "-m", f"Bump version to {new}", "--", *files],
            cwd=ROOT,
            check=True,
        )
        print("\nCommitted. To publish, after `git push origin master`:")
    else:
        print("\nTo publish, commit, push, then:")
    print(f"  git tag sdk-v{new} && git tag server-v{new}")
    print(f"  git push origin sdk-v{new} server-v{new}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
