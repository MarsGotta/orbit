"""Grounded patches, schema roles, conservative contradictions and strict write checks.

A model's answer is checked note by note. Obsidian links and unreadable task
dates are repaired without inventing anything; a note that still fails is left
out of the proposal with its cause in `dropped`, and the rest is proposed.
"""

import contextlib
import re
from urllib.parse import quote

from ..errors import HubError
from ..vault.markdown import links
from ..vault.method import file_name, validate_note
from ..vault.models import MAX_NOTE_BYTES, MAX_SCAN_BYTES, refusal
from ..vault.paths import checked, resolve
from ..vault.schema import frontmatter, load, properties
from ..vault.syntax import MARKER, valid_date
from .models import Answer
from .patches import render
from .repository import digest

CONTROL_NOTES = frozenset({"SCHEMA.md", "AGENTS.md", "README.md", "index.md", "log.md"})
# Notes the hub writes itself; a new link may point at them before they exist.
MANAGED_TARGETS = frozenset({"SCHEMA.md", "index.md", "log.md"})
WIKILINK = re.compile(r"\[\[([^\]\n]+)\]\]")
# A Tasks date runs from its marker to the next Tasks marker or the end of the
# line, as the vault reads it (vault/syntax.py).
DATE_FIELD = re.compile(
    r"[ \t]*(📅|⏳|🛫|✅)[ \t]*([^\n]*?)(?=[ \t]*(?:" + MARKER.pattern + r")|[ \t]*$)",
    re.MULTILINE,
)
VAULT_LINK = re.compile(r"\[([^]\n]+)\]\((/[^)]+)\)")
# Characters the vault refuses in a path (vault/paths.checked) that are only
# characters in a file name, such as the time in «Meeting 10:30».
REFUSED_IN_NAME = re.compile(r"[:\\\x00-\x1f]")


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
    if len({q["id"] for q in data["quotes"]}) != len(data["quotes"]):
        raise ValueError("duplicate_quote_id")
    # An invented quote is never shown; its id maps to None, so what only it
    # supports is dropped (`_quotes`).
    quotes = {
        q["id"]: q
        if any(
            q["at_ms"] == line["at_ms"]
            and q["speaker"] == line["speaker"]
            and q["text"] in line["text"]
            for line in material["lines"]
        )
        else None
        for q in data["quotes"]
    }
    value.update(
        title=material["title"],
        quotes=[q for q in quotes.values() if q is not None],
        people=people_for(snapshot, schema, material["lines"]),
        dropped=[],
    )
    targets = _targets(snapshot, data["changes"])
    seen, roles, failures = set(), {}, []
    for index, incoming in enumerate(data["changes"]):
        conflicts = len(value["conflicts"])
        try:
            change, role = _change(index, incoming, value, snapshot, schema, quotes, seen, targets)
        except (ValueError, HubError) as exc:
            del value["conflicts"][conflicts:]
            _drop(value, incoming["path"], exc, failures)
            continue
        roles[change["id"]] = role
        value["changes"].append(change)
    _drop_broken_links(value, snapshot, failures)
    meetings = [roles[c["id"]] for c in value["changes"]].count("meeting")
    if meetings != 1:
        # The meeting note anchors the proposal: without it nothing valid is left.
        value.update(changes=[], conflicts=[])
        if meetings == 0 and failures:
            raise failures[0]
        raise ValueError("exactly_one_meeting_note_required")
    value["summary"] = data["summary"]
    return value


def _change(index, incoming, value, snapshot, schema, quotes, seen, targets):
    path = checked(targets[2].get(incoming["path"], incoming["path"]))
    if path in seen or path.rsplit("/", 1)[-1] in CONTROL_NOTES:
        raise ValueError("duplicate_or_control_note")
    seen.add(path)
    original = snapshot.get(path, {}).get("raw")
    change = {
        **incoming,
        "path": path,
        "id": f"n{index}",
        "include": True,
        "kind": "create" if original is None else "update",
        "original": original,
        "base_hash": digest(original),
    }
    _patches(change, value, quotes, targets)
    result = render(change, {c["id"]: c for c in value["conflicts"]})
    role, warnings = validate(path, result, schema)
    if original is not None:
        old_fields, _ = frontmatter(original)
        if properties(path, old_fields, schema)[1] == "source":
            raise ValueError("source_is_immutable")
    _person_identity(role, path, result, value["people"], snapshot, schema)
    change["warnings"] = warnings
    return change, role


