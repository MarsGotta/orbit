"""The contract kit: tests that reach a hub only through its address, and the
sample database every hub has to read.

The kit lives in `contracts/kit/`. It starts a hub as a program over a copy of
the sample database (`contracts/fixtures/hub-storage/golden-v1.sqlite3`, sealed
with a public, invented test key) and reads it back over HTTP. It imports
nothing of the Python hub, so it can test a hub written in any language: the
command that runs the hub comes from `ORBIT_KIT_HUB`, and the kit appends
`--config <file> <command> [arguments]` to it, as the hub's administrator
commands take them.

These tests may import the hub: they are its oracle. They check that the kit
stays out of it, that the sample database is the one today's hub writes, that
it holds nothing a hub changes on its own as time passes or when it starts,
that no device token of the sample and no path of this machine is kept beside
it (the kit pairs its own device on its copy), and that the kit reads, through
the network, every sealed column the first three slices cover, and notices
when a hub reads one of them wrong. The second slice reads through a signed
private route too, signed as the contract writes it; the third signs with the
meetings key as well, and reads the AI settings, which hide the transcription
preference from every route while they hold a choice, and a meeting the
sample's device recorded by HTTP and an invented transcription service
transcribed. The kit also records a meeting of its own against an invented
transcription service it starts on loopback, and every process it starts (its
hubs and its invented services) ends with it.
"""

import ast
import contextlib
import hashlib
import json
import os
import re
import shlex
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path
from urllib.parse import urlsplit
from xml.etree import ElementTree

import pytest

from orbit_hub.app import create_app
from orbit_hub.storage import Store

