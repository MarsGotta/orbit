"""Write planning and recovery. Logs are prepend-only; source bodies are immutable."""

import re
from datetime import UTC, datetime
from urllib.parse import quote

import yaml

from ..vault.models import MAX_NOTE_BYTES, refusal
from ..vault.schema import frontmatter
from .patches import metadata, render
from .repository import digest
from .validation import validate, validate_links


def timestamp(now):
    return datetime.fromtimestamp(now, UTC).isoformat()


def actor(owner, device):
    from ..vault.method import file_name

    normalized = file_name(owner or "")[:-3]
    return "human:" + (normalized or device)


def source_for(value, schema, material, by):
    kinds = [(kind, spec) for kind, spec in schema["types"].items() if spec["role"] == "source"]
    if not kinds:
        raise ValueError("source_role_missing")
    kind, spec = kinds[0]
    transcript = "\n".join(
        f"[{line['at_ms']}ms] {line['speaker']}: {line['text']}" for line in material["lines"]
    )
    body = f"# {material['title']}\n\n{value['summary']}\n\n## Transcript\n\n{transcript}\n"
    source_id = digest(value["device_id"] + ":" + value["meeting_id"] + ":" + digest(body))[:24]
    path = spec.get("folder", "Sources/") + source_id + ".md"
    raw = metadata(
        body,
        {
            "type": kind,
            "title": material["title"],
            "resource": "orbit://meeting/" + value["meeting_id"],
            "sha256": digest(body),
            "generated": {"by": by, "at": value["created_at"]},
        },
    )
    return {"path": path, "summary": value["summary"], "sha256": digest(body)}, raw


def with_provenance(raw, value, by, owner, at, *, meeting=False):
    frontmatter(raw)  # Validate before using a loader that retains nested scalar types.
    fields = yaml.safe_load(re.split(r"^---\s*$", raw, maxsplit=2, flags=re.MULTILINE)[1])
    sources = list(fields.get("sources", []))
    resource = "/" + quote(value["new_source"]["path"], safe="/")
    for citation in value["quotes"]:
        source = {
            "resource": resource,
            "id": citation_prefix(value) + "-" + citation["id"],
            "title": value["title"],
        }
        if source not in sources:
            sources.append(source)
    if meeting:
        original = {
            "resource": "orbit://meeting/" + value["meeting_id"],
            "id": "orbit-" + value["meeting_id"],
            "title": value["title"],
        }
        if original not in sources:
            sources.append(original)
    verified = fields.get("verified", [])
    verified = [verified] if isinstance(verified, dict) else list(verified)
    verified.append({"by": owner, "at": at})
    raw = metadata(
        raw,
        {
            "sources": sources,
            "generated": {**fields.get("generated", {}), "by": by, "at": at},
            "verified": verified,
        },
    )
    title = value["title"].replace("[", r"\[").replace("]", r"\]").replace("\n", " ")
    for citation in value["quotes"]:
        key = citation_prefix(value) + "-" + citation["id"]
        definition = f"[^{key}]: [{title}]({resource}#t={citation['at_ms']}ms)"
        if f"[^{key}]:" not in raw:
            raw = raw.rstrip("\n") + "\n\n" + definition + "\n"
    return raw


def index_update(raw, notes):
    if raw is None:
        raw = '---\nokf_version: "0.2"\n---\n# Notes\n'
    fields, _ = frontmatter(raw)
    if fields.get("okf_version") != "0.2":
        raw = metadata(raw, {"okf_version": "0.2"})
    for path, text in notes.items():
        target = "/" + quote(path, safe="/")
        expression = r"^\* \[[^\n]*\]\(" + re.escape(target) + r"\) - [^\n]*(?:\n|$)"
        raw = re.sub(expression, "", raw, flags=re.MULTILINE)
        if text is not None:
            fields, _ = frontmatter(text)
            title = (
                str(fields.get("title", path))
                .replace("\n", " ")
                .replace("[", r"\[")
                .replace("]", r"\]")
            )
            description = str(fields.get("description", "")).replace("\n", " ")
            if not re.search(r"^## ", raw, re.MULTILINE):
                raw += "\n## Notes\n"
            raw = raw.rstrip("\n") + f"\n* [{title}]({target}) - {description}\n"
    return raw


def log_update(raw, entries, by, at, *, undo=False):
    day, clock = at[:10], at[11:16]
    rows = []
    for path, previous, result in entries:
        fields, _ = frontmatter(result or previous or "")
        title = (
            str(fields.get("title", path))
            .replace("\n", " ")
            .replace("[", r"\[")
            .replace("]", r"\]")
        )
        action = "delete" if result is None else "create" if previous is None else "update"
        detail = "Undo meeting distillation" if undo else "Meeting distillation"
        rows.append(
            f"- {clock} · {action} · [{title}](/" + quote(path, safe="/") + f") · {by} · {detail}\n"
        )
    raw = raw or "# Log\n"
    heading = f"## {day}\n"
    if heading in raw:
        return raw.replace(heading, heading + "".join(rows), 1)
    match = re.search(r"^## \d{4}-\d{2}-\d{2}", raw, re.MULTILINE)
    offset = match.start() if match else len(raw)
    return raw[:offset].rstrip("\n") + "\n\n" + heading + "".join(rows) + "\n" + raw[offset:]


def plan(value, schema, by, owner, at):
    conflicts = {c["id"]: c for c in value["conflicts"]}
    notes = {}
    for change in value["changes"]:
        if change["include"]:
            text = render(
                change, conflicts, require_decisions=True, citation_prefix=citation_prefix(value)
            )
            role, _ = validate(change["path"], text, schema)
            text = with_provenance(text, value, by, owner, at, meeting=role == "meeting")
            _, change["warnings"] = validate(change["path"], text, schema)
            notes[change["path"]] = text
    paths = set(value["paths"]) | set(notes)
    validate_links([c for c in value["changes"] if c["include"]], paths)
    source = value["new_source"]["path"]
    old_source = value["snapshot"][source]["raw"]
    if old_source is not None:
        fields, body = frontmatter(old_source)
        if (
            fields.get("sha256") != digest(body)
            or fields.get("sha256") != value["new_source"]["sha256"]
        ):
            raise ValueError("source_already_exists_with_different_body")
    else:
        notes[source] = metadata(value["source_raw"], {"verified": [{"by": owner, "at": at}]})
    index = value["snapshot"]["index.md"]["raw"]
    notes["index.md"] = index_update(index, {**notes, source: old_source or value["source_raw"]})
    entries = [(p, value["snapshot"][p]["raw"], raw) for p, raw in notes.items()]
    entries.append(("log.md", value["snapshot"]["log.md"]["raw"], ""))
    notes["log.md"] = log_update(value["snapshot"]["log.md"]["raw"], entries, by, at)
    for raw in notes.values():
        size = len(raw.encode())
        if size > MAX_NOTE_BYTES:
            raise refusal(
                "vault_note_too_large",
                f"The proposed file is {size} bytes; maximum {MAX_NOTE_BYTES}.",
                413,
                bytes=size,
                max_bytes=MAX_NOTE_BYTES,
            )
    return [{"path": p, "before": value["snapshot"][p], "after": raw} for p, raw in notes.items()]


def citation_prefix(value):
    return "s" + digest(value["new_source"]["path"])[:12]
