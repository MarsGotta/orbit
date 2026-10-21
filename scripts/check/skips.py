#!/usr/bin/env python3
"""A skipped test is not a passed test.

Reads what a suite really ran, from its own report, and fails when a test was
skipped (or marked as an expected failure) without an entry in
`allowed-skips.toml` that names the suite, the test and why. The reports:

- ``--junit``: JUnit XML from pytest or vitest;
- ``--cargo-log``: the output of ``cargo test``, colours included;
- ``--unittest``: the JSON written by ``unittest_runner.py``.

Every text is read with its colour codes removed first, so a coloured
``ignored`` cannot hide a skip.

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
        raise InvalidAllowlist(f"cannot read {path}: {error}") from error
    allowed = {}
    for index, entry in enumerate(document.get("skip", []), start=1):
        missing = [
            field
            for field in ("suite", "test", "reason")
            if not isinstance(entry.get(field), str) or not entry[field].strip()
        ]
        if missing:
            names = {"suite": "suite", "test": "test", "reason": "reason (why)"}
            raise InvalidAllowlist(
                f"{path}: entry {index} ({entry.get('test', 'no test')}) has no "
                + ", ".join(names[field] for field in missing)
                + "; a skip is allowed only with its suite, its test and its reason"
            )
        key = (entry["suite"].strip(), entry["test"].strip())
        if key in allowed:
            raise InvalidAllowlist(f"{path}: {key[1]} appears twice in {key[0]}")
        allowed[key] = entry["reason"].strip()
    unknown = set(document) - {"skip"}
    if unknown:
        raise InvalidAllowlist(f"{path}: unknown keys {sorted(unknown)}")
    return allowed


def read_junit(path: Path) -> Report:
    if not path.is_file():
        return Report(
            0, [], [f"the report {path} was not written; without it nobody knows which tests ran"]
        )
    try:
        root = ElementTree.fromstring(strip_ansi(path.read_text(encoding="utf-8")))
    except ElementTree.ParseError as error:
        return Report(0, [], [f"the report {path} is not valid XML: {error}"])
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
        return Report(0, [], [f"there is no cargo output {path}"])
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
            f"cargo counted {counted_ignored} ignored tests and only {len(skips)} were named; "
            "without a name they cannot be compared with the allowed skips"
        )
    return Report(tests, skips, problems, failed)


def read_unittest(path: Path) -> Report:
    if not path.is_file():
        return Report(
            0, [], [f"the report {path} was not written; without it nobody knows which guards ran"]
        )
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        skips = [Skip(test, reason) for test, reason in data["skipped"]]
        skips += [
            Skip(test, "marked as an expected failure") for test, _ in data["expected_failures"]
        ]
        failed = [test for test, _ in data.get("failures", []) + data.get("errors", [])]
        return Report(int(data["tests_run"]), skips, [], failed)
    except (ValueError, KeyError, TypeError) as error:
        return Report(0, [], [f"the report {path} cannot be read: {error}"])


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
        print(f"  failed: {test}")
    for problem in problems:
        print(f"  {problem}")
    if not problems and tests == 0:
        problems.append("the report contains no test")
        print(
            f"  {suite}: the report contains no test; an empty suite is not a passed suite"
        )
    refused = []
    for skip in skips:
        reason = allowed.get((suite, skip.test))
        if reason is None:
            refused.append(skip)
        else:
            print(f"  allowed skip: {skip.test} · {reason}")
    for skip in refused:
        said = f" (the test says: {skip.reason})" if skip.reason else ""
        print(f"  skip not allowed: {skip.test}{said}")
    if refused:
        print(
            f"  {suite}: {plural(len(refused), 'skip not allowed', 'skips not allowed')}. "
            f"A skipped test is not a passed test: make it run, or add it to {ALLOWLIST_HINT} "
            "with its suite and its reason."
        )
    causes = []
    if failed:
        named = ", ".join(failed[:NAMED_FAILURES])
        if len(failed) > NAMED_FAILURES:
            named += f" and {len(failed) - NAMED_FAILURES} more"
        causes.append(f"{len(failed)} of {tests} tests failed: {named}")
    if refused:
        causes.append(plural(len(refused), "skip not allowed", "skips not allowed"))
    causes.extend(problems)
    status = 1 if refused or problems else 0
    if not causes:
        print(
            f"  {suite}: {tests} tests, {plural(len(skips), 'allowed skip', 'allowed skips')}"
        )
    return status, "; ".join(causes)


def main(arguments: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Compares a suite's skips with its allowed list."
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
            options.cause_file.write_text("invalid list of allowed skips\n", encoding="utf-8")
        return 2
    if options.validate:
        print(
            f"{options.allow}: {plural(len(allowed), 'allowed skip', 'allowed skips')}, all with a reason"
        )
        return 0
    if not options.suite:
        parser.error("--suite is required except with --validate")
    reports = (
        [read_junit(path) for path in options.junit]
        + [read_cargo_log(path) for path in options.cargo_log]
        + [read_unittest(path) for path in options.unittest]
    )
    if not reports:
        parser.error("a report is missing: --junit, --cargo-log or --unittest")
    status, cause = check(options.suite, allowed, reports)
    if options.cause_file and cause:
        options.cause_file.write_text(cause + "\n", encoding="utf-8")
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
