#!/usr/bin/env python3
"""A domain uses another domain only through its handle and its types.

Orbit's Rust is split by domain in two places, and this guard reads both from
the module graph (rust_modules.py), outside tests:

  core    each child module of `orbit_core::transition` is a domain's pure
          decisions (capture, assistant, ...), except the coordinators that
          boundaries.toml names, which route one event to several domains.
  daemon  each module of `orbit_daemon::daemon` that owns a field of
          `Runtime` with a `…Handle` type is a domain (calendar, audio, ...);
          every other module of the daemon is the coordinator.

A reference from one domain's code to another domain is an *edge*. Through
it, a domain may name the other's types (a CamelCase item at the top of its
module, and what hangs from them), the functions it offers (`offers` in
boundaries.toml) and the methods of its handle (`self.calendar.read(…)`).
Anything else is a *reach* into its internals: a function or constant it does
not offer, one of its submodules (`writes::apply`), or a field of its handle
(`.pending`, from `self.calendar.pending`).

boundaries.toml writes down the allowed edges and, like the size ratchet, the
reaches that already existed. The guard fails on a new edge, on a new reach,
and on an entry that no longer matches the code, so both lists only shrink.
Coordinators are not restricted: crossing domains is their job.

Usage: boundaries.py
"""

import re
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))

from code_scan import line_of, line_starts
from rust_modules import RustFile, crate_files

ROOT = Path(__file__).resolve().parents[2]
CONFIG = Path(__file__).resolve().parent / "boundaries.toml"
USE = re.compile(r"\buse\s+([^;]+);")
PATH = re.compile(r"(?<![\w:.])((?:[A-Za-z_]\w*\s*::\s*)+[A-Za-z_*]\w*)")
HANDLE_FIELD = re.compile(r"\b(\w+)\s*:\s*(\w+)\s*::\s*(\w+Handle)\b")
HANDLE_USE = re.compile(r"\b(?:self|runtime)\s*\.\s*(\w+)\s*\.\s*(\w+)\s*(\(|::\s*<)?")


@dataclass
class Area:
    """A parent module whose child modules are domains."""

    name: str
    files: list[RustFile]
    parent: tuple[str, ...]
    domains: set[str]
    handles: dict[str, str] = field(default_factory=dict)
    offers: dict[str, set[str]] = field(default_factory=dict)


@dataclass
class Graph:
    """Edges between domains and reaches into internals, each with where it was first seen."""

    edges: dict[tuple[str, str], str] = field(default_factory=dict)
    reaches: dict[tuple[str, str, str], str] = field(default_factory=dict)


def expand_use(tree: str) -> list[str]:
    """`a::{b, c::{d as e, f}}` -> ["a::b", "a::c::d", "a::c::f"]."""
    tree = re.sub(r"\s+", "", re.sub(r"\s+as\s+\w+", "", tree))
    brace = tree.find("{")
    if brace < 0:
        return [tree]
    prefix, inner = tree[:brace], tree[brace + 1 : tree.rfind("}")]
    parts, depth, current = [], 0, ""
    for char in inner:
        if char == "," and depth == 0:
            parts.append(current)
            current = ""
            continue
        depth += {"{": 1, "}": -1}.get(char, 0)
        current += char
    parts.append(current)
    return [path for part in parts if part for path in expand_use(prefix + part)]


def resolve(file: RustFile, segments: list[str], area: Area, known) -> tuple[str, ...] | None:
    """The absolute module path `segments` names from `file`; None outside the crate."""
    head = segments[0]
    if head == "crate":
        return tuple(segments[1:])
    if head in ("super", "self"):
        base, rest = list(file.module), list(segments)
        while rest and rest[0] in ("super", "self"):
            if rest.pop(0) == "super":
                base = base[:-1]
        return tuple(base + rest)
    if file.module + (head,) in known:
        return file.module + tuple(segments)
    if head in area.domains:
        return area.parent + tuple(segments)
    return None


def domain_of(module: tuple[str, ...], area: Area) -> str | None:
    depth = len(area.parent)
    if module[:depth] == area.parent and len(module) > depth and module[depth] in area.domains:
        return module[depth]
    return None


def reach_of(rest: list[str], target: str, area: Area) -> str | None:
    """None for what `target` lets other domains use; otherwise the reach, as a path."""
    if not rest or rest[0][0].isupper():
        return None
    if len(rest) == 1 and rest[0] in area.offers.get(target, set()):
        return None
    return "::".join(rest)


def _imported_from_outside(file: RustFile, area: Area, known) -> set[str]:
    """Names a `use` brings from outside the area: a bare path starting there means them."""
    names = set()
    for statement in USE.finditer(file.masked):
        for path in expand_use(statement.group(1)):
            segments = [segment for segment in path.split("::") if segment]
            if not segments:
                continue
            target = resolve(file, segments, area, known)
            if target is None or domain_of(target, area) is None:
                names.add(segments[-1])
    return names


def _paths(file: RustFile):
    """(offset, segments, from a use) of every path in the file's code, outside tests."""
    uses = []
    for statement in USE.finditer(file.masked):
        uses.append((statement.start(), statement.end()))
        if not file.in_test(statement.start()):
            for path in expand_use(statement.group(1)):
                yield statement.start(), [s for s in path.split("::") if s], True
    for match in PATH.finditer(file.masked):
        inside_use = any(start <= match.start() < end for start, end in uses)
        if not inside_use and not file.in_test(match.start()):
            yield match.start(), re.sub(r"\s+", "", match.group(1)).split("::"), False


