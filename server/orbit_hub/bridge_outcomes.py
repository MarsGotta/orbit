"""What the device bridge answers: an action's outcome and a route's refusal.

Pure: each function builds the payload one outcome is journaled or answered
with (`bridge_route`). An outcome the hub cannot confirm is `unknown` and is
never retried: the action may have run on the Mac.
"""

from .assistant.access import disabled_message
from .errors import render
from .private_route import json_response

RESULT_PROTOCOL = "orbit_device_bridge_result_v1"
_DO_NOT_REPEAT = "Do not repeat the action; check its effect on the Mac."


def publish_failed() -> dict:
    """The Mac never got the action."""
    return {
        "outcome": "failed",
        "code": "connection_failed",
        "notice": "The Mac did not receive the action request.",
        "next_step": "Check the Orbit connection before preparing another action.",
    }


def reply_timeout(deadline_ms: int) -> dict:
    """The Mac did not confirm the action within its deadline."""
    expected = "perform_action response"
    notice = (
        f"The Mac did not confirm the action within {deadline_ms} ms; the outcome is uncertain."
    )
    return {
        "outcome": "unknown",
        "code": "device_reply_timeout",
        "expected": expected,
        "deadline_ms": deadline_ms,
        "refusal": render(
            "device_reply_timeout",
            notice,
            numbers={"expected": expected, "deadline_ms": deadline_ms},
        )["refusal"],
        "notice": notice,
        "next_step": _DO_NOT_REPEAT,
    }


def receipt_lost() -> dict:
    """The Mac's answer was lost after the action was sent."""
    return {
        "outcome": "unknown",
        "code": "receipt_lost",
        "notice": "The response was lost after sending the action.",
        "next_step": _DO_NOT_REPEAT,
    }


def rendered_action(payload: dict) -> dict:
    """An action's journaled outcome as the route answers it: a typed
    refusal for one that failed, is unknown, expired or was rejected."""
    if payload.get("outcome") not in {"failed", "unknown", "expired", "rejected"}:
        return payload
    receipt = payload.get("receipt")
    code = payload.get("code")
    if not isinstance(code, str) and isinstance(receipt, dict):
        code = receipt.get("code")
    if not isinstance(code, str):
        code = None
        payload.pop("code", None)
    fallback = {
        "failed": "The action failed",
        "unknown": "Outcome unconfirmed; check its effect on the Mac",
        "expired": "The operation expired",
        "rejected": "The action was refused",
    }[payload["outcome"]]
    notice = payload.get("notice")
    message = notice if isinstance(notice, str) and notice else fallback
    return render(code, message, payload=payload)


def action_outcome(answer: dict) -> dict:
    refusal = answer.get("refusal")
    if refusal is not None:
        if refusal.get("retry", {}).get("kind") == "check_outcome":
            return {
                "outcome": "unknown",
                "code": refusal["code"],
                "refusal": refusal,
                "notice": "The Mac could not confirm the action outcome.",
                "next_step": "Do not repeat the action; read its outcome and check its effect on the Mac.",
            }
        return {
            "outcome": "failed",
            "code": refusal["code"],
            "refusal": refusal,
            "notice": "The Mac did not complete the action.",
            "next_step": "Check the cause before preparing another action.",
        }
    result = answer.get("result")
    receipt = result.get("receipt") if isinstance(result, dict) else None
    if not isinstance(receipt, dict):
        return {
            "outcome": "unknown",
            "result": result,
            "code": "invalid_action_receipt",
            "notice": "The Mac returned an invalid action receipt.",
            "next_step": _DO_NOT_REPEAT,
        }
    state = receipt.get("state")
    code = receipt.get("code")
    if state == "performed":
        return {"outcome": "performed", "receipt": receipt, "result": result}
    return {
        "outcome": "failed",
        "code": code if isinstance(code, str) else "action_failed",
        "notice": "The Mac could not execute the action.",
        "next_step": "Check Orbit and prepare another action only if it was never sent.",
        "receipt": receipt,
        "result": result,
    }


def failure(exc):
    status, code, numbers, detail = exc.status, exc.code, exc.numbers, exc.detail
    message = {
        "unauthorized": "Unauthorized access",
        "invalid_request": "Request violates the contract",
        "no_answer": "The Mac did not respond within the deadline",
        "device_unavailable": "The Mac is not connected",
        "agent_not_found": "This hub does not have that agent",
    }.get(code, "Could not complete the request")
    if code == "agent_access_disabled" and numbers is not None:
        message = disabled_message(str(numbers.get("agent_id")), str(numbers.get("group")))
    if code == "skill_not_enabled" and numbers is not None:
        enabled = str(numbers.get("enabled")).replace(",", ", ") or "none"
        message = f"Agent «{numbers.get('agent_id')}» has no enabled skill named «{numbers.get('name')}»; enabled skills: {enabled}"
    if code == "skills_not_in_hub" and numbers is not None:
        message = f"Agent «{numbers.get('agent_id')}» es de tipo {numbers.get('kind')}: its skills come from its Hermes profile, not Orbit storage"
    if detail is not None:
        message = f"Parameter {detail['field']} must be {detail['expected']}."
    payload = render(
        code,
        message,
        payload={"protocol": RESULT_PROTOCOL, "state": "unavailable"},
        numbers=numbers,
    )
    if detail is not None:
        payload["refusal"].update(detail)
    return json_response(payload, status)