def code(exc):
    """Only our own validation codes, never provider text or raw input."""
    if isinstance(exc, HubError):
        return exc.code
    reason = str(exc)
    safe = reason and all(c.isalnum() or c in "_:," for c in reason)
    return reason[:160] if safe else "invalid_proposal"


def _drop(value, path, exc, failures):
    failures.append(exc)
    value["dropped"].append({"path": path, "code": code(exc)})


def _targets(snapshot, changes):
    """Vault notes and the notes this answer creates: the only link targets."""
    paths, renamed = set(snapshot), {}
    for change in changes:
        path = _safe_name(change["path"], snapshot)
        if path != change["path"]:
            renamed[change["path"]] = path
        with contextlib.suppress(HubError):
            paths.add(checked(path))
    basenames = {}
    for path in paths:
        basenames.setdefault(file_name(path.rsplit("/", 1)[-1][:-3]), []).append(path)
    return paths, basenames, renamed


def _safe_name(path, snapshot):
    """A new note's file name refused only for its characters gets `-` for each.

    The folder is left as written, so absolute, hidden or `..` paths are still
    refused; a name that would then be an existing note is not changed.
    """
    folder, slash, name = path.rpartition("/")
    safe = folder + slash + REFUSED_IN_NAME.sub("-", name)
    return safe if safe not in snapshot else path


def _repair(text, source, targets):
    """Obsidian links become Orbit's links, or words; an unreadable task date goes."""

    paths, basenames, renamed = targets

    def link(match):
        target, _, label = match[1].partition("|")
        found = resolve(source, target, paths, wiki=True, basenames=basenames)
        words = (label or target).strip()
        return f"[{words}](/{quote(found, safe='/')})" if found else words

    def date(match):
        value = match[2].split()
        return match[0] if value and valid_date(value[0]) else ""

    for old, new in renamed.items():
        for written in {old, quote(old, safe="/"), quote(old, safe="/:")}:
            text = text.replace(f"](/{written})", f"](/{quote(new, safe='/')})")
    return DATE_FIELD.sub(date, WIKILINK.sub(link, text))


def _drop_broken_links(value, snapshot, failures):
    """Links to notes no longer created become words; a link to nothing drops its note."""
    while True:
        kept = {c["path"] for c in value["changes"]}
        gone = {
            "/" + quote(d["path"], safe="/")
            for d in value["dropped"]
            if d["path"] not in snapshot and d["path"] not in kept
        }
        conflicts = {c["id"]: c for c in value["conflicts"]}
        for change in value["changes"]:
            for number, line in enumerate(change["lines"]):
                text = _unlink(line["after"], gone)
                if text != line["after"]:
                    line["after"] = line["unfiltered_after"] = text
                    if f"{change['id']}:lines:{number}" in conflicts:
                        conflicts[f"{change['id']}:lines:{number}"]["incoming"] = text
        broken = []
        for change in value["changes"]:
            try:
                validate_links([change], set(snapshot) | kept | MANAGED_TARGETS)
            except ValueError as exc:
                broken.append((change, exc))
        if not broken:
            return
        for change, exc in broken:
            value["changes"].remove(change)
            value["conflicts"] = [
                c for c in value["conflicts"] if not c["id"].startswith(change["id"] + ":")
            ]
            _drop(value, change["path"], exc, failures)


def _patches(change, value, quotes, targets):
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
        line["after"] = _repair(line["after"], change["path"], targets)
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
    patch["quote_ids"] = [q for q in patch["quote_ids"] if quotes[q] is not None]
    if not patch["quote_ids"]:
        raise ValueError("quote_not_in_transcript")


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
            if not valid_date(date):
                raise ValueError("invalid_task_date")


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
    excluded = {
        "/" + quote(c["path"], safe="/")
        for c in changes
        if not c["include"] and c["original"] is None
    }
    for change in changes:
        if change["include"]:
            for line in change["lines"]:
                line["after"] = _unlink(line.get("unfiltered_after", line["after"]), excluded)


def _unlink(text, targets):
    return VAULT_LINK.sub(lambda match: match[1] if match[2] in targets else match[0], text)


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
