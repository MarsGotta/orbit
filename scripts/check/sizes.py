#!/usr/bin/env python3
"""At most 500 lines of code per production file and 80 per function, in Rust,
Python and Swift, with a ratchet for what was already over.

Why: a file of thousands of lines or a function of hundreds is where one
domain's change breaks another's, and where a reviewer stops reading. The
interface already has these limits from ESLint (`apps/desktop/eslint.config.js`,
tighter there); this is the same rule for the rest of the repository.

What is measured
  Rust    every file a crate builds outside tests (rust_modules.py): crates/
          and apps/desktop/src-tauri, crate roots and build scripts included;
          integration tests, examples, `#[cfg(test)]` modules and items, and
          `#[test]` functions are not.
  Python  every .py file in server/orbit_hub, server/orbit_mcp and scripts,
          except scripts/tests, any `tests` folder, test_*.py, *_test.py and
          conftest.py.
  Swift   every .swift file in native/macos/Sources (the tests live in
          native/macos/Tests).
  A file whose first five lines say `@generated` or `DO NOT EDIT` is generated
  and not measured. The generated TypeScript is outside these folders.

What a line is
  A line of code holds something other than a comment or blanks: comments,
  doc comments, Python docstrings and blank lines do not count; every line of
  a multi-line string does.

What a function is
  A `fn`, `def`, `func`, `init` or `deinit` with a body, from the line of its
  keyword (attributes and decorators excluded) to the line that ends its body.
  A nested function or closure counts in the function around it, and a nested
  function is measured on its own too. A declaration without a body (a trait
  or protocol requirement) is not a function. Its name is the path of the
  types, impls, modules and functions around it (`Runtime::run`,
  `Store.open`); a name repeated in one file gets `#2`, `#3`.

The ratchet (size-ratchet.toml) holds every file and function that was over
when the limits arrived, at its size then. An entry may only go down: the gate
fails when an entry grows, when a new file or function goes over, and when an
entry is no longer exact (it shrank, or is back under the limit), so that the
list always says the truth and what shrinks cannot grow back.

Usage:
  sizes.py           checks the repository against the ratchet
  sizes.py --shrink  lowers the entries that shrank and removes the ones back
                     under the limits; it never adds an entry or raises one
"""

import ast
import io
import json
import sys
import tokenize
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))

from code_scan import (
    RUST_FN,
    SWIFT_FUNCTION,
    classify_swift,
    code_lines,
    line_of,
    line_starts,
    mask_swift,
    number_duplicates,
    qualified_name,
    scopes,
)
from rust_modules import crate_files, rust_crates

ROOT = Path(__file__).resolve().parents[2]
RATCHET = Path(__file__).resolve().parent / "size-ratchet.toml"
FILE_LIMIT = 500
FUNCTION_LIMIT = 80
RUST_FOLDERS = ["crates", "apps/desktop/src-tauri"]
PYTHON_FOLDERS = ["server/orbit_hub", "server/orbit_mcp", "scripts"]
SWIFT_FOLDERS = ["native/macos/Sources"]
SKIPPED_FOLDERS = {"__pycache__", "node_modules", "target", ".build", ".venv"}
GENERATED_MARKERS = ("@generated", "DO NOT EDIT")


@dataclass
class Measurement:
    """Lines of code of every production file and function, by repository path."""

    files: dict[str, int] = field(default_factory=dict)
    functions: dict[str, dict[str, int]] = field(default_factory=dict)
    language: dict[str, str] = field(default_factory=dict)

    def add(self, path: str, language: str, lines: int, functions: list[tuple[str, int]]):
        self.files[path] = lines
        self.language[path] = language
        names = number_duplicates([name for name, _ in functions])
        self.functions[path] = {
            name: size for name, (_, size) in zip(names, functions, strict=True)
        }


def is_generated(source: str) -> bool:
    head = "\n".join(source.split("\n")[:5])
    return any(marker in head for marker in GENERATED_MARKERS)


def counted(lines: list[bool], first: int, last: int) -> int:
    return sum(lines[first : last + 1])


def measure_rust_file(file) -> tuple[int, list[tuple[str, int]]]:
    lines = code_lines(file.masked)
    starts = line_starts(file.masked)
    for start, end in file.test_ranges:
        for number in range(line_of(starts, start), line_of(starts, end) + 1):
            lines[number] = False
    functions = []
    for scope in file.scopes:
        if scope.kind != "function" or file.in_test(scope.open):
            continue
        keywords = list(RUST_FN.finditer(file.masked, scope.header_start, scope.open))
        first = line_of(starts, keywords[-1].start()) if keywords else line_of(starts, scope.open)
        size = counted(lines, first, line_of(starts, scope.close))
        functions.append((qualified_name(scope, "::"), size))
    return sum(lines), functions


