"""The assistant's folders on the Mac, read through the device bridge, in English.

Every answer names whole paths, so the assistant can ask for the next one
with what it was given, and says where to go on when there is more. A refusal
names the path asked for, where it led and every folder: the facts the
assistant needs to correct its request.
"""

from __future__ import annotations

from orbit_hub.meetings_bridge.page import MAX_ANSWER_CHARS

REFUSALS = frozenset(
    {"no_folders", "outside_folders", "folder_unavailable", "file_unavailable", "file_too_large"}
)

KINDS = {"text": "text", "pdf": "PDF", "image": "images"}

STOPS = {
    "results": "There are more than {limit} results; refine the search.",
    "visited": "The search stopped after traversing {limit} entries; search a narrower folder.",
    "files_read": "The search stopped after reading {limit} files; search a narrower folder.",
    "bytes_read": "The search stopped after reading {limit} bytes; search a narrower folder.",
    "deadline": "The search stopped at {limit} ms; search a narrower folder.",
    "page_bytes": "The results filled the response ({limit} bytes); refine the search.",
}


def _joined(directory: str, name: str) -> str:
    return f"{directory.rstrip('/')}/{name}"


def folder_cause(cause) -> str:
    if not isinstance(cause, dict):
        return "unknown cause"
    kind = cause.get("kind")
    if kind == "bookmark_unresolvable":
        return (
            "macOS cannot find the folder: it was deleted or its disk is disconnected "
            f"({cause.get('domain')} {cause.get('code')})"
        )
    if kind == "not_a_folder":
        return "what is there is no longer a folder"
    if kind == "access_denied":
        return f"macOS refuses to open it (errno {cause.get('errno')})"
    if kind in {"path_too_long", "bookmark_too_large"}:
        what = "its path" if kind == "path_too_long" else "its bookmark"
        return f"{what} takes {cause.get('bytes')} bytes; Orbit allows {cause.get('max_bytes')}"
    return f"cause {kind}"


def file_cause(cause) -> str:
    if not isinstance(cause, dict):
        return "unknown cause"
    kind = cause.get("kind")
    sentences = {
        "not_found": "it does not exist",
        "not_a_file": "it is not a file",
        "not_a_folder": "it is not a folder",
        "not_text": "it is not UTF-8 text (it is binary or uses another encoding)",
        "pdf_unreadable": "PDFKit cannot open the PDF (it is damaged or password-protected)",
        "image_unreadable": "ImageIO cannot open the image",
        "changed": "it changed during the read (a link appeared in its path); nothing was read, request it again",
    }
    if kind in sentences:
        return sentences[kind]
    if kind == "image_does_not_fit":
        return (
            f"even at its smallest size it does not fit in a screenshot: {cause.get('smallest_png_bytes')} bytes "
            f"of PNG versus {cause.get('max_png_bytes')}"
        )
    if kind == "read_failed":
        return f"macOS refused to read it (errno {cause.get('errno')})"
    if kind == "past_end":
        return f"after_index={cause.get('after_index')} is past the end ({cause.get('size_bytes')} bytes)"
    if kind == "not_character_boundary":
        return f"after_index={cause.get('after_index')} falls inside a character; use the value from the previous response"
    if kind == "past_last_page":
        return f"after_page={cause.get('after_page')} is past the last page (page count: {cause.get('page_count')})"
    return f"cause {kind}"


def refusal_message(code: str, refusal) -> str:
    """The sentence for a folder refusal, with its path, folders and numbers."""
    refusal = refusal if isinstance(refusal, dict) else {}
    path = refusal.get("path")
    if code == "no_folders":
        return (
            "The Orbit user has not given the assistant any folders on this Mac. They can add one "
            "in Orbit, in Settings › Assistant folders › “Add folder”."
        )
    if code == "outside_folders":
        folders = refusal.get("folders")
        listed = ", ".join(folders) if isinstance(folders, list) and folders else "none"
        resolved = refusal.get("resolved")
        leads = f", which leads to {resolved}," if isinstance(resolved, str) else ""
        return (
            f"{path}{leads} is outside the folders the assistant can read. "
            f"The folders are: {listed}. Request a path inside them."
        )
    if code == "folder_unavailable":
        return (
            f"The folder {path} cannot be opened: {folder_cause(refusal.get('cause'))}. "
            "The Orbit user can remove and add it again in Settings."
        )
    if code == "file_unavailable":
        return f"{path}: {file_cause(refusal.get('cause'))}."
    kind = KINDS.get(refusal.get("content"), "files")
    return (
        f"{path} is {refusal.get('bytes')} bytes and Orbit reads {kind} up to "
        f"{refusal.get('max_bytes')} bytes, so it did not open it."
    )


