"""The dry run of the real test on the Mac, and its parts.

The dry run walks the orchestrator of scripts/e2e-mac and its checks
against the example states in scripts/e2e-mac/dry-run-examples/, with the real
test hub started on loopback (fake Whisper and note model that are never
called, no Hermes) and the Mac and its interface replaced by replays. It
proves the logic of the checks and that each one can fail: every check fails
against one example spoiled on purpose and passes against the good one, but
the menu bar's, which the harness cannot read and always says so. It is not
the real test and says nothing about Orbit on a Mac.

A pytest module that needs the hub's own environment (server/.venv): the gate
runs it in its hub zone. Its name ends in `_test.py` so that the guards'
`unittest discover -p 'test_*.py'`, which runs with the system's python3, does
not import it.
"""

import json
import re
import sqlite3
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "e2e-mac"))
# The hub suite's network fence: a test connects only to ports this process
# bound itself, never to a live service of the machine that runs the gate.
sys.path.insert(0, str(ROOT / "server" / "tests"))

import dry_run
import interface
import mac as mac_side
import mac_controls
import meeting_control_checks
from support_network_fence import install

FENCE = install()
# Every check that can pass or fail; 16, the menu bar's, is always skipped.
ALL = list(range(1, 16))
SKIPPED = {16: "skipped"}


@pytest.fixture(autouse=True)
def only_ports_this_process_bound():
    """Fail the test that tried to reach a port it did not open, even if swallowed."""
    mark = FENCE.refusals()
    yield
    refused = FENCE.refused_since(mark)
    assert not refused, f"this test tried to connect to {', '.join(refused)}"


@pytest.fixture(scope="module")
def good(tmp_path_factory):
    results, directory = dry_run.run("good", tmp_path_factory.mktemp("good"))
    return results, directory


def statuses(results):
    return {r["number"]: r["status"] for r in results}


def test_the_good_example_passes_every_check_but_the_skipped_one(good):
    results, directory = good
    assert statuses(results) == {**{n: "pass" for n in ALL}, **SKIPPED}, json.dumps(
        results, ensure_ascii=False, indent=1
    )
    report = (directory / "report.txt").read_text(encoding="utf-8")
    for number in ALL:
        assert f"{number:>2}. [pass]" in report
    # The menu bar is said unread, with why, never passed.
    assert "16. [skipped]" in report
    assert "cannot read Orbit Pruebas's menu bar item" in report
    # Every wait of the run is in the report with what it took.
    data = json.loads((directory / "report.json").read_text(encoding="utf-8"))
    names = {w["wait"] for w in data["waits"]}
    assert {
        "hub",
        "paired",
        "recording",
        "answer",
        "note",
        "chat",
        "voice-note",
        "call-listening",
        "muted",
        "unmuted",
        "hub-chunks",
        "discarded",
        "deleted",
        "hub-deleted",
    } <= names
    assert all("seconds" in w for w in data["waits"])
    # What the run does not cover is said in the report, every time.
    assert "Not covered:" in report
    assert "synthetic voice" in report
    # Only the call's audio lines of daemon.log come back from the Mac.
    call = (directory / "daemon-call.log").read_text(encoding="utf-8")
    events = [json.loads(line)["event"] for line in call.splitlines()]
    assert events == ["call_audio_opened", "call_audio_heard"]


def test_the_dry_run_starts_and_stops_the_real_test_hub(good):
    _, directory = good
    data = json.loads((directory / "report.json").read_text(encoding="utf-8"))
    stopped = data["stopped"]
    assert [p["process"] for p in stopped] == ["test hub"]
    assert stopped[0]["forced"] is False
    log = (directory / "hub.log").read_text(encoding="utf-8")
    assert "Uvicorn running" in log


@pytest.mark.parametrize("number", ALL)
def test_each_check_fails_against_its_spoiled_example(number, tmp_path):
    results, _ = dry_run.run(f"bad-{number}", tmp_path)
    seen = statuses(results)
    assert seen[number] == "fail", json.dumps(results, ensure_ascii=False, indent=1)
    assert {n: s for n, s in seen.items() if n != number} == {
        **{n: "pass" for n in ALL if n != number},
        **SKIPPED,
    }, json.dumps(results, ensure_ascii=False, indent=1)
    detail = next(r["detail"] for r in results if r["number"] == number)
    assert detail, "a failure without a cause"


# ── The parts ───────────────────────────────────────────────────────────────