def scan_file(file: RustFile, area: Area, known, graph: Graph, root: Path) -> None:
    """Adds the file's edges and reaches to `graph`, each at its first line in the file."""
    source = domain_of(file.module, area)
    if source is None or file.test:
        return
    found = []
    outside = _imported_from_outside(file, area, known)
    for offset, segments, from_use in _paths(file):
        if not segments or (not from_use and segments[0] in outside):
            continue
        target_path = resolve(file, segments, area, known)
        target = domain_of(target_path, area) if target_path else None
        if target is not None and target != source:
            rest = list(target_path[len(area.parent) + 1 :])
            found.append((offset, target, reach_of(rest, target, area)))
    for use in HANDLE_USE.finditer(file.masked):
        target = area.handles.get(use.group(1))
        if target is not None and target != source and not file.in_test(use.start()):
            found.append((use.start(), target, None if use.group(3) else f".{use.group(2)}"))
    starts = line_starts(file.masked)
    relative = file.path.relative_to(root).as_posix()
    for offset, target, reach in sorted(found, key=lambda item: item[0]):
        where = f"{relative}:{line_of(starts, offset)}"
        graph.edges.setdefault((source, target), where)
        if reach is not None:
            graph.reaches.setdefault((source, target, reach), where)


def scan(area: Area, root: Path) -> Graph:
    graph = Graph()
    known = {file.module for file in area.files}
    for file in sorted(area.files, key=lambda file: file.path):
        scan_file(file, area, known, graph, root)
    return graph


def build_area(name: str, settings: dict, root: Path) -> Area:
    files = list(crate_files(root / settings["crate"]).values())
    parent = tuple(settings["parent"].split("::"))
    area = Area(name, files, parent, set())
    if "handles" in settings:
        source = (root / settings["handles"]).read_text(encoding="utf-8")
        area.handles = {match.group(1): match.group(2) for match in HANDLE_FIELD.finditer(source)}
        area.domains = set(area.handles.values())
    else:
        children = {
            file.module[len(parent)]
            for file in files
            if not file.test
            and file.module[: len(parent)] == parent
            and len(file.module) > len(parent)
        }
        area.domains = children - set(settings.get("coordinators", []))
    area.offers = {domain: set(items) for domain, items in settings.get("offers", {}).items()}
    return area


def _declared(settings: dict) -> tuple[set[tuple[str, str]], set[tuple[str, str, str]]]:
    edges = {
        (source, target)
        for source, targets in settings.get("edges", {}).items()
        for target in targets
    }
    reaches = set()
    for edge, items in settings.get("reaches", {}).items():
        source, _, target = (part.strip() for part in edge.partition("->"))
        reaches.update((source, target, item) for item in items)
    return edges, reaches


def problems(area: Area, graph: Graph, settings: dict) -> list[str]:
    """Every difference between the code's edges and reaches and what boundaries.toml says."""
    edges, reaches = _declared(settings)
    found = []
    for source, target in sorted(graph.edges.keys() - edges):
        found.append(
            f"{area.name}: new edge {source} -> {target} (first at {graph.edges[(source, target)]}). "
            f"A domain uses another only through an edge boundaries.toml allows; add it to "
            f"[{area.name}.edges] only if {source} is meant to depend on {target}."
        )
    for source, target in sorted(edges - graph.edges.keys()):
        found.append(
            f"{area.name}: boundaries.toml allows {source} -> {target}, but the code has "
            "no such edge any more: remove it."
        )
    for source, target, item in sorted(graph.reaches.keys() - reaches):
        found.append(
            f"{area.name}: {source} reaches into {target}'s internals with `{item}` "
            f"(at {graph.reaches[(source, target, item)]}). Use {target}'s handle methods or its "
            "types, or move the code to the coordinator; the list of reaches takes no new entries."
        )
    for source, target, item in sorted(reaches - graph.reaches.keys()):
        found.append(
            f"{area.name}: boundaries.toml lists the reach {source} -> {target} `{item}`, "
            "which the code no longer has: remove it."
        )
    for domain in sorted({part for edge in edges for part in edge} - area.domains):
        found.append(f"{area.name}: boundaries.toml names `{domain}`, which is not a domain here.")
    return found


def main(arguments: list[str]) -> int:
    if arguments:
        print(__doc__, file=sys.stderr)
        return 2
    config = tomllib.loads(CONFIG.read_text(encoding="utf-8"))
    found, lines = [], []
    for name, settings in config.items():
        area = build_area(name, settings, ROOT)
        graph = scan(area, ROOT)
        found.extend(problems(area, graph, settings))
        lines.append(
            f"{name}: {len(area.domains)} domains, {len(graph.edges)} edges, "
            f"{len(graph.reaches)} reaches into internals"
        )
    print("; ".join(lines))
    for problem in found:
        print(f"  {problem}")
    if found:
        print(f"{len(found)} differences from {CONFIG.relative_to(ROOT)}")
        return 1
    print(f"every edge and reach is the one {CONFIG.relative_to(ROOT)} lists")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
