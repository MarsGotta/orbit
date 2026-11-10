"""The tests of what the Mac sends, moved to the contract kit.

They upload a meeting's audio chunks over `PUT /v1/chunks/{id}`, close it with
each closing manifest the Mac writes (`contracts/fixtures/recording-manifest-*`),
read the device's listing and the one signed for the assistant, and check a
meeting's note by what the notes model is asked. Here they started today's hub
inside the test process, wrote the chunks with its store and read the note's
inputs from its repository, so they could test no other hub; in the kit they
reach a hub only through its address and the command in `ORBIT_KIT_HUB`.

These tests may import the hub: they are its oracle. They check that the old
tests are gone from here, that the kit does what they did through the network,
and that the kit still notices each wrong hub the old tests noticed: a wrapper
changes one thing today's hub does, and the kit must fail.
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
MOVED = SERVER / "tests" / "test_mac_meeting_contract.py"
# What a kit file that closes recordings with the Mac's manifests names.
MANIFESTS = "recording-manifest-"
# The tests that moved, by their names here.
OLD_TESTS = (
    "test_every_closing_manifest_the_mac_writes_is_accepted_with_its_kind_and_event",
    "test_a_voice_note_left_open_is_not_a_meeting_and_a_meeting_just_started_is",
    "test_meetings_that_start_together_are_all_listed_across_pages",
)
OLD_CASES = 3
# What the old tests had the hub hold: five chunks for each of the six
# manifests, two open recordings of one chunk and three meetings of one; and
# the nine recordings they closed.
OLD_CHUNKS = 6 * 5 + 2 + 3
OLD_FINALIZED = 6 + 3
# A page asked to go on after a row (`before_id`), and the list signed for
# the assistant (`since_ms`).
OPERATIONS = {"page", "assistant_list"}
# Wrong hubs, each named by what it gets wrong: each one made the old tests
# fail here (measured before the move).
MUTANTS = (
    "a voice note left open is listed",
    "a voice call is listed as a meeting",
    "the muted spans are lost",
    "a page ignores the id it goes on after",
    "the note is asked without the event",
    "the note is asked without the attendees",
    "the note is asked with the attendees reversed",
    "the note is asked without the series",
)
KIT_RUN_SECONDS = 180
# A hub the kit stopped may still be closing when the kit's process ends: it is
# given this long before it counts as left running. Uvicorn closes in well
# under a second.
HUB_EXIT_SECONDS = 5

WATCHED_HUB = """
import json, os, sys

LOG = os.environ["KIT_WATCH_LOG"]
MUTANT = os.environ.get("KIT_MAC_MUTANT", "")


def note(operation):
    with open(LOG, "a") as out:
        out.write(json.dumps({"pid": os.getpid(), "operation": operation}) + "\\n")


with open(LOG, "a") as out:
    out.write(json.dumps({"pid": os.getpid(), "argv": sys.argv[1:]}) + "\\n")

from orbit_hub.meetings import muted, views
from orbit_hub.notes.repository import NoteRepository
from orbit_hub.storage.recordings import RecordingRepository

real_put, real_finalize = RecordingRepository.put_chunk, RecordingRepository.finalize
real_listing, real_before = views.listing, views.before
real_voice_note, real_manifest = views.voice_note, views._manifest
real_muted, real_planned = muted.read, NoteRepository.planned


def put_chunk(self, *arguments, **named):
    note("chunk")
    return real_put(self, *arguments, **named)


def finalize(self, *arguments, **named):
    note("finalize")
    return real_finalize(self, *arguments, **named)


def listing(*arguments, **named):
    if named.get("since_ms") is not None:
        note("assistant_list")
    if len(arguments) > 6 and arguments[6] is not None:
        note("page")
    return real_listing(*arguments, **named)


def before(entry, before_ms, before_id=None):
    if MUTANT == "a page ignores the id it goes on after":
        before_id = None
    return real_before(entry, before_ms, before_id)


def voice_note(*arguments):
    if MUTANT == "a voice note left open is listed":
        return False
    return real_voice_note(*arguments)


def manifest(*arguments):
    value = real_manifest(*arguments)
    if MUTANT == "a voice call is listed as a meeting":
        value = {key: inner for key, inner in value.items() if key != "kind"}
    return value


def muted_read(*arguments):
    if MUTANT == "the muted spans are lost":
        return ()
    return real_muted(*arguments)