def test_the_menu_bar_never_reaches_the_evidence(tmp_path):
    # Real run 36: a whole-window read brought macOS's menu bar, and under the
    # Apple menu «Ítems recientes» named the user's own documents.
    read = (
        '- AXWindow "Orbit Pruebas"\n'
        '  - [3] AXButton "Grabar" [actions=[press]]\n'
        "- [50] AXMenuBar [actions=[cancel]]\n"
        '  - [51] AXMenuBarItem "Apple" [actions=[cancel,press,pick]]\n'
        '    - [89] AXMenuItem "documento-privado.pdf" [id=_recentItemRequested: actions=[press]]\n'
        "      con una segunda línea\n"
        '- AXStaticText = "después"\n'
    )

    class Bridge:
        last_reply_bytes = len(read)

        def action(self, tool, arguments):
            return {"tree_markdown": read, "snapshot_id": "s1"}

    ui = interface.Interface(Bridge(), tmp_path, 7)
    nodes = ui.read(interface.Window(7, 1, "Orbit Pruebas"), "home", None)
    saved = (tmp_path / "001-home.txt").read_text(encoding="utf-8")
    for text in (saved, ui.trees["home"]):
        assert "documento-privado" not in text and "segunda línea" not in text
        assert "AXMenuBar" not in text
    assert [n.role for n in nodes] == ["AXWindow", "AXButton", "AXStaticText"]


def test_a_saved_input_no_longer_connected_does_not_stop_the_next_run(tmp_path):
    # Real run of 2026-10-21: a run that died had saved a pair of headphones
    # as the input to put back, and they were no longer connected.
    example = dry_run.Example("good")
    paths = mac_side.Paths(tmp_path)
    system = dry_run.DryRunSystem(example, paths)
    mac = mac_side.Mac(paths, system, dry_run.DryRunOrbit(example))
    paths.own.mkdir()
    paths.previous_input.write_text("Auriculares de Alina\n", encoding="utf-8")

    done = mac.restore(1.0)

    assert "«Auriculares de Alina» is no longer connected" in done["restored_input"]
    assert not paths.previous_input.exists()
    assert system.input == "Micrófono del MacBook Pro"


def test_the_note_s_language_is_read_before_its_dictionaries():
    # The note's header button names the template, the language and, since
    # the annex of the redesign, how many dictionaries it uses
    # (NoteSettingsMenu.tsx, `es.noteScreen.howLabel`).
    import checks

    assert checks.language_of("Plantilla: General · Español") == "Español"
    assert checks.language_of("Plantilla: General · Español · 2 diccionarios") == "Español"
    assert checks.language_of("Plantilla: General · English · 1 diccionario") == "English"
    assert checks.language_of("Plantilla: General") is None
    assert checks.language_of(None) is None


def test_the_texts_the_controls_look_for_are_the_app_s():
    # The checks of «Silenciar», «Terminar sin guardar» and «Borrar reunión»
    # press and read the app's own Spanish texts: a change there must not
    # leave the harness looking for an old one.
    sources = "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted((ROOT / "apps" / "desktop" / "src" / "i18n" / "es").rglob("*.ts"))
        if not path.name.endswith(".test.ts")
    )
    texts = dict(meeting_control_checks.TEXTS)
    prefix = texts.pop("muted_line")
    for name, text in texts.items():
        assert f'"{text}"' in sources, f"{name}: «{text}» is not a text of the app"
    line = re.search(rf"`({re.escape(prefix)}\$\{{[^}}]+\}} – \$\{{[^}}]+\}})`", sources)
    assert line is not None, "the transcript's «Micrófono silenciado · … – …» changed"


def test_what_the_hub_keeps_is_counted_in_every_table_that_names_it(tmp_path):
    import hub_traces

    database = tmp_path / "hub.sqlite3"
    connection = sqlite3.connect(database)
    connection.executescript(
        """
        CREATE TABLE chunks (id TEXT, device_id TEXT, recording TEXT);
        CREATE TABLE transcription_groups (id TEXT, device_id TEXT, recording TEXT);
        CREATE TABLE transcription_group_chunks (device_id TEXT, group_id TEXT, chunk_id TEXT);
        CREATE TABLE recording_notes (device_id TEXT, recording TEXT);
        CREATE TABLE deleted_recordings (device_id TEXT, recording TEXT);
        CREATE TABLE devices (id TEXT);
        INSERT INTO devices VALUES ('d1');
        INSERT INTO chunks VALUES ('c1', 'd1', 'kept'), ('c2', 'd1', 'kept'), ('c3', 'd1', 'other');
        INSERT INTO transcription_groups VALUES ('g1', 'd1', 'kept');
        INSERT INTO transcription_group_chunks VALUES ('d1', 'g1', 'c1'), ('d1', 'g1', 'c2');
        INSERT INTO recording_notes VALUES ('d1', 'kept'), ('d1', 'other');
        INSERT INTO deleted_recordings VALUES ('d1', 'gone');
        """
    )
    connection.commit()
    connection.close()

    assert hub_traces.kept(database, "kept") == {
        "rows": {
            "chunks": 2,
            "transcription_groups": 1,
            # Each row once, though it names the recording by chunk and group.
            "transcription_group_chunks": 2,
            "recording_notes": 1,
        },
        "deleted": False,
    }
    assert hub_traces.kept(database, "gone") == {"rows": {}, "deleted": True}


