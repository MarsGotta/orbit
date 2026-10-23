"""What the device bridge answers: an action's outcome and a route's refusal.

Pure: each function builds the payload one outcome is journaled or answered
with (`bridge_route`). An outcome the hub cannot confirm is `unknown` and is
never retried: the action may have run on the Mac.
"""

from .assistant.access import disabled_message
from .errors import render
from .private_route import json_response

RESULT_PROTOCOL = "orbit_device_bridge_result_v1"
_DO_NOT_REPEAT = "No repitas la acción; revisa su efecto en el Mac."


def publish_failed() -> dict:
    """The Mac never got the action."""
    return {
        "outcome": "failed",
        "code": "connection_failed",
        "notice": "El Mac no recibió la petición de acción.",
        "next_step": "Comprueba la conexión de Orbit antes de preparar otra acción.",
    }


def reply_timeout(deadline_ms: int) -> dict:
    """The Mac did not confirm the action within its deadline."""
    expected = "respuesta de perform_action"
    notice = f"El Mac no confirmó la acción en {deadline_ms} ms; el resultado es incierto."
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
        "notice": "La respuesta se perdió después de enviar la acción.",
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
        "failed": "La acción falló",
        "unknown": "No se confirmó el resultado; comprueba su efecto en el Mac",
        "expired": "La operación caducó",
        "rejected": "La acción fue rechazada",
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
                "notice": "El Mac no pudo confirmar el resultado de la acción.",
                "next_step": "No repitas la acción; consulta su resultado y revisa su efecto en el Mac.",
            }
        return {
            "outcome": "failed",
            "code": refusal["code"],
            "refusal": refusal,
            "notice": "El Mac no completó la acción.",
            "next_step": "Consulta la causa antes de preparar otra acción.",
        }
    result = answer.get("result")
    receipt = result.get("receipt") if isinstance(result, dict) else None
    if not isinstance(receipt, dict):
        return {
            "outcome": "unknown",
            "result": result,
            "code": "invalid_action_receipt",
            "notice": "El Mac devolvió un recibo de acción no válido.",
            "next_step": _DO_NOT_REPEAT,
        }
    state = receipt.get("state")
    code = receipt.get("code")
    if state == "performed":
        return {"outcome": "performed", "receipt": receipt, "result": result}
    return {
        "outcome": "failed",
        "code": code if isinstance(code, str) else "action_failed",
        "notice": "El Mac no pudo ejecutar la acción.",
        "next_step": "Revisa Orbit y prepara otra acción sólo si no llegó a enviarse.",
        "receipt": receipt,
        "result": result,
    }


def failure(exc):
    status, code, numbers, detail = exc.status, exc.code, exc.numbers, exc.detail
    message = {
        "unauthorized": "Acceso no autorizado",
        "invalid_request": "Solicitud fuera de contrato",
        "no_answer": "El Mac no respondió dentro del plazo",
        "device_unavailable": "El Mac no está conectado",
        "agent_not_found": "Este hub no tiene ese agente",
    }.get(code, "No se pudo completar la petición")
    if code == "agent_access_disabled" and numbers is not None:
        message = disabled_message(str(numbers.get("agent_id")), str(numbers.get("group")))
    if code == "skill_not_enabled" and numbers is not None:
        enabled = str(numbers.get("enabled")).replace(",", ", ") or "ninguna"
        message = (
            f"El agente «{numbers.get('agent_id')}» no tiene activada ninguna skill llamada "
            f"«{numbers.get('name')}»; las activadas son: {enabled}"
        )
    if code == "skills_not_in_hub" and numbers is not None:
        message = (
            f"El agente «{numbers.get('agent_id')}» es de tipo {numbers.get('kind')}: sus skills "
            "son las de su perfil de Hermes, no las guarda Orbit"
        )
    if detail is not None:
        message = f"El parámetro {detail['field']} debe ser {detail['expected']}."
    payload = render(
        code,
        message,
        payload={"protocol": RESULT_PROTOCOL, "state": "unavailable"},
        numbers=numbers,
    )
    if detail is not None:
        payload["refusal"].update(detail)
    return json_response(payload, status)
