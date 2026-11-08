#!/usr/bin/env python3
"""Writes the licenses of the Rust crates a build ships, and checks they're allowed.

    python3 scripts/third_party_notices.py --package losscape-server --output NOTICES.txt \\
        --json list.json       # names, versions and licenses, for the About page
    python3 scripts/third_party_notices.py --check      # every shipped crate, no output

Exits with an error if a crate's license isn't in ALLOWED. Only what ends up in the binary
counts: not build scripts' or tests' dependencies. Run from the repository root.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

# Permissive licenses: use, modify and redistribute, closed or open, with attribution at most.
ALLOWED = {
    "0BSD",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",  # Boost Software License, not the Business Source License (BUSL)
    "CC0-1.0",
    "CDLA-Permissive-2.0",
    "ISC",
    "MIT",
    "MIT-0",
    "OFL-1.1",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "Unlicense",
    "Zlib",
}
SHIPPED = ["losscape-server", "losscape-py", "losscape-client"]
LICENSE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)", re.IGNORECASE)


def satisfied(expression: str) -> bool:
    """A license expression: OR needs one side allowed, AND needs both."""
    text = expression.replace("/", " OR ").replace("(", " ( ").replace(")", " ) ")
    tokens = text.split()
    position = 0

    def parse_or() -> bool:
        nonlocal position
        value = parse_and()
        while position < len(tokens) and tokens[position] == "OR":
            position += 1
            value = parse_and() or value
        return value

    def parse_and() -> bool:
        nonlocal position
        value = parse_one()
        while position < len(tokens) and tokens[position] == "AND":
            position += 1
            value = parse_one() and value
        return value

    def parse_one() -> bool:
        nonlocal position
        token = tokens[position]
        position += 1
        if token == "(":
            value = parse_or()
            position += 1
            return value
        if position < len(tokens) and tokens[position] == "WITH":
            position += 2
            return token in ALLOWED or token == "Apache-2.0"
        return token in ALLOWED

    return parse_or() and position == len(tokens)


def crates(metadata: dict, root_name: str) -> list[dict]:
    """A crate and what it needs at run time, without itself or the workspace's own."""
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    workspace = set(metadata["workspace_members"])
    root = next(i for i in workspace if packages[i]["name"] == root_name)
    seen = set()
    stack = [root]
    while stack:
        current = stack.pop()
        if current in seen:
            continue
        seen.add(current)
        for dep in nodes[current]["deps"]:
            if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    return sorted(
        (packages[i] for i in seen if i not in workspace),
        key=lambda p: (p["name"], p["version"]),
    )


def license_texts(package: dict) -> list[tuple[str, str]]:
    folder = Path(package["manifest_path"]).parent
    found = sorted(
        path
        for path in folder.iterdir()
        if path.is_file() and LICENSE_FILE.match(path.name) and path.suffix != ".rs"
    )
    return [(path.name, path.read_text(encoding="utf-8", errors="replace")) for path in found]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--package", default=None)
    parser.add_argument("--output", default=None)
    parser.add_argument("--json", default=None)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    metadata = json.loads(
        subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked"])
    )
    roots = [args.package] if args.package else SHIPPED
    shipped = {p["id"]: p for root in roots for p in crates(metadata, root)}
    packages = sorted(shipped.values(), key=lambda p: (p["name"], p["version"]))

    refused = [p for p in packages if not satisfied(p.get("license") or "unknown")]
    for package in refused:
        print(
            f"NOT ALLOWED: {package['name']} {package['version']}: {package.get('license')}",
            file=sys.stderr,
        )

    if args.output:
        # Crates share license texts: each text once, with who it's for.
        by_text: dict[str, list[str]] = {}
        without_text = []
        for package in packages:
            label = f"{package['name']} {package['version']} ({package.get('license')})"
            texts = license_texts(package)
            if not texts:
                without_text.append(label)
            for _, text in texts:
                by_text.setdefault(text.strip(), []).append(label)
        lines = ["losscape is built with these open source projects.\n"]
        for text, users in by_text.items():
            lines += ["=" * 80, *users, "=" * 80, "", text, ""]
        if without_text:
            lines += ["=" * 80, "Licensed as stated, without a license file of their own:"]
            lines += [*without_text, ""]
        Path(args.output).parent.mkdir(parents=True, exist_ok=True)
        Path(args.output).write_text("\n".join(lines), encoding="utf-8")

    if args.json:
        listed = [
            {
                "name": p["name"],
                "version": p["version"],
                "license": p.get("license") or "unknown",
                "repository": p.get("repository"),
            }
            for p in packages
        ]
        Path(args.json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.json).write_text(json.dumps({"packages": listed}), encoding="utf-8")

    print(f"{len(packages)} crates, {len(refused)} not allowed")
    return 1 if refused else 0


if __name__ == "__main__":
    sys.exit(main())
