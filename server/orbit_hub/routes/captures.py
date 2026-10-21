"""Capture and recording chunk upload routes."""

import hashlib
import io
import re
import wave

from fastapi import Request

from ..captures import MAX_CAPTURE_BYTES
from ..errors import HubError
from ..identifiers import canonical_uuid
from ..log import in_store


def mount(
    app,
    store,
    identity,
    bounded_body,
    validate_wav,
    settings,
    captures,
    store_executor,
    prepare_signal,
    now,
):
    @app.put("/v1/captures/{capture_id}")
    async def capture(capture_id: str, request: Request):
        device = identity(request)
        try:
            capture_id = canonical_uuid(capture_id)
            request_id = request.headers["x-orbit-request"]
            digest = request.headers["x-orbit-sha256"]
            if (
                not 1 <= len(request_id) <= 160
                or any(ord(character) < 32 or ord(character) == 127 for character in request_id)
                or not re.fullmatch(r"[a-f0-9]{64}", digest)
            ):
                raise ValueError("Invalid capture metadata")
        except (KeyError, ValueError) as exc:
            raise HubError("invalid_capture", "Metadatos de captura no válidos") from exc
        if request.headers.get("content-type") != "image/png":
            raise HubError(
                "unsupported_capture_type", "Se requiere una imagen PNG", http_status=415
            )
        body = await bounded_body(request, MAX_CAPTURE_BYTES)
        if hashlib.sha256(body).hexdigest() != digest:
            raise HubError("invalid_capture", "El resumen de la captura no coincide")
        stored = captures.put(device, capture_id, request_id, body)
        if stored.cause == "full":
            raise HubError(
                "too_many",
                "Demasiadas capturas pendientes",
                http_status=429,
                numbers={"resource": "capture", "count": stored.count, "max": stored.max},
            )
        if stored.cause == "duplicate":
            raise HubError("capture_duplicate", "La captura ya está guardada", http_status=409)
        return {"capture_id": capture_id, "stored": True}

    @app.put("/v1/chunks/{chunk_id}")
    async def chunk(chunk_id: str, request: Request):
        device = identity(request)
        try:
            if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:-]{0,120}", chunk_id):
                raise ValueError("Invalid chunk ID")
            recording = canonical_uuid(request.headers["x-orbit-recording"])
            sequence = int(request.headers["x-orbit-sequence"])
            track = request.headers["x-orbit-track"]
            digest = request.headers["x-orbit-sha256"]
            if (
                not 0 <= sequence <= 2**53
                or track not in {"mic_local", "remote_app"}
                or not re.fullmatch(r"[a-f0-9]{64}", digest)
            ):
                raise ValueError("Invalid metadata")
        except (KeyError, ValueError) as exc:
            raise HubError("invalid_chunk", "Metadatos de fragmento no válidos") from exc
        body = await bounded_body(request, settings.max_chunk_bytes)
        validate_wav(body)
        metadata = {}
        fields = ('started_at_ms', 'monotonic_ns', 'first_sample', 'frames', 'sample_rate', 'channels')  # fmt: skip
        try:
            for field in fields:
                header = "x-orbit-" + field.replace("_", "-")
                if header in request.headers:
                    value = int(request.headers[header])
                    if not 0 <= value <= 2**63 - 1:
                        raise ValueError("Invalid capture metadata")
                    metadata[field] = value
            if metadata and set(metadata) != set(fields):
                raise ValueError("Capture metadata must be complete")
            if "x-orbit-capture-epoch-id" in request.headers:
                epoch = request.headers["x-orbit-capture-epoch-id"]
                if set(metadata) != set(fields):
                    raise ValueError("Epoch requires complete native timing")
                metadata["capture_epoch_id"] = canonical_uuid(epoch)
            with wave.open(io.BytesIO(body), "rb") as audio:
                if metadata and (
                    metadata["frames"],
                    metadata["sample_rate"],
                    metadata["channels"],
                ) != (audio.getnframes(), audio.getframerate(), audio.getnchannels()):
                    raise ValueError("Metadata does not match WAV")
        except ValueError as exc:
            raise HubError(
                "invalid_chunk", "Metadatos de reloj/formato incompletos o distintos del WAV"
            ) from exc
        metadata["wall_time_precision"] = (
            "estimated_reception" if "started_at_ms" in metadata else "unknown"
        )
        if not store.is_active(device):
            raise HubError("revoked", "Dispositivo revocado")
        outcome = await in_store(
            store_executor,
            store.put_chunk,
            device,
            chunk_id,
            recording,
            track,
            sequence,
            digest,
            body,
            metadata,
            received_at=now(),
        )
        prepare_signal.set()
        return outcome