def measure_swift_source(source: str) -> tuple[int, list[tuple[str, int]]]:
    masked = mask_swift(source)
    lines = code_lines(masked)
    starts = line_starts(masked)
    functions = []
    for scope in scopes(masked, classify_swift):
        if scope.kind != "function":
            continue
        keywords = list(SWIFT_FUNCTION.finditer(masked, scope.header_start, scope.open))
        first = line_of(starts, keywords[-1].start() if keywords else scope.open)
        functions.append(
            (qualified_name(scope, "."), counted(lines, first, line_of(starts, scope.close)))
        )
    return sum(lines), functions


def _docstring_lines(tree: ast.AST) -> set[int]:
    found = set()
    for node in ast.walk(tree):
        if not isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        body = node.body
        first = body[0] if body else None
        constant = isinstance(first, ast.Expr) and isinstance(first.value, ast.Constant)
        if constant and isinstance(first.value.value, str):
            found.update(range(first.lineno, first.end_lineno + 1))
    return found


def _python_code_lines(source: str, tree: ast.AST) -> list[bool]:
    total = source.count("\n") + 2
    lines = [False] * total
    skipped = {tokenize.COMMENT, tokenize.NL, tokenize.NEWLINE, tokenize.INDENT}
    skipped |= {tokenize.DEDENT, tokenize.ENDMARKER, tokenize.ENCODING}
    for token in tokenize.generate_tokens(io.StringIO(source).readline):
        if token.type not in skipped:
            for number in range(token.start[0], token.end[0] + 1):
                lines[number] = True
    for number in _docstring_lines(tree):
        lines[number] = False
    return lines


def measure_python_source(source: str) -> tuple[int, list[tuple[str, int]]]:
    tree = ast.parse(source)
    lines = _python_code_lines(source, tree)
    functions = []

    def visit(node: ast.AST, outer: list[str]) -> None:
        for child in ast.iter_child_nodes(node):
            if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef)):
                name = ".".join(outer + [child.name])
                functions.append((name, counted(lines, child.lineno, child.end_lineno)))
                visit(child, outer + [child.name])
            elif isinstance(child, ast.ClassDef):
                visit(child, outer + [child.name])
            else:
                visit(child, outer)

    visit(tree, [])
    return sum(lines), functions


def _walk(root: Path, folder: str, suffix: str):
    for path in sorted((root / folder).rglob(f"*{suffix}")):
        parts = path.relative_to(root).parts
        if not any(part in SKIPPED_FOLDERS or part.startswith(".") for part in parts):
            yield path


def is_python_test(relative: Path) -> bool:
    name = relative.name
    return (
        "tests" in relative.parts[:-1]
        or name.startswith("test_")
        or name.endswith("_test.py")
        or name == "conftest.py"
    )


def measure(root: Path) -> Measurement:
    measurement = Measurement()
    for crate in rust_crates(root, RUST_FOLDERS):
        for path, file in sorted(crate_files(crate).items()):
            if not file.test and not is_generated(file.source):
                lines, functions = measure_rust_file(file)
                measurement.add(
                    path.relative_to(root.resolve()).as_posix(), "rust", lines, functions
                )
    for folder in PYTHON_FOLDERS:
        for path in _walk(root, folder, ".py"):
            source = path.read_text(encoding="utf-8")
            relative = path.relative_to(root)
            if not is_python_test(relative) and not is_generated(source):
                measurement.add(relative.as_posix(), "python", *measure_python_source(source))
    for folder in SWIFT_FOLDERS:
        for path in _walk(root, folder, ".swift"):
            source = path.read_text(encoding="utf-8")
            if not is_generated(source):
                relative = path.relative_to(root).as_posix()
                measurement.add(relative, "swift", *measure_swift_source(source))
    return measurement


@dataclass
class Ratchet:
    files: dict[str, int] = field(default_factory=dict)
    functions: dict[str, dict[str, int]] = field(default_factory=dict)


def load_ratchet(path: Path) -> Ratchet:
    data = tomllib.loads(path.read_text(encoding="utf-8")) if path.is_file() else {}
    return Ratchet(
        dict(data.get("files", {})), {k: dict(v) for k, v in data.get("functions", {}).items()}
    )


def over_limits(measurement: Measurement) -> Ratchet:
    """What the measurement has over the limits, in the ratchet's shape."""
    files = {path: size for path, size in measurement.files.items() if size > FILE_LIMIT}
    functions = {}
    for path, sizes in measurement.functions.items():
        over = {name: size for name, size in sizes.items() if size > FUNCTION_LIMIT}
        if over:
            functions[path] = over
    return Ratchet(files, functions)


