"""Version-specific Hermes transport. Device credentials never reach Hermes."""

import asyncio
import base64
import json
import re
from collections.abc import Awaitable, Callable
from contextlib import suppress
from dataclasses import dataclass
from urllib.parse import urlencode
from uuid import uuid4

import httpx
from websockets.asyncio.client import connect

from .config import Settings
from .log import clear_op, logger, report
from .protocol import HubError
from .secret_file import read_secret
from .transcription_failure import from_provider

# How Hermes begins the error of a resume whose stored transcript it could not
# read; the rest of its message is Hermes's detail and is never relayed.
RESUME_HISTORY_ERROR_PREFIX = "Could not reopen that session (its transcript could not be read)."
MAX_SPEECH_CLIP_BYTES = 4 * 1024 * 1024
MAX_SPEECH_RESPONSE_BYTES = 6 * 1024 * 1024
# Existing 180 s provider and 10 s connection budgets, shared with the
# refusal's millisecond numbers. They bound one dictation call, not meeting
# ASR, queue wait, or an automatic retry.
TRANSCRIPTION_TIMEOUT_SECONDS = 180
TRANSCRIPTION_CONNECT_TIMEOUT_SECONDS = 10


@dataclass(frozen=True)
class SpeechClip:
    mime_type: str
    provider: str
    body: bytes


