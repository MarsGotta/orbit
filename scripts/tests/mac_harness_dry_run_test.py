"""The dry run of the real test on the Mac, and its parts.

The dry run walks the orchestrator of scripts/e2e-mac and its checks
against the example states in scripts/e2e-mac/dry-run-examples/, with the real
test hub started on loopback (fake Whisper and note model that are never
called, no Hermes) and the Mac and its interface replaced by replays. It
proves the logic of the checks and that each one can fail: every check fails
against one example spoiled on purpose and passes against the good one, including
the menu bar through the in-app test bridge. It is not
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

import drafts_panel_checks
import dry_run
import first_run
import interface
import mac as mac_side
import mac_controls
import mac_drafts_panel
import meeting_control_checks
from support_network_fence import install

FENCE = install()
# Every check can pass or fail, including the menu bar through the bridge.
ALL = list(range(1, 22))
SKIPPED = {}
# A check whose evidence another one's step brings: with no answer of «Oye
# Deimos» in the panel (7), there is none to hold under the pointer (18).
FOLLOWS = {
    7: {18: "unchecked"},
    # An interrupted capture ends the script before its audio/UI evidence.
    # «Oye Deimos» (7) and its panel (18) come before the input change.
    20: {n: "unchecked" for n in ALL if n not in (7, 10, 16, 18, 20, 21)},
}


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


def test_the_good_example_passes_every_check(good):
    results, directory = good
    assert statuses(results) == {**{n: "pass" for n in ALL}, **SKIPPED}, json.dumps(
        results, ensure_ascii=False, indent=1
    )
    report = (directory / "report.txt").read_text(encoding="utf-8")
    for number in ALL:
        assert f"{number:>2}. [pass]" in report
    # The menu bar is read through the test bridge.
    assert "16. [pass]" in report
    assert "00:12" in report
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
        "pointer-held",
        "answer-released",
    } <= names
    assert all("seconds" in w for w in data["waits"])
    # What the run does not cover is said in the report, every time.
    assert "Not covered:" in report
    assert "synthetic voice" in report
    assert "second screen" in report
    # Only the call's audio lines of daemon.log come back from the Mac.
    call = (directory / "daemon-call.log").read_text(encoding="utf-8")
    events = [json.loads(line)["event"] for line in call.splitlines()]
    assert events == ["call_audio_opened", "call_audio_heard"]


def _skips(directory: Path) -> tuple[dict, str, int]:
    """report.json, report.txt and how many times «Configurarlo más tarde»
    was pressed: each press is found by one read of its own."""
    data = json.loads((directory / "report.json").read_text(encoding="utf-8"))
    report = (directory / "report.txt").read_text(encoding="utf-8")
    presses = [r["label"] for r in data["reads"]].count(f"find {first_run.SKIP}")
    return data, report, presses


def test_a_run_that_opens_on_the_main_window_presses_nothing_more(good):
    _, directory = good
    data, report, presses = _skips(directory)
    assert presses == 0
    assert data["welcome_skipped"] == []
    assert first_run.REPORTED not in report


def test_a_first_run_s_welcome_is_skipped_with_one_press_and_said_once(good, tmp_path):
    # Run 20261025T141530Z: Orbit Pruebas opened on its first run's welcome,
    # painted after the window's first read, and the run looked for «Grabar
    # reunión» there. The example's window is blank at its first read, shows
    # the welcome at the next, and the main view once the welcome is skipped.
    results, directory = dry_run.run("first-run", tmp_path)

    assert statuses(results) == statuses(good[0]), json.dumps(results, ensure_ascii=False, indent=1)
    data, report, presses = _skips(directory)
    assert presses == 1
    # Skipped once: the reopened app of check 17 kept the first run's completion.
    assert data["welcome_skipped"] == ["home"]
    assert report.count(first_run.REPORTED) == 1


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
        **FOLLOWS.get(number, {}),
    }, json.dumps(results, ensure_ascii=False, indent=1)
    detail = next(r["detail"] for r in results if r["number"] == number)
    assert detail, "a failure without a cause"
    for follower in FOLLOWS.get(number, {}):
        why = next(r["detail"] for r in results if r["number"] == follower)
        cause = "change the default input" if number == 20 else f"(check {number})"
        assert cause in why, why


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

        def call(self, request):
            return {"tree_markdown": read, "elements": []}

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
    # The first run's way out and the main window's button.
    assert f'skipAll: "{first_run.SKIP}"' in sources
    assert f'record: "{first_run.MAIN}"' in sources


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


# ── A draft across the Dock's quit, and the panel under the pointer ──


def test_quitting_and_reopening_on_purpose_is_not_taken_for_a_relaunch(good):
    # Check 17 quits Orbit Pruebas once, with the Dock's quit, and opens it
    # again itself: the guard of the clean-up, which fails the run when Orbit
    # comes back after it was closed, must still see one app to close and
    # nothing coming back.
    _, directory = good
    data = json.loads((directory / "report.json").read_text(encoding="utf-8"))
    assert data["restored"]["orbit_pruebas"]["came_back"] == 0
    assert data["restored"]["orbit_pruebas"]["closed"] >= 1
    quits = [row["quit"] for row in data["drafts"]]
    assert quits == ["apple-event"]
    assert all(row["came_back"] == 0 for row in data["drafts"])
    # The draft is this run's own, typed right before the quit.
    assert data["drafts"][0]["text"].startswith("Borrador de prueba ")


def test_the_quit_is_the_dock_s_only(tmp_path):
    # ⌘Q sent through CuaDriver never reached the app's menu on the Mac (run
    # 20261025T150223Z): it ends in the same terminate as the Dock's quit,
    # which the run sends instead, and asking for it is refused, not tried.
    assert mac_drafts_panel.QUITS == ("apple-event",)
    assert list(drafts_panel_checks.QUIT_NAMES) == ["apple-event"]
    not_covered = " ".join(drafts_panel_checks.NOT_COVERED)
    assert "⌘Q" in not_covered and "terminate" in not_covered
    example = dry_run.Example("good")
    paths = mac_side.Paths(tmp_path)
    system = dry_run.DryRunSystem(example, paths)
    mac = mac_side.Mac(paths, system, dry_run.DryRunOrbit(example))
    system.app_open = True
    with pytest.raises(mac_side.MacFailure, match="unknown quit «cmd-q»"):
        mac.draft_and_quit("cmd-q", "{}", 1.0, 1.0)
    assert system.commands == [] and system.app_open


def test_the_quit_types_its_draft_and_confirms_it_on_screen_right_before_quitting(tmp_path):
    example = dry_run.Example("good")
    paths = mac_side.Paths(tmp_path)
    system = dry_run.DryRunSystem(example, paths)
    mac = mac_side.Mac(paths, system, dry_run.DryRunOrbit(example))
    system.app_open = True
    system.view.generation = 1
    system.view.screen = "assistant"
    mac.bridge = dry_run.DryRunBridge(example, system.view)
    ui = interface.Interface(mac.bridge, tmp_path, dry_run.PID)
    typing = ui.typing(
        ui.windows()["main"], "Mensaje para", "Borrador inventado 1a2b3c", contains=True
    )

    done = mac.draft_and_quit("apple-event", json.dumps(typing), 1.0, 1.0)

    assert [request["command"] for request in mac.bridge.calls][-2:] == ["type", "read"]
    assert not any(command[0] == str(paths.cua) for command in system.commands)
    assert system.commands[0][0] == "osascript"
    assert done["quit"] == "apple-event"
    assert done["in_box"] is True
    assert done["came_back"] == 0
    assert not system.app_open
    assert done["gap_s"] >= 0


def test_a_draft_that_never_reached_the_box_is_not_quit_on(tmp_path):
    # The quit tests what WebKit keeps of a draft on screen; a draft the box
    # never showed would test nothing, and Orbit stays open.
    example = dry_run.Example("good")
    paths = mac_side.Paths(tmp_path)
    system = dry_run.DryRunSystem(example, paths)
    mac = mac_side.Mac(paths, system, dry_run.DryRunOrbit(example))
    system.app_open = True
    system.view.generation = 1
    system.view.screen = "assistant"
    mac.bridge = dry_run.DryRunBridge(example, system.view)
    ui = interface.Interface(mac.bridge, tmp_path, dry_run.PID)
    typing = ui.typing(
        ui.windows()["main"], "Mensaje para", "Borrador inventado 4d5e6f", contains=True
    )
    # The fake view accepts the event but never paints the typed draft.
    system.view.transitions["assistant"] = {}

    with pytest.raises(mac_side.MacFailure, match="did not show"):
        mac.draft_and_quit("apple-event", json.dumps(typing), 0.0, 1.0)
    assert system.app_open
    assert not [c for c in system.commands if c[0] == "osascript"]


def test_the_quit_apple_event_goes_to_orbit_pruebas_by_its_bundle_identifier(tmp_path):
    example = dry_run.Example("good")
    paths = mac_side.Paths(tmp_path)
    system = dry_run.DryRunSystem(example, paths)
    mac = mac_side.Mac(paths, system, dry_run.DryRunOrbit(example))
    system.app_open = True
    system.view.generation = 1
    system.view.screen = "assistant"
    mac.bridge = dry_run.DryRunBridge(example, system.view)
    ui = interface.Interface(mac.bridge, tmp_path, dry_run.PID)
    typing = ui.typing(
        ui.windows()["main"], "Mensaje para", "Borrador inventado 7a8b9c", contains=True
    )

    mac.draft_and_quit("apple-event", json.dumps(typing), 1.0, 1.0)

    script = next(c for c in system.commands if c[0] == "osascript")[-1]
    # Asked only while it runs: telling a closed app to quit would open it.
    assert script == (
        f'if application id "{mac_side.BUNDLE_ID}" is running then '
        f'tell application id "{mac_side.BUNDLE_ID}" to quit'
    )


def _webkit_storage(path: Path, rows: dict[str, str]) -> None:
    # WebKit's localStorage file: ItemTable, its values UTF-16 in a BLOB.
    path.parent.mkdir(parents=True)
    connection = sqlite3.connect(path)
    connection.execute("CREATE TABLE ItemTable (key TEXT UNIQUE, value BLOB NOT NULL)")
    for key, value in rows.items():
        connection.execute("INSERT INTO ItemTable VALUES (?, ?)", (key, value.encode("utf-16-le")))
    connection.commit()
    connection.close()


def _stored_keys(path: Path) -> set[str]:
    connection = sqlite3.connect(path)
    try:
        return {key for (key,) in connection.execute("SELECT key FROM ItemTable")}
    finally:
        connection.close()


# What WebKit keeps for Orbit Pruebas besides its drafts: the first run's
# completion (OnboardingView.tsx), the theme (theme.ts), a dismissed update
# notice (UpdateNotice.tsx), and a key that only looks like a draft's.
KEPT = {
    "orbit.onboarding.done.local": "true",
    "orbit.ui.theme.local": "dark",
    "orbit.update.dismissed-version": "0.0.1",
    "orbit.drafts-elsewhere": '"no es un borrador"',
}
DRAFTS = {
    "orbit.draft.chat": json.dumps("Borrador inventado de otra ejecución"),
    "orbit.draft.note.11111111": json.dumps("Nota inventada sin enviar"),
}


def test_prepare_removes_only_the_drafts_from_orbit_pruebas_s_webkit_storage(tmp_path):
    # Drafts live in WebKit's storage, outside the data directory: a draft an
    # earlier run left would be in the box when check 8 types its message.
    # Run 20261025T141530Z: deleting that whole storage also deleted the first
    # run's completion, and Orbit Pruebas opened on its welcome.
    example = dry_run.Example("good")
    paths = mac_side.Paths(tmp_path)
    (paths.app / "Contents").mkdir(parents=True)
    (paths.app / "Contents" / "Info.plist").write_bytes(
        dry_run.plistlib.dumps({"CFBundleIdentifier": mac_side.BUNDLE_ID})
    )
    system = dry_run.DryRunSystem(example, paths)
    mac = mac_side.Mac(paths, system, dry_run.DryRunOrbit(example))
    storage = paths.webkit / "WebsiteData" / "Default" / "a1" / "a1" / "LocalStorage"
    _webkit_storage(storage / "localstorage.sqlite3", {**KEPT, **DRAFTS})
    left = paths.data / "orbit-local.json"
    left.parent.mkdir(parents=True)
    left.write_text("{}", encoding="utf-8")
    users = tmp_path / "Library" / "WebKit" / "app.orbit.companion" / "localstorage.sqlite3"
    _webkit_storage(users, DRAFTS)

    done = mac.prepare(1.0)

    assert paths.webkit.name == mac_side.BUNDLE_ID
    assert _stored_keys(storage / "localstorage.sqlite3") == set(KEPT)
    assert done["drafts_removed"] == len(DRAFTS)
    language = paths.data / "presentation.toml"
    assert language.read_text(encoding="utf-8") == 'language = "es"\n'
    assert language.stat().st_mode & 0o777 == 0o600
    # The data directory is still cleared whole (the app recreated its logs).
    assert not left.exists()
    # The user's own Orbit is never touched.
    assert _stored_keys(users) == set(DRAFTS)


def test_the_drafts_clean_up_deletes_only_draft_keys(tmp_path):
    paths = mac_side.Paths(tmp_path)
    mac = mac_side.Mac(paths, mac_side.System(), None)
    assert mac.clear_web_drafts() == 0, "no storage: no drafts to remove"
    first = paths.webkit / "WebsiteData" / "Default" / "a1" / "a1" / "localstorage.sqlite3"
    second = paths.webkit / "WebsiteData" / "Default" / "b2" / "b2" / "localstorage.sqlite3"
    _webkit_storage(first, {**KEPT, **DRAFTS})
    _webkit_storage(second, {"orbit.draft.chat": '"otro"', "orbit.ui.theme.local": "light"})

    assert mac.clear_web_drafts() == len(DRAFTS) + 1

    assert _stored_keys(first) == set(KEPT)
    assert _stored_keys(second) == {"orbit.ui.theme.local"}
    assert mac.clear_web_drafts() == 0


def test_a_storage_the_clean_up_cannot_read_stops_the_run_by_name(tmp_path):
    # A draft left in it would be in the box when check 8 types its message.
    paths = mac_side.Paths(tmp_path)
    mac = mac_side.Mac(paths, mac_side.System(), None)
    broken = paths.webkit / "WebsiteData" / "localstorage.sqlite3"
    broken.parent.mkdir(parents=True)
    broken.write_text("no es una base de datos", encoding="utf-8")

    with pytest.raises(mac_side.MacFailure, match="localstorage.sqlite3"):
        mac.clear_web_drafts()
    assert broken.exists()


def test_the_draft_is_looked_for_in_webkit_s_own_storage(tmp_path):
    paths = mac_side.Paths(tmp_path)
    mac = mac_side.Mac(paths, mac_side.System(), None)
    assert mac.web_draft("Borrador inventado") is None, "no storage: unknown, not «no»"
    key = mac_drafts_panel.DRAFT_KEY
    _webkit_storage(
        paths.webkit
        / "WebsiteData"
        / "Default"
        / "a1"
        / "a1"
        / "LocalStorage"
        / "localstorage.sqlite3",
        {key: json.dumps("Borrador inventado 1 Borrador inventado 2"), "otra": '"x"'},
    )
    assert mac.web_draft("Borrador inventado 2") is True
    assert mac.web_draft("Borrador inventado 3") is False


def test_the_texts_of_the_draft_check_are_the_app_s():
    source = ROOT / "apps" / "desktop" / "src"
    assistant = (source / "i18n" / "es" / "assistant.ts").read_text(encoding="utf-8")
    assert f'draftRecovered: "{drafts_panel_checks.TEXTS["recovered"]}"' in assistant
    assert f"composerLabel: `{drafts_panel_checks.TEXTS['composer']} ${{" in assistant
    drafts = (source / "state" / "local-drafts.ts").read_text(encoding="utf-8")
    composer = (source / "features" / "assistant" / "use-composer.ts").read_text(encoding="utf-8")
    assert f'DRAFT_PREFIX = "{mac_drafts_panel.DRAFT_PREFIX}";' in drafts
    prefix, name = mac_drafts_panel.DRAFT_KEY.rsplit(".", 1)
    assert f"{prefix}." == mac_drafts_panel.DRAFT_PREFIX
    assert f'useLocalDraft("{name}", "", asText)' in composer


def _draft_ev(rows, screens):
    return {"drafts": rows, "trees": screens}


COMPOSER = """- [0] AXWindow "Orbit"
  - [1] AXWebArea (Orbit)
    - [4] AXLink "Asistente" [actions=[press]]
{recovered}    - [6] AXTextArea = "{value}" (Mensaje para Deimos) [actions=[confirm]]
"""
RECOVERED = '    - AXStaticText = "Recuperado el borrador que no llegaste a enviar."\n'


def _draft_row(**fields):
    return {"quit": "apple-event", "text": "Borrador uno", "gap_s": 0.3, "came_back": 0, **fields}


def test_the_draft_check_says_the_quit_lost_its_draft():
    screens = {
        # The box came back empty, with its placeholder.
        "draft-after-apple-event": COMPOSER.format(recovered="", value="Escribe a Deimos…"),
    }
    rows = [_draft_row(stored=False)]
    ok, detail = drafts_panel_checks.c17_drafts(_draft_ev(rows, screens))
    assert not ok
    assert "Dock" in detail and "⌘Q" not in detail, detail
    assert "did not have it" in detail, detail


def test_a_draft_back_without_saying_so_or_an_earlier_one_does_not_pass():
    unsaid = {"draft-after-apple-event": COMPOSER.format(recovered="", value="Borrador uno")}
    ok, detail = drafts_panel_checks.c17_drafts(_draft_ev([_draft_row()], unsaid))
    assert not ok
    assert "«Recuperado" in detail and "Dock" in detail, detail
    # Only what was there before the draft was typed came back.
    earlier = {
        "draft-after-apple-event": COMPOSER.format(recovered=RECOVERED, value="Borrador cero")
    }
    ok, detail = drafts_panel_checks.c17_drafts(_draft_ev([_draft_row()], earlier))
    assert not ok
    assert "without the draft" in detail, detail


def test_orbit_coming_back_by_itself_after_the_quit_fails_the_draft_check():
    screens = {
        "draft-after-apple-event": COMPOSER.format(recovered=RECOVERED, value="Borrador uno"),
    }
    ok, detail = drafts_panel_checks.c17_drafts(_draft_ev([_draft_row(came_back=2)], screens))
    assert not ok
    assert "came back by itself" in detail


def test_the_draft_check_passes_on_the_dock_s_quit_alone():
    screens = {
        "draft-after-apple-event": COMPOSER.format(recovered=RECOVERED, value="Borrador uno"),
    }
    ok, detail = drafts_panel_checks.c17_drafts(_draft_ev([_draft_row()], screens))
    assert ok, detail
    assert "Dock" in detail and "⌘Q" not in detail, detail
    # Evidence of the two quits of before is not this check's.
    two = [_draft_row(quit="cmd-q"), _draft_row()]
    ok, _ = drafts_panel_checks.c17_drafts(_draft_ev(two, screens))
    assert not ok


def _question(**fields):
    question = {"id": "q1", "stage": "answered", "finished_at_ms": 1, **fields}
    return {"recording": {"state": "present", "value": {"question": question}}}


def test_the_hold_is_the_daemon_s_and_an_answer_gone_before_it_never_comes_back():
    held = mac_drafts_panel.pointer_held
    state = _question()
    assert held(state, "q1") is None
    assert "not held" in state["seen"]
    assert held(_question(held=True), "q1") == {"held": True}
    for gone in ({"recording": {"state": "present", "value": {}}}, _question(id="q2")):
        with pytest.raises(mac_side.MacFailure, match="left the panel"):
            held(gone, "q1")


def test_the_answer_is_released_only_once_the_daemon_lets_it_go():
    released = mac_drafts_panel.answer_released
    state = _question(released_at_ms=5)
    assert released(state, "q1") is None
    assert "still" in state["seen"]
    assert released({"recording": {"state": "present", "value": {}}}, "q1") == {"gone": "q1"}


def test_an_answer_still_on_screen_after_the_daemon_let_it_go_fails_the_panel_check(good):
    _, directory = good
    data = json.loads((directory / "report.json").read_text(encoding="utf-8"))
    panel = (SCREENS / "good" / "screens" / "panel.md").read_text(encoding="utf-8")
    ev = {
        "panel_hold": data["panel_hold"],
        "question": {"answer": "Estáis hablando del proyecto de la biblioteca municipal."},
        "trees": {"panel-held": panel, "panel-released": panel},
    }
    ok, detail = drafts_panel_checks.c18_panel_hold(ev)
    assert not ok
    assert "still shows the answer" in detail, detail


def test_silent_recording_closes_normally_and_keeps_hub_chunks(good):
    results, directory = good
    assert statuses(results).get(19) == "pass"
    data = json.loads((directory / "report.json").read_text(encoding="utf-8"))
    evidence = data["silence"]
    assert evidence["saved"]["recording_id"] == evidence["recording"]["id"]
    assert evidence["saved"]["capture"] == "closed"
    assert evidence["hub"]["rows"]["chunks"] > 0
    assert {"silence-question", "silence-saved", "silence-hub"} <= {
        wait["wait"] for wait in data["waits"]
    }


def test_silence_waits_follow_the_rust_constants(monkeypatch):
    import real_test

    original = real_test._rust_constant
    values = {"RECORDING_SILENCE_MS": 165_000, "RECORDING_ANSWER_MS": 47_000}
    monkeypatch.setattr(
        real_test,
        "_rust_constant",
        lambda path, name: values[name] if name in values else original(path, name),
    )
    waits = real_test.bounds()
    assert (
        waits["silence-question"][0]
        == 165
        + original("crates/orbit-core/src/audio_signal.rs", "SAMPLE_DELIVERY_TIMEOUT_MS") / 1000
    )
    finalizing = original("crates/orbit-core/src/lib.rs", "FINALIZING_TIMEOUT_MS") / 1000
    http = original("crates/orbit-daemon/src/hub.rs", "HUB_HTTP_TIMEOUT_SECONDS")
    assert (
        waits["silence-saved"][0]
        == 2 * 47
        + original("crates/orbit-core/src/audio_signal.rs", "SAMPLE_DELIVERY_TIMEOUT_MS") / 1000
        + finalizing
        + http
    )
    assert waits["silence-hub"][0] == finalizing + http


@pytest.mark.parametrize("missing", ("question", "saved", "hub", "list"))
def test_silence_check_requires_positive_question_save_and_hub_evidence(good, missing):
    import silence_recording

    _, directory = good
    data = json.loads((directory / "report.json").read_text(encoding="utf-8"))
    ev = {
        "silence": data["silence"],
        "trees": {
            label: (SCREENS / "good" / "screens" / f"{screen}.md").read_text(encoding="utf-8")
            for label, screen in (
                ("silence-question", "silence-recording"),
                ("silence-meetings", "silence-meetings"),
            )
        },
    }
    if missing == "list":
        ev["trees"]["silence-meetings"] = ""
    else:
        ev["silence"][missing] = {}
    ok, _ = silence_recording.c19_saved(ev)
    assert not ok


def test_silence_question_wait_rejects_unknown_old_resolved_or_ended_recordings():
    import mac_silence

    def state(phase="recording", codes=("recording_silence_question",)):
        return {
            "recording": {
                "state": "present",
                "value": {"recording": {"id": "test-meeting", "phase": phase}},
            },
            "notices": {
                "state": "present",
                "value": {"notices": [{"code": code} for code in codes]},
            },
        }

    wait = mac_silence.silence_question
    assert wait(state(), "test-meeting")["code"] == "recording_silence_question"
    assert wait({}, "test-meeting") is None
    assert wait(state(), "another-meeting") is None
    assert wait(state(codes=()), "test-meeting") is None
    assert (
        wait(
            state(codes=("recording_silence_question", "recording_silence_resolved")),
            "test-meeting",
        )
        is None
    )
    for phase in ("finalizing", "idle", "interrupted"):
        with pytest.raises(mac_side.MacFailure, match="before its question"):
            wait(state(phase=phase), "test-meeting")


@pytest.mark.parametrize("code", ("recording_silence_question", "call_silence_question"))
def test_check_ten_accepts_a_question_but_rejects_actual_failures(code):
    import checks

    def evidence(code, notice_class):
        return {
            "logs": {
                "report-daemon.log": json.dumps(
                    {
                        "code": code,
                        "class": notice_class,
                        "op": "test-question",
                        "data": {},
                        "ts": "2026-10-26T10:00:00Z",
                    }
                )
            }
        }

    assert checks.c10_no_failures(evidence(code, "working"))[0]
    for failed, notice_class in (
        ("recording_close_unconfirmed", "failed"),
        ("track_stopped", "fixable"),
    ):
        assert not checks.c10_no_failures(evidence(failed, notice_class))[0]
    # Even a question must be rejected if a producer still reports it as a failure.
    assert not checks.c10_no_failures(evidence(code, "fixable"))[0]


def test_input_route_changes_are_exercised_during_the_same_recording(good):
    results, directory = good
    data = json.loads((directory / "report.json").read_text())
    assert statuses(results).get(20) == "pass"
    steps = [step["name"] for step in data["steps"]]
    # After «Oye Deimos», whose listening the input change restarts.
    assert steps.index("«Oye Deimos» and its answer in the panel") < steps.index(
        "change the default input during the meeting and restore it"
    )
    route = data["audio_route"]
    ident = data["recording"]["id"]
    assert route["away"]["recording_id"] == ident
    assert route["back"]["recording_id"] == ident
    assert route["back"]["gaps"] > route["away"]["gaps"] > 0
    assert all(route[side]["code"] == "audio_route_recovered" for side in ("away", "back"))


def test_menu_check_requires_enabled_rows_and_a_time_in_the_title():
    import copy

    menus = json.loads((SCREENS / "good" / "bridge.json").read_text())
    assert meeting_control_checks.c16_menu_bar({"menus": menus})[0]
    for mutate in (
        lambda tray: tray.update(title=""),
        lambda tray: tray["rows"].pop(),
        lambda tray: tray["rows"].append(dict(tray["rows"][0])),
        lambda tray: tray["rows"][0].update(enabled=False),
    ):
        spoiled = copy.deepcopy(menus)
        mutate(spoiled["tray"])
        assert not meeting_control_checks.c16_menu_bar({"menus": spoiled})[0]


def test_mac_bridge_menus_command_uses_one_bounded_socket_request(tmp_path, capsys):
    import socket
    import tempfile
    import threading

    paths = mac_side.Paths(tmp_path)
    paths.data.mkdir(parents=True)
    menu = json.loads((SCREENS / "good" / "bridge.json").read_text())
    requests = []
    # AF_UNIX also limits path length. Keep this transport fixture short;
    # pytest's descriptive directory exceeds the kernel's sun_path ceiling.
    with (
        tempfile.TemporaryDirectory(prefix="orbit-bridge-", dir="/tmp") as root,
        socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener,
    ):
        paths.bridge = Path(root) / "bridge.sock"
        listener.bind(str(paths.bridge))
        listener.listen(1)

        def reply():
            with listener.accept()[0] as connection:
                requests.append(json.loads(connection.makefile("rb").readline()))
                connection.sendall((json.dumps(menu) + "\n").encode())

        worker = threading.Thread(target=reply, daemon=True)
        worker.start()
        assert mac_side.main(["bridge-menus"], mac_side.Mac(paths)) == 0
        worker.join(timeout=2)
        assert not worker.is_alive()
    assert requests == [{"command": "menus"}]
    assert json.loads(capsys.readouterr().out) == menu


def test_bridge_connection_failure_names_the_transport_and_command(capsys):
    import tempfile

    with tempfile.TemporaryDirectory(prefix="orbit-bridge-", dir="/tmp") as root:
        paths = mac_side.Paths(Path(root))
        # A short, absent path inside this fixture's own directory.
        paths.bridge = Path(root) / "missing.sock"
        assert mac_side.main(["bridge-menus"], mac_side.Mac(paths)) == 1
    error = json.loads(capsys.readouterr().out)["error"]
    assert "test bridge transport failed for menus" in error
    assert "errno 2" in error
    assert "no retry" in error


def test_good_run_uses_bridge_for_input_and_reconnects_after_quit(tmp_path):
    results, directory = dry_run.run("good", tmp_path)
    assert {row["number"]: row["status"] for row in results} == {n: "pass" for n in range(1, 22)}
    data = json.loads((directory / "report.json").read_text())
    assert set(data["accessibility"]["trees"]) == {"idle", "recording"}
    assert (directory / "trees/accessibility-idle.txt").is_file()
    assert (directory / "trees/accessibility-recording.txt").is_file()
    assert data["cua"]["sessions"] == 1
    assert all(row["source"] == "bridge" for row in data["reads"])
    assert data["drafts"][0]["reconnected"]["window_id"] != data["drafts"][0]["previous_window_id"]
    assert any(directory.joinpath("trees").glob("*draft-after*"))


def test_accessibility_failure_does_not_change_any_other_check(tmp_path):
    results, _ = dry_run.run("bad-21", tmp_path)
    assert {row["number"]: row["status"] for row in results} == {
        n: "fail" if n == 21 else "pass" for n in range(1, 22)
    }


def test_accessibility_waits_for_webkit_to_build_its_tree(tmp_path):
    # Real run of 2026-10-29: the first macOS read held only the window's own
    # buttons, and check 21 failed although the app named every control.
    results, directory = dry_run.run("accessibility-lazy-tree", tmp_path)
    assert {row["number"]: row["status"] for row in results} == {n: "pass" for n in range(1, 22)}
    data = json.loads((directory / "report.json").read_text())
    assert data["accessibility"]["reads"] == {"idle": 3, "recording": 3}
    assert "Grabar reunión" in (directory / "trees/accessibility-idle.txt").read_text()


def test_interface_sends_a_guarded_bridge_press_and_saves_its_reading(tmp_path):
    class Bridge:
        def __init__(self):
            self.calls = []

        def call(self, request):
            self.calls.append(request)
            if request["command"] == "windows":
                return {
                    "ok": True,
                    "windows": [{"label": "main", "window_id": 17, "visible": True}],
                }
            if request["command"] == "read":
                return {
                    "ok": True,
                    "tree_markdown": '- [3] AXButton "Record"\n',
                    "elements": [
                        {
                            "index": 3,
                            "role": "button",
                            "name": "Record",
                            "text": "Record",
                            "target": [0],
                        }
                    ],
                }
            return {"ok": True, "count": 1}

    bridge = Bridge()
    ui = interface.Interface(bridge, tmp_path, 42)
    main = ui.windows()["main"]
    ui.press(main, "Record", ("AXButton",))
    assert bridge.calls[-1] == {
        "command": "press",
        "window": "main",
        "role": "button",
        "name": "Record",
        "target": [0],
    }
    assert (tmp_path / "001-find-Record.txt").read_text() == '- [3] AXButton "Record"\n'


def _dom_read(html, request=None):
    """Run the actual injected DOM function, then feed its wire tree to checks."""
    import subprocess

    source = (ROOT / "apps/desktop/src-tauri/src/test_bridge/dom.js").read_text()
    javascript = """
