"""Pairing and session routes."""

from math import ceil

from fastapi import Request
from pydantic import ValidationError

from ..assistant import handover
from ..errors import HubError
from ..identifiers import canonical_uuid
from ..log import in_store
from ..protocol import HUB_PROTOCOL, PairRequest
from ..storage import SESSION_LIST_LIMIT, SESSION_MESSAGES_PAGE_LIMIT

PAIR_ATTEMPT_RETENTION_SECONDS = 60
# Pairing comes before any device credential, so its bounds protect
# the hub from anyone who can reach it, not the code: a pairing code is
# `token_urlsafe(24)` (192 bits) and lives at most 600 s, past guessing at any
# rate.
# - 10 attempts per address per 60 s keep one peer from turning pairing into
#   unbounded store work; a person mistyping a code needs a few. Why 10 was not
#   measured.
# - 4,096 addresses bound the table of attempts in memory: measured on
#   2026-10-21, 4,096 IPv4 entries take about 0.6 MiB (156 bytes each). It does
#   not limit devices already paired.
# - 2,048 bytes of body hold every valid request: a code of at most 160
#   characters and a name of at most 80 encode, in the Mac's JSON (control
#   characters as \u00XX), to at most 1,461 bytes (computed 2026-10-21).


def mount(app, store, agents, identity, bounded_body, now, pair_attempts, store_executor):
    @app.get("/health")
    async def health():
        return {"ok": True, "protocol": HUB_PROTOCOL}

    @app.post("/v1/pair")
    async def pair(request: Request):
        peer = request.client.host if request.client else "unknown"
        timestamp = now()
        admit_pair_attempt(pair_attempts, peer, timestamp)
        try:
            payload = PairRequest.model_validate_json(await bounded_body(request, 2048))
        except ValidationError as exc:
            raise HubError("invalid_request", "Solicitud de emparejado no válida") from exc
        return store.devices.pair(payload.code, payload.name, timestamp)

    @app.get("/v1/sessions")
    async def sessions(request: Request, before: str | None = None, limit: int | None = None):
        # The most recent conversations, a page of at most
        # SESSION_LIST_LIMIT, and how many the hub holds.
        identity(request)
        if before:
            try:
                canonical_uuid(before)
            except ValueError as exc:
                raise HubError("invalid_request", "Cursor de sesiones no válido") from exc
        page, total, next_cursor = store.chat.sessions_after(
            None, before or None, bounded_limit(limit, SESSION_LIST_LIMIT, "sesiones")
        )
        # Origins are read on opening: repeating them in 128 rows would exceed
        # the Mac's 32 KiB list bound even though the messages read has room.
        sessions = [{**row, "agent_id": agents.owner(row)} for row in page]
        return {"sessions": sessions, "total": total, "next": next_cursor}

    @app.get("/v1/sessions/{session_id}/messages")
    async def session_messages(
        session_id: str, request: Request, before: int | None = None, limit: int | None = None
    ):
        # The Mac reads a conversation from here, newest page first.
        # Pairing authenticates the owner for read-only conversation access
        # across Macs. Runtime session bindings remain device-scoped.
        identity(request)
        page = await in_store(
            store_executor,
            store.chat.session_messages,
            None,
            session_id,
            before,
            bounded_limit(limit, SESSION_MESSAGES_PAGE_LIMIT, "mensajes"),
        )
        handed = await in_store(store_executor, handover.origin, store, session_id)
        return {
            **page,
            "agent_id": agents.owner(page),
            **({"handed_over_from": handed} if handed is not None else {}),
        }


def admit_pair_attempt(pair_attempts: dict, peer: str, timestamp: float) -> None:
    """Count one pairing attempt of `peer`, refused past the bounds above."""
    expired_peers = [
        address
        for address, (started_at, _) in pair_attempts.items()
        if timestamp - started_at >= PAIR_ATTEMPT_RETENTION_SECONDS
    ]
    for address in expired_peers:
        del pair_attempts[address]
    started, attempts = pair_attempts.get(peer, (timestamp, 0))
    if attempts >= 10:
        raise HubError(
            "rate_limited",
            "Demasiados intentos; espera un minuto",
            numbers={
                "attempts": attempts,
                "max_attempts": 10,
                "window_seconds": PAIR_ATTEMPT_RETENTION_SECONDS,
                "retry_after_ms": max(
                    0, ceil((started + PAIR_ATTEMPT_RETENTION_SECONDS - timestamp) * 1000)
                ),
            },
        )
    if len(pair_attempts) >= 4096 and peer not in pair_attempts:
        next_expiration = min(started_at for started_at, _ in pair_attempts.values())
        raise HubError(
            "rate_limited",
            "Límite de emparejado alcanzado",
            numbers={
                "peers": len(pair_attempts),
                "max_peers": 4096,
                "retry_after_ms": max(
                    0,
                    ceil((next_expiration + PAIR_ATTEMPT_RETENTION_SECONDS - timestamp) * 1000),
                ),
            },
        )
    pair_attempts[peer] = (started, attempts + 1)


def bounded_limit(limit: int | None, maximum: int, what: str) -> int:
    """`limit` as asked, between 1 and the page's ceiling; absent, the ceiling."""
    if limit is None:
        return maximum
    if not 1 <= limit <= maximum:
        raise HubError(
            "invalid_request",
            f"El límite de {what} va de 1 a {maximum}",
            numbers={"limit": limit, "max": maximum},
        )
    return limit
