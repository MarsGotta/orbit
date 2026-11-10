"""The hub-command test of `serve` without its transport, moved to the contract kit.

A hub run as a program must refuse to serve without TLS beyond loopback, on
loopback without `allow_insecure_localhost`, and with only one of its two TLS
files: it exits with 1, says the cause in one line of its error output and
never listens. Here the test called the hub's `main()` inside the test process
and replaced its web server, so it could test no other hub; in the kit it
starts a hub only through the command in `ORBIT_KIT_HUB`.

These tests may import the hub: they are its oracle. They check that the old
test is gone from here, that the kit sees today's hub refuse each case, and
that the kit notices a hub that serves, refuses for another cause, refuses in
silence, with a zero exit or with more than its cause, while a hub that only
words the start of its line otherwise still passes.
"""

import ast
import json
import os
import shlex
import signal
import subprocess
import sys
import time
from pathlib import Path
from xml.etree import ElementTree

import pytest

ROOT = Path(__file__).resolve().parents[2]
SERVER = ROOT / "server"
KIT = ROOT / "contracts" / "kit"
MOVED = SERVER / "tests" / "test_hub_cli.py"
OLD_TEST = "test_serve_refuses_to_start_without_its_transport"
# What names the kit's tests of these refusals among its files.
MARK = "--tls-cert"
# The old test's cases: beyond loopback, loopback without the permission, one TLS file.
OLD_CASES = 3
# Modules the kit cannot use: it reaches a hub only as a program.
HUB_MODULES = ("orbit_hub", "orbit_mcp", "uvicorn")
LOOPBACK = ("127.0.0.1", "localhost", "::1")
KIT_RUN_SECONDS = 120
# A hub the kit stopped may still be closing when the kit's process ends: it is
# given this long before it counts as left running. Uvicorn closes in well
# under a second.
HUB_EXIT_SECONDS = 5
# Wrong hubs, by what the wrapper changes. Each made the old test fail here
# (measured before the move); `prefix` did not, and the kit must take it.
MUTANTS = {
    "it serves without TLS beyond loopback": "public",
    "it serves on loopback without the permission": "permission",
    "it takes one TLS file and fails for another cause": "one-file",
    "it refuses without saying why": "silent",
    "it refuses with a zero exit": "zero",
    "it writes another line beside its cause": "extra-line",
}

WATCHED_HUB = """
import json, os, sys, tomllib

LOG = os.environ["KIT_WATCH_LOG"]
MUTANT = os.environ.get("KIT_SERVE_MUTANT", "")
argv = sys.argv[1:]
configuration = argv[argv.index("--config") + 1]
with open(configuration, "rb") as source:
    permission = tomllib.load(source).get("allow_insecure_localhost")
with open(LOG, "a") as out:
    out.write(json.dumps({"pid": os.getpid(), "argv": argv, "permission": permission}) + "\\n")

from dataclasses import replace

import uvicorn

from orbit_hub import cli

real_run, real_serve = uvicorn.run, cli.serve


def loopback_run(*args, **kwargs):
    # Whatever a wrong hub was asked, it listens on loopback only.
    with open(LOG, "a") as out:
        out.write(json.dumps({"pid": os.getpid(), "served": kwargs.get("port")}) + "\\n")
    return real_run(*args, **{**kwargs, "host": "127.0.0.1"})


uvicorn.run = loopback_run


def serve(settings, args):
    if MUTANT == "permission":
        settings = replace(settings, allow_insecure_localhost=True)
    if MUTANT == "one-file" and args.tls_cert and not args.tls_key:
        args.tls_key = args.tls_cert
    return real_serve(settings, args)


cli.serve = serve
if MUTANT == "public":
    cli.LOOPBACK_HOSTS = cli.LOOPBACK_HOSTS | {"0.0.0.0"}
if MUTANT == "silent":
    sys.stderr = open(os.devnull, "w")
if MUTANT == "extra-line":
    print("Orbit Hub: reading the configuration", file=sys.stderr)


class Reworded:
    def __init__(self, stream):
        self.stream = stream

    def write(self, text):
        return self.stream.write(text.replace("Orbit Hub: ", "hub refused: "))

    def __getattr__(self, name):
        return getattr(self.stream, name)


if MUTANT == "prefix":
    sys.stderr = Reworded(sys.stderr)
try:
    cli.main()
    code = 0
except SystemExit as stopped:
    code = stopped.code if isinstance(stopped.code, int) else 1
with open(LOG, "a") as out:
    out.write(json.dumps({"pid": os.getpid(), "exit": code}) + "\\n")
sys.exit(0 if MUTANT == "zero" else code)
"""