ROOT = Path(__file__).resolve().parents[2]
SERVER = ROOT / "server"
KIT = ROOT / "contracts" / "kit"
SAMPLE = ROOT / "contracts" / "fixtures" / "hub-storage"
GOLDEN = SAMPLE / "golden-v1.sqlite3"
TEST_KEY = SAMPLE / "golden-v1.test-key.hex"
HUB_PACKAGES = ("orbit_hub", "orbit_mcp")
# The sample is in the public repository for good, and every regeneration adds
# its size to the history: it holds each kind of row once, not volume. An
# empty database of today's hub is about 0.5 MiB of pages.
GOLDEN_CAP_BYTES = 1024 * 1024
# A file the operator's secret rules would forbid anyone to read; the test key
# is public and must not carry such a name.
SECRET_NAMES = re.compile(
    r"(.*\.key|.*\.pem|token|\.env.*|credentials.*|\.npmrc|\.netrc|config\.toml"
    r"|auth\.json|settings.*\.json)"
)
# The first slice: each sealed column the kit reads through some route, by
# the context the hub opens it with.
FIRST_SLICE = {
    "storage_key_check.body": r"storage:key-check",
    "personal_reminders.body": r"reminder:[^:]+",
    "meeting_dictionaries.body": r"dictionary:[^:]+:[^:]+",
    "note_templates.body": r"note-template:[^:]+:[^:]+",
    "transcription_preferences.body": r"hub-transcription-preference",
}
# The second slice: a device's name (only the signed device list shows it), an
# API agent's personality and skill, and a vault's configuration. Not the AI
# settings: once they hold a choice, the transcription preference is read from
# them and its own row (first slice) by nothing.
SECOND_SLICE = {
    "devices.name": r"device:[^:]+",
    "agent_personalities.body": r"agent-personality:[^:]+",
    "agent_skills.body": r"agent-skill:[^:]+:[^:]+",
    "vault_configuration.body": r"vault-configuration:v1",
}
# The third slice, first part: the AI settings. While they hold a provider or
# a function's choice, the hub reads the transcription preference from them
# and its own row (first slice) by no route; a choice is taken back by a route
# (`operator`), a provider by none. So the sample holds a choice and no
# provider, and the kit reads the choice, takes it back, and then reads the
# preference.
THIRD_SLICE = {
    "ai_settings.body": r"hub-ai-settings",
}
# The third slice, second part: a meeting the sample's device recorded by HTTP
# (a chunk and its close), transcribed by an invented service, with a title,
# a speaker's name and private notes. The contexts name the device that
# recorded it (`{device}`, the sample's): the kit records a meeting of its own
# too, on the device it pairs, so only these tell the sample's values apart.
# Not yet the final pass (it asks for diarization), the voices, the note and
# the distillation: they come in the next part.
MEETING_SLICE = {
    "recording_manifests.body": r"manifest:{device}:[^:]+",
    "transcription_groups.manifest": r"group:{device}:[^:]+",
    "group_transcripts.body": r"group-transcript:{device}:[^:]+",
    "recording_titles.body": r"recording-title:{device}:[^:]+",
    "recording_speakers.body": r"recording-speakers:{device}:[^:]+",
    "recording_private_notes.body": r"recording-private-notes:{device}:[^:]+",
}
COLUMNS = {**FIRST_SLICE, **SECOND_SLICE, **THIRD_SLICE, **MEETING_SLICE}
# Rows the sample holds beyond the slices' columns: the device that wrote them.
ROWS = ("devices",)
# The signed routes the kit must reach, by their protocol: `devices.name` shows
# through no other route, and the meetings key signs the assistant's reads of
# meetings: the list (with the sample's meeting in it) and its transcript.
SIGNED_READS = {
    "/internal/v1/devices": "orbit_device_bridge_v1",
    "/internal/v1/meetings/list": "orbit_meetings_bridge_v1",
    "/internal/v1/meetings/transcript": "orbit_meetings_bridge_v1",
}
# The headers of a signed private request, as the contract has to name them:
# a kit written from the contract alone has to know them.
SIGNED_HEADERS = [
    f"x-orbit-{prefix}-{name}"
    for prefix in ("bridge", "meetings")
    for name in ("key-id", "issued-at-ms", "nonce", "signature")
]
# Rows today's hub deletes or changes by itself, while serving or when it
# starts: a sample holding one would be read differently after an hour, or
# only once. An attachment no message holds is deleted an hour after its
# upload (`attachments.expire`, at start and every minute); work left running
# is put back in the queue, or its outcome marked unknown, at start
# (`Store.recover_interrupted`).
CHANGED_BY_THE_HUB = {
    "an attachment no message holds": "attachments WHERE message_id IS NULL",
    "a conversation left mid-turn": (
        "sessions WHERE status IN ('submitting','streaming','interrupting')"
    ),
    "a transcription job left running": "transcription_jobs WHERE state='running'",
    "a transcription group left running": "transcription_groups WHERE state='running'",
    "a final pass left running": "recording_final_passes WHERE state='running'",
    # Work a serving hub picks up on its own: a chunk not in a group yet, a
    # group waiting for (another) try at the transcription service, a final
    # pass waiting its turn, a note waiting for its transcript or its model,
    # and a recording whose close never came (open: its chunks and no manifest).
    "a transcription job not grouped yet": "transcription_jobs WHERE state='pending'",
    "a transcription group waiting to be sent": "transcription_groups WHERE state='pending'",
    "a final pass waiting its turn": "recording_final_passes WHERE state='queued'",
    "a note waiting to be written": (
        "recording_notes WHERE state IN ('queued','generating','waiting_transcript')"
    ),
    "a recording left open": (
        "chunks AS c WHERE NOT EXISTS (SELECT 1 FROM recording_manifests AS m "
        "WHERE m.device_id = c.device_id AND m.recording = c.recording)"
    ),
}
# What could be a device token: the hub makes them with token_urlsafe(32).
TOKEN_LIKE = re.compile(r"[A-Za-z0-9_-]{20,}")
KIT_RUN_SECONDS = 180
# A hub the kit stopped may still be closing when the kit's process ends (or,
# already stopped, wait to be reaped by its new parent): it is given this long
# before it counts as left running. Uvicorn closes in well under a second.
HUB_EXIT_SECONDS = 5

