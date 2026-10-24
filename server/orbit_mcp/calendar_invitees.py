"""An event's invitees as Deimos reads them: grouped by email domain, every
word the Mac sent kept."""

from __future__ import annotations

_ROLE = {
    "required": "",
    "optional": "opcional",
    "chair": "organiza",
    "non_participant": "no participa",
    "unknown": "rol desconocido",
}
_RESPONSE = {
    "accepted": "acepta",
    "declined": "rechaza",
    "tentative": "provisional",
    "pending": "pendiente",
    "delegated": "delega",
    "completed": "completado",
    "in_process": "en curso",
    "unknown": "respuesta desconocida",
}
_KIND = {
    "person": "",
    "room": "sala",
    "resource": "recurso",
    "group": "grupo",
    "unknown": "tipo desconocido",
}


def _invitee(value: dict) -> tuple[str | None, str]:
    """(domain, how the guest is written inside it) — no word of it dropped.

    A guest with an email is written by the part before the @ under its
    domain, with the name in front only when it is not that part; the rest by
    their address. The parenthesis says «tú», the role unless required, the
    response, the kind unless a person, and an address that is not the email.
    """
    email = value.get("email") if isinstance(value.get("email"), str) else None
    address = value.get("participant_url") if isinstance(value.get("participant_url"), str) else ""
    domain = None
    who = address
    if email and "@" in email:
        local, domain = email.rsplit("@", 1)
        who = local
    elif email:
        who = email
    notes = []
    if value.get("is_current_user") is True:
        notes.append("tú")
    role = value.get("role")
    notes.append(_ROLE.get(role, str(role)) if isinstance(role, str) else "")
    response = value.get("response_status")
    notes.append(_RESPONSE.get(response, str(response)) if isinstance(response, str) else "")
    kind = value.get("participant_type")
    notes.append(_KIND.get(kind, str(kind)) if isinstance(kind, str) else "")
    if email and address and address.lower() != f"mailto:{email}".lower():
        notes.append(address)
    name = value.get("name") if isinstance(value.get("name"), str) else None
    if name and name != who:
        who = f"{name} <{who}>"
    said = [note for note in notes if note]
    return domain, f"{who} ({', '.join(said)})" if said else who


def written_invitees(values: list) -> str:
    """The guests grouped by email domain, in the order they came."""
    groups: dict = {}
    for value in values:
        if isinstance(value, dict):
            domain, written = _invitee(value)
            groups.setdefault(domain, []).append(written)
    return "; ".join(
        ", ".join(written) if domain is None else f"@{domain}: {', '.join(written)}"
        for domain, written in groups.items()
    )
