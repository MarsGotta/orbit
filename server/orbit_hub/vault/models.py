"""Vault contracts and separate bounds for I/O, pages and future method validation."""

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, SecretStr, ValidationError, model_validator

from ..errors import HubError

# Bounds memory per decoded note, not the method's word count (old notes remain readable).
MAX_NOTE_BYTES = 256 * 1024
# Bounds one listing response, not the number of notes in the vault.
MAX_PAGE_SIZE = 100
# Bounds a catalogue walk, including hidden entries, independently of page size.
MAX_SCAN_ENTRIES = 10_000
# Bounds content-search/backlink disk reads per request, not stored vault size.
MAX_SCAN_BYTES = 16 * 1024 * 1024
# JSON escaping may expand one note sixfold; metadata fits in the remaining 128 KiB.
MAX_UPSTREAM_BYTES = 6 * MAX_NOTE_BYTES + 128 * 1024
# Protects config parsing, independently of note reads; paths/URL/token fit in 16 KiB.
MAX_REQUEST_BYTES = 16 * 1024
# Notes carry raw Markdown and parsed views together, plus a page of links.
# This separate wire ceiling includes JSON escaping; it does not enlarge a note.
MAX_RESPONSE_BYTES = 4 * 1024 * 1024
# Leaves the daemon's eight-second HTTP budget room to deliver a named refusal.
CONNECTOR_SECONDS = 7
# Two simultaneous scans fit a note read plus its backlinks without allowing
# each paired client to occupy the hub's entire thread pool and scan memory.
MAX_READS = 2
# Caps open traversal descriptors and recursive indexing stack. Sixty-four
# levels allows deeply nested vaults without approaching Python's stack limit.
MAX_FOLDER_DEPTH = 64


def refusal(code, message, status=400, **numbers):
    return HubError(code, message, http_status=status, numbers=numbers or None)


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True, hide_input_in_errors=True)


class Configuration(Strict):
    kind: Literal["folder", "fast_note_sync"]
    path: str = Field(default="", max_length=4096)
    url: str = Field(default="", max_length=2048)
    vault: str = Field(default="", max_length=256)
    token: SecretStr | None = Field(default=None, max_length=8192)
    link_style: Literal["markdown", "wikilink"] = "markdown"
    maintenance_model: str = Field(default="", max_length=256)

    def public(self):
        return {
            "configured": True,
            "kind": self.kind,
            "path": self.path,
            "url": self.url,
            "vault": self.vault,
            "has_token": bool(self.token),
            "link_style": self.link_style,
            "maintenance_model": self.maintenance_model,
            "maintenance_enabled": False,
        }


class Read(Strict):
    operation: Literal[
        "list", "search", "read", "backlinks", "outlinks", "index", "catalog", "tasks"
    ]
    path: str = Field(default="", max_length=4096)
    query: str = Field(default="", max_length=256)
    content: bool = True
    page: int = Field(default=1, ge=1, le=MAX_SCAN_ENTRIES)
    page_size: int = Field(default=50, ge=1, le=MAX_PAGE_SIZE)
    agent_id: str | None = Field(default=None, pattern=r"^[a-z0-9-]{1,32}$")
    state: Literal["all", "open", "done", "overdue", "mine"] = "all"
    owner: str = Field(default="", max_length=256)
    today: str = Field(default="", pattern=r"^(?:\d{4}-\d{2}-\d{2})?$")

    @model_validator(mode="after")
    def bounded_offset(self):
        if (self.page - 1) * self.page_size >= MAX_SCAN_ENTRIES:
            raise ValueError("page outside catalogue budget")
        return self


def parse(model, raw):
    try:
        return model.model_validate_json(raw)
    except ValidationError as exc:
        error = exc.errors(include_input=False, include_context=False)[0]
        field = next(
            (str(part) for part in error["loc"] if str(part) in model.model_fields), "solicitud"
        )
        reasons = {
            "missing": "falta un valor obligatorio",
            "extra_forbidden": "incluye un campo desconocido",
            "string_too_long": "supera la longitud admitida",
            "literal_error": "no es una opción admitida",
            "greater_than_equal": "es menor que el mínimo",
            "less_than_equal": "supera el máximo",
        }
        reason = reasons.get(error["type"], "no tiene el formato admitido")
        raise refusal(
            "vault_invalid_request",
            f"El campo «{field}» {reason}. Las páginas admiten de 1 a {MAX_PAGE_SIZE} entradas.",
            max_page_size=MAX_PAGE_SIZE,
            max_page=MAX_SCAN_ENTRIES,
        ) from None
    except (ValueError, TypeError, RecursionError):
        raise refusal(
            "vault_invalid_request",
            "Revisa los campos de Notas: página de 1 a 100 entradas y ruta relativa al vault.",
            max_page_size=MAX_PAGE_SIZE,
            max_page=MAX_SCAN_ENTRIES,
        ) from None
