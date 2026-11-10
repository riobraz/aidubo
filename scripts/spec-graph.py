#!/usr/bin/env python3
"""Validate and query the post-MVP spec graph.

Usage:
    python3 scripts/spec-graph.py --validate
    python3 scripts/spec-graph.py --ready
    python3 scripts/spec-graph.py --waves
    python3 scripts/spec-graph.py --set-status M9-01 in-progress

The graph lives in docs/specs/graph.json. This script has no dependencies
beyond the standard library and never mutates files except with --set-status.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GRAPH = ROOT / "docs/specs/graph.json"
STATUSES = {"ready", "in-progress", "blocked", "done", "cancelled"}
SIZES = {"S", "M", "L"}


def load() -> dict:
    with GRAPH.open(encoding="utf-8") as handle:
        return json.load(handle)


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


def computed_waves(nodes: dict[str, dict]) -> dict[str, int]:
    waves: dict[str, int] = {}

    def visit(node_id: str, stack: tuple[str, ...]) -> int:
        if node_id in waves:
            return waves[node_id]
        if node_id in stack:
            fail(f"cycle: {' -> '.join(stack + (node_id,))}")
        node = nodes[node_id]
        if not node["deps"]:
            waves[node_id] = 0
            return 0
        value = 1 + max(visit(dep, stack + (node_id,)) for dep in node["deps"])
        waves[node_id] = value
        return value

    for node_id in nodes:
        visit(node_id, ())
    return waves


def validate(graph: dict) -> None:
    nodes: dict[str, dict] = {}
    for node in graph["nodes"]:
        node_id = node["id"]
        if node_id in nodes:
            fail(f"duplicate node id {node_id}")
        nodes[node_id] = node

    for node in graph["nodes"]:
        node_id = node["id"]
        if node["status"] not in STATUSES:
            fail(f"{node_id}: unknown status {node['status']!r}")
        if node["size"] not in SIZES:
            fail(f"{node_id}: size must be one of {sorted(SIZES)}")
        for dep in node["deps"]:
            if dep not in nodes:
                fail(f"{node_id}: unknown dependency {dep}")
            if dep == node_id:
                fail(f"{node_id}: depends on itself")
        if node.get("kind") == "spec":
            if not node.get("e2e"):
                fail(f"{node_id}: spec must name an e2e test or a reason it has none")
            path = ROOT / node["file"]
            if not path.is_file():
                fail(f"{node_id}: missing spec file {node['file']}")
            text = path.read_text(encoding="utf-8")
            if not text.startswith(f"# {node_id} "):
                fail(f"{node_id}: {node['file']} must start with '# {node_id} '")
            if "## " not in text:
                fail(f"{node_id}: {node['file']} has no sections")
            if "E2E" not in text:
                fail(f"{node_id}: {node['file']} does not mention an E2E test")
            header = re.search(r"^\| Status \| (.+?) \|$", text, re.MULTILINE)
            if not header:
                fail(f"{node_id}: {node['file']} has no '| Status |' header row")
        else:
            if node["kind"] != "gate":
                fail(f"{node_id}: unknown kind {node['kind']!r}")

    waves = computed_waves(nodes)
    for node_id, wave in waves.items():
        declared = nodes[node_id]["wave"]
        if nodes[node_id]["kind"] == "gate":
            if not nodes[node_id]["deps"]:
                fail(f"{node_id}: a gate needs at least one dependency")
            closing = max(nodes[dep]["wave"] for dep in nodes[node_id]["deps"])
            if declared != closing:
                fail(f"{node_id}: declared wave {declared}, closes wave {closing}")
            continue
        if declared != wave:
            fail(f"{node_id}: declared wave {declared}, computed {wave}")

    for node in graph["nodes"]:
        for dep in node["deps"]:
            if nodes[dep]["wave"] > node["wave"]:
                fail(f"{node['id']}: dependency {dep} is in a later wave")


def ready(nodes: dict[str, dict]) -> list[dict]:
    done = {node_id for node_id, node in nodes.items() if node["status"] == "done"}
    return [
        node
        for node in nodes.values()
        if node["status"] == "ready" and all(dep in done for dep in node["deps"])
    ]


def print_node(node: dict) -> None:
    location = node["file"] or "(gate, see docs/specs/README.md §4)"
    print(f"  {node['id']:<8} wave {node['wave']}  {node['title']}")
    print(f"           {location}")


def set_status(node_id: str, status: str) -> None:
    if status not in STATUSES:
        fail(f"unknown status {status!r}")
    graph = load()
    node = next((item for item in graph["nodes"] if item["id"] == node_id), None)
    if node is None:
        fail(f"unknown node {node_id}")
    node["status"] = status
    GRAPH.write_text(json.dumps(graph, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    if node.get("file"):
        path = ROOT / node["file"]
        text = path.read_text(encoding="utf-8")
        text = re.sub(
            r"^\| Status \| .+? \|$",
            f"| Status | {status} |",
            text,
            count=1,
            flags=re.MULTILINE,
        )
        path.write_text(text, encoding="utf-8")
    print(f"{node_id}: {status}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--validate", action="store_true")
    parser.add_argument("--ready", action="store_true")
    parser.add_argument("--waves", action="store_true")
    parser.add_argument("--set-status", nargs=2, metavar=("NODE", "STATUS"))
    args = parser.parse_args()

    graph = load()
    nodes = {node["id"]: node for node in graph["nodes"]}

    if args.set_status:
        set_status(*args.set_status)
        return

    if args.validate:
        validate(graph)
        print(f"ok: {len(nodes)} nodes, {len({n['wave'] for n in nodes.values()})} waves")
        return

    if args.waves:
        validate(graph)
        for wave in sorted({node["wave"] for node in nodes.values()}):
            specs = sorted(
                node["id"] for node in nodes.values()
                if node["wave"] == wave and node["kind"] == "spec"
            )
            gates = sorted(
                node["id"] for node in nodes.values()
                if node["wave"] == wave and node["kind"] == "gate"
            )
            suffix = f"   gate: {' '.join(gates)}" if gates else ""
            print(f"wave {wave} ({len(specs)}): {' '.join(specs)}{suffix}")
        return

    if args.ready:
        validate(graph)
        items = sorted(ready(nodes), key=lambda node: (node["wave"], node["id"]))
        if not items:
            print("nothing ready: finish or unblock a node first")
            return
        print(f"{len(items)} ready:")
        for node in items:
            print_node(node)
        return

    parser.error("choose --validate, --ready, --waves, or --set-status")


if __name__ == "__main__":
    main()
