"""Preflight every before image, journal before effects, and never retry uncertain writes."""

from ..errors import HubError
from ..vault.fast_note_sync import FastNoteSync
from ..vault.models import refusal
from .effects import index_update, log_update
from .repository import digest


def preflight(connector, value):
    changed = []
    expected_notes = {
        p: {"hash": digest(item["raw"]), "version": item.get("version")}
        for p, item in value["snapshot"].items()
    }
    expected_notes.update(value["read_set"])
    versions = None
    if isinstance(connector, FastNoteSync):
        versions = {item["path"]: item.get("stamp") for item in connector.catalog()}
    for path, expected in expected_notes.items():
        if versions is not None and path not in value["snapshot"]:
            stamp = versions.get(path)
            # Upstream advances this version on each content change. A fresh
            # catalogue version is evidence of the same captured hash, not a
            # cached negative fact. Missing/versionless rows get a direct read.
            if stamp and type(stamp[1]) is int and stamp[1] == expected.get("version"):
                continue
        current = connector.snapshot(path)
        if digest(current["raw"]) != expected["hash"] or current.get("version") != expected.get(
            "version"
        ):
            changed.append(path)
    return sorted(changed)


def apply(connector, value, journal, repository):
    changed = preflight(connector, value)
    if changed:
        value.update(status="stale", changed_paths=changed)
        repository.save(value)
        raise refusal(
            "distillation_content_changed",
            f"{len(changed)} notes changed; prepare the proposal again.",
            409,
            changed_notes=len(changed),
        )
    value.update(status="applying", journal=journal)
    repository.save(value)
    try:
        for entry in journal:
            written = connector.write(entry["path"], entry["after"], entry["before"])
            entry["written"] = {k: v for k, v in written.items() if k != "raw"}
            repository.save(value)
    except Exception as exc:  # noqa: BLE001 — every partial write must enter recovery.
        # Lost receipts are reconciled by reading, never by retrying an input.
        value["status"] = "recovery_required"
        numbers = getattr(exc, "numbers", None) or {}
        value["error"] = {"code": getattr(exc, "code", "vault_write_failed"), **numbers}
        try:
            repository.save(value)
        finally:
            _rollback(connector, value, repository)
        raise refusal(
            "distillation_write_failed",
            "The write failed; read the proposal for rollback and recovery details.",
            409,
            **numbers,
        ) from None
    value.update(status="applied", error=None)
    repository.save(value)
    return value


def _rollback(connector, value, repository):
    unresolved = []
    for entry in reversed(value["journal"]):
        try:
            current = connector.snapshot(entry["path"])
            if current["raw"] == entry["before"]["raw"]:
                continue
            if current["raw"] != entry["after"]:
                unresolved.append({"path": entry["path"], "code": "content_changed"})
                continue
            _restore(connector, entry["path"], entry["before"]["raw"], current)
        except (HubError, OSError):
            unresolved.append({"path": entry["path"], "code": "outcome_unknown"})
    value.update(status="recovery_required" if unresolved else "failed", skipped=unresolved)
    repository.save(value)


def _restore(connector, path, original, current):
    if original is None:
        connector.remove(path, current)
    else:
        connector.write(path, original, current)


def undo(connector, value, repository, by, at):
    if value["status"] in {"applying", "recovery_required"}:
        _rollback(connector, value, repository)
        return value
    source = value["new_source"]["path"]
    skipped, restored = [], {}
    value["status"] = "undoing"
    repository.save(value)
    for entry in value["journal"]:
        path = entry["path"]
        if path in {source, "index.md", "log.md"} or entry.get("undone"):
            continue
        try:
            current = connector.snapshot(path)
        except (HubError, OSError):
            skipped.append({"path": path, "code": "outcome_unknown"})
            continue
        if current["raw"] != entry["after"] or current.get("version") != entry.get(
            "written", {}
        ).get("version"):
            skipped.append({"path": path, "code": "content_changed"})
            continue
        try:
            _restore(connector, path, entry["before"]["raw"], current)
        except (HubError, OSError):
            skipped.append({"path": path, "code": "outcome_unknown"})
            continue
        entry["undone"] = True
        restored[path] = entry["before"]["raw"]
        repository.save(value)
    _undo_controls(connector, value, restored, skipped, by, at)
    value.update(status="undone", skipped=skipped)
    repository.save(value)
    return value


def _undo_controls(connector, value, restored, skipped, by, at):
    index = connector.snapshot("index.md")
    applied_index = next(e for e in value["journal"] if e["path"] == "index.md")
    if index["raw"] != applied_index["after"]:
        skipped.append({"path": "index.md", "code": "content_changed"})
    else:
        try:
            connector.write("index.md", index_update(index["raw"], restored), index)
        except (HubError, OSError):
            skipped.append({"path": "index.md", "code": "outcome_unknown"})
    log = connector.snapshot("log.md")
    entries = [
        (e["path"], e["after"], e["before"]["raw"])
        for e in value["journal"]
        if e["path"] in restored
    ]
    if entries:
        try:
            connector.write("log.md", log_update(log["raw"], entries, by, at, undo=True), log)
        except (HubError, OSError):
            skipped.append({"path": "log.md", "code": "outcome_unknown"})