def planned(self, *arguments, **named):
    inputs = real_planned(self, *arguments, **named)
    event = inputs.event
    if event is None:
        return inputs
    if MUTANT == "the note is asked without the event":
        return inputs.model_copy(update={"event": None})
    if MUTANT == "the note is asked without the attendees":
        event = event.model_copy(update={"attendees": ()})
    elif MUTANT == "the note is asked with the attendees reversed":
        event = event.model_copy(update={"attendees": tuple(reversed(event.attendees))})
    elif MUTANT == "the note is asked without the series":
        event = event.model_copy(update={"calendar_item_id": None})
    return inputs.model_copy(update={"event": event})


RecordingRepository.put_chunk, RecordingRepository.finalize = put_chunk, finalize
views.listing, views.before, views.voice_note, views._manifest = listing, before, voice_note, manifest
muted.read, NoteRepository.planned = muted_read, planned
from orbit_hub.cli import main

main()
"""


def kit_mac_files() -> list[Path]:
    if not KIT.is_dir():
        return []
    return sorted(path for path in KIT.rglob("test_*.py") if MANIFESTS in path.read_text())


def defined_tests(path: Path) -> set[str]:
    tree = ast.parse(path.read_text())
    return {
        node.name
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }


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
    """The kit's tests of what the Mac sends, in their own process, against
    today's hub run as a program through a wrapper that writes down what it
    does (and, with `mutant`, does that one thing wrong)."""
    work = tmp_path / name
    work.mkdir()
    (work / "watched_hub.py").write_text(WATCHED_HUB)
    log, junit = work / "hub.log", work / "kit.xml"
    environment = {
        **os.environ,
        "ORBIT_KIT_HUB": shlex.join([sys.executable, str(work / "watched_hub.py")]),
        "KIT_WATCH_LOG": str(log),
        "KIT_MAC_MUTANT": mutant,
        "KIT_WATCH_RUN": str(work),
        "PYTHONPATH": str(work),
    }
    environment.pop("PYTEST_ADDOPTS", None)
    files = kit_mac_files()
    assert files, f"no test of {KIT} closes recordings with the Mac's {MANIFESTS}* manifests"
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
    operations = [line["operation"] for line in lines if "operation" in line]
    return {
        "code": kit.returncode,
        "timed_out": timed_out,
        "left": left,
        "output": (work / "kit.out").read_text(errors="replace")[-4000:],
        "commands": [line["argv"] for line in lines if "argv" in line],
        "operations": operations,
        **counts,
    }


def test_the_mac_meeting_tests_left_the_hub_tests():
    assert not MOVED.exists(), f"{MOVED.name} is still here: it moves to the kit"
    here = sorted((SERVER / "tests").glob("*.py"))
    kept = {
        path.name: sorted(defined_tests(path) & set(OLD_TESTS))
        for path in here
        if path != Path(__file__).resolve()
    }
    assert not {name: tests for name, tests in kept.items() if tests}, kept
    assert kit_mac_files(), f"no test of {KIT} closes recordings with the Mac's manifests"


def test_the_kit_checks_what_the_mac_sends_through_the_network(tmp_path):
    run = run_kit(tmp_path, "clean")
    assert not run["timed_out"], run["output"]
    assert run["code"] == 0, run["output"]
    assert run["left"] == [], f"the kit left its hub running: {run['left']}"
    assert run["skipped"] == 0, run["output"]
    assert run["tests"] - run["failures"] - run["errors"] >= OLD_CASES, run["output"]
    assert any("pair-code" in command for command in run["commands"]), run["commands"]
    assert any("serve" in command for command in run["commands"]), run["commands"]
    assert run["operations"].count("chunk") >= OLD_CHUNKS, run["operations"]
    assert run["operations"].count("finalize") >= OLD_FINALIZED, run["operations"]
    assert OPERATIONS <= set(run["operations"]), run["operations"]


@pytest.mark.parametrize("mutant", MUTANTS)
def test_the_kit_notices_a_hub_that_gets_what_the_mac_sends_wrong(tmp_path, mutant):
    run = run_kit(tmp_path, "mutant", mutant)
    assert not run["timed_out"], run["output"]
    assert run["left"] == [], f"the kit left its hub running: {run['left']}"
    # The hub served and took the kit's audio: the kit failed on what it
    # answered, not on a hub that never started.
    assert "chunk" in run["operations"], run["output"]
    assert run["code"] != 0 and run["failures"] + run["errors"] >= 1, (
        f"the kit passed with a hub where {mutant}:\n{run['output']}"
    )
