"""Redacted causes for both transcription transports."""

import httpx

from .errors import HubError


def from_provider(error: Exception, *, timeout_ms: int, connect_timeout_ms: int) -> HubError:
    if isinstance(error, httpx.HTTPStatusError):
        status = error.response.status_code
        return HubError(
            "transcription_http_error",
            f"El servicio de transcripción devolvió HTTP {status}",
            http_status=502,
            numbers={"provider_status": status},
        )
    if isinstance(error, httpx.ConnectTimeout):
        return HubError(
            "transcription_connection_timeout",
            f"La conexión con el servicio de transcripción tardó más de {connect_timeout_ms} ms",
            http_status=504,
            numbers={"timeout_ms": connect_timeout_ms},
        )
    if isinstance(error, httpx.TimeoutException):
        return HubError(
            "transcription_timeout",
            f"El servicio de transcripción tardó más de {timeout_ms} ms",
            http_status=504,
            numbers={"timeout_ms": timeout_ms},
        )
    if isinstance(error, httpx.RemoteProtocolError):
        return HubError(
            "transcription_protocol_error",
            "El servicio de transcripción cerró la respuesta sin completarla",
            http_status=502,
        )
    if isinstance(error, httpx.ConnectError):
        return HubError(
            "transcription_connection_error",
            "No se pudo conectar con el servicio de transcripción",
            http_status=503,
        )
    if isinstance(error, (ValueError, TypeError)):
        return HubError(
            "transcription_invalid_response",
            "El servicio de transcripción devolvió una respuesta inválida",
            http_status=502,
        )
    return HubError(
        "transcription_provider_error",
        "El servicio de transcripción falló antes de entregar una respuesta válida",
        http_status=502,
    )


def durable_code(error: HubError) -> str:
    if error.code == "transcription_http_error" and error.numbers:
        return f"transcription_http_{error.numbers['provider_status']}"
    if error.code == "transcription_timeout" and error.numbers:
        return f"transcription_timeout_{error.numbers['timeout_ms']}"
    if error.code == "transcription_connection_timeout" and error.numbers:
        return f"transcription_connection_timeout_{error.numbers['timeout_ms']}"
    return error.code


def durable_details(code: str | None) -> dict[str, int | str]:
    if code is None:
        return {"cause": "transcription_cause_missing"}
    if code.startswith("transcription_http_"):
        status = code.removeprefix("transcription_http_")
        if status.isdecimal() and 100 <= int(status) <= 599:
            return {"cause": "transcription_http_error", "provider_status": int(status)}
    if code.startswith("transcription_timeout_"):
        timeout = code.removeprefix("transcription_timeout_")
        if timeout.isdecimal():
            return {"cause": "transcription_timeout", "timeout_ms": int(timeout)}
    if code.startswith("transcription_connection_timeout_"):
        timeout = code.removeprefix("transcription_connection_timeout_")
        if timeout.isdecimal():
            return {"cause": "transcription_connection_timeout", "timeout_ms": int(timeout)}
    return {"cause": code}
