"""Check 26: the first meeting distilled into an invented vault, applied and undone.

The model is already there: the test hub's `vault` function uses the
operator's processing model (server/orbit_hub/ai/defaults.py), the test
model. The run adds the vault: a folder of invented notes in its own
directory on this server, connected as Settings › Notes connects a server
folder (the app's `orbit_vault` «configure», through Orbit Pruebas's daemon),
only while this check runs, so no other meeting of the run is distilled.

The meeting script already names the person (Laura, Clara in English) and
the reading room's start, 3 November; the project here starts on another
day, so the proposal has a contradiction to decide. The check decides it on
screen 53, applies, reads the vault back from disk and presses «Deshacer».
It never checks what the summary says, which is the model's: only that the
proposal validated, that what was decided was written with its source,
index, log and provenance, and that undoing restored the notes.
"""

from __future__ import annotations

import hashlib
import json
import re
from datetime import date
from pathlib import Path
from urllib.parse import quote

from checks import _fact, _title
from harness_copy import text as copy_text
from interface import StepFailed

NAMES = {26: "The first meeting is distilled into an invented vault, applied as decided and undone"}
NOT_COVERED = (
    (
        "what a distillation says (its summary, people and tasks are the model's), "
        "Fast Note Sync, and the native notice of a new proposal"
    ),
)

# Named in the meeting script: «Laura, ¿puedes llamar…», «Clara, can you telephone…».
PERSON = {"es": "Laura", "en": "Clara"}
OWNER = "maple"
# The script starts the reading room's works on 3 November; the vault does not.
PROJECT_START = "2026-11-17"
# The status each of the check's waits must end in.
SETTLED = {"distillation": "proposed", "apply": "applied", "undo": "undone"}
DISCUSSION = "<!-- in discussion -->"


def bounds() -> dict[str, tuple[float, str]]:
    from orbit_hub.distillation.models import CONNECTOR_JOB_SECONDS
    from orbit_hub.notes.worker import IDLE_TICK_SECONDS
    from orbit_hub.processing.config import ProcessingConfiguration

    model_s = ProcessingConfiguration.model_fields["timeout_seconds"].default
    return {
        "distillation": (
            IDLE_TICK_SECONDS + model_s + CONNECTOR_JOB_SECONDS,
            (
                f"what the hub takes to give a preparation up: the notes worker's "
                f"{IDLE_TICK_SECONDS} s idle tick (IDLE_TICK_SECONDS), the model's "
                f"{model_s:.0f} s and the vault connector's {CONNECTOR_JOB_SECONDS} s "
                "(CONNECTOR_JOB_SECONDS); a failed preparation ends it earlier"
            ),
        ),
        "distillation-write": (
            CONNECTOR_JOB_SECONDS,
            (
                f"the vault connector's {CONNECTOR_JOB_SECONDS} s background budget "
                "(CONNECTOR_JOB_SECONDS), which bounds an apply's or undo's vault I/O"
            ),
        ),
    }


def vault_notes(language: str) -> dict[str, str]:
    person = PERSON[language]
    return {
        "SCHEMA.md": (
            f"---\ntype: schema\nowner: {OWNER}\ntypes:\n"
            "  person: {role: person, folder: People/}\n"
            "  project: {role: project, folder: Projects/}\n"
            "  meeting: {role: meeting, folder: Meetings/}\n"
            "  source: {role: source, folder: Sources/}\n---\n"
            "Invented vault of Orbit's real test. Keep each project's start date current.\n"
        ),
        f"People/{person.lower()}-cedar.md": (
            f"---\ntype: person\ntitle: {person} Cedar\naliases: [{person}]\n---\n"
            f"# {person} Cedar\n\nHandles the library's suppliers.\n"
        ),
        "Projects/reading-room-works.md": (
            "---\ntype: project\ntitle: Reading room works\nstage: planning\n"
            f"start: '{PROJECT_START}'\n---\n# Reading room works\n\n"
            "The public library's reading room is rebuilt.\n\n## Status\n"
            "Waiting for the signed drawings.\n"
        ),
        "index.md": (
            '---\nokf_version: "0.2"\n---\n# Maple notes\n\n## Notes\n'
            f"* [{person} Cedar](/People/{person.lower()}-cedar.md) - Suppliers\n"
            "* [Reading room works](/Projects/reading-room-works.md) - The library's works\n"
        ),
        "log.md": "# Log\n",
    }


