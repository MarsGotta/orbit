"""Grounded patches, schema roles, conservative contradictions and strict write checks."""

import re

from ..vault.markdown import links
from ..vault.method import validate_note
from ..vault.models import MAX_NOTE_BYTES, MAX_SCAN_BYTES, refusal
from ..vault.paths import checked
from ..vault.schema import frontmatter, load, properties
from .models import Answer
from .patches import render
from .repository import digest


def snapshot(connector):
    entries = connector.catalog()
    result, size = {}, 0
    for entry in entries:
        if entry["kind"] != "note":
            continue
        path = entry["path"]
        item = connector.snapshot(path)
        if item["raw"] is None:
            raise ValueError("vault_changed_during_read")
        size += len(item["raw"].encode())
        if size > MAX_SCAN_BYTES:
            raise refusal(
                "vault_scan_bytes",
                f"The proposal read is {size} bytes; maximum {MAX_SCAN_BYTES}.",
                bytes=size,
                max_bytes=MAX_SCAN_BYTES,
            )
        result[path] = item
    schema = load(result.get("SCHEMA.md", {}).get("raw", ""))
    if schema["diagnostics"]:
        problem = schema["diagnostics"][0]
        raise refusal(
            problem["code"],
            "SCHEMA.md is invalid; correct it before preparing a proposal.",
            **problem.get("numbers", {}),
        )
    return result, schema


def people_for(snapshot, schema, lines):
    names = {}
    for path, item in snapshot.items():
        fields, _ = frontmatter(item["raw"])
        if properties(path, fields, schema)[1] != "person":
            continue
        aliases = fields.get("aliases", [])
        if not isinstance(aliases, list):
            raise ValueError("invalid_person_aliases")  # noqa: TRY004 — recorded validation code.
        for name in [fields.get("title", ""), *aliases]:
            if isinstance(name, str) and name:
                names.setdefault(name.casefold(), set()).add(path)
    result = {}
    for line in lines:
        found = names.get(line["speaker"].casefold(), set())
        if len(found) > 1:
            raise ValueError("ambiguous_person_name")
        result[line["speaker"]] = next(iter(found), None)
    return result


def build(value, content, snapshot, schema, material):
    answer = Answer.model_validate(content)
    data = answer.model_dump()
    quotes = {q["id"]: q for q in data["quotes"]}
    if len(quotes) != len(data["quotes"]):
        raise ValueError("duplicate_quote_id")
    for quote in quotes.values():
        if not any(
            quote["at_ms"] == line["at_ms"]
            and quote["speaker"] == line["speaker"]
            and quote["text"] in line["text"]
            for line in material["lines"]
        ):
            raise ValueError("quote_not_in_transcript")
    value.update(
        title=material["title"],
        quotes=data["quotes"],
        people=people_for(snapshot, schema, material["lines"]),
    )
    paths, meeting_count = set(), 0
    for index, incoming in enumerate(data["changes"]):
        path = checked(incoming["path"])
        if path in paths or path.rsplit("/", 1)[-1] in {
            "SCHEMA.md",
            "AGENTS.md",
            "README.md",
            "index.md",
            "log.md",
        }:
            raise ValueError("duplicate_or_control_note")
        paths.add(path)
        original = snapshot.get(path, {}).get("raw")
        change = {
            **incoming,
            "id": f"n{index}",
            "include": True,
            "kind": "create" if original is None else "update",
            "original": original,
            "base_hash": digest(original),
        }
        _patches(change, value, quotes)
        result = render(change, {c["id"]: c for c in value["conflicts"]})
        role, warnings = validate(path, result, schema)
        if original is not None:
            old_fields, _ = frontmatter(original)
            if properties(path, old_fields, schema)[1] == "source":
                raise ValueError("source_is_immutable")
        if role == "meeting":
            meeting_count += 1
        _person_identity(role, path, result, value["people"], snapshot, schema)
        change["warnings"] = warnings
        value["changes"].append(change)
    if meeting_count != 1:
        raise ValueError("exactly_one_meeting_note_required")
    value["summary"] = data["summary"]
    return value