def kit_serve_files() -> list[Path]:
    if not KIT.is_dir():
        return []
    return sorted(path for path in KIT.rglob("test_*.py") if MARK in path.read_text())


def defined_tests(path: Path) -> set[str]:
    tree = ast.parse(path.read_text())
    return {
        node.name
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }


def imported_modules(path: Path) -> set[str]:
    found = set()
    for node in ast.walk(ast.parse(path.read_text())):
        if isinstance(node, ast.Import):
            found |= {alias.name.split(".")[0] for alias in node.names}
        elif isinstance(node, ast.ImportFrom) and node.module and node.level == 0:
            found.add(node.module.split(".")[0])
    return found


def processes_of(work: Path) -> list[int]:
    """Every process that inherited this run's mark (the kit's and its hubs')."""
    mark = f"KIT_WATCH_RUN={work}".encode()
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid():
            continue
        try:
            if mark in (entry / "environ").read_bytes().split(b"\0"):
                found.append(int(entry.name))
        except OSError:
            continue
    return found


def stop_processes_left(work: Path) -> list[int]:
    deadline = time.monotonic() + HUB_EXIT_SECONDS
    left = processes_of(work)
    while left and time.monotonic() < deadline:
        time.sleep(0.1)
        left = processes_of(work)
    for pid in left:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    return left


def run_kit(tmp_path: Path, name: str, mutant: str = "") -> dict:
    """The kit's tests of these refusals, in their own process, against
    today's hub run as a program through a wrapper that writes down each
    command, its configuration's permission and its exit (and, with
    `mutant`, behaves wrong)."""
    work = tmp_path / name
    work.mkdir()
    (work / "watched_hub.py").write_text(WATCHED_HUB)
    log, junit = work / "hub.log", work / "kit.xml"
    environment = {
        **os.environ,
        "ORBIT_KIT_HUB": shlex.join([sys.executable, str(work / "watched_hub.py")]),
        "KIT_WATCH_LOG": str(log),
        "KIT_SERVE_MUTANT": mutant,
        "KIT_WATCH_RUN": str(work),
        "PYTHONPATH": str(work),
    }
    environment.pop("PYTEST_ADDOPTS", None)
    files = kit_serve_files()
    assert files, f"no test of {KIT} names {MARK}"
    # Output to files, not pipes: a hub left running would hold a pipe open.
    with open(work / "kit.out", "wb") as out:
        kit = subprocess.Popen(
            [
                sys.executable,
                "-m",
                "pytest",
                "-q",
                "-p",
                "no:cacheprovider",
                "-c",
                str(SERVER / "pyproject.toml"),
                "--rootdir",
                str(SERVER),
                f"--basetemp={work / 'base'}",
                f"--junitxml={junit}",
                *map(str, files),
            ],
            cwd=SERVER,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=out,
            stderr=subprocess.STDOUT,
        )
        timed_out = False
        try:
            kit.wait(timeout=KIT_RUN_SECONDS)
        except subprocess.TimeoutExpired:
            timed_out = True
            kit.kill()
            kit.wait()
        finally:
            left = stop_processes_left(work)
    counts = {"tests": 0, "failures": 0, "errors": 0, "skipped": 0}
    if junit.exists():
        for suite in ElementTree.parse(junit).getroot().iter("testsuite"):
            for key in counts:
                counts[key] += int(suite.get(key, 0))
    lines = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
    starts = {line["pid"]: line for line in lines if "argv" in line}
    for line in lines:
        if "exit" in line and line["pid"] in starts:
            starts[line["pid"]]["exit"] = line["exit"]
    return {
        "code": kit.returncode,
        "timed_out": timed_out,
        "left": left,
        "output": (work / "kit.out").read_text(errors="replace")[-4000:],
        "serves": [start for start in starts.values() if "serve" in start["argv"]],
        "served": [line["served"] for line in lines if "served" in line],
        **counts,
    }


