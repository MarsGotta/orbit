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


# Only Tasks metadata is removed from the description; links stay in their original order.
MARKER = re.compile(r"📅|⏳|🛫|✅|⏫|🔼|🔽|🔁|🆔|⛔")
TASK_LINK = re.compile(r"\[\[([^\]\n]+)\]\]|\[([^\]\n]+)\]\(<?([^\s)>]+)>?(?:\s+\"[^\"]*\")?\)")


def tasks(path, fields, body, line_offset=0, paths=(), people=(), role="note"):
    result, fenced = [], False
    sources = {c["id"]: c for c in citations(fields, body)}
    for number, line in enumerate(body.splitlines(), 1):
        if line.startswith(("```", "~~~")):
            fenced = not fenced
        match = re.match(r"^\s*- \[([ xX])\]\s+(.+)$", line)
        if fenced or not match:
            continue
        raw = match[2]
        who, who_path = _assignee(path, raw, paths, people)
        block = re.search(r"\s\^([\w-]+)\s*$", raw)
        citation = re.search(r"\[\^([^\]]+)\]", raw)
        source_id = citation[1] if citation else ""
        text, metadata = _metadata(raw)
        result.append(
            {
                "file": path,
                "line": number + line_offset,
                "block_id": block[1] if block else "",
                "text": text,
                "who": who,
                "who_path": who_path,
                "done": match[1].lower() == "x",
                "project": path if role == "project" else "",
                "source_id": source_id,
                "source": sources.get(source_id, {}).get("resource", ""),
                "tags": re.findall(r"(?<![\w/])#[\w/-]+", raw),
                **metadata,
            }
        )
    return result


def _metadata(raw):
    values = {
        "due": "",
        "scheduled": "",
        "start": "",
        "done_at": "",
        "priority": "",
        "recurrence": "",
        "id": "",
        "depends_on": [],
    }
    dates = {"📅": "due", "⏳": "scheduled", "🛫": "start", "✅": "done_at"}
    clean = re.sub(r"\[\^[^\]]+\]", " ", raw)
    clean = re.sub(r"\s+\^[\w-]+\s*$", "", clean)
    spans = []
    markers = list(MARKER.finditer(clean))
    for index, marker in enumerate(markers):
        start = marker.end()
        end = markers[index + 1].start() if index + 1 < len(markers) else len(clean)
        tail, symbol = clean[start:end], marker[0]
        used = 0
        if symbol in dates:
            match = re.match(r"\s*(\d{4}-\d{2}-\d{2})", tail)
            if match and valid_date(match[1]):
                values[dates[symbol]], used = match[1], match.end()
        elif symbol in {"⏫", "🔼", "🔽"}:
            values["priority"] = symbol
        elif symbol == "🔁":
            values["recurrence"], used = tail.strip(), len(tail)
        else:
            match = re.match(r"\s*([A-Za-z0-9_-]+(?:\s*,\s*[A-Za-z0-9_-]+)*)", tail)
            if match:
                value, used = match[1], match.end()
                if symbol == "🆔":
                    values["id"] = value
                else:
                    values["depends_on"] = [v.strip() for v in value.split(",")]
        spans.append((marker.start(), start + used))
    for start, end in reversed(spans):
        clean = clean[:start] + clean[end:]
    clean = TASK_LINK.sub(_link_label, clean)
    return re.sub(r"\s+", " ", clean).strip(), values


def _link_label(match):
    if match[1] is not None:
        target, _, label = match[1].partition("|")
        return label or target
    return match[2]


def valid_date(value):
    try:
        date.fromisoformat(value)
        return True
    except ValueError:
        return False


def _assignee(path, raw, paths, people):
    for link in TASK_LINK.finditer(prose(raw)):
        if link[1] is not None:
            target, _, label = link[1].partition("|")
            resolved = resolve(path, target, paths, wiki=True)
            label = label or target
        else:
            label, resolved = link[2], resolve(path, link[3], paths)
        if resolved in people:
            return label, resolved
    return "", ""


def activities(body):
    result, pending, day = [], [], ""

    def flush():
        if pending:
            result.append(_activity(day, text="\n".join(pending)))
            pending.clear()

    for line in body.splitlines():
        if re.fullmatch(r"## \d{4}-\d{2}-\d{2}", line) and valid_date(line[3:]):
            flush()
            day = line[3:]
        elif line.startswith("# ") and not day:
            continue
        elif not line.strip():
            flush()
        elif match := re.match(r"^- (\d{2}:\d{2}) · ([^·]+) · (.+)$", line):
            flush()
            pieces = match[3].split(" · ", 2)
            link = LINK.search(pieces[0])
            result.append(
                _activity(
                    day,
                    time=match[1],
                    action=match[2].strip(),
                    title=link[1] if link else pieces[0],
                    path=link[2].lstrip("/") if link else "",
                    by=pieces[1] if len(pieces) > 1 else "",
                    detail=pieces[2] if len(pieces) > 2 else "",
                )
            )
        else:
            if re.match(r"^[-*+] ", line):
                flush()
            pending.append(line)
    flush()
    return result


def _activity(day, **values):
    return {
        "date": day,
        "time": "",
        "action": "",
        "title": "",
        "path": "",
        "by": "",
        "detail": "",
        "text": "",
        **values,
    }
