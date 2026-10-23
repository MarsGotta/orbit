"""Conversations and their messages: what the hub keeps of every chat."""

import json
from uuid import uuid4

from ..protocol import HubError

# The conversations `/v1/sessions` lists for a Mac, the most recent
# ones. The hub keeps every conversation ever opened and each restart opens new
# ones, so the whole list only grows.
# What it protects: the Mac that reads the list, which keeps at most 128 of
# them (`MAX_SESSIONS` in crates/orbit-daemon/src/hub.rs).
# Why 128: it is what the daemon keeps, and it is far more than the app's list
# of recent conversations shows. The two are kept equal by hand.
# What it is not: a retention limit. Nothing is deleted; the route says how
# many there are (`total`), and older conversations stay stored.
SESSION_LIST_LIMIT = 128

# One page of a conversation's messages (`GET /v1/sessions/{id}/messages`).
# What it protects: the Mac that reads it, whose HTTP reader stops at 1 MiB
# (`MAX_HUB_HTTP_BODY_BYTES`) and which passes the page on to the app inside
# one 640 KiB read (`SessionMessages`, crates/orbit-core/src/reads.rs). Why
# 512 KiB: it is what `chat.history` returned before it, and it leaves the
# daemon room for what it adds to each message. What it is not: a limit on a
# conversation. Older messages come with `next`; only a single message larger
# than a page is refused, with its size.
SESSION_MESSAGES_PAGE_BYTES = 512 * 1024
# The most messages one page decrypts: the 200 `chat.history` read, which
# brings most conversations whole in one page. The bytes are bounded apart.
SESSION_MESSAGES_PAGE_LIMIT = 200
# The bytes of a page around its messages: `{"messages":[`, `],"next":`, a
# message id, `,"total":`, a count and `,"agent_id":` with the session's agent.
# Measured as 116 with two 19-digit numbers and a 32-character agent id; 128
# leaves room.
_PAGE_ENVELOPE_BYTES = 128

_UNKNOWN_SESSION = "Sesión no disponible para este dispositivo"