def _problem(item: str, unit: str, size: int, recorded: int | None, limit: int) -> str | None:
    if recorded is None:
        if size > limit:
            return (
                f"{item}: {size} lines of code, over the limit of {limit} {unit}. "
                "The ratchet takes no new entries: split it."
            )
        return None
    if size > recorded:
        return (
            f"{item}: {size} lines of code, {size - recorded} more than the {recorded} "
            f"the ratchet holds it to (limit {limit} {unit}). Split it or move code out."
        )
    if size <= limit:
        return (
            f"{item}: {size} lines of code, back within the limit of {limit} {unit}: "
            f"remove its entry ({recorded}) from the ratchet (sizes.py --shrink)."
        )
    if size < recorded:
        return (
            f"{item}: {size} lines of code, below the {recorded} the ratchet records: "
            f"lower the entry to {size} (sizes.py --shrink)."
        )
    return None


def problems(measurement: Measurement, ratchet: Ratchet) -> list[str]:
    """Every difference between the measurement and the ratchet, each with both numbers."""
    found = []
    for path in sorted(measurement.files.keys() | ratchet.files.keys()):
        recorded = ratchet.files.get(path)
        if path not in measurement.files:
            found.append(
                f"{path}: the ratchet holds it at {recorded} lines, but it is no "
                "longer a production file: remove its entry."
            )
            continue
        found.append(_problem(path, "per file", measurement.files[path], recorded, FILE_LIMIT))
    for path in sorted(measurement.functions.keys() | ratchet.functions.keys()):
        measured = measurement.functions.get(path, {})
        held = ratchet.functions.get(path, {})
        for name in sorted(measured.keys() | held.keys()):
            item = f"{path}: function `{name}`"
            if name not in measured:
                found.append(
                    f"{item}: the ratchet holds it at {held[name]} lines, but there is "
                    "no such function any more: remove its entry."
                )
                continue
            found.append(
                _problem(item, "per function", measured[name], held.get(name), FUNCTION_LIMIT)
            )
    return [problem for problem in found if problem]


def shrink(measurement: Measurement, ratchet: Ratchet) -> Ratchet:
    """The ratchet with every entry lowered to what was measured, never raised or added."""
    over = over_limits(measurement)
    files = {
        path: min(size, over.files[path])
        for path, size in ratchet.files.items()
        if path in over.files
    }
    functions = {}
    for path, held in ratchet.functions.items():
        measured = over.functions.get(path, {})
        kept = {name: min(size, measured[name]) for name, size in held.items() if name in measured}
        if kept:
            functions[path] = kept
    return Ratchet(files, functions)


RATCHET_HEADER = """\
# The size ratchet of scripts/check/sizes.py: every production file over 500
# lines of code and every function over 80, at the size each had when the
# limits arrived. The gate fails when one grows, when anything new goes over,
# and when an entry is no longer exact. Lower entries with
# `python3 scripts/check/sizes.py --shrink`; never add one or raise one.
"""


def _key(text: str) -> str:
    """A TOML basic string: JSON's escapes are a subset of TOML's."""
    return json.dumps(text, ensure_ascii=False)


def render_ratchet(ratchet: Ratchet) -> str:
    lines = [RATCHET_HEADER, "[files]"]
    lines.extend(f"{_key(path)} = {size}" for path, size in sorted(ratchet.files.items()))
    for path in sorted(ratchet.functions):
        lines.append("")
        lines.append(f"[functions.{_key(path)}]")
        held = ratchet.functions[path]
        lines.extend(f"{_key(name)} = {size}" for name, size in sorted(held.items()))
    return "\n".join(lines) + "\n"


def summary(measurement: Measurement) -> str:
    over = over_limits(measurement)
    parts = []
    for language in ("rust", "python", "swift"):
        paths = [p for p, lang in measurement.language.items() if lang == language]
        files = sum(1 for path in paths if path in over.files)
        functions = sum(len(over.functions.get(path, {})) for path in paths)
        total = sum(len(measurement.functions[path]) for path in paths)
        parts.append(
            f"{language}: {len(paths)} files ({files} over {FILE_LIMIT}), "
            f"{total} functions ({functions} over {FUNCTION_LIMIT})"
        )
    return "; ".join(parts)


def main(arguments: list[str]) -> int:
    if arguments not in ([], ["--shrink"]):
        print(__doc__, file=sys.stderr)
        return 2
    measurement = measure(ROOT)
    ratchet = load_ratchet(RATCHET)
    if arguments == ["--shrink"]:
        lowered = shrink(measurement, ratchet)
        RATCHET.write_text(render_ratchet(lowered), encoding="utf-8")
        ratchet = lowered
    found = problems(measurement, ratchet)
    print(summary(measurement))
    for problem in found:
        print(f"  {problem}")
    if found:
        print(f"{len(found)} sizes differ from {RATCHET.relative_to(ROOT)}")
        return 1
    print(
        f"every file and function is within the limits or held, exactly, by {RATCHET.relative_to(ROOT)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