def test_a_meeting_is_gone_only_from_a_list_read_whole():
    # Absence read from a list the daemon or the hub could not give would
    # pass «Terminar sin guardar» and «Borrar reunión» on nothing.
    other = {"recording_id": "r2"}
    for read in (
        {"refusal": {"code": "hub_unreachable"}},
        {"items": [other, {"hub_refusal": {"code": "hub_unreachable"}}]},
        {"items": [other, {"recording_id": "r1"}]},
        {},
    ):
        state = {"read": read}
        assert mac_controls.gone(state, "r1") is None, read
        assert state["seen"]
    recording = {
        "state": "present",
        "value": {"recording": {"id": "r1", "phase": "recording"}},
    }
    state = {"recording": recording, "read": {"items": [other]}}
    assert mac_controls.gone(state, "r1") is None
    assert "still at «recording»" in state["seen"]
    assert mac_controls.gone({"read": {"items": [other]}}, "r1") == {"gone": "r1", "listed": 1}


SCREENS = ROOT / "scripts" / "e2e-mac" / "dry-run-examples"
FIRST, SECOND = "Reunión 16/10/26, 10:00", "Reunión 16/10/26, 10:20"
# A list still being read from the hub: its heading and «Actualizar», then
# «Leyendo tus reuniones del hub…» where the rows go (MeetingsColumn.tsx).
LOADING = """- [0] AXWindow "Orbit"
  - [1] AXWebArea (Orbit)
    - [8] AXLink "Reuniones" [actions=[press]]
    - [15] AXHeading "Reuniones"
      - [16] AXStaticText = "Reuniones"
    - [17] AXButton "Actualizar" (Actualizar) [actions=[press]]
    - [18] AXStaticText = "Leyendo tus reuniones del hub…"
"""


def screen(name: str) -> dict:
    return {"trees": {"meetings": (SCREENS / name).read_text(encoding="utf-8")}}


def test_the_list_of_meetings_is_recognised_as_macos_shows_it():
    # WebKit gives macOS no group named after the list's label («Lista de
    # reuniones»; real run of 2026-10-23): the list is the «Reuniones»
    # heading, «Actualizar», a heading per day and a link per meeting.
    gone = meeting_control_checks._gone_from_list
    # One meeting left, its note open beside the list (tree 040).
    after_discard = screen("good/screens/meetings-after-discard.md")
    assert gone(after_discard, "meetings", SECOND) is None
    assert "still" in (gone(after_discard, "meetings", FIRST) or "")
    # Nothing left: the list is read, and empty, with or without the deleted
    # meeting's note still open beside it (tree 055).
    for name in ("good/screens/meetings-empty.md", "deleted-note-open/screens/meetings-empty.md"):
        assert gone(screen(name), "meetings", FIRST) is None, name
    # A list not read yet, or no list at all, never says a meeting is gone.
    assert "does not show" in (gone({"trees": {"meetings": LOADING}}, "meetings", FIRST) or "")
    assert "does not show" in (gone(screen("good/screens/discarded.md"), "meetings", FIRST) or "")


def test_the_deleted_meeting_s_note_is_seen_still_open_beside_the_list():
    def still_open(name: str) -> bool:
        nodes = interface.ax_tree.parse(screen(name)["trees"]["meetings"])
        return meeting_control_checks.note_still_open(nodes, FIRST)

    assert still_open("deleted-note-open/screens/meetings-empty.md")
    assert not still_open("good/screens/meetings-empty.md")


def test_a_deleted_meeting_whose_note_stays_open_fails_only_its_check(tmp_path):
    results, _ = dry_run.run("deleted-note-open", tmp_path)
    assert statuses(results) == {
        **{n: "pass" for n in ALL if n != 15},
        15: "fail",
        **SKIPPED,
    }, json.dumps(results, ensure_ascii=False, indent=1)
    detail = next(r["detail"] for r in results if r["number"] == 15)
    assert "still open" in detail, detail
