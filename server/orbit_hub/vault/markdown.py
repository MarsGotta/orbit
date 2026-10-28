"""Safe, bounded YAML frontmatter and internal Markdown link extraction."""

import re

from .conformance import check
from .diagnostics import SchemaProblem
from .method import file_name
from .models import MAX_NOTE_BYTES, refusal
from .paths import resolve
from .schema import frontmatter, load, properties
from .syntax import citations, tasks


def document(path, text, paths, schema=None, people=()):
    size = len(text.encode("utf-8"))
    if size > MAX_NOTE_BYTES:
        raise refusal(
            "vault_note_too_large",
            f"The note is {size} bytes; the maximum is {MAX_NOTE_BYTES}.",
            413,
            bytes=size,
            max_bytes=MAX_NOTE_BYTES,
        )
    schema = load() if schema is None else schema
    try:
        fields, body = frontmatter(text)
    except (ValueError, TypeError) as exc:
        if path != "SCHEMA.md":
            problem = (
                exc
                if isinstance(exc, SchemaProblem)
                else SchemaProblem(
                    "schema_frontmatter_mapping", "frontmatter must be a field mapping"
                )
            )
            line = f" (line {problem.line})" if problem.line else ""
            raise refusal(
                "vault_frontmatter_invalid",
                f"The note YAML frontmatter: {problem}{line}.",
                message_code=problem.code,
                message_fields={"field": problem.field},
                **({"line": problem.line} if problem.line else {}),
            ) from None
        fields, body = {}, schema["body"] or text
    kind, role, values = properties(path, fields, schema)
    return_value = {
        "type": kind,
        "role": role,
        "properties": values,
        "schema_warnings": schema["warnings"],
        "schema_diagnostics": schema["diagnostics"],
        "conformance": check(
            fields, path=path, has_frontmatter=text.startswith(("---\n", "---\r\n"))
        ),
    }
    return {
        **return_value,
        "path": path,
        # A later writer must round-trip this complete map, including unknown
        # keys nested in provenance events and sources. Properties are only a
        # normalized reading projection; never write them over the original.
        "frontmatter": fields,
        "body": body,
        "outlinks": links(path, body, paths),
        "synthesized": False,
        "raw": text,
        "citations": citations(values, body),
        "tasks": tasks(
            path, values, body, text[: len(text) - len(body)].count("\n"), paths, people, role
        ),
        "dependencies": [],
        "dependency_tasks": [],
        "queries": [],
    }


def links(path, body, paths):
    # Code samples are not links. Links in normal prose and embeds are reads.
    prose = re.sub(r"```.*?```|~~~.*?~~~|`[^`\n]*`", "", body, flags=re.DOTALL)
    found, basenames, seen = [], {}, set()
    for existing in paths:
        basenames.setdefault(file_name(existing.rsplit("/", 1)[-1][:-3]), []).append(existing)
    for match in re.finditer(
        r"\[\[([^\]\n]+)\]\]|\[[^\]\n]*\]\(<?([^\s)>]+)>?(?:\s+\"[^\"]*\")?\)", prose
    ):
        raw = match.group(1) or match.group(2)
        target = raw.split("|", 1)[0]
        if target.startswith("#"):
            continue
        resolved = resolve(
            path, target, paths, wiki=match.group(1) is not None, basenames=basenames
        )
        entry = {"target": target, "path": resolved}
        if resolved is not None and (target, resolved) not in seen:
            found.append(entry)
            seen.add((target, resolved))
    return found
