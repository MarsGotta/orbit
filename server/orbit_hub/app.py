"""HTTP/WebSocket boundary: bounded input, authenticated device ownership, no admin API."""

import asyncio
import fcntl
import io
import json
import os
import time
import wave
from concurrent.futures import ThreadPoolExecutor
from contextlib import asynccontextmanager
from importlib.metadata import PackageNotFoundError
from importlib.metadata import version as installed_version

from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse, Response
from starlette.middleware.trustedhost import TrustedHostMiddleware

from . import error_reports
from .assistant.access import AgentAccess
from .assistant.agent_config import AgentConfigStore
from .assistant.agents import Agents
from .assistant.backend import AssistantBackend
from .assistant.speech import SpeechBackend
from .bridge_route import BridgeService
from .bridge_route import mount as mount_device_bridge
from .captures import CaptureStore
from .config import Settings
from .device_bridge import Bridge as DeviceBridge
from .errors import http_response, render
from .log import bind_op, report, supervised
from .meeting_final_pass import FinalPassWorker
from .meeting_transcription import MeetingTranscriber
from .meetings.routes import mount as mount_meetings
from .meetings_bridge.routes import mount as mount_meetings_bridge
from .meetings_bridge.service import MeetingsBridgeService
from .notes import NoteRepository, NotesWorker
from .notes import mount as mount_notes
from .processing import open_client as open_processing_client
from .protocol import HubError, emit
from .runtime import DeviceRuntime, TurnInputTracker
from .storage import Store
from .transcription import TranscriptionWorker
from .urls import LOOPBACK_HOSTS


def hub_version() -> str:
    """What this hub reports about itself, read from its installed metadata.

    A literal here was a second copy of the number with nothing keeping it in
    step, and it sat one release behind the rest of the repository. A checkout
    that was never installed has no metadata to read, and saying so is honest
    where naming a version this build may not be is not.
    """
    try:
        return installed_version("orbit-hub")
    except PackageNotFoundError:
        return "desconocida"


def bearer(headers) -> str:
    value = headers.get("authorization", "")
    if not value.startswith("Bearer "):
        raise HubError("unauthorized", "Credencial de dispositivo requerida")
    return value[7:]


def allowed_transport(scope, settings: Settings) -> bool:
    if scope["scheme"] in {"https", "wss"}:
        return True
    host = scope.get("client", ("",))[0]
    return settings.allow_insecure_localhost and host in LOOPBACK_HOSTS


async def bounded_body(request: Request, maximum: int, resource: str = "hub_request") -> bytes:
    """The request's body, refused past `maximum` bytes with what it was.

    `resource` names the body in the refusal: `capture`, `recording` or `audio`
    for a route of one of those (a chunk, a manifest, a WAV or a text to speak),
    `hub_request` for any other body.
    `bytes` is what had arrived when the ceiling was crossed, not the whole body.
    """
    body = bytearray()
    async for piece in request.stream():
        if len(body) + len(piece) > maximum:
            raise HubError(
                "too_large",
                "Carga demasiado grande",
                numbers={
                    "resource": resource,
                    "bytes": len(body) + len(piece),
                    "max_bytes": maximum,
                },
            )
        body.extend(piece)
    if not body:
        raise HubError("invalid_payload", "Carga vacía")
    return bytes(body)


def validate_wav(body: bytes) -> None:
    try:
        with wave.open(io.BytesIO(body), "rb") as audio:
            channels, width, rate = audio.getnchannels(), audio.getsampwidth(), audio.getframerate()
            if channels not in {1, 2} or width != 2 or not 8000 <= rate <= 192000:
                raise ValueError("Unsupported PCM")
            expected = audio.getnframes() * channels * width
            if expected == 0 or len(audio.readframes(audio.getnframes())) != expected:
                raise ValueError("Truncated WAV")
    except (wave.Error, EOFError, ValueError) as exc:
        raise HubError("invalid_audio", "Se requiere WAV PCM de 16 bits válido") from exc