def write_vault(root: Path, language: str) -> None:
    root.mkdir(mode=0o700, parents=True)
    for path, text in vault_notes(language).items():
        (root / path).parent.mkdir(exist_ok=True)
        (root / path).write_text(text, encoding="utf-8")


def read_vault(root: Path) -> dict[str, str]:
    return {
        path.relative_to(root).as_posix(): path.read_text(encoding="utf-8")
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


def exercise(test):
    # The first meeting's ready note is what the hub distills.
    if test.ev.get("note_phase") is None:
        return
    with test.step(
        "distill the first meeting into an invented vault, then undo", (26,), critical=False
    ):
        row = test.ev["distillation"] = {}
        meeting = test.ev["recording"]["id"]
        root = test.directory / "vault"
        try:
            write_vault(root, test.language)
            # The connection Settings › Notes saves for a server folder (ConnectionCard.tsx).
            connection = {
                "kind": "folder",
                "path": str(root),
                "url": "",
                "vault": "",
                "link_style": "markdown",
                "maintenance_model": "",
            }
            configured = test.mac.vault({"call": "configure", "configuration": connection})
            row["connected"] = configured["configuration"]
            row["proposed"] = _settled(test, "distillation", meeting, "none")
            row["decided"] = _review(test, row["proposed"])
            row["files"] = {"before": test.vault_files("before")}
            ui, main = test.ui, test.main
            ui.press(main, copy_text("distillation.applyPrefix"), ("AXButton",), contains=True)
            row["applied"] = _settled(test, "apply", meeting, "proposed")
            row["files"]["applied"] = test.vault_files("applied")
            ui.read(main, "distill-applied", None)
            ui.press(main, copy_text("distillation.undo"), ("AXButton",))
            row["undone"] = _settled(test, "undo", meeting, "applied")
            row["files"]["undone"] = test.vault_files("undone")
            ui.read(main, "distill-undone", None)
        finally:
            row["disconnected"] = _disconnect(test, row)


def _settled(test, step, meeting, after):
    what = "distillation" if step == "distillation" else "distillation-write"
    wait = {"wait": what, "max_s": test.bounds[what][0]}
    test.ev["waits"].append(wait)
    try:
        done = test.mac.distillation(meeting, after, wait["max_s"])
    except StepFailed as cause:
        wait["failure"] = str(cause)
        raise
    wait["seconds"] = done["seconds"]
    proposal = done["proposal"]
    if proposal.get("status") != SETTLED[step]:
        error = proposal.get("error") or {}
        # The notes the hub left out, with why: the invented vault's paths only.
        dropped = [f"{d.get('path')} ({d.get('code')})" for d in proposal.get("dropped") or []]
        raise StepFailed(
            f"the proposal is «{proposal.get('status')}», not «{SETTLED[step]}»"
            + (f": {json.dumps(error, ensure_ascii=False)[:200]}" if error else "")
            + (f"; left out: {', '.join(dropped)[:300]}" if dropped else "")
        )
    return proposal


def _review(test, proposal):
    """Opens screen 53 from the meeting's pill and decides every contradiction:
    a property takes the meeting's value, a text keeps both «in discussion»."""
    ui, main, screen_s = test.ui, test.main, test.bounds["screen"][0]
    ui.press(main, copy_text("shell.notes"), ("AXLink",), first=True)
    ui.press(main, _title(test.ev), ("AXLink", "AXButton"), contains=True, first=True)
    # «● Revisar el destilado · N cambios», once the note reads its proposal
    # (MeetingDistillation.tsx); the press waits for it to be drawn.
    ui.press(main, copy_text("distillation.reviewPillPrefix"), ("AXLink",), contains=True)
    decide = copy_text("distillation.decide")
    test.measure(
        "screen", ui.read_until(main, "distill-review", None, lambda t: decide in t, screen_s)
    )
    decided, shown = {}, []
    for at, conflict in enumerate(proposal.get("conflicts", [])):
        if at:
            # The column's button names the property and «Falta decidir» as one
            # text (DistillationColumn.tsx); the first undecided one is this one.
            name = _clean(conflict["property"] + copy_text("distillation.missing"))
            ui.press(main, name, ("AXButton",), first=True)
        choice, label = _choice(conflict)
        # Only the selected contradiction's options are drawn.
        ui.press(main, label, ("AXRadioButton",))
        decided[conflict["id"]] = choice
        # Saved before the next press: the review locks its controls meanwhile.
        # Its column button then names the choice; two alike are counted.
        shown.append(_clean(conflict["property"] + label))
        name, alike = shown[-1], shown.count(shown[-1])
        test.measure(
            "screen",
            ui.read_until(
                main,
                "distill-decided",
                None,
                lambda t, name=name, alike=alike: t.count(name) >= alike,
                screen_s,
            ),
        )
    return decided


def _choice(conflict):
    if ":property:" in conflict["id"]:
        return "incoming", copy_text("distillation.use", value=_value_text(conflict["incoming"]))
    return "both", copy_text("distillation.both")


def _value_text(value) -> str:
    """distillation-format.ts `valueText`."""
    if value is None:
        return "—"
    if isinstance(value, str):
        return value
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def _clean(text: str) -> str:
    """The bridge's accessible names collapse whitespace (dom.js `clean`)."""
    return " ".join(text.split())


def _disconnect(test, row) -> bool:
    try:
        return test.mac.vault({"call": "disconnect"})["configuration"].get("configured") is False
    except StepFailed as cause:
        row["disconnect_failure"] = str(cause)
        return False


# ── The check ───────────────────────────────────────────────────────────────


def _front(text: str | None) -> tuple[dict, str]:
    """A note's frontmatter and body, split as the hub splits them
    (orbit_hub/vault/schema.py `frontmatter`)."""
    import yaml  # The orchestrator runs in the hub's environment (server/.venv).

    if text is None or not text.startswith("---"):
        return {}, text or ""
    parts = re.split(r"^---\s*$", text, maxsplit=2, flags=re.MULTILINE)
    try:
        fields = yaml.safe_load(parts[1]) if len(parts) == 3 else None
    except yaml.YAMLError:
        fields = None
    if not isinstance(fields, dict):
        return {}, text
    return fields, parts[2].lstrip("\r\n")


def _plain(value):
    return value.isoformat() if isinstance(value, date) else value


def _verified_by(fields: dict):
    """The newest `verified` record's author (OKF 0.2 allows one record or a list)."""
    records = fields.get("verified") or []
    records = [records] if isinstance(records, dict) else records
    return records[-1].get("by") if records and isinstance(records[-1], dict) else None


def _link(path: str) -> str:
    return "](/" + quote(path, safe="/") + ")"


def c26_distillation(ev):
    row = _fact(ev, "distillation")
    if row.get("disconnected") is not True:
        return False, "the invented vault was not disconnected after the check"
    proposed, applied, undone = (row.get(k) or {} for k in ("proposed", "applied", "undone"))
    files = row.get("files") or {}
    before, written, restored = (files.get(k) or {} for k in ("before", "applied", "undone"))
    if proposed.get("meeting_id") != (ev.get("recording") or {}).get("id"):
        return False, "the proposal is not the first meeting's"
    if not any(":property:" in c["id"] for c in proposed.get("conflicts", [])):
        return False, "the proposal has no contradiction on a property (the project's start)"
    if {c["id"]: c["choice"] for c in applied.get("conflicts", [])} != row.get("decided"):
        return False, f"the applied proposal does not carry the decisions {row.get('decided')}"
    for verdict in (
        _applied(applied, before, written),
        _decisions(applied, written),
        _undone(applied, undone, before, written, restored),
    ):
        if verdict is not None:
            return False, verdict
    notes = len([c for c in applied["changes"] if c["include"]])
    return (
        True,
        (
            f"{notes} notes and a source written as decided, with index, log and "
            f"human:{_owner(before)}; undoing restored them and kept the source"
        ),
    )


def _owner(before):
    return _front(before.get("SCHEMA.md"))[0].get("owner")


def _applied(applied, before, written):
    owner, meeting = _owner(before), applied.get("meeting_id")
    source = applied.get("new_source") or {}
    changes = [c for c in applied.get("changes", []) if c["include"]]
    for change in changes:
        fields, _ = _front(written.get(change["path"]))
        if not fields:
            return f"{change['path']} was not written"
        if (change["kind"] == "create") == (change["path"] in before):
            had = "had" if change["path"] in before else "lacked"
            return f"{change['path']} is a {change['kind']}, but the vault {had} it"
        if (fields.get("generated") or {}).get("by") != applied.get("by"):
            return f"{change['path']} lacks generated.by {applied.get('by')}"
        if _verified_by(fields) != f"human:{owner}":
            return f"{change['path']} is not verified by human:{owner}"
    if not any(
        f"orbit://meeting/{meeting}"
        in [s.get("resource") for s in _front(written.get(c["path"]))[0].get("sources", [])]
        for c in changes
    ):
        return "no written note is the meeting's, with its orbit://meeting source"
    fields, body = _front(written.get(source.get("path")))
    digest = hashlib.sha256(body.encode()).hexdigest()
    if not fields or not (fields.get("sha256") == source.get("sha256") == digest):
        return f"the source {source.get('path')} is missing or its sha256 is not its body's"
    if _verified_by(fields) != f"human:{owner}":
        return f"the source is not verified by human:{owner}"
    paths = [c["path"] for c in changes] + [source["path"]]
    missing = [p for p in paths if _link(p) not in written.get("index.md", "")]
    if missing:
        return f"index.md does not list {missing}"
    lines = {
        p: written.get("log.md", "").count(_link(p)) - before.get("log.md", "").count(_link(p))
        for p in [*paths, "index.md", "log.md"]
    }
    if any(n != 1 for n in lines.values()):
        return f"log.md needs one new line per written file: {lines}"
    return None


def _decisions(applied, written):
    for conflict in applied.get("conflicts", []):
        fields, body = _front(written.get(conflict["path"]))
        if conflict["choice"] == "incoming":
            if _plain(fields.get(conflict["property"])) != conflict["incoming"]:
                where = f"{conflict['path']} {conflict['property']}"
                return f"{where} is not the meeting's {conflict['incoming']}"
        elif conflict["choice"] == "both" and DISCUSSION not in body:
            return f"{conflict['path']} does not keep both «{conflict['property']}» in discussion"
    return None


def _undone(applied, undone, before, written, restored):
    if undone.get("skipped"):
        return f"undoing skipped {undone['skipped']}"
    changed = [
        p
        for p, text in before.items()
        if p not in ("index.md", "log.md") and restored.get(p) != text
    ]
    if changed:
        return f"undoing did not restore {changed}: not restored as they were"
    created = [c["path"] for c in applied["changes"] if c["include"] and c["kind"] == "create"]
    if any(p in restored for p in created):
        return (
            f"undoing left the created notes {[p for p in created if p in restored]}: not restored"
        )
    source = applied["new_source"]["path"]
    if restored.get(source) is None or restored[source] != written.get(source):
        return f"undoing did not keep the source {source}"
    return None


CHECKS = {26: c26_distillation}
