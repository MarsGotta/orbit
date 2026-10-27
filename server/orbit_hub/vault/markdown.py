"""Safe, bounded YAML frontmatter and internal Markdown link extraction."""

import re

import yaml
from yaml.tokens import AliasToken, AnchorToken, TagToken

from .method import file_name
from .models import MAX_NOTE_BYTES, refusal
from .paths import resolve
from .syntax import citations, dependencies, query_blocks, tasks


def document(path, text, paths):
    size = len(text.encode("utf-8"))
    if size > MAX_NOTE_BYTES:
        raise refusal(
            "vault_note_too_large",
            f"La nota ocupa {size} bytes; el máximo es {MAX_NOTE_BYTES}.",
            413,
            bytes=size,
            max_bytes=MAX_NOTE_BYTES,
        )
    fields, body = {}, text
    if text.startswith(("---\n", "---\r\n")):
        parts = re.split(r"^---\s*$", text, maxsplit=2, flags=re.MULTILINE)
        if len(parts) != 3:
            raise refusal(
                "vault_frontmatter_invalid", "El frontmatter YAML de la nota no está cerrado."
            )
        try:
            # Aliases/anchors/tags are unnecessary for OKF and can amplify or make
            # recursive data. BaseLoader keeps dates as strings; no object constructors.
            tokens = list(yaml.scan(parts[1]))
            if any(isinstance(t, (AliasToken, AnchorToken, TagToken)) for t in tokens):
                raise ValueError()
            fields = yaml.load(parts[1], Loader=yaml.BaseLoader) or {}
            if not isinstance(fields, dict):
                raise TypeError()
        except (yaml.YAMLError, ValueError, TypeError, RecursionError):
            raise refusal(
                "vault_frontmatter_invalid", "El frontmatter YAML no es un mapa seguro de campos."
            ) from None
        body = parts[2].lstrip("\r\n")
    return {
        "path": path,
        "frontmatter": fields,
        "body": body,
        "outlinks": links(path, body, paths),
        "synthesized": False,
        "raw": text,
        "citations": citations(fields, body),
        "tasks": tasks(path, fields, body, text[: len(text) - len(body)].count("\n"), paths),
        "dependencies": dependencies(path, body, paths),
        "queries": query_blocks(body),
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
