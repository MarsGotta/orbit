"""Encrypted proposals and before images in the hub's existing store."""

import hashlib
import json
from uuid import uuid4

from ..vault.models import refusal
from .models import MAX_PROPOSAL_BYTES


def namespace(config):
    return hashlib.sha256(
        json.dumps([config.kind, config.path, config.url, config.vault]).encode()
    ).hexdigest()


def digest(text):
    return hashlib.sha256(text.encode()).hexdigest() if text is not None else None


class Proposals:
    def __init__(self, store):
        self.store = store
        with store.transaction() as db:
            db.execute(
                "CREATE TABLE IF NOT EXISTS vault_distillations (id TEXT PRIMARY KEY, device_id TEXT NOT NULL, recording TEXT NOT NULL, namespace TEXT NOT NULL, state TEXT NOT NULL, body BLOB NOT NULL)"
            )
            db.execute(
                "CREATE INDEX IF NOT EXISTS vault_distillations_work ON vault_distillations(namespace,state)"
            )
            db.execute(
                "CREATE INDEX IF NOT EXISTS vault_distillations_meeting ON vault_distillations(device_id,recording)"
            )

    def recover(self):
        with self.store.read_snapshot() as db:
            ids = [row[0] for row in db.execute("SELECT id FROM vault_distillations")]
        for identity in ids:
            value = self.get(identity)
            if value["status"] == "preparing":
                value.update(status="failed", error={"code": "preparation_interrupted"})
                self.save(value)
            elif value["status"] == "applying":
                value.update(status="recovery_required", error={"code": "write_interrupted"})
                self.save(value)

    def create(self, device, recording, config, at):
        value = {
            "id": str(uuid4()),
            "device_id": device,
            "meeting_id": recording,
            "namespace": namespace(config),
            "source_kind": "meeting",
            "status": "preparing",
            "revision": 0,
            "created_at": at,
            "changes": [],
            "conflicts": [],
            "quotes": [],
            "new_source": None,
            "error": None,
            "changed_paths": [],
            "skipped": [],
            "dropped": [],
            "can_prepare_again": True,
        }
        self.save(value)
        return value

    def save(self, value):
        body = json.dumps(value, ensure_ascii=False).encode()
        if len(body) > MAX_PROPOSAL_BYTES:
            raise refusal(
                "distillation_too_large",
                f"The proposal is {len(body)} bytes; maximum {MAX_PROPOSAL_BYTES}.",
                413,
                bytes=len(body),
                max_bytes=MAX_PROPOSAL_BYTES,
            )
        sealed = self.store.encrypt(body, "vault-distillation:" + value["id"])
        with self.store.transaction() as db:
            # Deleted meetings must never resurrect a proposal after a model call.
            if db.execute(
                "SELECT 1 FROM deleted_recordings WHERE device_id=? AND recording=?",
                (value["device_id"], value["meeting_id"]),
            ).fetchone():
                raise refusal("recording_not_found", "The meeting was deleted.", 404)
            self.store.ensure_capacity(len(sealed))
            db.execute(
                "INSERT INTO vault_distillations VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET state=excluded.state,body=excluded.body",
                (
                    value["id"],
                    value["device_id"],
                    value["meeting_id"],
                    value["namespace"],
                    value["status"],
                    sealed,
                ),
            )

    def get(self, identity):
        with self.store.read_snapshot() as db:
            row = db.execute(
                "SELECT body FROM vault_distillations WHERE id=?", (identity,)
            ).fetchone()
        if row is None:
            raise refusal("distillation_not_found", "The proposal does not exist.", 404)
        return json.loads(self.store.decrypt(row[0], "vault-distillation:" + identity))

    def queued(self, namespace_id):
        with self.store.read_snapshot() as db:
            row = db.execute(
                "SELECT id FROM vault_distillations WHERE namespace=? AND state='queued' ORDER BY rowid LIMIT 1",
                (namespace_id,),
            ).fetchone()
        return self.get(row[0]) if row else None

    def list(self, namespace_id, meeting=None, offset=0, limit=50):
        with self.store.read_snapshot() as db:
            rows = db.execute(
                "SELECT id FROM vault_distillations WHERE namespace=? AND (? IS NULL OR recording=?) ORDER BY rowid DESC LIMIT ? OFFSET ?",
                (namespace_id, meeting, meeting, limit, offset),
            ).fetchall()
        return [self.get(row[0]) for row in rows]

    @staticmethod
    def public(value):
        hidden = {
            "device_id",
            "namespace",
            "snapshot",
            "journal",
            "source_raw",
            "read_set",
            "paths",
        }
        result = {k: v for k, v in value.items() if k not in hidden}
        result["changes"] = [
            {k: v for k, v in c.items() if k not in {"original", "result"}}
            for c in value["changes"]
        ]
        for change in result["changes"]:
            change["lines"] = [
                {k: v for k, v in line.items() if k != "unfiltered_after"}
                for line in change["lines"]
            ]
        return result
