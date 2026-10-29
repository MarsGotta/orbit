"""Advisory OKF 0.2 checks. Reading never edits or refuses metadata values."""

import re
from datetime import datetime

OKF_FIELDS = (
    "type",
    "title",
    "description",
    "resource",
    "tags",
    "sources",
    "generated",
    "verified",
    "status",
    "stale_after",
)


def reserved(path):
    return path.rsplit("/", 1)[-1] in {"index.md", "log.md"}


def issue(field, code, message):
    return {"field": field, "code": code, "message": f"{field}: {message}."}


def check(fields, *, path="", has_frontmatter=None):
    issues = []
    if reserved(path):
        present = bool(fields) if has_frontmatter is None else has_frontmatter
        if present and path != "index.md":
            issues.append(
                issue(
                    "frontmatter", "reserved_frontmatter", "this file must not contain frontmatter"
                )
            )
        return {"conformant": not issues, "issues": issues}
    kind = fields.get("type")
    if not isinstance(kind, str) or not kind.strip():
        issues.append(issue("type", "invalid_type", "a non-empty concept type is missing"))
    for name in ("title", "description", "resource"):
        if name in fields and not isinstance(fields[name], str):
            issues.append(issue(name, f"invalid_{name}", "must be text"))
    if "tags" in fields and (
        not isinstance(fields["tags"], list)
        or any(not isinstance(tag, str) for tag in fields["tags"])
    ):
        issues.append(issue("tags", "invalid_tags", "must be a list of strings"))
    if "sources" in fields and not _sources(fields["sources"]):
        issues.append(
            issue("sources", "invalid_sources", "must be a list of sources with non-empty resource")
        )
    if "status" in fields and fields["status"] not in ("draft", "stable", "deprecated"):
        issues.append(
            issue(
                "status",
                "invalid_status",
                "must be draft, stable or deprecated; defaults to stable when absent",
            )
        )
    if "generated" in fields:
        issues.extend(_event(fields["generated"], "generated", "invalid_generated"))
    if "verified" in fields:
        events = fields["verified"]
        events = [events] if isinstance(events, dict) else events
        if not isinstance(events, list):
            issues.append(
                issue(
                    "verified",
                    "invalid_verified",
                    "must be a list of revisions or one revision with by and at",
                )
            )
        else:
            for number, event in enumerate(events):
                issues.extend(_event(event, f"verified[{number}]", "invalid_verified"))
    if "stale_after" in fields and not timestamp(fields["stale_after"]):
        issues.append(
            issue(
                "stale_after",
                "invalid_stale_after",
                "must be an ISO 8601 date and time with explicit UTC offset (Z or ±HH:MM)",
            )
        )
    return {"conformant": not issues, "issues": issues}


def timestamp(value):
    if not isinstance(value, str):
        return False
    try:
        return datetime.fromisoformat(value).utcoffset() is not None
    except ValueError:
        return False


def _event(event, field, code):
    if not isinstance(event, dict):
        return [issue(field, code, "must be a mapping with by and at")]
    issues = []
    actor = event.get("by")
    if not isinstance(actor, str) or not re.fullmatch(
        r"(?:human|process):[^\s:]+|[^\s/:]+/[^\s/]+", actor
    ):
        issues.append(
            issue(
                field + ".by",
                code,
                "must identify the author as producer/version, human:id or process:id",
            )
        )
    if not timestamp(event.get("at")):
        issues.append(
            issue(
                field + ".at",
                code,
                "must be an ISO 8601 date and time with explicit UTC offset (Z or ±HH:MM)",
            )
        )
    return issues


def _sources(value):
    return isinstance(value, list) and all(
        isinstance(source, dict)
        and isinstance(source.get("resource"), str)
        and source["resource"].strip()
        for source in value
    )
