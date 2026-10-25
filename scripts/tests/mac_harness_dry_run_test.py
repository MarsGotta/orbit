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
# Every check that can pass or fail; 16, the menu bar's, is always skipped.
ALL = [*range(1, 16), 17, 18]
SKIPPED = {16: "skipped"}
# A check whose evidence another one's step brings: with no answer of «Oye
# Deimos» in the panel (7), there is none to hold under the pointer (18).
FOLLOWS = {7: {18: "unchecked"}}


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
        assert f"(check {number})" in why, why


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
    system.cua.running = True
    system.cua.screen = "assistant"
    typing = {
        "pid": dry_run.PID,
        "window_id": dry_run.MAIN_WINDOW,
        "element_index": 6,
        "snapshot_id": None,
        "text": "Borrador inventado 1a2b3c",
        "delivery_mode": "foreground",
    }

    done = mac.draft_and_quit("apple-event", json.dumps(typing), 1.0, 1.0)

    calls = [c for c in system.commands if c[0] == str(paths.cua)]
    tools = [c[2] for c in calls]
    # Typed, read back from the box, and only then the quit: no other
    # command between them.
    assert tools[-2:] == ["type_text", "get_window_state"]
    after = system.commands[system.commands.index(calls[-1]) + 1 :]
    assert after[0][0] == "osascript", after
    assert "press_key" not in tools
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
    system.cua.running = True
    # The empty composer: typing there leads to no screen showing the text.
    system.cua.screen = "briefing"
    typing = {"pid": dry_run.PID, "window_id": dry_run.MAIN_WINDOW, "element_index": 1}
    typing |= {"text": "Borrador inventado 4d5e6f", "delivery_mode": "foreground"}

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
    system.cua.running = True
    system.cua.screen = "assistant"
    typing = {"pid": dry_run.PID, "window_id": dry_run.MAIN_WINDOW, "element_index": 6}
    typing |= {"text": "Borrador inventado 7a8b9c", "delivery_mode": "foreground"}

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