def create_app(
    settings: Settings,
    *,
    store: Store | None = None,
    assistant: AssistantBackend | None = None,
    speech: SpeechBackend | None = None,
    now=time.time,
    processing_client=None,
) -> FastAPI:
    owned_store = store is None
    store = store or Store(settings.data_dir, settings.key_file, settings.storage_quota_bytes)
    # `assistant` and `speech`, when given, serve every configured agent: tests
    # drive the hub with doubles.
    # Each agent's personality, skills and access groups (Settings › Agents).
    configs = AgentConfigStore(store)
    agents = Agents.from_settings(settings, assistant, speech, configs)
    access = AgentAccess(agents, configs)
    # Dictation and read-aloud are the default agent's: neither route names a
    # conversation (contracts/hub-v1.md), so neither can name its agent.
    speech = agents.default.speech
    runtimes: dict[str, DeviceRuntime] = {}
    turn_inputs = TurnInputTracker(now)
    # Questions a remote assistant has put to a Mac, and the callers waiting on
    # them. Kept beside the runtimes rather than inside one: a device that
    # disconnects has to take its outstanding questions with it, and that is
    # easier to get right when they are not owned by the connection.
    bridge = DeviceBridge()
    bridge_waiters: dict[str, asyncio.Future] = {}
    captures = CaptureStore(now=now)
    # One thread for the Store's heavy work: hashing and encrypting uploads,
    # assembling a meeting group, reading transcripts. Small transactions
    # (a message, a status, a receipt) still run on the loop under the
    # Store's lock. A single worker keeps these writes ordered for the WAL.
    store_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="orbit-store")
    # Fired whenever a new chunk is persisted or a recording is finalized, so
    # the transcription worker can react to fresh work without polling.
    prepare_signal: asyncio.Event = asyncio.Event()

    device_bridge = BridgeService(
        settings.device_bridge,
        runtimes,
        bridge,
        bridge_waiters,
        lambda: now() * 1000,
        captures,
        store,
        access,
    )
    pair_attempts: dict[str, tuple[float, int]] = {}
    # Durable meetings use hub-owned STT; Hermes remains interactive-only.
    meeting_transcriber = MeetingTranscriber(settings.meeting_transcription)
    # What a transcript group stored without a language reads as.
    meeting_language = meeting_transcriber.configuration.language
    meetings_bridge = MeetingsBridgeService(
        store, settings.meetings_bridge, now, default_language=meeting_language, access=access
    )

    def recording_changed(device: str, recording: str) -> None:
        """Tell the owning device's connections, and no other, to read it again."""
        runtime = runtimes.get(device)
        if runtime is not None:
            runtime.offer(emit("recording.changed", {"recording_id": recording}))

    # Fired when a transcript group finishes, so a note waiting for the last
    # group is written at once instead of on the next tick.
    notes_signal: asyncio.Event = asyncio.Event()
    if processing_client is None and settings.processing is not None:
        processing_client = open_processing_client(settings.processing)
    notes = NotesWorker(
        NoteRepository(store),
        now,
        signal=notes_signal,
        store_executor=store_executor,
        client=processing_client,
        default_language=meeting_language,
        recording_changed=recording_changed,
    )

    final_passes = FinalPassWorker(
        store,
        meeting_transcriber,
        settings.diarization,
        notes_signal,
        recording_changed,
        store_executor,
    )
    transcriptions = TranscriptionWorker(
        store,
        speech,
        now,
        meetings=meeting_transcriber,
        prepare_signal=prepare_signal,
        store_executor=store_executor,
        group_finished=notes_signal,
        recording_changed=recording_changed,
    )

    async def enforce_revocations():
        # A viewer can disconnect while its upstream turn remains active. Revocation
        # therefore belongs to the server lifespan, never to an individual socket.
        while True:
            revoked = [
                (device, runtime)
                for device, runtime in list(runtimes.items())
                if not store.is_active(device)
            ]
            if revoked:
                await asyncio.gather(*(runtime.revoke() for _, runtime in revoked))
                for device, runtime in revoked:
                    if runtimes.get(device) is runtime:
                        runtimes.pop(device)
            await asyncio.sleep(1)

    async def shutdown(descriptor, revocations):
        # This task, including Store.close below, has a single owner and is
        # never cancelled by a lifespan waiter.
        try:
            if revocations is not None:
                revocations.cancel()
                await asyncio.gather(revocations, return_exceptions=True)
            await final_passes.close()
            await transcriptions.close()
            await notes.close()
            await asyncio.gather(*(runtime.close() for runtime in runtimes.values()))
        finally:
            try:
                device_bridge.close()
                meetings_bridge.close()
            finally:
                try:
                    store_executor.shutdown(wait=False)
                    os.close(descriptor)
                    if owned_store:
                        store.close()
                finally:
                    if settings.error_reports is not None:
                        error_reports.stop()

    @asynccontextmanager
    async def lifespan(app):
        if settings.error_reports is not None:
            error_reports.start(settings.error_reports, release=f"orbit-hub@{hub_version()}")
        descriptor = os.open(
            settings.data_dir / "hub.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600
        )
        revocations = None
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            store.recover_interrupted()
            device_bridge.start()
            meetings_bridge.start()
            transcriptions.start()
            final_passes.start()
            notes.start()
            revocations = asyncio.create_task(
                supervised("enforce_revocations", enforce_revocations)
            )
            yield
        finally:
            physical = asyncio.create_task(shutdown(descriptor, revocations))
            cancelled = False
            while not physical.done():
                try:
                    await asyncio.shield(physical)
                except asyncio.CancelledError:
                    cancelled = True
            # Retrieve the actual shutdown outcome first. An internal failure
            # must not be mistaken for cancellation of this external waiter.
            physical.result()
            if cancelled:
                raise asyncio.CancelledError

    app = FastAPI(
        title="Orbit Hub",
        version=hub_version(),
        docs_url=None,
        redoc_url=None,
        lifespan=lifespan,
    )
    app.add_middleware(TrustedHostMiddleware, allowed_hosts=list(settings.allowed_hosts))
    app.state.store = store
    app.state.notes = notes
    app.state.device_bridge = device_bridge

    @app.exception_handler(HubError)
    async def hub_error(_request, exc):
        return http_response(
            exc.code,
            exc.message,
            exc.http_status,
            numbers=exc.numbers,
        )

    @app.exception_handler(RequestValidationError)
    async def invalid_request(_request, _exc):
        # Pydantic's default errors echo submitted values (including pairing codes).
        return http_response("invalid_request", "Solicitud fuera de contrato", 422)

    @app.middleware("http")
    async def boundary(request, call_next):
        # The one handler for whatever a route did not foresee, and the place
        # every HTTP error body gets the op of its request.
        op = bind_op(request.headers.get("x-orbit-op"))
        if not allowed_transport(request.scope, settings):
            return http_response("tls_required", "Se requiere HTTPS", 403, payload={"op": op})
        if request.headers.get("origin"):
            return http_response(
                "origin_forbidden", "API exclusiva del cliente Orbit", 403, payload={"op": op}
            )
        try:
            response = await call_next(request)
        except Exception:  # noqa: BLE001 - redact arbitrary adapter/storage exceptions at HTTP boundary
            # The template, never the path: a path holds the ids of one request.
            route = getattr(request.scope.get("route"), "path", None)
            report("internal_error", "internal_error", route=route, status=500)
            return http_response(
                "internal_error",
                "Error del servidor; conserva la copia local",
                500,
                payload={"op": op},
            )
        if response.status_code >= 400 and response.headers.get("content-type", "").startswith(
            "application/json"
        ):
            body = b"".join([chunk async for chunk in response.body_iterator])
            headers = {
                key: value
                for key, value in response.headers.items()
                if key.lower() != "content-length"
            }
            try:
                existing = json.loads(body)
            except (ValueError, UnicodeError):
                existing = None
            code = existing.get("code") if isinstance(existing, dict) else None
            if code is None and response.status_code in {405, 422}:
                code = "invalid_request"
            if isinstance(existing, dict) and ("refusal" in existing or isinstance(code, str)):
                payload = existing
                if "refusal" not in existing:
                    message = existing.get("message") or existing.get("notice") or code
                    if not isinstance(message, str):
                        message = code
                    if "message" not in existing and "notice" not in existing:
                        message = "Solicitud fuera de contrato"
                    payload = render(code, message, payload=existing)
                response = JSONResponse(
                    {**payload, "op": op}, status_code=response.status_code, headers=headers
                )
            else:
                response = Response(content=body, status_code=response.status_code, headers=headers)
        response.headers["Cache-Control"] = "no-store"
        response.headers["X-Content-Type-Options"] = "nosniff"
        return response

    def identity(request) -> str:
        return store.authenticate(bearer(request.headers))

    def runtime_for(device: str) -> DeviceRuntime:
        """One runtime per device, whichever authenticated socket asks first."""
        return runtimes.setdefault(
            device,
            DeviceRuntime(
                device,
                store,
                agents,
                now,
                store_executor=store_executor,
                input_tracker=turn_inputs,
            ),
        )

    mount_notes(app, notes, identity, bounded_body, store_executor, now, recording_changed)
    mount_meetings(
        app,
        store,
        identity,
        bounded_body,
        store_executor,
        now,
        notes,
        meeting_language,
        recording_changed,
        prepare_signal,
        settings.diarization is not None,
    )
    mount_device_bridge(app, device_bridge, lambda scope: allowed_transport(scope, settings))
    mount_meetings_bridge(app, meetings_bridge, lambda scope: allowed_transport(scope, settings))

    from .routes import agents as agent_routes
    from .routes import audio, health, recordings, session
    from .routes import captures as capture_routes
    from .ws import mount as mount_ws

    session.mount(app, store, agents, identity, bounded_body, now, pair_attempts, store_executor)
    health.mount(app, settings)
    agent_routes.mount(app, settings, agents, configs, identity, bounded_body)
    audio.mount(
        app,
        store,
        identity,
        bounded_body,
        validate_wav,
        settings,
        agents.default,
        transcriptions,
        turn_inputs,
    )
    capture_routes.mount(
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
    )
    recordings.mount(
        app,
        store,
        identity,
        bounded_body,
        store_executor,
        prepare_signal,
        now,
        notes,
        recording_changed,
        settings.diarization is not None,
    )
    mount_ws(
        app,
        settings,
        agents,
        identity,
        allowed_transport,
        runtime_for,
        store,
        now,
        bridge,
        bridge_waiters,
        hub_version,
    )

    return app
