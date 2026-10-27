"""Design method data; SCHEMA.md supplies the vault's own templates and rules.

Validation is advisory on reads. It never rewrites legacy metadata, source
material or reserved files. A long fiche is a review warning, not a refusal.
"""

import re
import unicodedata
from datetime import date, datetime

NOTE_TYPES = (
    "persona",
    "proyecto",
    "organizacion",
    "decision",
    "tema",
    "consulta",
    "nota",
    "fuente",
)
FOLDERS = {
    "persona": "personas",
    "proyecto": "proyectos",
    "organizacion": "organizaciones",
    "decision": "decisiones",
    "tema": "temas",
    "consulta": "consultas",
    "fuente": "fuentes",
}
TYPE_PROPERTIES = {
    "persona": ("rol", "organizacion", "correo"),
    "proyecto": ("estado", "responsable", "cliente", "arranque"),
    "fuente": ("resource", "fecha", "duracion_min", "serie", "participantes", "sha256"),
}
REQUIRED_FIELDS = ("type",)
OPTIONAL_FIELDS = (
    "title",
    "description",
    "tags",
    "status",
    "sources",
    "generated",
    "verified",
    "stale_after",
)
# A review warning for proposed fiches, never a byte ceiling on source material.
MAX_NOTE_WORDS = 400
METHOD_SCHEMA = {
    "version": "OKF 0.2",
    "types": NOTE_TYPES,
    "required": REQUIRED_FIELDS,
    "optional": OPTIONAL_FIELDS,
    "properties": TYPE_PROPERTIES,
    "folders": FOLDERS,
    "schema_path": "SCHEMA.md",
    "max_words": MAX_NOTE_WORDS,
    "length_severity": "warning",
}


def validate_note(fields, body, *, path=""):
    """Return codes and numeric facts only. Unknown extensions remain readable."""
    errors, warnings = [], []
    if not isinstance(fields, dict) or not isinstance(body, str):
        return {"errors": ["invalid_document"], "warnings": []}
    kind = fields.get("type")
    if kind not in NOTE_TYPES and path not in {"SCHEMA.md", "index.md", "log.md"}:
        errors.append("invalid_type")
    for name in ("title", "description"):
        if name in fields and not isinstance(fields[name], str):
            errors.append(f"invalid_{name}")
    if "tags" in fields and (
        not isinstance(fields["tags"], list) or any(not isinstance(v, str) for v in fields["tags"])
    ):
        errors.append("invalid_tags")
    if "sources" in fields and not _sources(fields["sources"]):
        errors.append("invalid_sources")
    if "generated" in fields and not _actor_event(fields["generated"]):
        errors.append("invalid_generated")
    if "verified" in fields:
        events = fields["verified"]
        events = [events] if isinstance(events, dict) else events
        if not isinstance(events, list) or any(not _actor_event(event) for event in events):
            errors.append("invalid_verified")
    if "status" in fields and fields["status"] not in ("draft", "stable", "deprecated"):
        errors.append("invalid_status")
    for name in ("stale_after", "arranque", "fecha"):
        if name in fields and not _date(fields[name]):
            errors.append(f"invalid_{name}")
    errors.extend(_properties(fields, kind))
    words = len(body.split())
    if kind not in ("fuente", "esquema", "indice", "registro") and words > MAX_NOTE_WORDS:
        warnings.append({"code": "word_limit", "words": words, "max_words": MAX_NOTE_WORDS})
    if (
        path
        and isinstance(kind, str)
        and kind in FOLDERS
        and not path.startswith(FOLDERS[kind] + "/")
    ):
        warnings.append({"code": "folder_layout"})
    if (
        path
        and path not in {"SCHEMA.md", "index.md", "log.md"}
        and not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*\.md", path.rsplit("/", 1)[-1])
    ):
        warnings.append({"code": "file_naming"})
    return {"errors": errors, "warnings": warnings}


def file_name(title):
    plain = unicodedata.normalize("NFKD", title).encode("ascii", "ignore").decode().lower()
    return re.sub(r"[^a-z0-9]+", "-", plain).strip("-") + ".md"


def _sources(value):
    return isinstance(value, list) and all(
        isinstance(item, dict)
        and isinstance(item.get("resource"), str)
        and item["resource"].strip()
        for item in value
    )


def _date(value):
    if not isinstance(value, str):
        return False
    try:
        date.fromisoformat(value)
        return True
    except ValueError:
        try:
            return datetime.fromisoformat(value).utcoffset() is not None
        except ValueError:
            return False


def _actor_event(value):
    if not isinstance(value, dict) or set(value) != {"by", "at"}:
        return False
    if not isinstance(value["by"], str) or not value["by"].strip():
        return False
    try:
        return (
            isinstance(value["at"], str)
            and datetime.fromisoformat(value["at"]).utcoffset() is not None
        )
    except ValueError:
        return False


def _properties(fields, kind):
    names = TYPE_PROPERTIES.get(kind, ()) if isinstance(kind, str) else ()
    errors = []
    for name in names:
        if name not in fields:
            continue
        value = fields[name]
        if name == "duracion_min":
            # BaseLoader deliberately keeps YAML scalars as strings.
            valid = (
                isinstance(value, (int, float)) and not isinstance(value, bool) and value >= 0
            ) or (isinstance(value, str) and re.fullmatch(r"[0-9]+(?:\.[0-9]+)?", value))
        elif name == "participantes":
            valid = isinstance(value, list) and all(isinstance(item, str) for item in value)
        elif name == "estado":
            valid = value in ("en marcha", "en riesgo", "en pausa")
        elif name == "sha256":
            valid = isinstance(value, str) and re.fullmatch(r"[a-fA-F0-9]{64}", value)
        else:
            valid = isinstance(value, str)
        if not valid:
            errors.append(f"invalid_{name}")
    return errors
