"""Vault-wide Tasks dependencies and the supported read-only query subset."""

import re

from .models import MAX_PAGE_SIZE
from .syntax import valid_date


def reference(task):
    return task["file"] + "#" + (task["id"] or task["block_id"] or f"line-{task['line']}")


def task_index(all_tasks):
    ids, by_reference = {}, {}
    for task in all_tasks:
        by_reference[reference(task)] = task
        if task["id"]:
            ids.setdefault(task["id"], []).append(task)
    return ids, by_reference


def dependencies(note, ids):
    edges = []
    for task in note["tasks"]:
        for identifier in task["depends_on"]:
            matches = ids.get(identifier, [])
            # Missing or duplicate Tasks ids are unresolved references, never a
            # silently selected prerequisite from an unrelated note.
            prerequisite = reference(matches[0]) if len(matches) == 1 else "task:" + identifier
            edges.append(
                {"prerequisite": prerequisite, "dependent": reference(task), "relation": "requires"}
            )
        if (
            task["depends_on"]
            and task["project"]
            and not any(task[field] for field in ("due", "scheduled", "start", "done_at"))
        ):
            edges.append(
                {
                    "prerequisite": note["path"] + "#milestone-start",
                    "dependent": reference(task),
                    "relation": "after",
                }
            )
    return edges


def queries(body, all_tasks):
    result = []
    for block in re.finditer(r"(?m)^```tasks\s*\n(.*?)^```\s*$", body, re.DOTALL):
        lines, warnings, diagnostics, checks = [], [], [], []
        for raw in block[1].splitlines():
            line = raw.strip()
            if not line:
                continue
            lines.append(line)
            check = predicate(line)
            if check is None:
                warnings.append(f"This query line is not applied: {line}")
                diagnostics.append({"code": "query_line_unsupported", "field": line, "numbers": {}})
            else:
                checks.append(check)
        matches = [task for task in all_tasks if all(check(task) for check in checks)]
        if len(matches) > MAX_PAGE_SIZE:
            warnings.append(
                f"The query shows {MAX_PAGE_SIZE} of {len(matches)} tasks; narrow the filters to see the rest."
            )
        if len(matches) > MAX_PAGE_SIZE:
            diagnostics.append(
                {
                    "code": "query_page_limit",
                    "field": "",
                    "numbers": {"count": len(matches), "max": MAX_PAGE_SIZE},
                }
            )
        result.append(
            {
                "kind": "tasks",
                "lines": lines,
                "warnings": warnings,
                "diagnostics": diagnostics,
                "tasks": matches[:MAX_PAGE_SIZE],
                "total": len(matches),
            }
        )
    return result


def predicate(line):
    if line in {"done", "not done"}:
        return lambda task: task["done"] == (line == "done")
    match = re.fullmatch(r"due (before|after|on) (\d{4}-\d{2}-\d{2})", line)
    if match and valid_date(match[2]):
        operation, value = match.groups()
        return lambda task: (
            bool(task["due"])
            and {
                "before": task["due"] < value,
                "after": task["due"] > value,
                "on": task["due"] == value,
            }[operation]
        )
    match = re.fullmatch(r"(path includes|description includes|tags include) (.+)", line)
    if match:
        field, value = match[1], match[2].casefold()
        if field == "tags include":
            tag = value.removeprefix("#")
            return lambda task: tag in [t.removeprefix("#").casefold() for t in task["tags"]]
        key = "file" if field == "path includes" else "text"
        return lambda task: value in task[key].casefold()
    return None
