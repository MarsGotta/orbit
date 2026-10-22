"""The dry run of the real test on the Mac, and its parts.

The dry run walks the orchestrator of scripts/e2e-mac and its twelve checks
against the example states in scripts/e2e-mac/dry-run-examples/, with the real
test hub started on loopback (fake Whisper and note model that are never
called, no Hermes) and the Mac and its interface replaced by replays. It
proves the logic of the checks and that each one can fail: every check fails
against one example spoiled on purpose and passes against the good one. It is
not the real test and says nothing about Orbit on a Mac.

A pytest module that needs the hub's own environment (server/.venv): the gate
runs it in its hub zone. Its name ends in `_test.py` so that the guards'
`unittest discover -p 'test_*.py'`, which runs with the system's python3, does
not import it.
"""

import json
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
from support_network_fence import install

FENCE = install()
ALL = list(range(1, 13))


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


def test_the_good_example_passes_all_twelve_checks(good):
    results, directory = good
    assert statuses(results) == {n: "pass" for n in ALL}, json.dumps(
        results, ensure_ascii=False, indent=1
    )
    report = (directory / "report.txt").read_text(encoding="utf-8")
    for number in ALL:
        assert f"{number:>2}. [pass]" in report
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
        n: "pass" for n in ALL if n != number
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