class HermesConnection:
    def __init__(self, adapter: "HermesAdapter", on_event: Callable[[dict], Awaitable[None]]):
        self.adapter, self.on_event = adapter, on_event
        self.epoch = ""
        self.live_sessions: set[str] = set()
        self.device_name: str | None = None
        self._ws = None
        self._reader = None
        self._pending: dict[str, asyncio.Future] = {}
        self._ready = asyncio.Event()
        self._closing = False

    @property
    def connected(self) -> bool:
        return self._reader is not None and not self._reader.done()

    def bind_device(self, device_name: str) -> None:
        if not 1 <= len(device_name) <= 80 or any(ord(char) < 32 for char in device_name):
            raise HubError("configuration", "Nombre de dispositivo no válido")
        self.device_name = device_name

    async def open(self) -> None:
        try:
            self._ws = await connect(
                self.adapter.ws_url("/api/ws"),
                max_size=4 * 1024 * 1024,
                max_queue=32,
                open_timeout=10,
                close_timeout=2,
            )
            self._reader = asyncio.create_task(self._read())
            await asyncio.wait_for(self._ready.wait(), 15)
            if not self.epoch:
                raise ValueError("Missing gateway.ready replay_epoch")
        except Exception as exc:
            report(event="hermes_connect_failed", code="upstream_unavailable")
            await self.close()
            raise HubError("upstream_unavailable", "No se pudo conectar con Hermes") from exc

    async def _read(self) -> None:
        # The connection outlives the request that opened it.
        clear_op()
        try:
            async for raw in self._ws:
                if not isinstance(raw, str):
                    continue
                frame = json.loads(raw)
                if not isinstance(frame, dict):
                    continue
                if frame.get("method") == "event":
                    params = frame.get("params", {})
                    if params.get("type") == "gateway.ready":
                        self.epoch = str(params.get("payload", {}).get("replay_epoch") or "")
                        self._ready.set()
                    else:
                        await self.on_event(params)
                elif (future := self._pending.pop(str(frame.get("id")), None)) is not None:
                    if not future.done():
                        future.set_result(frame)
        except Exception:  # noqa: BLE001 - failure becomes a typed disconnect event in finally
            # Exceptions may contain auth query strings; no raw upstream logging.
            report(event="hermes_reader_failed", code="upstream_lost")
            self.epoch = ""
        finally:
            self._ready.set()
            self.live_sessions.clear()
            for future in self._pending.values():
                if not future.done():
                    future.set_exception(
                        HubError(
                            "outcome_unknown",
                            "Conexión Hermes perdida; no se repetirá la operación",
                        )
                    )
            self._pending.clear()
            if not self._closing:
                await self.on_event({"type": "orbit.upstream_lost"})

    async def rpc(self, method: str, params: dict) -> dict:
        if not self.connected:
            raise HubError("upstream_unavailable", "Hermes no está conectado")
        if method not in {
            "session.create",
            "session.resume",
            "session.status",
            "prompt.submit",
            "session.interrupt",
            "approval.respond",
            "clarify.respond",
        }:
            raise HubError("method_forbidden", "Método Hermes fuera de contrato")
        request_id = str(uuid4())
        future = asyncio.get_running_loop().create_future()
        self._pending[request_id] = future
        # Existing RPC receipt budget; it bounds one acknowledgement, not a model turn.
        rpc_timeout_seconds = 30
        try:
            await self._ws.send(
                json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
            )
            frame = await asyncio.wait_for(future, rpc_timeout_seconds)
            if "error" in frame:
                error = frame["error"] if isinstance(frame["error"], dict) else {}
                code = error.get("code")
                if method == "prompt.submit" and code == 4001:
                    self.live_sessions.discard(params.get("session_id"))
                    raise HubError(
                        "session_runtime_not_found",
                        "Hermes no encontró la sesión activa (código 4001); "
                        "el mensaje no se aceptó y puedes volver a enviarlo tras reanudar",
                    )
                if method == "session.resume":
                    reason = error.get("message")
                    if code == -32601:
                        raise HubError(
                            "session_resume_unsupported",
                            "Esta versión de Hermes no permite reanudar sesiones guardadas "
                            "(código -32601)",
                        )
                    if code == 4007 and reason == "session not found":
                        raise HubError(
                            "session_resume_not_found",
                            "Hermes no conserva la conversación guardada (código 4007); "
                            "el historial local sigue disponible",
                        )
                    if code == 4007 and reason == "session no longer live; retry resume":
                        raise HubError(
                            "session_resume_race",
                            "Hermes cerró la sesión durante la reanudación (código 4007); "
                            "vuelve a intentarlo",
                        )
                    if code == 4130:
                        numbers = (
                            re.search(r"\((\d+) messages; limit (\d+)\)", reason)
                            if isinstance(reason, str)
                            else None
                        )
                        if numbers is not None:
                            raise HubError(
                                "session_resume_too_large",
                                f"Hermes no puede reanudar {numbers[1]} mensajes; "
                                f"límite {numbers[2]} (código 4130)",
                            )
                        raise HubError(
                            "session_resume_too_large",
                            "Hermes rechazó reanudar por tamaño (código 4130); "
                            "no entregó cuenta y límite",
                        )
                    if code == 4009 and reason == "session disconnect interrupt settling":
                        raise HubError(
                            "session_resume_busy",
                            "Hermes sigue cerrando el turno anterior (código 4009); espera",
                        )
                    if (
                        code == 5000
                        and isinstance(reason, str)
                        and reason.startswith("Session storage is unavailable:")
                    ):
                        raise HubError(
                            "session_resume_storage_unavailable",
                            "Hermes no puede abrir su almacén de sesiones (código 5000)",
                        )
                    if (
                        code == 5000
                        and isinstance(reason, str)
                        and reason.startswith(RESUME_HISTORY_ERROR_PREFIX)
                    ):
                        raise HubError(
                            "session_resume_history_unreadable",
                            "Hermes no pudo leer el historial guardado (código 5000); "
                            "no se envió el mensaje",
                        )
                raise HubError("upstream_rejected", f"Hermes rechazó la operación (código {code})")
            if not isinstance(result := frame.get("result"), dict):
                raise HubError("upstream_protocol", "Respuesta Hermes no válida")
            return result
        except TimeoutError as exc:
            if method == "session.resume":
                raise HubError(
                    "session_resume_timeout",
                    f"Hermes no confirmó la reanudación; plazo de {rpc_timeout_seconds} segundos; "
                    "no se envió el mensaje",
                ) from exc
            raise HubError(
                "outcome_unknown", "Hermes no confirmó la operación; no se repetirá"
            ) from exc
        finally:
            self._pending.pop(request_id, None)

    async def create_session(self) -> dict:
        title = "Orbit" if self.device_name is None else f"Orbit · {self.device_name}"
        result = await self.rpc(
            "session.create",
            {
                "profile": self.adapter.settings.profile,
                "close_on_disconnect": False,
                "title": title,
                "source": "orbit",
                "follow_profile_config": True,
            },
        )
        if not result.get("session_id") or not result.get("stored_session_id"):
            raise HubError("upstream_protocol", "Hermes omitió la identidad durable")
        self.live_sessions.add(result["session_id"])
        return result

    async def resume_session(self, stored_session_id: str) -> dict:
        try:
            result = await self.rpc(
                "session.resume",
                {
                    "profile": self.adapter.settings.profile,
                    "session_id": stored_session_id,
                    "source": "orbit",
                    "close_on_disconnect": False,
                    # Bound the WS reply while Hermes hydrates model history.
                    # omit_messages=True would load only the tip segment.
                    "defer_history": True,
                    "omit_messages": False,
                },
            )
        except HubError as exc:
            if exc.code == "outcome_unknown":
                raise HubError(
                    "session_resume_connection_lost",
                    "Se perdió Hermes durante la reanudación; no se envió el mensaje",
                ) from exc
            raise
        runtime_id = result.get("session_id")
        if not isinstance(runtime_id, str) or not runtime_id:
            raise HubError("upstream_protocol", "Hermes omitió la sesión reanudada")
        resumed = result.get("resumed") or result.get("stored_session_id") or stored_session_id
        if not isinstance(resumed, str) or not resumed:
            raise HubError("upstream_protocol", "Hermes omitió la identidad guardada")
        self.live_sessions.add(runtime_id)
        count = result.get("message_count")
        hydrating = result.get("hydrating")
        return {
            "session_id": runtime_id,
            "stored_session_id": resumed,
            **({"message_count": count} if type(count) is int and count > 0 else {}),
            **({"hydrating": hydrating} if type(hydrating) is bool else {}),
        }

    async def close(self) -> None:
        self._closing = True
        try:
            if self._ws is not None:
                with suppress(TimeoutError):
                    await asyncio.wait_for(self._ws.close(), 2.5)
        finally:
            if self._reader is not None and self._reader is not asyncio.current_task():
                self._reader.cancel()
                await asyncio.gather(self._reader, return_exceptions=True)