import { JSDOM } from 'jsdom';
import { readFileSync } from 'node:fs';
const fixture = JSON.parse(readFileSync(0, 'utf8'));
const { window } = new JSDOM(fixture.html, {runScripts: 'outside-only'});
window.HTMLElement.prototype.getClientRects = () => [new window.DOMRect(0, 0, 100, 30)];
window.TextEncoder = TextEncoder;
const request = {command:'read', deadline:Date.now()+2000, max_reply_bytes:500000, ...fixture.request};
const result = window.eval('(() => { const run = '+fixture.source+'; return run('+JSON.stringify(request)+'); })()');
process.stdout.write(JSON.stringify(result));
"""
    result = subprocess.run(
        ["node", "--input-type=module", "-e", javascript],
        cwd=ROOT / "apps/desktop",
        input=json.dumps({"html": html, "source": source, "request": request or {}}),
        capture_output=True,
        text=True,
        timeout=30,
        check=True,
    )
    reply = json.loads(result.stdout)
    assert reply["ok"], reply
    return reply


def test_actual_dom_projection_keeps_list_structure_and_never_reads_an_unloaded_list_as_empty():
    import ax_tree

    html = """<aside aria-label="Lista de reuniones"><header><h2>Reuniones</h2>
    <button aria-label="Actualizar">↻</button></header><div><section><h3>Hoy</h3>
    <a href="#one"><span><span>Invented meeting</span><time>10:00</time></span><span>Ready</span></a>
    </section></div></aside><main><a href="#meetings">Reuniones</a><h1>Other view</h1></main>"""
    tree = _dom_read(html)["tree_markdown"]
    assert meeting_control_checks.listed(ax_tree.parse(tree)) == ["Invented meeting"]
    unloaded = html.replace(
        "<section><h3>Hoy</h3>", "<section><p>Leyendo tus reuniones del hub…</p><h3>Hoy</h3>"
    )
    assert (
        meeting_control_checks.listed(ax_tree.parse(_dom_read(unloaded)["tree_markdown"])) is None
    )


def test_actual_dom_projection_keeps_transcript_times_speakers_and_words_without_duplication():
    import ax_tree

    html = """<div><p>Pulsa un nombre para cambiar quién habla.</p>
    <article><time>00:00</time><div><button>Invented speaker</button><p>Hello <span>world</span>.</p></div></article>
    <article><time>00:05</time><div><button>Invented speaker</button><p>Second turn.</p></div></article>
    </div><aside aria-label="Aviso de Orbit"><p>A notice</p></aside>"""
    tree = _dom_read(html)["tree_markdown"]
    _, turns = ax_tree.transcript(ax_tree.parse(tree), "Pulsa un nombre", "Aviso de Orbit")
    assert [(turn.time, turn.speaker) for turn in turns] == [
        ("00:00", "Invented speaker"),
        ("00:05", "Invented speaker"),
    ]
    assert turns[0].text == "Hello world."
    assert turns[1].text == "Second turn."


def test_actual_dom_projection_keeps_the_recovered_composer_field_and_its_value():
    html = """<label>Mensaje para Atlas<textarea>Invented draft</textarea></label>
    <p>Recuperado el borrador que no llegaste a enviar.</p>"""
    tree = _dom_read(html)["tree_markdown"]
    row = {
        "quit": "apple-event",
        "text": "Invented draft",
        "gap_s": 0.01,
        "closed": 1,
        "in_box": True,
        "came_back": 0,
        "stored": True,
    }
    assert drafts_panel_checks.c17_drafts(
        {"drafts": [row], "trees": {"draft-after-apple-event": tree}}
    )[0]
    empty = _dom_read(html.replace("Invented draft", ""))["tree_markdown"]
    assert not drafts_panel_checks.c17_drafts(
        {"drafts": [row], "trees": {"draft-after-apple-event": empty}}
    )[0]


def test_bridge_driver_finds_a_sidebar_label_beside_dynamic_metadata(tmp_path):
    html = '<nav aria-label="Principal"><a href="#meetings"><span>Reuniones</span><span>1 en curso</span></a></nav>'

    class View:
        def __init__(self):
            self.calls = []

        def call(self, request):
            self.calls.append(request)
            if request["command"] == "read":
                return _dom_read(html, request)
            return {"ok": True, "count": 1}

    view = View()
    ui = interface.Interface(view, tmp_path)
    ui.press(interface.Window(42, 11, "Orbit"), "Reuniones", ("AXLink",), first=True)
    assert view.calls[-1]["name"] == "Reuniones1 en curso"
    assert len(view.calls[-1]["target"]) == 3


@pytest.mark.parametrize(
    "phase,label,role",
    [
        ("idle", "Grabar reunión", "AXButton"),
        ("recording", "Terminar", "AXButton"),
        *[
            ("recording", label, "AXLink")
            for label in ("Hoy", "Calendario", "Reuniones", "Notas", "Asistente", "Ajustes")
        ],
    ],
)
def test_accessibility_check_fails_if_any_required_control_loses_its_name(phase, label, role):
    import accessibility

    trees = {
        name: (SCREENS / "good" / f"accessibility-{name}.md").read_text()
        for name in ("idle", "recording")
    }
    assert accessibility.c21_accessibility({"accessibility": {"trees": trees}})[0]
    trees[phase] = trees[phase].replace(f'{role} "{label}"', f'{role} "Unnamed control"')
    ok, detail = accessibility.c21_accessibility({"accessibility": {"trees": trees}})
    assert not ok
    assert label in detail and phase in detail


def test_cua_manifest_cannot_press_or_type_and_its_refusal_only_fails_accessibility(tmp_path):
    paths = mac_side.Paths(tmp_path / "fixture")
    manifest = mac_side.Mac(paths).manifest(42)
    assert "tools: [get_window_state]" in manifest
    assert "press_key" not in manifest and "type_text" not in manifest and "click" not in manifest
    results, _ = dry_run.run("own-process", tmp_path / "run")
    assert {row["number"]: row["status"] for row in results} == {
        n: "fail" if n == 21 else "pass" for n in range(1, 22)
    }


def test_a_partially_started_cua_driver_is_stopped_before_the_recording_script(tmp_path):
    results, _ = dry_run.run("accessibility-start-refused", tmp_path)
    assert {row["number"]: row["status"] for row in results} == {
        n: "fail" if n == 21 else "pass" for n in range(1, 22)
    }
