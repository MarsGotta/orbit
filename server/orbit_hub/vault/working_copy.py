"""The hub's encrypted working copy. No connector ever exposes a write method."""

import hashlib
import json

from .models import MAX_NOTE_BYTES, refusal


class WorkingCopy:
    def __init__(self, store):
        self.store = store
        with store.transaction() as db:
            db.execute(
                "CREATE TABLE IF NOT EXISTS vault_working_copy (namespace TEXT NOT NULL, id TEXT NOT NULL, body BLOB NOT NULL, PRIMARY KEY(namespace,id))"
            )

    def reader(self, connector, configuration):
        identity = [configuration.kind, configuration.path, configuration.url, configuration.vault]
        namespace = hashlib.sha256(json.dumps(identity).encode()).hexdigest()
        return CopiedReader(self.store, connector, namespace)

    def clear(self, db=None):
        if db is not None:
            db.execute("DELETE FROM vault_working_copy")
            return
        with self.store.transaction() as opened:
            self.clear(opened)


class CopiedReader:
    def __init__(self, store, connector, namespace):
        self.store, self.connector, self.namespace = store, connector, namespace
        self.stamps = {}

    def catalog(self):
        entries = self.connector.catalog()
        self.stamps = {entry["path"]: entry.get("stamp") for entry in entries}
        return entries

    def prune(self, paths):
        live = {hashlib.sha256(path.encode()).hexdigest() for path in paths}
        with self.store.transaction() as db:
            rows = db.execute(
                "SELECT id FROM vault_working_copy WHERE namespace=?", (self.namespace,)
            ).fetchall()
            db.executemany(
                "DELETE FROM vault_working_copy WHERE namespace=? AND id=?",
                ((self.namespace, row[0]) for row in rows if row[0] not in live),
            )

    def read(self, path):
        note_id = hashlib.sha256(path.encode()).hexdigest()
        stamp = self.stamps.get(path)
        if stamp:
            with self.store.read_snapshot() as db:
                row = db.execute(
                    "SELECT body FROM vault_working_copy WHERE namespace=? AND id=?",
                    (self.namespace, note_id),
                ).fetchone()
            if row:
                saved = json.loads(
                    self.store.decrypt(row[0], f"vault-copy:{self.namespace}:{note_id}")
                )
                if saved.get("stamp") == stamp:
                    return saved["raw"]
        body = self.connector.read(path)
        size = len(body.encode())
        if size > MAX_NOTE_BYTES:
            raise refusal(
                "vault_note_too_large",
                f"La nota ocupa {size} bytes; el máximo es {MAX_NOTE_BYTES}.",
                413,
                bytes=size,
                max_bytes=MAX_NOTE_BYTES,
            )
        sealed = self.store.encrypt(
            json.dumps({"path": path, "raw": body, "stamp": stamp}).encode(),
            f"vault-copy:{self.namespace}:{note_id}",
        )
        with self.store.transaction() as db:
            db.execute(
                "INSERT INTO vault_working_copy VALUES(?,?,?) ON CONFLICT(namespace,id) DO UPDATE SET body=excluded.body",
                (self.namespace, note_id, sealed),
            )
        return body
