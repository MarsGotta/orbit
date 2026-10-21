"""Pairing and session routes."""

from math import ceil

from fastapi import Request
from pydantic import ValidationError

from ..errors import HubError
from ..identifiers import canonical_uuid
from ..log import in_store
from ..protocol import HUB_PROTOCOL, PairRequest
from ..storage import SESSION_LIST_LIMIT, SESSION_MESSAGES_PAGE_LIMIT

PAIR_ATTEMPT_RETENTION_SECONDS = 60


def mount(app, store, identity, bounded_body, now, pair_attempts, store_executor):
    @app.get("/health")
    async def health():
        return {"ok": True, "protocol": HUB_PROTOCOL}

    @app.post("/v1/pair")
    async def pair(request: Request):
        peer = request.client.host if request.client else "unknown"
        timestamp = now()
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
        try:
            payload = PairRequest.model_validate_json(await bounded_body(request, 2048))
        except ValidationError as exc:
            raise HubError("invalid_request", "Solicitud de emparejado no válida") from exc
        return store.pair(payload.code, payload.name, timestamp)

    @app.get("/v1/sessions")
    async def sessions(request: Request, before: str | None = None, limit: int | None = None):
        # REF-04 A7: the most recent conversations, a page of at most
        # SESSION_LIST_LIMIT, and how many the hub holds.
        device = identity(request)
        if before:
            try:
                canonical_uuid(before)
            except ValueError as exc:
                raise HubError("invalid_request", "Cursor de sesiones no válido") from exc
        page, total, next_cursor = store.sessions_after(
            device, before or None, bounded_limit(limit, SESSION_LIST_LIMIT, "sesiones")
        )
        return {"sessions": page, "total": total, "next": next_cursor}

    @app.get("/v1/sessions/{session_id}/messages")
    async def session_messages(
        session_id: str, request: Request, before: int | None = None, limit: int | None = None
    ):
        # SIM-30: the Mac reads a conversation from here, newest page first.
        device = identity(request)
        return await in_store(
            store_executor,
            store.session_messages,
            device,
            session_id,
            before,
            bounded_limit(limit, SESSION_MESSAGES_PAGE_LIMIT, "mensajes"),
        )


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
