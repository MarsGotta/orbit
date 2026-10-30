"""Surgical YAML field and text edits: unrelated bytes never pass through a dumper."""

import re

import yaml

from ..vault.models import MAX_NOTE_BYTES, refusal
from ..vault.schema import frontmatter


def metadata(raw, updates):
    frontmatter(raw)  # Reject duplicate keys, anchors and unsafe input before composing.
    ending = "\r\n" if "\r\n" in raw else "\n"
    if not raw.startswith(("---\n", "---\r\n")):
        prefix = "---" + ending
        return prefix + _dump(updates, ending) + prefix + raw
    match = re.search(r"^---[ \t]*\r?$", raw[3:], re.MULTILINE)
    if match is None:
        raise ValueError("unclosed_frontmatter")
    closing = match.start() + 3
    start = raw.index("\n") + 1
    text = raw[start:closing]
    node = yaml.compose(text, Loader=yaml.BaseLoader)
    spans = {}
    if node:
        for key, value in node.value:
            begin = key.start_mark.index
            # Block values may end at the next key. Scalar/flow values end
            # inside their last line: keep independent comments and blank lines.
            end = value.end_mark.index
            if end and text[end - 1] != "\n":
                newline = text.find("\n", end)
                end = len(text) if newline < 0 else newline + 1
            spans[key.value] = (begin, end)
    replacements = []
    for key, value in updates.items():
        block = _dump({key: value}, ending)
        if key in spans:
            begin, finish = spans[key]
            replacements.append((begin, finish, block))
        else:
            replacements.append((len(text), len(text), block))
    for begin, finish, block in sorted(replacements, reverse=True):
        text = text[:begin] + block + text[finish:]
    return raw[:start] + text + raw[closing:]


def _dump(values, ending):
    return yaml.safe_dump(values, allow_unicode=True, sort_keys=False, width=10000).replace(
        "\n", ending
    )


def render(change, conflicts, *, require_decisions=False, citation_prefix=None):
    raw = change["original"] or ""
    _fields, body = frontmatter(raw)
    updates = {}
    discussion = []
    for index, prop in enumerate(change["properties"]):
        conflict = conflicts.get(f"{change['id']}:property:{index}")
        choice = _choice(conflict, require_decisions)
        if choice == "incoming":
            updates[prop["key"]] = prop["after"]
        elif choice == "both":
            discussion.append(f"{prop['key']}: {prop['before']} / {prop['after']} — in discussion")
    for index, lines in enumerate(change["lines"]):
        conflict = conflicts.get(f"{change['id']}:lines:{index}")
        choice = _choice(conflict, require_decisions)
        if choice == "current":
            continue
        before, after = lines["before"], lines["after"]
        if citation_prefix:
            after = cite(after, lines["quote_ids"], citation_prefix)
        if choice == "both":
            after = before + "\n\n" + after + "\n\n<!-- in discussion -->"
        if before:
            if body.count(before) != 1:
                raise ValueError("patch_context_not_unique")
            body = body.replace(before, after, 1)
        else:
            body += ("\n" if body and not body.endswith("\n") else "") + after
    if discussion:
        body += "\n\n## In discussion\n" + "\n".join("- " + line for line in discussion) + "\n"
    if change["original"] is None:
        return metadata("", updates) + body
    # Retain every leading newline in the original body (frontmatter() strips them).
    prefix = raw[: len(raw) - len(frontmatter(raw)[1])]
    rendered = metadata(prefix + body, updates)
    if len(rendered.encode()) > MAX_NOTE_BYTES:
        raise refusal(
            "vault_note_too_large",
            f"The proposed note exceeds {MAX_NOTE_BYTES} bytes.",
            413,
            bytes=len(rendered.encode()),
            max_bytes=MAX_NOTE_BYTES,
        )
    return rendered


def _choice(conflict, required):
    if not conflict:
        return "incoming"
    if not conflict.get("choice") and required:
        raise refusal("distillation_undecided", "Decide every contradiction before applying.", 409)
    return conflict.get("choice") or "incoming"


def cite(text, quote_ids, prefix):
    marker = "".join(f"[^{prefix}-{q}]" for q in quote_ids)
    output, fenced = [], False
    for line in text.splitlines(keepends=True):
        if line.startswith(("```", "~~~")):
            fenced = not fenced
        if line.strip() and not fenced and not line.startswith(("#", "```", "~~~", "[^")):
            ending = "\r\n" if line.endswith("\r\n") else "\n" if line.endswith("\n") else ""
            line = line.removesuffix(ending) + marker + ending if ending else line + marker
        output.append(line)
    return "".join(output)
