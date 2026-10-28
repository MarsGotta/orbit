"""The durable receipt of each operation a device asked for.

A `chat.send`, an answer to a question and an action the device bridge asks
a Mac to perform each reserve their id here before anything happens, and
keep their answer here once it is known: the same id sent again is answered
from the receipt and never applied twice.
"""

import hmac
import json

from ..protocol import HubError

# An operation's durable receipt is bounded public frame data (the
# largest interaction acknowledgement stays a few KiB). Its headroom is
# reserved when the operation is admitted, so recording the outcome of an
# already-applied operation can never fail on the content quota.
OPERATION_RECEIPT_RESERVE_BYTES = 16 * 1024


class OperationRepository:
    def __init__(self, store, fingerprint_key: bytes):
        self.store = store
        self._fingerprint_key = fingerprint_key

    def fingerprint(self, payload: dict) -> str:
        canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"))
        return hmac.new(self._fingerprint_key, canonical.encode(), "sha256").hexdigest()

    def reserve(self, device: str, operation_id: str, payload: dict) -> dict | None:
        fingerprint = self.fingerprint(payload)
        with self.store.transaction() as db:
            if db.execute(
                "SELECT 1 FROM safety_operations WHERE device_id=? AND operation_id=?",
                (device, operation_id),
            ).fetchone():
                raise HubError("operation_conflict", "The ID already belongs to a safety control")
            existing = db.execute(
                "SELECT * FROM operations WHERE device_id=? AND id=?", (device, operation_id)
            ).fetchone()
            if existing:
                if existing["fingerprint"] != fingerprint:
                    raise HubError(
                        "operation_conflict", "The ID already belongs to another operation"
                    )
                if existing["response"]:
                    return json.loads(
                        self.store.decrypt(
                            existing["response"], f"operation:{device}:{operation_id}"
                        )
                    )
                raise HubError(
                    "outcome_unknown",
                    "Operation already received; it will not be repeated automatically",
                )
            self.store.ensure_capacity(OPERATION_RECEIPT_RESERVE_BYTES)
            db.execute(
                "INSERT INTO operations(id,device_id,fingerprint,status) VALUES (?,?,?,'pending')",
                (operation_id, device, fingerprint),
            )
        return None

    def release(self, device: str, operation_id: str) -> None:
        """Give back the id of an operation refused before it kept or sent
        anything, so the same message can be sent again (`ws_dispatch`). Only a
        reservation still pending: a receipt is never removed."""
        with self.store.transaction() as db:
            db.execute(
                "DELETE FROM operations WHERE device_id=? AND id=? AND status='pending'",
                (device, operation_id),
            )

    def finish(self, device: str, operation_id: str, response: dict) -> None:
        """Record an operation's receipt; the quota was reserved at admission.

        The operation may already have been applied upstream: refusing here
        would tell the client the operation failed when it did not, and lose
        the receipt of an effect that cannot be retried.
        """
        body = self.store.encrypt(
            json.dumps(response).encode(), f"operation:{device}:{operation_id}"
        )
        with self.store.transaction() as db:
            db.execute(
                "UPDATE operations SET status='completed',response=? WHERE device_id=? AND id=?",
                (body, device, operation_id),
            )

    def get(self, device: str, operation_id: str) -> dict | None:
        with self.store.reading() as db:
            row = db.execute(
                "SELECT status,response FROM operations WHERE device_id=? AND id=?",
                (device, operation_id),
            ).fetchone()
        if row is None:
            return None
        response = (
            json.loads(self.store.decrypt(row["response"], f"operation:{device}:{operation_id}"))
            if row["response"]
            else None
        )
        return {"status": row["status"], "response": response}
