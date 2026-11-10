"""The vault-format tests, moved to the contract kit.

They read a vault's notes, roles, fields, tasks and warnings through the
authenticated reading API (`POST /v1/vault/read`), over a vault folder they
write, on a hub of their own with no sample database. Here they started today's
hub inside the test process, so they could test no other hub; in the kit they
reach a hub only through its address and the command in `ORBIT_KIT_HUB`.

These tests may import the hub: they are its oracle. They check that the old
tests are gone from here, that the kit reads the vault through the network
with as many cases as before, and that the kit still notices each wrong answer
the old tests noticed: a hub whose vault reads answer one field wrong (a
wrapper changes what `Vault.read` answers) must make the kit fail.
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
# The second shares the first one's vault and its helpers, so they move together.
MOVED = (SERVER / "tests" / "test_vault_format.py", SERVER / "tests" / "test_vault_okf.py")
READ_PATH = "/v1/vault/read"
# The tests that moved, by their names here.
OLD_TESTS = (
    "test_lifecycle_and_project_stage_are_independent",
    "test_schema_cannot_rename_or_reuse_okf_fields",
    "test_missing_type_is_exposed_without_refusing_or_inventing_conformance",
    "test_schema_is_a_concept_with_any_nonempty_type",
    "test_invalid_metadata_is_soft_and_names_the_field",
    "test_provenance_normalization_preserves_unknown_keys",
    "test_root_nested_indexes_and_free_prose_logs_are_tolerated",
    "test_synthesized_index_uses_okf_links_and_default_link_style",
    "test_reserved_frontmatter_violations_are_soft",
    "test_a_plain_log_entry_linking_a_note_appears_in_its_history",
    "test_defaults_without_schema",
    "test_custom_roles_folders_fields_and_unknown_type",
    "test_bad_schema_is_advisory_and_defaults_apply",
    "test_tasks_markers_owners_cross_note_dependencies_and_queries",
    "test_missing_schema_keys_use_english_defaults",
    "test_nearest_folder_and_no_type_name_special_cases",
    "test_schema_failure_discards_all_overrides",
    "test_tasks_after_metadata_and_completed_due_dates",
    "test_schema_and_query_warnings_carry_codes_without_removing_legacy_text",
)
# Their cases here, parametrized ones counted one by one: the kit keeps them all.
OLD_CASES = 14 + 31
# The read operations they ask for.
OPERATIONS = {"catalog", "read", "tasks"}
# Wrong answers, as the field of every object of a read's answer that changes
# and how: each one made the old tests fail here (measured before the move).
MUTANTS = {
    "a note's role is always note": ("role", "set:note"),
    "a note's start field is lost": ("start", "drop"),
    "a task's dependencies are lost": ("dependencies", "empty"),
    "a warning loses its code": ("code", "drop"),
    "the schema's types are lost": ("types", "empty"),
    "a note's queries are lost": ("queries", "empty"),
    "the warnings are lost": ("warnings", "empty"),
    "a note's conformance is lost": ("conformance", "drop"),
    "a conformance's issues are lost": ("issues", "empty"),
    "a note's properties are lost": ("properties", "empty"),
    "a note's history is lost": ("history", "empty"),
}
KIT_RUN_SECONDS = 120
# A hub the kit stopped may still be closing when the kit's process ends: it is
# given this long before it counts as left running. Uvicorn closes in well
# under a second.
HUB_EXIT_SECONDS = 5

WATCHED_HUB = """
import json, os, sys

LOG = os.environ["KIT_WATCH_LOG"]
MUTANT = os.environ.get("KIT_VAULT_MUTANT", "")
with open(LOG, "a") as out:
    out.write(json.dumps({"pid": os.getpid(), "argv": sys.argv[1:]}) + "\\n")

from orbit_hub.vault import service


