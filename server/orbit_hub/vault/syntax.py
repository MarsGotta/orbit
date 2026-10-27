"""The design's Markdown records: citations, tasks, query blocks and dependencies."""

import re
from datetime import date

from .paths import resolve

LINK = re.compile(r"\[([^\]\n]+)\]\(([^)\s]+)\)")


def citations(fields, body):
    sources = fields.get("sources", [])
    sources = sources if isinstance(sources, list) else []
    by_id = {s["id"]: s for s in sources if isinstance(s, dict) and isinstance(s.get("id"), str)}
    ids = list(dict.fromkeys(re.findall(r"\[\^([^\]\n]+)\](?!:)", prose(body))))
    ids.extend(key for key in by_id if key not in ids)
    return [
        {
            "id": key,
            "number": number + 1,
            "resource": str(by_id.get(key, {}).get("resource", "")),
            "title": str(by_id.get(key, {}).get("title", key)),
        }
        for number, key in enumerate(ids)
    ]


def prose(body):
    return re.sub(r"(?ms)^(`{3,}|~{3,})[^\n]*\n.*?^\1\s*$|`[^`\n]*`", "", body)


def query_blocks(body):
    blocks = []
    for match in re.finditer(r"(?m)^```(tareas|dependencias)\s*\n(.*?)^```\s*$", body, re.DOTALL):
        filters = {}
        for line in match[2].splitlines():
            key, separator, value = line.partition(":")
            if separator and key.strip() in {"persona", "proyecto", "estado"}:
                filters[key.strip()] = value.strip()
        blocks.append({"kind": match[1], "filters": filters})
    return blocks


def tasks(path, fields, body, line_offset=0, paths=()):
    result, fenced = [], False
    sources = {c["id"]: c for c in citations(fields, body)}
    for number, line in enumerate(body.splitlines(), 1):
        if line.startswith(("```", "~~~")):
            fenced = not fenced
        match = re.match(r"^\s*- \[([ xX])\]\s+(.+)$", line)
        if fenced or not match:
            continue
        raw = match[2]
        who, who_path = _assignee(path, raw, paths)
        block = re.search(r"\s\^([\w-]+)\s*$", raw)
        when = re.search(r"(?:para el|hecha el) (\d{4}-\d{2}-\d{2})", raw)
        citation = re.search(r"\[\^([^\]]+)\]", raw)
        source_id = citation[1] if citation else ""
        text = re.sub(r"\s*(?:\[\^[^\]]+\]|\^[\w-]+)\s*", "", raw.split(" · ", 1)[0]).strip()
        done = match[1].lower() == "x"
        value = when[1] if when and _date(when[1]) else ""
        result.append(
            {
                "file": path,
                "line": number + line_offset,
                "block_id": block[1] if block else "",
                "text": text,
                "who": who,
                "who_path": who_path,
                "due": "" if done else value,
                "done": done,
                "done_at": value if done else "",
                "project": path if fields.get("type") == "proyecto" else "",
                "source_id": source_id,
                "source": sources.get(source_id, {}).get("resource", ""),
            }
        )
    return result


def _date(value):
    try:
        date.fromisoformat(value)
        return True
    except ValueError:
        return False


def _assignee(path, raw, paths):
    if " · " not in raw:
        return "", ""
    tail = raw.split(" · ", 1)[1]
    link = LINK.search(tail)
    if link:
        return link[1], resolve(path, link[2], paths) or ""
    wiki = re.search(r"\[\[([^\]\n]+)\]\]", tail)
    if wiki:
        target, _, label = wiki[1].partition("|")
        return label or target, resolve(path, target, paths, wiki=True) or ""
    return "", ""


def _reference(path, target, paths):
    resolved = resolve(path, target, paths)
    if resolved is None:
        return None
    if "#^" in target:
        anchor = target.split("#^", 1)[1]
        return resolved + "#" + anchor if re.fullmatch(r"[\w-]+", anchor) else None
    return resolved


def dependencies(path, body, paths=()):
    section = re.search(r"(?ms)^## Dependencias\s*\n(.*?)(?=^## |\Z)", body)
    if section is None:
        return []
    result = []
    for line in section[1].splitlines():
        refs = [ref for match in LINK.finditer(line) if (ref := _reference(path, match[2], paths))]
        milestone = path + "#milestone-arranque"
        if "va después del arranque" in line:
            result.extend(
                {"prerequisite": milestone, "dependent": ref, "relation": "after"} for ref in refs
            )
        elif "depende de" in line or "dependía de" in line:
            before = re.split(r"depend(?:e|ía) de", line, maxsplit=1)[0]
            # An initial task link names the dependent; otherwise it is the
            # project's named milestone, whose date is the arranque property.
            initial = LINK.search(before)
            dependent = refs.pop(0) if initial is not None and refs else milestone
            result.extend(
                {"prerequisite": ref, "dependent": dependent, "relation": "requires"}
                for ref in refs
            )
    return result


def activities(body):
    result, day = [], ""
    for line in body.splitlines():
        if re.match(r"^## \d{4}-\d{2}-\d{2}$", line):
            day = line[3:]
        match = re.match(r"^- (\d{2}:\d{2}) · ([^·]+) · (.+)$", line)
        if match is None:
            continue
        pieces = match[3].split(" · ", 2)
        link = LINK.search(pieces[0])
        result.append(
            {
                "date": day,
                "time": match[1],
                "action": match[2].strip(),
                "title": link[1] if link else pieces[0],
                "path": link[2].lstrip("/") if link else "",
                "by": pieces[1] if len(pieces) > 1 else "",
                "detail": pieces[2] if len(pieces) > 2 else "",
            }
        )
    return result
