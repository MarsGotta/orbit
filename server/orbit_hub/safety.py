"""Bounded durable safety receipts, independent of content admission.

Only a bound-turn interrupt or explicit deny enters this journal. Targets, not
unbounded retry UUID aliases, deduplicate safe controls. Obsolete turn/request targets
remain rejected by DeviceRuntime after their receipts have been retired.
"""

import hashlib
import json

from .protocol import HubError

MAX_SAFETY_TARGETS = 128
MAX_SAFETY_RECEIPT_BYTES = 4096
SCHEMA = '\nCREATE TABLE IF NOT EXISTS safety_operations (\n    device_id TEXT NOT NULL REFERENCES devices(id), target TEXT NOT NULL,\n    session_id TEXT NOT NULL, turn_id TEXT NOT NULL, operation_id TEXT NOT NULL,\n    fingerprint TEXT NOT NULL, response BLOB,\n    PRIMARY KEY(device_id,target), UNIQUE(device_id,operation_id));\n'  # fmt: skip


def target_key(method: str, session: str, turn: str, interaction: str = "") -> str:
    return hashlib.sha256(json.dumps([method, session, turn, interaction]).encode()).hexdigest()


def reserve(
    store, device, operation_id, payload, *, target, session_id, turn_id, live_targets, validate
):
    fingerprint = store.operations.fingerprint(payload)
    with store.transaction() as db:
        normal = db.execute(
            "SELECT fingerprint,response FROM operations WHERE device_id=? AND id=?",
            (device, operation_id),
        ).fetchone()
        exact = db.execute(
            "SELECT fingerprint,response FROM safety_operations WHERE device_id=? AND operation_id=?",
            (device, operation_id),
        ).fetchone()
        if normal or exact:
            row = normal or exact
            if row["fingerprint"] != fingerprint:
                raise HubError("operation_conflict", "The ID already belongs to another operation")
            if not row["response"]:
                raise HubError(
                    "outcome_unknown",
                    "Control already received; it will not be repeated automatically",
                )
            context = (
                f"operation:{device}:{operation_id}" if normal else f"safety:{device}:{target}"
            )
            return json.loads(store.decrypt(row["response"], context))
        existing = db.execute(
            "SELECT response FROM safety_operations WHERE device_id=? AND target=?",
            (device, target),
        ).fetchone()
        if existing:
            if not existing["response"]:
                raise HubError(
                    "outcome_unknown",
                    "Control already requested for this turn; it will not be repeated",
                )
            response = json.loads(store.decrypt(existing["response"], f"safety:{device}:{target}"))
            if response.get("data", {}).get("status") == "error":
                db.execute(
                    "DELETE FROM safety_operations WHERE device_id=? AND target=?",
                    (device, target),
                )
            else:
                response["id"] = operation_id
                return response
        validate()
        # Retire only receipts for obsolete targets. Their original turn/request
        # can no longer pass runtime ownership/freshness validation.
        stale = db.execute(
            "SELECT target,turn_id FROM safety_operations WHERE device_id=?", (device,)
        ).fetchall()
        for row in stale:
            if row["target"] not in live_targets:
                db.execute(
                    "DELETE FROM safety_operations WHERE device_id=? AND target=?",
                    (device, row["target"]),
                )
        count = db.execute(
            "SELECT count(*) FROM safety_operations WHERE device_id=?", (device,)
        ).fetchone()[0]
        if count >= MAX_SAFETY_TARGETS:
            raise HubError(
                "safety_capacity",
                "Too many active controls; local revocation remains available",
            )
        # Content quota is deliberately not consulted. Payloads are fixed-size
        # metadata/receipts; physical ENOSPC remains a separate deployment limit.
        db.execute(
            "INSERT INTO safety_operations VALUES (?,?,?,?,?,?,NULL)",
            (device, target, session_id, turn_id, operation_id, fingerprint),
        )
    return None


def finish(store, device: str, target: str, response: dict) -> None:
    encoded = json.dumps(response).encode()
    if len(encoded) > MAX_SAFETY_RECEIPT_BYTES:
        raise HubError("safety_receipt_invalid", "Control receipt violates the contract")
    with store.transaction() as db:
        db.execute(
            "UPDATE safety_operations SET response=? WHERE device_id=? AND target=? AND response IS NULL",
            (store.encrypt(encoded, f"safety:{device}:{target}"), device, target),
        )