def mutate(value, key, action):
    if isinstance(value, dict):
        if key in value:
            if action == "drop":
                del value[key]
            elif action == "empty":
                value[key] = type(value[key])()
            elif action.startswith("set:"):
                value[key] = action[4:]
        for inner in list(value.values()):
            mutate(inner, key, action)
    elif isinstance(value, list):
        for inner in value:
            mutate(inner, key, action)


real = service.Vault.read


def read(self, request):
    answer = real(self, request)
    with open(LOG, "a") as out:
        out.write(json.dumps({"pid": os.getpid(), "operation": request.operation}) + "\\n")
    if MUTANT:
        mutate(answer, *json.loads(MUTANT))
    return answer


service.Vault.read = read
from orbit_hub.cli import main

main()
"""


def kit_vault_files() -> list[Path]:
    if not KIT.is_dir():
        return []
    return sorted(path for path in KIT.rglob("test_*.py") if READ_PATH in path.read_text())


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


def run_kit(tmp_path: Path, name: str, mutant: tuple[str, str] | None = None) -> dict:
    """The kit's vault tests, in their own process, against today's hub run as
    a program through a wrapper that writes down each vault read it answers
    (and, with `mutant`, answers it wrong)."""
    work = tmp_path / name
    work.mkdir()
    (work / "watched_hub.py").write_text(WATCHED_HUB)
    log, junit = work / "hub.log", work / "kit.xml"
    environment = {
        **os.environ,
        "ORBIT_KIT_HUB": shlex.join([sys.executable, str(work / "watched_hub.py")]),
        "KIT_WATCH_LOG": str(log),
        "KIT_VAULT_MUTANT": json.dumps(mutant) if mutant else "",
        "KIT_WATCH_RUN": str(work),
        "PYTHONPATH": str(work),
    }
    environment.pop("PYTEST_ADDOPTS", None)
    files = kit_vault_files()
    assert files, f"no test of {KIT} reads the vault through {READ_PATH}"
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
    return {
        "code": kit.returncode,
        "timed_out": timed_out,
        "left": left,
        "output": (work / "kit.out").read_text(errors="replace")[-4000:],
        "commands": [line["argv"] for line in lines if "argv" in line],
        "operations": [line["operation"] for line in lines if "operation" in line],
        **counts,
    }


def test_the_vault_format_tests_left_the_hub_tests():
    for moved in MOVED:
        assert not moved.exists(), f"{moved.name} is still here: it moves to the kit"
    here = sorted((SERVER / "tests").glob("*.py"))
    kept = {
        path.name: sorted(defined_tests(path) & set(OLD_TESTS))
        for path in here
        if path != Path(__file__).resolve()
    }
    assert not {name: tests for name, tests in kept.items() if tests}, kept
    assert kit_vault_files(), f"no test of {KIT} reads the vault through {READ_PATH}"


def test_the_kit_reads_the_vault_format_through_the_network(tmp_path):
    run = run_kit(tmp_path, "clean")
    assert not run["timed_out"], run["output"]
    assert run["code"] == 0, run["output"]
    assert run["left"] == [], f"the kit left its hub running: {run['left']}"
    assert run["skipped"] == 0, run["output"]
    assert run["tests"] - run["failures"] - run["errors"] >= OLD_CASES, run["output"]
    assert any("pair-code" in command for command in run["commands"]), run["commands"]
    assert any("serve" in command for command in run["commands"]), run["commands"]
    assert OPERATIONS <= set(run["operations"]), run["operations"]


@pytest.mark.parametrize("mutant", sorted(MUTANTS))
def test_the_kit_notices_a_hub_that_reads_the_vault_wrong(tmp_path, mutant):
    run = run_kit(tmp_path, "mutant", MUTANTS[mutant])
    assert not run["timed_out"], run["output"]
    assert run["left"] == [], f"the kit left its hub running: {run['left']}"
    # The hub served and answered: the kit failed on the answer, not on a hub
    # that never started.
    assert run["operations"], run["output"]
    assert run["code"] != 0 and run["failures"] + run["errors"] >= 1, (
        f"the kit passed with a hub where {mutant}:\n{run['output']}"
    )
