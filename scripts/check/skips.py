#!/usr/bin/env python3
"""A skipped test is not a passed test (REF-02 A2).

Reads what a suite really ran, from its own report, and fails when a test was
skipped (or marked as an expected failure) without an entry in
`allowed-skips.toml` that names the suite, the test and why. The reports:

- ``--junit``: JUnit XML from pytest or vitest;
- ``--cargo-log``: the output of ``cargo test``, colours included;
- ``--unittest``: the JSON written by ``unittest_runner.py``.

Every text is read with its colour codes removed first, so a coloured
``ignored`` cannot hide a skip (A8).

Usage:
  skips.py --suite ID --allow FILE [--cause-file FILE] (--junit X | --cargo-log X | --unittest X)...
  skips.py --validate --allow FILE
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from xml.etree import ElementTree

# CSI sequences (colours, cursor), OSC sequences (hyperlinks, titles) and every
# other escape: ESC, optional intermediate bytes, one final byte. libtest ends a
# colour with ESC ( B, which a CSI-only pattern leaves behind as "(B".
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[ -/]*[0-~]")
CARGO_RUNNING = re.compile(r"^\s*Running (?:unittests )?\S+ \((?P<binary>[^)]+)\)\s*$")
CARGO_DOCTESTS = re.compile(r"^\s*Doc-tests (?P<crate>\S+)\s*$")
CARGO_IGNORED = re.compile(r"^test (?P<name>.+?) \.\.\. ignored(?:, (?P<reason>.*))?$")
CARGO_FAILED = re.compile(r"^test (?P<name>.+?) \.\.\. FAILED$")
CARGO_RESULT = re.compile(
    r"^test result: \w+\. (?P<passed>\d+) passed; (?P<failed>\d+) failed; (?P<ignored>\d+) ignored;"
)
CARGO_HASH = re.compile(r"-[0-9a-f]{8,}$")
ALLOWLIST_HINT = "scripts/check/allowed-skips.toml"
# How many failed tests the one-line cause names; the step output lists them
# all. It keeps the final table readable, it does not hide the rest.
NAMED_FAILURES = 3


@dataclass(frozen=True)
class Skip:
    test: str
    reason: str


@dataclass
class Report:
    tests: int
    skips: list[Skip]
    problems: list[str]
    failed: list[str] = field(default_factory=list)


class InvalidAllowlist(ValueError):
    pass


def strip_ansi(text: str) -> str:
    return ANSI.sub("", text)


def load_allowlist(path: Path) -> dict[tuple[str, str], str]:
    try:
        document = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise InvalidAllowlist(f"no puedo leer {path}: {error}") from error
    allowed = {}
    for index, entry in enumerate(document.get("skip", []), start=1):
        missing = [
            field
            for field in ("suite", "test", "reason")
            if not isinstance(entry.get(field), str) or not entry[field].strip()
        ]
        if missing:
            names = {"suite": "suite", "test": "test", "reason": "reason (el motivo)"}
            raise InvalidAllowlist(
                f"{path}: la entrada {index} ({entry.get('test', 'sin test')}) no tiene "
                + ", ".join(names[field] for field in missing)
                + "; un salto sólo se permite con su suite, su test y su motivo"
            )
        key = (entry["suite"].strip(), entry["test"].strip())
        if key in allowed:
            raise InvalidAllowlist(f"{path}: {key[1]} aparece dos veces en {key[0]}")
        allowed[key] = entry["reason"].strip()
    unknown = set(document) - {"skip"}
    if unknown:
        raise InvalidAllowlist(f"{path}: claves desconocidas {sorted(unknown)}")
    return allowed


def read_junit(path: Path) -> Report:
    if not path.is_file():
        return Report(
            0, [], [f"no se generó el informe {path}; sin él no se sabe qué tests corrieron"]
        )
    try:
        root = ElementTree.fromstring(strip_ansi(path.read_text(encoding="utf-8")))
    except ElementTree.ParseError as error:
        return Report(0, [], [f"el informe {path} no es XML válido: {error}"])
    tests = 0
    skips = []
    failed = []
    for case in root.iter("testcase"):
        tests += 1
        test = f"{case.get('classname', '')}::{case.get('name', '')}"
        skipped = case.find("skipped")
        if skipped is not None:
            skips.append(Skip(test, (skipped.get("message") or skipped.text or "").strip()))
        if case.find("failure") is not None or case.find("error") is not None:
            failed.append(test)
    return Report(tests, skips, [], failed)


def read_cargo_log(path: Path) -> Report:
    if not path.is_file():
        return Report(0, [], [f"no existe la salida de cargo {path}"])
    binary = "?"
    tests = 0
    counted_ignored = 0
    skips = []
    failed = []
    for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = strip_ansi(raw).rstrip()
        if match := CARGO_RUNNING.match(line):
            binary = CARGO_HASH.sub("", Path(match["binary"]).name)
        elif match := CARGO_DOCTESTS.match(line):
            binary = f"doc-{match['crate']}"
        elif match := CARGO_IGNORED.match(line):
            skips.append(Skip(f"{binary}::{match['name']}", (match["reason"] or "").strip()))
        elif match := CARGO_FAILED.match(line):
            failed.append(f"{binary}::{match['name']}")
        elif match := CARGO_RESULT.match(line):
            tests += sum(int(match[field]) for field in ("passed", "failed", "ignored"))
            counted_ignored += int(match["ignored"])
    problems = []
    if counted_ignored != len(skips):
        problems.append(
            f"cargo contó {counted_ignored} tests ignorados y sólo se identificaron {len(skips)}; "
            "sin nombre no se pueden comparar con la lista de saltos permitidos"
        )
    return Report(tests, skips, problems, failed)


def read_unittest(path: Path) -> Report:
    if not path.is_file():
        return Report(
            0, [], [f"no se generó el informe {path}; sin él no se sabe qué guards corrieron"]
        )
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        skips = [Skip(test, reason) for test, reason in data["skipped"]]
        skips += [
            Skip(test, "marcado como fallo esperado") for test, _ in data["expected_failures"]
        ]
        failed = [test for test, _ in data.get("failures", []) + data.get("errors", [])]
        return Report(int(data["tests_run"]), skips, [], failed)
    except (ValueError, KeyError, TypeError) as error:
        return Report(0, [], [f"el informe {path} no se puede leer: {error}"])


def plural(count: int, singular: str, plural_form: str) -> str:
    return f"{count} {singular if count == 1 else plural_form}"


def check(
    suite: str, allowed: dict[tuple[str, str], str], reports: list[Report]
) -> tuple[int, str]:
    problems = [problem for report in reports for problem in report.problems]
    tests = sum(report.tests for report in reports)
    skips = [skip for report in reports for skip in report.skips]
    failed = [test for report in reports for test in report.failed]
    for test in failed:
        print(f"  falló: {test}")
    for problem in problems:
        print(f"  {problem}")
    if not problems and tests == 0:
        problems.append("el informe no contiene ningún test")
        print(
            f"  {suite}: el informe no contiene ningún test; una suite vacía no es una suite pasada"
        )
    refused = []
    for skip in skips:
        reason = allowed.get((suite, skip.test))
        if reason is None:
            refused.append(skip)
        else:
            print(f"  salto permitido: {skip.test} · {reason}")
    for skip in refused:
        said = f" (el test dice: {skip.reason})" if skip.reason else ""
        print(f"  salto no permitido: {skip.test}{said}")
    if refused:
        print(
            f"  {suite}: {plural(len(refused), 'salto no permitido', 'saltos no permitidos')}. "
            f"Un test saltado no es un test pasado: haz que corra, o añádelo a {ALLOWLIST_HINT} "
            "con su suite y su motivo."
        )
    causes = []
    if failed:
        named = ", ".join(failed[:NAMED_FAILURES])
        if len(failed) > NAMED_FAILURES:
            named += f" y {len(failed) - NAMED_FAILURES} más"
        causes.append(f"{len(failed)} de {tests} tests fallaron: {named}")
    if refused:
        causes.append(plural(len(refused), "salto no permitido", "saltos no permitidos"))
    causes.extend(problems)
    status = 1 if refused or problems else 0
    if not causes:
        print(
            f"  {suite}: {tests} tests, {plural(len(skips), 'salto permitido', 'saltos permitidos')}"
        )
    return status, "; ".join(causes)


def main(arguments: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Compara los saltos de una suite con su lista permitida."
    )
    parser.add_argument("--allow", required=True, type=Path)
    parser.add_argument("--suite")
    parser.add_argument("--validate", action="store_true")
    parser.add_argument("--cause-file", type=Path)
    parser.add_argument("--junit", action="append", type=Path, default=[])
    parser.add_argument("--cargo-log", action="append", type=Path, default=[])
    parser.add_argument("--unittest", action="append", type=Path, default=[])
    options = parser.parse_args(arguments)
    try:
        allowed = load_allowlist(options.allow)
    except InvalidAllowlist as error:
        print(error)
        if options.cause_file:
            options.cause_file.write_text("lista de saltos permitidos inválida\n", encoding="utf-8")
        return 2
    if options.validate:
        print(
            f"{options.allow}: {plural(len(allowed), 'salto permitido', 'saltos permitidos')}, todos con motivo"
        )
        return 0
    if not options.suite:
        parser.error("--suite es obligatorio salvo con --validate")
    reports = (
        [read_junit(path) for path in options.junit]
        + [read_cargo_log(path) for path in options.cargo_log]
        + [read_unittest(path) for path in options.unittest]
    )
    if not reports:
        parser.error("falta un informe: --junit, --cargo-log o --unittest")
    status, cause = check(options.suite, allowed, reports)
    if options.cause_file and cause:
        options.cause_file.write_text(cause + "\n", encoding="utf-8")
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