class ChatRepository:
    def __init__(self, store):
        self.store = store

    def create_session(
        self, device: str, agent: str, runtime: str, stored: str, epoch: str, now: float
    ) -> str:
        session_id = str(uuid4())
        with self.store.transaction() as db:
            self.store.ensure_capacity(1024)
            db.execute(
                "INSERT INTO sessions(id,device_id,runtime_id,stored_id,epoch,status,created,"
                "agent_id) VALUES (?,?,?,?,?,?,?,?)",
                (session_id, device, runtime, stored, epoch, "idle", now, agent),
            )
        return session_id

    def session(self, device: str, session_id: str) -> dict:
        with self.store.reading() as db:
            row = db.execute(
                "SELECT * FROM sessions WHERE device_id=? AND id=?", (device, session_id)
            ).fetchone()
        if row is None:
            raise HubError("session_not_found", _UNKNOWN_SESSION)
        return dict(row)

    def sessions_after(
        self, device: str, before: str | None, limit: int = SESSION_LIST_LIMIT
    ) -> tuple[list[dict], int, str | None]:
        """One page of the device's conversations, newest first, after `before`.

        Each row says when it started, when its last message was written (its
        start while it has none, or none with a time), how many it holds and
        its agent (None: stored before agents existed).
        """
        with self.store.reading() as db:
            anchor = None
            if before is not None:
                anchor = db.execute(
                    "SELECT created,id FROM sessions WHERE device_id=? AND id=?",
                    (device, before),
                ).fetchone()
                if anchor is None:
                    raise HubError("session_not_found", _UNKNOWN_SESSION)
            created = anchor["created"] if anchor is not None else None
            # The extra row proves that another page exists. No stored row is cut.
            rows = db.execute(
                "SELECT s.id,s.created,s.agent_id,"
                " (SELECT created_at_ms FROM messages m WHERE m.session_id=s.id"
                "  ORDER BY m.id DESC LIMIT 1) AS last_ms,"
                " (SELECT COUNT(*) FROM messages m WHERE m.session_id=s.id) AS count"
                " FROM sessions s WHERE s.device_id=? "
                "AND (? IS NULL OR s.created < ? OR (s.created = ? AND s.id > ?)) "
                "ORDER BY s.created DESC, s.id ASC LIMIT ?",
                (device, before, created, created, before, limit + 1),
            ).fetchall()
            total = int(
                db.execute(
                    "SELECT COUNT(*) AS total FROM sessions WHERE device_id=?", (device,)
                ).fetchone()["total"]
            )
        page = [
            {
                "id": row["id"],
                "created_at_ms": round(row["created"] * 1000),
                "last_at_ms": row["last_ms"]
                if row["last_ms"] is not None
                else round(row["created"] * 1000),
                "message_count": row["count"],
                "agent_id": row["agent_id"],
            }
            for row in rows[:limit]
        ]
        next_cursor = page[-1]["id"] if len(rows) > limit else None
        return page, total, next_cursor

    def set_status(self, session_id: str, status: str) -> None:
        with self.store.transaction() as db:
            db.execute("UPDATE sessions SET status=? WHERE id=?", (status, session_id))

    def resume_session(
        self, device: str, session_id: str, runtime: str, stored: str, epoch: str
    ) -> None:
        with self.store.transaction() as db:
            cursor = db.execute(
                "UPDATE sessions SET runtime_id=?, stored_id=?, epoch=? WHERE device_id=? AND id=?",
                (runtime, stored, epoch, device, session_id),
            )
            if cursor.rowcount != 1:
                raise HubError("session_not_found", _UNKNOWN_SESSION)

    def append_message(
        self,
        session_id: str,
        role: str,
        text: str,
        *,
        now_ms: int,
        kind: str = "text",
        duration_ms: int | None = None,
        client_id: str | None = None,
        attachments: list[str] | tuple[str, ...] = (),
    ) -> None:
        """Append one message; `attachments`, the ids its turn named, are bound
        to it in the same transaction (`AttachmentRepository.bind`)."""
        encoded = text.encode()
        with self.store.transaction() as db:
            self.store.ensure_capacity(len(encoded) + 256)
            cursor = db.execute(
                "INSERT INTO messages(session_id,role,body,created_at_ms,kind,duration_ms,"
                "client_id) VALUES (?,?,?,?,?,?,?)",
                (
                    session_id,
                    role,
                    self.store.encrypt(encoded, f"message:{session_id}:{role}"),
                    now_ms,
                    kind,
                    duration_ms,
                    client_id,
                ),
            )
            if attachments:
                self.store.attachments.bind(db, session_id, cursor.lastrowid, list(attachments))

    def session_messages(
        self, device: str, session_id: str, before: int | None, limit: int
    ) -> dict:
        """One page of a conversation, oldest first, ending before `before`.

        Newest messages first fill the page up to `limit` and
        `SESSION_MESSAGES_PAGE_BYTES` of JSON, whole; `next` reads the ones
        before and `total` counts the conversation's. A conversation of
        another device is `session_not_found`. `agent_id` is the session's
        (None: stored before agents existed).
        """
        session = self.session(device, session_id)
        budget = SESSION_MESSAGES_PAGE_BYTES - _PAGE_ENVELOPE_BYTES
        messages: list[dict] = []
        with self.store.reading() as db:
            # Sizes first: a long conversation never decrypts more than a page.
            total = db.execute(
                "SELECT COUNT(*) FROM messages WHERE session_id=?", (session_id,)
            ).fetchone()[0]
            rows = db.execute(
                "SELECT id,role,length(body) AS size,created_at_ms,kind,duration_ms,client_id"
                " FROM messages WHERE session_id=? AND (? IS NULL OR id < ?)"
                " ORDER BY id DESC LIMIT ?",
                (session_id, before, before, limit + 1),
            ).fetchall()
            # Each message's attachments, listed (not their content).
            attached = self.store.attachments.of_messages(db, [row["id"] for row in rows[:limit]])
            for row in rows[:limit]:
                message = self._message(db, session_id, row, attached)
                # As the route writes it (FastAPI: compact, UTF-8), plus its comma.
                size = (
                    len(json.dumps(message, ensure_ascii=False, separators=(",", ":")).encode()) + 1
                )
                if size > budget:
                    if not messages:
                        raise HubError(
                            "session_message_too_large",
                            "Un mensaje de la conversación no cabe en una página",
                            numbers={
                                "bytes": size,
                                "max_bytes": SESSION_MESSAGES_PAGE_BYTES - _PAGE_ENVELOPE_BYTES,
                            },
                        )
                    break
                messages.append(message)
                budget -= size
        older = len(messages) < len(rows)
        messages.reverse()
        return {
            "messages": messages,
            "next": messages[0]["id"] if older and messages else None,
            "total": total,
            "agent_id": session["agent_id"],
        }

    def _message(self, db, session_id: str, row, attached: dict) -> dict:
        """One stored message as the page carries it, decrypted."""
        body = db.execute("SELECT body FROM messages WHERE id=?", (row["id"],)).fetchone()[0]
        message = {
            "id": row["id"],
            "role": row["role"],
            "text": self.store.decrypt(body, f"message:{session_id}:{row['role']}").decode(),
            "at_ms": row["created_at_ms"],
            "kind": row["kind"],
        }
        if row["duration_ms"] is not None:
            message["duration_ms"] = row["duration_ms"]
        if row["client_id"] is not None:
            message["client_id"] = row["client_id"]
        if row["id"] in attached:
            message["attachments"] = attached[row["id"]]
        return message