class HermesAdapter:
    def __init__(self, settings: Settings):
        self.settings = settings

    def credential(self) -> str:
        value = read_secret(self.settings.hermes_credential_file, 64 * 1024).decode().strip()
        if not value or len(value) > 4096:
            raise HubError("configuration", "Credencial Hermes ausente o no válida")
        return value

    def ws_url(self, path: str) -> str:
        base = self.settings.hermes_url.rstrip("/")
        base = ("wss" if base.startswith("https:") else "ws") + base[base.index(":") :]
        return base + path + "?" + urlencode({"token": self.credential()})

    def connection(self, callback: Callable[[dict], Awaitable[None]]) -> HermesConnection:
        return HermesConnection(self, callback)

    async def transcribe(self, audio: bytes) -> str:
        async with httpx.AsyncClient(
            timeout=httpx.Timeout(
                TRANSCRIPTION_TIMEOUT_SECONDS, connect=TRANSCRIPTION_CONNECT_TIMEOUT_SECONDS
            ),
            follow_redirects=False,
        ) as client:
            try:
                response = await client.post(
                    self.settings.hermes_url.rstrip("/") + "/api/audio/transcribe",
                    params={"profile": self.settings.profile},
                    headers={"X-Hermes-Session-Token": self.credential()},
                    json={
                        "data_url": "data:audio/wav;base64," + base64.b64encode(audio).decode(),
                        "mime_type": "audio/wav",
                    },
                )
                response.raise_for_status()
                payload = response.json()
                if payload.get("ok") is not True or not isinstance(payload.get("transcript"), str):
                    raise ValueError("Invalid transcription response")
                return payload["transcript"]
            except Exception as exc:
                report(event="hermes_transcription_failed", code="transcription_failed")
                raise from_provider(
                    exc,
                    timeout_ms=TRANSCRIPTION_TIMEOUT_SECONDS * 1000,
                    connect_timeout_ms=TRANSCRIPTION_CONNECT_TIMEOUT_SECONDS * 1000,
                ) from exc

    async def synthesize(self, text: str) -> SpeechClip:
        try:
            async with (
                httpx.AsyncClient(
                    timeout=httpx.Timeout(180, connect=10), follow_redirects=False
                ) as client,
                client.stream(
                    "POST",
                    self.settings.hermes_url.rstrip("/") + "/api/audio/speak",
                    headers={"X-Hermes-Session-Token": self.credential()},
                    json={"text": text, "profile": self.settings.profile},
                ) as response,
            ):
                response.raise_for_status()
                body = bytearray()
                async for chunk in response.aiter_bytes():
                    if len(chunk) > MAX_SPEECH_RESPONSE_BYTES - len(body):
                        raise HubError(
                            "speech_too_large", "El clip de voz supera el límite de 4 MiB"
                        )
                    body.extend(chunk)
            payload = json.loads(body)
            mime_type = payload.get("mime_type")
            provider = payload.get("provider")
            data_url = payload.get("data_url")
            if (
                payload.get("ok") is not True
                or not isinstance(mime_type, str)
                or not 1 <= len(mime_type) <= 128
                or any(ord(character) < 33 or ord(character) > 126 for character in mime_type)
                or not isinstance(provider, str)
                or not 1 <= len(provider) <= 128
                or any(ord(character) < 32 or ord(character) > 126 for character in provider)
                or not isinstance(data_url, str)
            ):
                raise ValueError("Invalid speech response")
            prefix, separator, encoded = data_url.partition(",")
            if separator != "," or prefix != f"data:{mime_type};base64":
                raise ValueError("Invalid speech data URL")
            maximum_encoded = 4 * ((MAX_SPEECH_CLIP_BYTES + 2) // 3)
            if len(encoded) > maximum_encoded:
                raise HubError("speech_too_large", "El clip de voz supera el límite de 4 MiB")
            audio = base64.b64decode(encoded, validate=True)
            if not audio:
                raise ValueError("Empty speech clip")
            if len(audio) > MAX_SPEECH_CLIP_BYTES:
                raise HubError("speech_too_large", "El clip de voz supera el límite de 4 MiB")
            logger.info(
                "hermes_speech_clip",
                extra={"event": "hermes_speech_clip", "code": f"{provider}|{mime_type}"},
            )
            return SpeechClip(mime_type, provider, audio)
        except HubError:
            raise
        except Exception as exc:
            report(event="hermes_speech_clip_failed", code="speech_unavailable")
            raise HubError(
                "speech_unavailable", "Hermes no entregó el clip de voz configurado"
            ) from exc