def _patches(change, value, quotes):
    fields, body = frontmatter(change["original"] or "")
    keys = set()
    for index, prop in enumerate(change["properties"]):
        key = prop["key"]
        if key in keys or key in {"generated", "verified", "sources", "updated", "okf_version"}:
            raise ValueError("duplicate_or_managed_property")
        keys.add(key)
        prop["before"] = fields.get(key)
        if key == "type" and key in fields and fields[key] != prop["after"]:
            raise ValueError("cannot_change_note_type")
        _quotes(prop, quotes)
        if key in fields and fields[key] != prop["after"]:
            _conflict(
                value,
                change,
                f"property:{index}",
                key,
                fields[key],
                prop["after"],
                prop["quote_ids"],
            )
    spans = []
    for index, line in enumerate(change["lines"]):
        _quotes(line, quotes)
        before = line["before"]
        if before:
            if body.count(before) != 1:
                raise ValueError("patch_context_not_unique")
            start = body.index(before)
            end = start + len(before)
            if any(start < b and end > a for a, b in spans):
                raise ValueError("overlapping_patches")
            spans.append((start, end))
            if before != line["after"]:
                _conflict(
                    value,
                    change,
                    f"lines:{index}",
                    line["section"],
                    before,
                    line["after"],
                    line["quote_ids"],
                )
        _new_links(line["after"])
        line["unfiltered_after"] = line["after"]


def _quotes(patch, quotes):
    if any(q not in quotes for q in patch["quote_ids"]):
        raise ValueError("patch_quote_missing")


def _conflict(value, change, key, label, current, incoming, quote_ids):
    value["conflicts"].append(
        {
            "id": f"{change['id']}:{key}",
            "path": change["path"],
            "property": label,
            "current": current,
            "incoming": incoming,
            "quote_ids": quote_ids,
            "choice": None,
        }
    )


def _new_links(text):
    if "[[" in text:
        raise ValueError("new_links_must_be_markdown")
    for target in re.findall(r"\]\(([^)]+)\)", text):
        if not target.startswith(("/", "#", "https://", "http://", "orbit://")):
            raise ValueError("new_links_must_be_vault_absolute")
    for mark in ("📅", "⏳", "🛫", "✅"):
        for date in re.findall(mark + r"\s*([^\s]*)", text):
            from datetime import date as calendar_date

            try:
                calendar_date.fromisoformat(date)
            except ValueError:
                raise ValueError("invalid_task_date") from None


def _person_identity(role, path, raw, people, snapshot, schema):
    if role != "person" or path in snapshot:
        return
    fields, _ = frontmatter(raw)
    title = fields.get("title", "").casefold()
    for existing, item in snapshot.items():
        old, _ = frontmatter(item["raw"])
        if properties(existing, old, schema)[1] == "person" and title in [
            str(n).casefold() for n in [old.get("title"), *old.get("aliases", [])]
        ]:
            raise ValueError("person_already_exists")


def validate(path, raw, schema):
    size = len(raw.encode())
    if size > MAX_NOTE_BYTES:
        raise refusal(
            "vault_note_too_large",
            f"The proposed note is {size} bytes; maximum {MAX_NOTE_BYTES}.",
            413,
            bytes=size,
            max_bytes=MAX_NOTE_BYTES,
        )
    fields, body = frontmatter(raw)
    verdict = validate_note(fields, body, path=path, schema=schema)
    for name, alias in schema["fields"].items():
        if alias != name and name in fields:
            raise ValueError("unmapped_project_field")
    kind, role, _ = properties(path, fields, schema)
    if verdict["errors"]:
        raise ValueError("invalid_note:" + ",".join(verdict["errors"]))
    if kind not in schema["types"] or role not in {"meeting", "person", "project", "topic", "note"}:
        raise ValueError("type_or_role_not_writable")
    folder = schema["types"][kind].get("folder")
    if folder and not path.startswith(folder):
        raise ValueError("note_outside_role_folder")
    return role, verdict["warnings"]


def validate_links(changes, paths):
    for change in changes:
        for line in change["lines"]:
            for target in re.findall(r"\]\((/[^)]+)\)", line["after"]):
                if not links(change["path"], f"[target]({target})", paths):
                    raise ValueError("new_link_target_missing")


def exclude_links(changes):
    from urllib.parse import quote

    excluded = {
        "/" + quote(c["path"], safe="/")
        for c in changes
        if not c["include"] and c["original"] is None
    }
    for change in changes:
        if change["include"]:
            for line in change["lines"]:
                line["after"] = re.sub(
                    r"\[([^]\n]+)\]\((/[^)]+)\)",
                    lambda match: match[1] if match[2] in excluded else match[0],
                    line.get("unfiltered_after", line["after"]),
                )


def validate_source(raw, path, schema):
    fields, body = frontmatter(raw)
    size = len(raw.encode())
    if size > MAX_NOTE_BYTES:
        raise refusal(
            "vault_note_too_large",
            f"The meeting source is {size} bytes; maximum {MAX_NOTE_BYTES}.",
            413,
            bytes=size,
            max_bytes=MAX_NOTE_BYTES,
        )
    if validate_note(fields, body, path=path, schema=schema)["errors"]:
        raise ValueError("invalid_source_metadata")
