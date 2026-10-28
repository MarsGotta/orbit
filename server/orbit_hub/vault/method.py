"""Design method data; SCHEMA.md supplies the vault's own templates and rules.

Validation is advisory on reads. It never rewrites legacy metadata, source
material or reserved files. A long fiche is a review warning, not a refusal.
"""

import re
import unicodedata
from datetime import date

from .conformance import check, reserved
from .schema import DEFAULT_TYPES, load, properties

NOTE_TYPES = tuple(DEFAULT_TYPES)
REQUIRED_FIELDS = ("type",)
OPTIONAL_FIELDS = (
    "title",
    "description",
    "tags",
    "status",
    "sources",
    "resource",
    "generated",
    "verified",
    "stale_after",
    "stage",
    "start",
)
# A review warning for proposed fiches, never a byte ceiling on source material.
MAX_NOTE_WORDS = 400
METHOD_SCHEMA = {
    "version": "OKF 0.2",
    "types": NOTE_TYPES,
    "required": REQUIRED_FIELDS,
    "optional": OPTIONAL_FIELDS,
    "roles": tuple(DEFAULT_TYPES) + ("note",),
    "fields": load()["fields"],
    "schema_path": "SCHEMA.md",
    "max_words": MAX_NOTE_WORDS,
    "length_severity": "warning",
}


def validate_note(fields, body, *, path="", schema=None):
    """Return codes and numeric facts only. Unknown extensions remain readable."""
    errors, warnings = [], []
    if not isinstance(fields, dict) or not isinstance(body, str):
        return {"errors": ["invalid_document"], "warnings": []}
    _, role, values = properties(path, fields, load() if schema is None else schema)
    errors = list(dict.fromkeys(issue["code"] for issue in check(fields, path=path)["issues"]))
    if "start" in values and not _date(values["start"]):
        errors.append("invalid_start")
    words = len(body.split())
    if role != "source" and not reserved(path) and path != "SCHEMA.md" and words > MAX_NOTE_WORDS:
        warnings.append({"code": "word_limit", "words": words, "max_words": MAX_NOTE_WORDS})
    if (
        path
        and not reserved(path)
        and path != "SCHEMA.md"
        and not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*\.md", path.rsplit("/", 1)[-1])
    ):
        warnings.append({"code": "file_naming"})
    return {"errors": errors, "warnings": warnings}


def file_name(title):
    plain = unicodedata.normalize("NFKD", title).encode("ascii", "ignore").decode().lower()
    return re.sub(r"[^a-z0-9]+", "-", plain).strip("-") + ".md"


def _date(value):
    if not isinstance(value, str):
        return False
    try:
        date.fromisoformat(value)
        return True
    except ValueError:
        return False
