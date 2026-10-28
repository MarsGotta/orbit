"""Bounded indexing of the working copy, with no invented notes or missing-as-empty reads."""

from .conformance import reserved
from .markdown import document
from .models import MAX_SCAN_BYTES, refusal
from .schema import load
from .syntax import activities, tasks
from .task_views import dependencies, queries, task_index

# These control concepts stay in the navigation footer, but still need an OKF type.
CONTROL_DOCUMENTS = {"SCHEMA.md", "README.md", "AGENTS.md"}


def build(reader, entries):
    paths = {entry["path"] for entry in entries if entry["kind"] == "note"}
    raws, scanned = {}, 0
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
        raws[path] = raw
    schema = load(raws.get("SCHEMA.md", ""))
    if "SCHEMA.md" in raws:
        schema["path"] = "SCHEMA.md"
    documents = {path: document(path, raw, paths, schema) for path, raw in raws.items()}
    people = {path for path, note in documents.items() if note["role"] == "person"}
    for path, note in documents.items():
        offset = note["raw"][: len(note["raw"]) - len(note["body"])].count("\n")
        note["tasks"] = tasks(
            path, note["properties"], note["body"], offset, paths, people, note["role"]
        )
    all_tasks = [task for note in documents.values() for task in note["tasks"]]
    ids, by_reference = task_index(all_tasks)
    for note in documents.values():
        note["dependencies"] = dependencies(note, ids)
        refs = {edge[key] for edge in note["dependencies"] for key in ("prerequisite", "dependent")}
        note["dependency_tasks"] = [
            by_reference[ref] for ref in sorted(refs) if ref in by_reference
        ]
        note["queries"] = queries(note["body"], all_tasks)
    notes = [
        summary(note)
        for path, note in documents.items()
        if not reserved(path) and path not in CONTROL_DOCUMENTS
    ]
    backlinks = {path: 0 for path in paths}
    for note in documents.values():
        for target in {link["path"] for link in note["outlinks"]}:
            backlinks[target] += 1
    for note in notes:
        note["backlink_count"] = backlinks[note["path"]]
    return {
        "notes": notes,
        "tasks": all_tasks,
        "documents": documents,
        "recent": activities(documents.get("log.md", {}).get("body", "")),
        "okf_version": str(
            documents.get("index.md", {}).get("frontmatter", {}).get("okf_version", "")
        ),
        "schema": schema,
    }


def summary(note):
    fields = note["properties"]
    generated = note["frontmatter"].get("generated")
    generated = generated if isinstance(generated, dict) else {}
    return {
        "path": note["path"],
        "kind": "note",
        "title": str(note["frontmatter"].get("title", note["path"].rsplit("/", 1)[-1][:-3]))[:240],
        "type": note["type"],
        "role": note["role"],
        "description": str(fields.get("description", ""))[:500],
        "state": str(fields.get("stage", ""))[:64],
        "start": str(fields.get("start", ""))[:64],
        "links": list(dict.fromkeys(link["path"] for link in note["outlinks"])),
        "backlink_count": 0,
        "task_count": sum(not task["done"] for task in note["tasks"]),
        "modified_at": str(generated.get("at", ""))[:64],
    }


def catalog(index, request):
    start = (request.page - 1) * request.page_size
    notes = index["notes"]
    return {
        "notes": notes[start : start + request.page_size],
        "next_page": request.page + 1 if start + request.page_size < len(notes) else None,
        "notes_count": sum(note["role"] != "source" for note in notes),
        "sources_count": sum(note["role"] == "source" for note in notes),
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
