"""Bounded indexing of the working copy, with no invented notes or missing-as-empty reads."""

import re

from .markdown import document
from .models import MAX_SCAN_BYTES, refusal
from .syntax import activities

RESERVED = {"SCHEMA.md", "index.md", "log.md", "LEEME.md", "AGENTS.md"}


def build(reader, entries):
    paths = {entry["path"] for entry in entries if entry["kind"] == "note"}
    documents, scanned = {}, 0
    for path in sorted(paths):
        raw = reader.read(path)
        scanned += len(raw.encode())
        if scanned > MAX_SCAN_BYTES:
            raise refusal(
                "vault_scan_bytes",
                f"El índice supera {MAX_SCAN_BYTES} bytes leídos por recorrido.",
                bytes=scanned,
                max_bytes=MAX_SCAN_BYTES,
            )
        documents[path] = document(path, raw, paths)
    notes = [summary(note) for path, note in documents.items() if path not in RESERVED]
    backlinks = {path: 0 for path in paths}
    for note in documents.values():
        for target in {link["path"] for link in note["outlinks"]}:
            backlinks[target] += 1
    for note in notes:
        note["backlink_count"] = backlinks[note["path"]]
    all_tasks = [task for note in documents.values() for task in note["tasks"]]
    schema = documents.get("SCHEMA.md")
    return {
        "notes": notes,
        "tasks": all_tasks,
        "documents": documents,
        "recent": activities(documents.get("log.md", {}).get("body", "")),
        "okf_version": str(
            documents.get("index.md", {}).get("frontmatter", {}).get("okf_version", "")
        ),
        "schema": {
            "path": "SCHEMA.md" if schema else "",
            "body": schema["body"] if schema else "",
            "folders": schema_folders(schema["body"]) if schema else {},
        },
    }


def summary(note):
    fields = note["frontmatter"]
    generated = fields.get("generated")
    generated = generated if isinstance(generated, dict) else {}
    return {
        "path": note["path"],
        "kind": "note",
        "title": str(fields.get("title", note["path"].rsplit("/", 1)[-1][:-3]))[:240],
        "type": str(fields.get("type", "nota"))[:32],
        "description": str(fields.get("description", ""))[:500],
        "state": str(fields.get("estado", ""))[:64],
        "start": str(fields.get("arranque", ""))[:64],
        "links": list(dict.fromkeys(link["path"] for link in note["outlinks"])),
        "backlink_count": 0,
        "task_count": sum(not task["done"] for task in note["tasks"]),
        "modified_at": str(generated.get("at", ""))[:64],
    }


def schema_folders(body):
    return {
        match[2]: match[1] for match in re.finditer(r"`([^`]+/)`\s*·\s*`type:\s*([^`]+)`", body)
    }


def catalog(index, request):
    start = (request.page - 1) * request.page_size
    notes = index["notes"]
    return {
        "notes": notes[start : start + request.page_size],
        "next_page": request.page + 1 if start + request.page_size < len(notes) else None,
        "notes_count": sum(note["type"] != "fuente" for note in notes),
        "sources_count": sum(note["type"] == "fuente" for note in notes),
        "open_tasks": sum(not task["done"] for task in index["tasks"]),
        "type_counts": {
            kind: sum(n["type"] == kind for n in notes) for kind in {n["type"] for n in notes}
        },
        "okf_version": index["okf_version"],
        "schema": index["schema"],
        "recent": index["recent"][:4],
    }


def task_page(index, request):
    tasks = [
        task
        for task in index["tasks"]
        if (
            not request.query
            or request.query.casefold() in (task["who"] + " " + task["text"]).casefold()
        )
        and (not request.path or request.path in {task["file"], task["who_path"]})
    ]
    predicates = {
        "all": lambda t: True,
        "open": lambda t: not t["done"],
        "done": lambda t: t["done"],
        "overdue": lambda t: bool(
            request.today and t["due"] and t["due"] < request.today and not t["done"]
        ),
        "mine": lambda t: bool(request.owner and t["who"] == request.owner and not t["done"]),
    }
    counts = {kind: sum(check(task) for task in tasks) for kind, check in predicates.items()}
    tasks = [task for task in tasks if predicates[request.state](task)]
    start = (request.page - 1) * request.page_size
    return {
        "tasks": tasks[start : start + request.page_size],
        "total": len(tasks),
        "counts": counts,
        "next_page": request.page + 1 if start + request.page_size < len(tasks) else None,
    }