WATCHED_HUB = """
import json, os, sys

LOG = os.environ["KIT_WATCH_LOG"]
CORRUPT = os.environ.get("KIT_WATCH_CORRUPT", "")
with open(LOG, "a") as out:
    out.write(json.dumps({"pid": os.getpid(), "argv": sys.argv[1:]}) + "\\n")

import re
from orbit_hub.storage import database

real = database.Store.decrypt


def changed(plain):
    try:
        value = json.loads(plain)
    except ValueError:
        try:
            return (plain.decode("utf-8") + " ~").encode()
        except UnicodeDecodeError:
            return plain + b"~"

    # Only a person's text (one with a space), so that ids and kinds still
    # validate and the route answers 200 with the other value; without any,
    # every text of 4 characters or more.
    def texts(item):
        if isinstance(item, str):
            return [item]
        inner = item.values() if isinstance(item, dict) else item if isinstance(item, list) else []
        return [text for part in inner for text in texts(part)]

    spaced = any(" " in text for text in texts(value))

    def walk(item):
        if isinstance(item, str):
            return item + " ~" if (" " in item if spaced else len(item) > 3) else item
        if isinstance(item, dict):
            return {key: walk(inner) for key, inner in item.items()}
        if isinstance(item, list):
            return [walk(inner) for inner in item]
        return item

    return json.dumps(walk(value), ensure_ascii=False).encode()


def watched(self, data, context):
    plain = real(self, data, context)
    with open(LOG, "a") as out:
        out.write(json.dumps({"pid": os.getpid(), "argv": sys.argv[1:], "context": context}) + "\\n")
    if CORRUPT and re.fullmatch(CORRUPT, context):
        return changed(plain)
    return plain


database.Store.decrypt = watched

from orbit_hub import private_auth

real_admit = private_auth.Authenticator.admit


def admitted(self, *arguments, **keywords):
    agent = real_admit(self, *arguments, **keywords)
    with open(LOG, "a") as out:
        line = {"pid": os.getpid(), "argv": sys.argv[1:], "signed": self._path,
                "protocol": self._protocol}
        out.write(json.dumps(line) + "\\n")
    return agent


private_auth.Authenticator.admit = admitted

from orbit_hub import meeting_transcription

real_transcribe = meeting_transcription.MeetingTranscriber.transcribe


async def transcribed(self, *arguments, **keywords):
    result = await real_transcribe(self, *arguments, **keywords)
    with open(LOG, "a") as out:
        line = {"pid": os.getpid(), "argv": sys.argv[1:], "transcribed": self.url,
                "chars": len(result.text)}
        out.write(json.dumps(line) + "\\n")
    return result


meeting_transcription.MeetingTranscriber.transcribe = transcribed
from orbit_hub.cli import main

main()
"""

MODULES_PLUGIN = """
import json, os, sys


def pytest_sessionfinish(session, exitstatus):
    with open(os.environ["KIT_WATCH_MODULES"], "w") as out:
        json.dump(sorted(sys.modules), out)
"""


def kit_files() -> list[Path]:
    return sorted(KIT.rglob("*.py")) if KIT.is_dir() else []


def hub_imports(text: str, label: str) -> list[str]:
    """Each place `text` reaches the Python hub's packages: an import, or a
    name handed to `importlib.import_module` or `__import__`."""
    found = []
    for node in ast.walk(ast.parse(text, filename=label)):
        names = []
        if isinstance(node, ast.Import):
            names = [alias.name for alias in node.names]
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            names = [node.module]
        elif isinstance(node, ast.Call) and ast.unparse(node.func) in (
            "importlib.import_module",
            "import_module",
            "__import__",
        ):
            names = [
                argument.value
                for argument in node.args[:1]
                if isinstance(argument, ast.Constant) and isinstance(argument.value, str)
            ]
        found += [
            f"{label}:{node.lineno} imports {name}"
            for name in names
            if name.split(".")[0] in HUB_PACKAGES
        ]
    return found


def schema(database: Path) -> set[tuple]:
    with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as db:
        rows = db.execute(
            "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'"
        ).fetchall()
        version = db.execute("PRAGMA user_version").fetchone()[0]
    return {*rows, ("pragma", "user_version", "", str(version))}


def sample_key() -> bytes:
    return bytes.fromhex(TEST_KEY.read_text().strip())


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def machine_paths() -> list[str]:
    """Where this machine keeps its user, this checkout and its temporary
    files: none may be written into the public sample (a bare `/tmp` says
    nothing of the machine)."""
    paths = {str(Path.home()), str(ROOT), tempfile.gettempdir()}
    return sorted(path for path in paths if path not in ("/", "/tmp"))