def option(argv: list[str], name: str) -> str | None:
    return argv[argv.index(name) + 1] if name in argv else None


def refused_cases(serves: list[dict]) -> set[str]:
    """Which of the old test's cases the kit asked and the hub refused with 1."""
    cases = set()
    for serve in serves:
        argv, refused = serve["argv"], serve.get("exit") == 1
        host = option(argv, "--host") or "127.0.0.1"
        cert, key = "--tls-cert" in argv, "--tls-key" in argv
        if not refused:
            continue
        if host == "0.0.0.0" and not cert and not key and serve["permission"] is True:
            cases.add("beyond loopback")
        if host in LOOPBACK and not cert and not key and serve["permission"] is False:
            cases.add("loopback without the permission")
        if cert and not key and serve["permission"] is False:
            cases.add("one TLS file")
    return cases


def test_the_hub_cli_test_left_the_hub_tests():
    assert not MOVED.exists(), f"{MOVED.name} is still here: it moves to the kit"
    here = sorted((SERVER / "tests").glob("*.py"))
    kept = [path.name for path in here if OLD_TEST in defined_tests(path)]
    assert not kept, kept
    files = kit_serve_files()
    assert files, f"no test of {KIT} names {MARK}"
    for path in files:
        assert not imported_modules(path) & set(HUB_MODULES), (
            f"{path.name} imports the hub: the kit starts it as a program"
        )


def test_the_kit_sees_the_hub_refuse_to_serve(tmp_path):
    run = run_kit(tmp_path, "clean")
    assert not run["timed_out"], run["output"]
    assert run["code"] == 0, run["output"]
    assert run["left"] == [], f"the kit left its hub running: {run['left']}"
    assert run["skipped"] == 0, run["output"]
    assert run["tests"] - run["failures"] - run["errors"] >= OLD_CASES, run["output"]
    assert refused_cases(run["serves"]) == {
        "beyond loopback",
        "loopback without the permission",
        "one TLS file",
    }, run["serves"]
    assert run["served"] == [], run["served"]


def test_the_kit_takes_a_hub_that_words_the_start_of_its_line_otherwise(tmp_path):
    run = run_kit(tmp_path, "prefix", "prefix")
    assert not run["timed_out"], run["output"]
    assert run["left"] == [], f"the kit left its hub running: {run['left']}"
    assert run["serves"], run["output"]
    assert run["code"] == 0, f"the kit compares the hub's prefix, not its cause:\n{run['output']}"


@pytest.mark.parametrize("mutant", sorted(MUTANTS))
def test_the_kit_notices_a_hub_that_does_not_refuse_as_it_should(tmp_path, mutant):
    run = run_kit(tmp_path, "mutant", MUTANTS[mutant])
    assert not run["timed_out"], run["output"]
    assert run["left"] == [], f"the kit left its hub running: {run['left']}"
    # The kit asked the hub to serve: it failed on the hub's answer, not
    # before starting one.
    assert run["serves"], run["output"]
    assert run["code"] != 0 and run["failures"] + run["errors"] >= 1, (
        f"the kit passed with a hub where {mutant}:\n{run['output']}"
    )