def _size(entry: dict) -> str:
    size = entry.get("size_bytes")
    return f" ({size} bytes)" if type(size) is int else ""


def render_list(result: dict) -> str:
    directory = result.get("path")
    entries = result["entries"]
    if directory is None:
        lines = []
        for entry in entries:
            unavailable = entry.get("unavailable")
            note = (
                f" — cannot be read now: {folder_cause(unavailable)}"
                if isinstance(unavailable, dict)
                else ""
            )
            lines.append(f"- {entry['name']}/{note}")
        if not lines:
            return "The Orbit user has not given the assistant any folders."
        return "Folders the assistant can read on this Mac:\n" + "\n".join(lines)
    start = result["after_index"]
    total = result["total"]
    lines = []
    shown = 0
    budget = MAX_ANSWER_CHARS - 400
    for entry in entries:
        full = _joined(directory, entry["name"])
        if entry["kind"] == "folder":
            line = f"- {full}/"
        elif entry["kind"] == "link":
            line = f"- {full} (link; read it to learn its destination)"
        elif entry["kind"] == "other":
            line = f"- {full} (neither file nor folder)"
        else:
            line = f"- {full}{_size(entry)}"
        if lines and budget - len(line) - 1 < 0:
            break
        budget -= len(line) + 1
        lines.append(line)
        shown += 1
    following = result.get("next_after_index")
    if shown < len(entries):
        following = start + shown
    if total == 0:
        return f"{directory} is empty."
    if not entries and start >= total:
        return (
            f"{directory} has {total} {'entry' if total == 1 else 'entries'}: "
            f"after_index={start} is past the last entry; use a value smaller than {total}."
        )
    header = f"Contents of {directory} (entries {start + 1}–{start + shown} of {total}):"
    text = header + "\n" + "\n".join(lines)
    if type(following) is int:
        text += f"\nThere is more: continue with after_index={following}."
    return text


def render_search(result: dict) -> str:
    where = result.get("path") or "all folders"
    how = "names and text" if result["content"] else "names"
    hits = result["results"]
    lines = []
    for hit in hits:
        place = hit["path"] + (f":{hit['line']}" if type(hit.get("line")) is int else "")
        if isinstance(hit.get("snippet"), str):
            lines.append(f"- {place} — {hit['snippet']}")
        elif hit["kind"] == "folder":
            lines.append(f"- {place}/")
        else:
            lines.append(f"- {place}{_size(hit)}")
    header = (
        f"Search for «{result['query']}» in {where} ({how}): {len(hits)} "
        f"{'result' if len(hits) == 1 else 'results'}; "
        f"{result['visited']} entries traversed"
    )
    if result["content"]:
        header += f", {result['files_read']} files and {result['bytes_read']} bytes read"
    text = header + ".\n" + ("\n".join(lines) if lines else "No matches.")
    if result["partial_files"]:
        text += (
            f"\n{result['partial_files']} files were read only in their first part; a "
            "match further inside will not appear."
        )
    stopped = result.get("stopped")
    if isinstance(stopped, dict) and stopped.get("cause") in STOPS:
        text += "\n" + STOPS[stopped["cause"]].format(limit=stopped.get("limit"))
    return text


def render_read(result: dict) -> str:
    path = result["path"]
    size = result["size_bytes"]
    if result["content"] == "text":
        start = result["after_index"]
        body = result["text"]
        end = start + len(body.encode("utf-8"))
        header = f"{path} · bytes {start}–{end} of {size}:\n"
        following = result.get("next_after_index")
        footer = f"\nContinue with after_index={following}." if type(following) is int else ""
        return header + body + footer
    if result["content"] == "pdf":
        count = result["page_count"]
        parts = [f"{path} · PDF with {count} pages ({size} bytes):"]
        for page in result["pages"]:
            if page["text"].strip():
                parts.append(f"[Page {page['page']} of {count}]\n{page['text']}")
            else:
                parts.append(f"[Page {page['page']} of {count}: no text]")
        following = result.get("next_after_page")
        if type(following) is int:
            parts.append(
                f"Continue with after_page={following} and after_index={result.get('next_after_index') or 0}."
            )
        return "\n".join(parts)
    return (
        f"{path}: image of {result['source_width']} × {result['source_height']} pixels, "
        f"sent at {result['width']} × {result['height']} ({size} bytes on the Mac)."
    )