def sample_devices() -> list[str]:
    """The ids of the devices the sample holds, read from a copy (SQLite
    leaves -wal and -shm beside a WAL database it opens, even read-only)."""
    with tempfile.TemporaryDirectory() as scratch:
        copy = Path(scratch) / "devices.sqlite3"
        shutil.copy(GOLDEN, copy)
        with sqlite3.connect(f"file:{copy}?mode=ro", uri=True) as db:
            return sorted(row[0] for row in db.execute("SELECT id FROM devices"))


def context_of(column: str) -> str:
    """The context a column is opened with; a meeting's names a device of the sample."""
    devices = "|".join(re.escape(device) for device in sample_devices())
    return COLUMNS[column].replace("{device}", f"(?:{devices})")


def run_kit(tmp_path: Path, name: str, corrupt: str = "") -> dict:
    """The kit, in its own process, against today's hub run as a program
    through a wrapper that writes down each value the hub opens (and, with
    `corrupt`, changes the ones whose context matches)."""
    work = tmp_path / name
    work.mkdir()
    (work / "watched_hub.py").write_text(WATCHED_HUB)
    (work / "kit_watch.py").write_text(MODULES_PLUGIN)
    log, modules, junit = work / "hub.log", work / "modules.json", work / "kit.xml"
    environment = {
        **os.environ,
        "ORBIT_KIT_HUB": shlex.join([sys.executable, str(work / "watched_hub.py")]),
        "KIT_WATCH_LOG": str(log),
        "KIT_WATCH_MODULES": str(modules),
        "KIT_WATCH_CORRUPT": corrupt,
        # Inherited by every process the kit starts, unless it takes it away:
        # how the processes of this run are told apart once the kit has ended.
        "KIT_WATCH_RUN": str(work),
        "PYTHONPATH": str(work),
    }
    environment.pop("PYTEST_ADDOPTS", None)
    # Output to files, not pipes: a hub left running with the kit's pipes
    # would hold them open after the kit ends.
    with open(work / "kit.out", "wb") as out:
        kit = subprocess.Popen(
            [
                sys.executable,
                "-m",
                "pytest",
                "-q",
                "-p",
                "kit_watch",
                "-p",
                "no:cacheprovider",
                "-c",
                str(SERVER / "pyproject.toml"),
                "--rootdir",
                str(SERVER),
                f"--basetemp={work / 'base'}",
                f"--junitxml={junit}",
                str(KIT),
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
            # Pass, fail or time out: each hub the kit started must be gone,
            # and so must every other process it started (its invented services).
            left = stop_hubs_left(log, work)
            others = stop_processes_left(work)
    output = (work / "kit.out").read_text(errors="replace")[-4000:]
    assert not timed_out, f"the kit ran past {KIT_RUN_SECONDS} s:\n{output}"
    assert not left, f"the kit left its hub running (pids {left}): it must stop it\n{output}"
    assert not others, (
        f"the kit left processes it started running ({others}): its hubs and its invented "
        f"services must end with it\n{output}"
    )
    counts = {"tests": 0, "failures": 0, "errors": 0, "skipped": 0}
    if junit.exists():
        for suite in ElementTree.parse(junit).getroot().iter("testsuite"):
            for key in counts:
                counts[key] += int(suite.get(key, 0))
    lines = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
    return {
        "code": kit.returncode,
        "output": output,
        "counts": counts,
        "hub": lines,
        "modules": json.loads(modules.read_text()) if modules.exists() else None,
    }


def running(pid: int, marker: str) -> bool:
    """Whether `pid` still runs the wrapper this run wrote (`marker`, its path):
    not gone, not a zombie and, where `/proc` tells, not a reused pid."""
    try:
        os.kill(pid, 0)
    except (ProcessLookupError, PermissionError):
        return False
    proc = Path(f"/proc/{pid}")
    if not proc.exists():
        return True
    try:
        state = (proc / "stat").read_text().rsplit(")", 1)[1].split()[0]
        command = (proc / "cmdline").read_bytes()
    except (OSError, IndexError):
        return False
    return state != "Z" and marker.encode() in command


def stop_hubs_left(log: Path, work: Path) -> list[int]:
    """The hubs of this run (the wrapper writes each start's pid) still running
    once the kit has ended, killed. By pid, not by process group: a kit may
    start its hub in a group of its own."""
    pids = set()
    for line in log.read_text().splitlines() if log.exists() else []:
        with contextlib.suppress(ValueError, KeyError):  # a line a hub is still writing
            pids.add(json.loads(line)["pid"])
    marker = str(work / "watched_hub.py")
    deadline = time.monotonic() + HUB_EXIT_SECONDS
    while (
        left := [pid for pid in sorted(pids) if running(pid, marker)]
    ) and time.monotonic() < deadline:
        time.sleep(0.1)
    for pid in left:
        with contextlib.suppress(ProcessLookupError):
            os.kill(pid, signal.SIGKILL)
    return left


def run_processes(work: Path) -> dict[int, str]:
    """This user's processes (not zombies) started under the kit of this run:
    their environment holds `KIT_WATCH_RUN` with this run's folder. By pid,
    with the start of their command line."""
    marker = f"KIT_WATCH_RUN={work}".encode() + b"\0"
    found = {}
    for proc in Path("/proc").glob("[0-9]*") if Path("/proc").is_dir() else []:
        try:
            if marker not in (proc / "environ").read_bytes():
                continue
            state = (proc / "stat").read_text().rsplit(")", 1)[1].split()[0]
            command = (proc / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
        except (OSError, IndexError):
            continue
        if state != "Z" and int(proc.name) != os.getpid():
            found[int(proc.name)] = command[:200]
    return found


def stop_processes_left(work: Path) -> dict[int, str]:
    """The processes of this run still running once the kit has ended (after
    the same grace as a hub), killed."""
    deadline = time.monotonic() + HUB_EXIT_SECONDS
    while (left := run_processes(work)) and time.monotonic() < deadline:
        time.sleep(0.1)
    for pid in left:
        with contextlib.suppress(ProcessLookupError):
            os.kill(pid, signal.SIGKILL)
    return left


def require_kit():
    missing = [str(path.relative_to(ROOT)) for path in (KIT, GOLDEN, TEST_KEY) if not path.exists()]
    assert not missing, f"the contract kit is not there yet: {missing} do not exist"


def test_the_kit_reaches_the_hub_only_by_its_address():
    """No file of the kit imports the Python hub, and the gate runs and lints it."""
    require_kit()
    files = kit_files()
    tests = [path for path in files if path.name.startswith("test_")]
    assert tests, "contracts/kit has no test_*.py file"
    problems = [
        problem
        for path in files
        for problem in hub_imports(path.read_text(), str(path.relative_to(ROOT)))
    ]
    assert not problems, "the kit must reach the hub only by its address:\n" + "\n".join(problems)
    # The gate collects the kit and server/tests in one pytest process, which
    # imports both folders' modules by their bare names: one name in both
    # (a second conftest.py) shadows the other's.
    theirs = {path.stem for path in (SERVER / "tests").glob("*.py")}
    clashes = sorted({path.stem for path in files} & theirs)
    assert not clashes, f"kit modules named as server/tests modules: {clashes}"
    configuration = tomllib.loads((SERVER / "pyproject.toml").read_text())
    testpaths = configuration["tool"]["pytest"]["ini_options"]["testpaths"]
    assert "../contracts/kit" in testpaths, f"hub-pytest does not collect the kit: {testpaths}"
    gate = (ROOT / "scripts" / "check.sh").read_text()
    for step in ("hub-ruff", "hub-format"):
        line = re.search(rf"check_run {step} .*?\n(.*?)\n", gate)
        assert line and "../contracts/kit" in line.group(1), f"{step} does not lint the kit"


def test_reading_the_kit_for_hub_imports():
    """The import guard itself, on invented files."""
    cases = {
        "import orbit_hub.app\n": 1,
        "from orbit_mcp import server\n": 1,
        "import importlib\nimportlib.import_module('orbit_hub.cli')\n": 1,
        "__import__('orbit_mcp')\n": 1,
        "import os, orbit_hub\nfrom orbit_hub.storage import Store\n": 2,
        "from . import orbit_hub\nimport orbit_hubble\nimport httpx\n": 0,
        "COMMAND = 'python -m orbit_hub.cli'\n": 0,
    }
    for text, expected in cases.items():
        assert len(hub_imports(text, "case.py")) == expected, (text, hub_imports(text, "case.py"))


def test_the_sample_database_is_today_hubs_small_and_sealed_with_a_test_key(environment, tmp_path):
    """The sample opens with its test key, has exactly the schema today's hub
    builds, a row in each column of the slices, AI settings with a choice and
    no provider, no work a serving hub would pick up on its own, and stays
    under its cap."""
    require_kit()
    settings, store, _ = environment
    names = [
        str(path.relative_to(ROOT))
        for folder in (KIT, SAMPLE)
        for path in folder.rglob("*")
        if SECRET_NAMES.fullmatch(path.name)
    ]
    assert not names, f"a public test file carries a secret's name: {names}"
    assert re.fullmatch(r"[0-9a-f]{64}\n?", TEST_KEY.read_text()), (
        "the test key is 64 lowercase hexadecimal characters"
    )
    beside = sorted(path.name for path in SAMPLE.glob("golden-v1.sqlite3-*"))
    assert not beside, f"the sample must be one file, without {beside}"
    size = GOLDEN.stat().st_size
    assert size <= GOLDEN_CAP_BYTES, f"the sample is {size} bytes; its cap is {GOLDEN_CAP_BYTES}"

    data, key = tmp_path / "sample" / "data", tmp_path / "sample" / "key"
    data.mkdir(mode=0o700, parents=True)
    shutil.copy(GOLDEN, data / "hub.sqlite3")
    key.write_bytes(sample_key())
    key.chmod(0o600)
    sample = Store(data, key)  # refuses a database sealed with another key
    try:
        with sqlite3.connect(f"file:{data / 'hub.sqlite3'}?mode=ro", uri=True) as db:
            sealed = [row[0] for row in db.execute("SELECT body FROM vault_configuration")]
            settings_rows = [row[0] for row in db.execute("SELECT body FROM ai_settings")]
        opened = [sample.decrypt(body, "vault-configuration:v1").decode() for body in sealed]
        ai = [json.loads(sample.decrypt(body, "hub-ai-settings")) for body in settings_rows]
    finally:
        sample.close()
    # A choice the kit can read and take back; a provider would hide the
    # transcription preference from every route for good.
    assert ai and all(document["functions"] for document in ai), (
        f"the sample's AI settings hold no function's choice: {ai}"
    )
    providers = sorted(name for document in ai for name in document["providers"])
    assert not providers, (
        f"the sample's AI settings hold providers {providers}: no route takes one back, "
        "so the transcription preference would be read by none"
    )

    create_app(settings)  # today's hub, with the tables its domains create
    store.close()
    built = schema(settings.data_dir / "hub.sqlite3")
    # Read through a copy: SQLite leaves -wal and -shm beside a WAL database it
    # opens, even read-only.
    copy = tmp_path / "sample" / "read.sqlite3"
    shutil.copy(GOLDEN, copy)
    written = schema(copy)
    differences = [f"only in the sample: {row}" for row in sorted(written - built, key=str)]
    differences += [f"only in today's hub: {row}" for row in sorted(built - written, key=str)]
    assert not differences, "the sample is not today's schema:\n" + "\n".join(differences)

    with sqlite3.connect(f"file:{copy}?mode=ro", uri=True) as db:
        assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        empty = [
            column
            for column in COLUMNS
            if not db.execute(
                "SELECT count(*) FROM {} WHERE {} IS NOT NULL".format(*column.split("."))
            ).fetchone()[0]
        ]
        empty += [
            table for table in ROWS if not db.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
        ]
        changing = [
            f"{count} {what}"
            for what, rows in CHANGED_BY_THE_HUB.items()
            if (count := db.execute(f"SELECT count(*) FROM {rows}").fetchone()[0])
        ]
        hashes = {row[0] for row in db.execute("SELECT token_hash FROM devices")}
    assert not empty, f"the sample holds no row in {empty}"
    assert not changing, f"the sample holds rows today's hub changes on its own: {changing}"
    kept = [
        str(path.relative_to(ROOT))
        for folder in (KIT, SAMPLE)
        for path in folder.rglob("*")
        if path.is_file()
        and path != GOLDEN
        and any(
            hashlib.sha256(word.encode()).hexdigest() in hashes
            for word in TOKEN_LIKE.findall(path.read_bytes().decode("utf-8", "replace"))
        )
    ]
    assert not kept, f"a device token of the sample is kept in {kept}: pair the kit's own"
    # The vault's folder is a path on the hub that wrote the sample: an
    # invented one, and so is every text kept beside the sample.
    texts = {"the vault's configuration": "\n".join(opened)}
    texts |= {
        str(path.relative_to(ROOT)): path.read_bytes().decode("utf-8", "replace")
        for folder in (KIT, SAMPLE)
        for path in folder.rglob("*")
        if path.is_file()
        and path != GOLDEN
        # What Python and the linters leave (compiled files name their source's path).
        and not any(part.startswith((".", "__pycache__")) for part in path.relative_to(ROOT).parts)
    }
    found = [
        f"{where} names {path}"
        for where, text in texts.items()
        for path in machine_paths()
        if path in text
    ]
    assert not found, "the sample keeps a path of this machine:\n" + "\n".join(found)


def test_the_kit_reads_the_sample_through_the_network(tmp_path):
    """The kit passes against today's hub run as a program, works on a copy,
    loads nothing of the hub, reaches the signed device list and the signed
    meetings list and transcript with signatures the hub admits, the hub
    opened every column of the three slices while serving it (a meeting's, of
    the sample's device), and transcribed a meeting of the kit's own through an
    invented service on loopback."""
    require_kit()
    before = sha256(GOLDEN)
    run = run_kit(tmp_path, "plain")
    assert run["code"] == 0, run["output"]
    counts = run["counts"]
    assert counts["tests"] >= 1 and not counts["skipped"], counts
    assert run["modules"] is not None, "the kit's process did not finish its session"
    loaded = [name for name in run["modules"] if name.split(".")[0] in HUB_PACKAGES]
    assert not loaded, f"the kit's process loaded the hub: {loaded[:10]}"
    assert sha256(GOLDEN) == before, "the kit changed the sample: it must work on a copy"
    beside = sorted(path.name for path in SAMPLE.glob("golden-v1.sqlite3-*"))
    assert not beside, f"a hub opened the sample itself, leaving {beside}"
    served = [line for line in run["hub"] if "serve" in line["argv"]]
    assert served, f"the kit never started the hub with `serve`: {run['hub'][:5]}"
    paired = [line for line in run["hub"] if "pair-code" in line["argv"]]
    assert paired, "the kit never asked its hub for a pairing code: it pairs its own device"
    opened = {line["context"] for line in served if "context" in line}
    unread = [
        column
        for column in COLUMNS
        if not any(re.fullmatch(context_of(column), seen) for seen in opened)
    ]
    assert not unread, f"the hub never opened {unread} while serving the kit"
    admitted = {(line["signed"], line["protocol"]) for line in served if "signed" in line}
    unsigned = [
        f"{path} ({protocol})"
        for path, protocol in SIGNED_READS.items()
        if (path, protocol) not in admitted
    ]
    assert not unsigned, f"the hub admitted no signed request to {unsigned} from the kit"
    # The kit's own meeting: the hub sent its audio to a service the kit
    # started on loopback, which answered some text (a failed call is not
    # written down).
    transcribed = [
        line
        for line in served
        if "transcribed" in line
        and line["chars"] > 0
        and urlsplit(line["transcribed"]).hostname in ("127.0.0.1", "localhost", "::1")
    ]
    assert transcribed, (
        "the hub transcribed nothing through a service on loopback while serving the kit: "
        "the kit records a meeting of its own against an invented transcription service"
    )


@pytest.mark.parametrize("column", sorted(COLUMNS))
def test_the_kit_notices_a_hub_that_reads_a_column_wrong(tmp_path, column):
    """A hub that opens one column of any slice into another value
    (a person's text gets ` ~`) must make the kit fail, not skip or pass."""
    require_kit()
    run = run_kit(tmp_path, "changed", corrupt=context_of(column))
    counts = run["counts"]
    assert run["code"] != 0 and (counts["failures"] or counts["errors"]), (
        f"the kit passed with {column} read wrong: {counts}\n{run['output'][-1500:]}"
    )
    assert not counts["skipped"], counts


def test_the_contract_writes_down_how_a_private_request_is_signed():
    """The kit signs as the contract says, so the contract names each header of
    both private protocols: today's hub reads them, and the recipe beside them."""
    contract = (ROOT / "contracts" / "hub-v1.md").read_text().lower()
    missing = [header for header in SIGNED_HEADERS if header not in contract]
    assert not missing, f"contracts/hub-v1.md does not name the signed headers {missing}"
