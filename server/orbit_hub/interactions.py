"""Bounded public representations of the assistant's requests and of their receipts."""

import json

from .assistant.backend import (
    ClarificationReceipt,
    ClarificationRequest,
    PermissionReceipt,
    PermissionRequest,
)


def pending_data(
    kind: str, request: PermissionRequest | ClarificationRequest, request_id: str
) -> dict:
    """The request as the Mac sees it, under `request_id`, the id the hub gave
    it on this device (`DeviceRuntime.pending`), not necessarily the backend's."""
    result = {
        "request_id": request_id,
        "kind": kind,
        "state": "pending",
        "expires_at": None,
    }
    if isinstance(request, PermissionRequest):
        result["description"] = request.description[:2048]
        if request.deletes is not None:
            result["deletes"] = request.deletes.public()
    else:
        result["question"] = request.question[:2048]
        questions = request.questions
        if questions is not None and len(json.dumps(questions).encode()) <= 8192:
            result["questions"] = list(questions[:16])
    if request.choices is not None:
        result["choices"] = [choice[:256] for choice in request.choices[:32]]
    return result


def public_receipt(receipt: PermissionReceipt | ClarificationReceipt) -> tuple[bool, dict]:
    """Return whether still pending and a truthful public acknowledgement."""
    if isinstance(receipt, PermissionReceipt):
        return False, {
            "status": "ok" if receipt.applied else "expired",
            "resolved": 1 if receipt.applied else 0,
            "applied": receipt.applied,
        }
    return receipt.status == "ok" and bool(receipt.remaining), {
        "status": receipt.status,
        "remaining": list(receipt.remaining),
        "applied": receipt.status == "ok",
    }
