"""The hub's FastAPI application: the composition root, with no logic of its own.

`create_app` builds the hub (`composition`), its start and stop (`lifespan`),
its HTTP boundary (`http_boundary`) and mounts each domain's routes and the
device WebSocket (`ws`).
"""

import time
from importlib.metadata import PackageNotFoundError
from importlib.metadata import version as installed_version

from fastapi import FastAPI
from starlette.middleware.trustedhost import TrustedHostMiddleware

from . import composition, http_boundary
from .assistant.backend import AssistantBackend
from .assistant.speech import SpeechBackend
from .bridge_route import mount as mount_device_bridge
from .config import Settings
from .http_boundary import allowed_transport, bounded_body, validate_wav
from .lifespan import lifespan
from .meetings.routes import mount as mount_meetings
from .meetings_bridge.routes import mount as mount_meetings_bridge
from .notes import mount as mount_notes
from .storage import Store


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
    hub = composition.build(settings, store, assistant, speech, now, processing_client)
    app = FastAPI(
        title="Orbit Hub",
        version=hub_version(),
        docs_url=None,
        redoc_url=None,
        lifespan=lifespan(hub, owned_store, f"orbit-hub@{hub_version()}"),
    )
    app.add_middleware(TrustedHostMiddleware, allowed_hosts=list(settings.allowed_hosts))
    app.state.store = store
    app.state.notes = hub.notes
    app.state.device_bridge = hub.device_bridge
    http_boundary.install(app, settings, store)
    mount_routes(app, hub)
    return app


def mount_routes(app: FastAPI, hub: composition.Hub) -> None:
    """Each domain's routes, and the device WebSocket, in the order they match."""
    store, settings = hub.store, hub.settings

    def identity(request) -> str:
        return store.devices.authenticate(http_boundary.bearer(request.headers))

    def transport(scope) -> bool:
        return allowed_transport(scope, settings)

    _mount_meetings(app, hub, identity, transport)
    _mount_devices(app, hub, identity)


def _mount_meetings(app: FastAPI, hub: composition.Hub, identity, transport) -> None:
    """Notes and meetings, the two private bridges, pairing and health."""
    from .routes import health, session

    store, now, executor = hub.store, hub.now, hub.store_executor
    changed = hub.runtimes.recording_changed
    mount_notes(app, hub.notes, identity, bounded_body, executor, now, changed)
    mount_meetings(
        app,
        store,
        identity,
        bounded_body,
        executor,
        now,
        hub.notes,
        hub.meeting_language,
        changed,
        hub.prepare_signal,
        hub.settings.diarization is not None,
    )
    mount_device_bridge(app, hub.device_bridge, transport)
    mount_meetings_bridge(app, hub.meetings_bridge, transport)
    session.mount(app, store, hub.agents, identity, bounded_body, now, hub.pair_attempts, executor)
    health.mount(app, hub.settings)


def _mount_devices(app: FastAPI, hub: composition.Hub, identity) -> None:
    """The agents, a device's uploads and recordings, and its WebSocket."""
    from .routes import agents as agent_routes
    from .routes import assistants as assistant_routes
    from .routes import attachments as attachment_routes
    from .routes import audio, recordings, reminders
    from .routes import captures as capture_routes
    from .ws import mount as mount_ws

    settings, store, now, executor = hub.settings, hub.store, hub.now, hub.store_executor
    agent_routes.mount(app, settings, hub.agents, hub.configs, identity, bounded_body)
    assistant_routes.mount(
        app, settings, hub.agents, hub.configs, hub.runtimes, hub.sign_ins, identity,
        bounded_body, now,
    )  # fmt: skip
    attachment_routes.mount(app, store, identity, bounded_body, executor, now, hub.captures)
    audio.mount(
        app, store, identity, bounded_body, validate_wav, settings, hub.agents,
        hub.transcriptions, hub.turn_inputs,
    )  # fmt: skip
    capture_routes.mount(
        app, store, identity, bounded_body, validate_wav, settings, hub.captures, executor,
        hub.prepare_signal, now,
    )  # fmt: skip
    recordings.mount(
        app, store, identity, bounded_body, executor, hub.prepare_signal, now, hub.notes,
        hub.runtimes.recording_changed, settings.diarization is not None,
    )  # fmt: skip
    reminders.mount(app, store, identity, bounded_body, executor)
    mount_ws(
        app, settings, hub.agents, identity, allowed_transport, hub.runtimes.of, store, now,
        hub.bridge, hub.bridge_waiters, hub_version,
    )  # fmt: skip
