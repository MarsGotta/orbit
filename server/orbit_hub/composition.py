"""What the hub is made of, built once for `app.create_app`.

Each piece is a domain's: the agents and their device runtimes, the device
bridge, the meetings and their workers. Nothing here handles a request.
"""

import asyncio
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field

from .assistant.access import AgentAccess
from .assistant.agent_config import AgentConfigStore
from .assistant.agents import Agents
from .assistant.roster import Roster
from .assistant.sign_in import SignIns
from .bridge_route import BridgeService
from .captures import CaptureStore
from .config import Settings
from .device_bridge import Bridge as DeviceBridge
from .meeting_final_pass import FinalPassWorker
from .meeting_transcription import MeetingTranscriber
from .meetings_bridge.service import MeetingsBridgeService
from .notes import NoteRepository, NotesWorker
from .processing import open_client as open_processing_client
from .runtime import Runtimes, TurnInputTracker
from .storage import Store
from .transcription import TranscriptionWorker


@dataclass
class Hub:
    settings: Settings
    store: Store
    now: Callable[[], float]
    # One thread for the Store's heavy work: hashing and encrypting uploads,
    # assembling a meeting group, reading transcripts. Small transactions
    # (a message, a status, a receipt) still run on the loop under the
    # Store's lock. A single worker keeps these writes ordered for the WAL.
    store_executor: ThreadPoolExecutor
    configs: AgentConfigStore
    agents: Agents
    access: AgentAccess
    runtimes: Runtimes
    sign_ins: SignIns
    turn_inputs: TurnInputTracker
    # Questions a remote assistant has put to a Mac, and the callers waiting on
    # them. Kept beside the runtimes rather than inside one: a device that
    # disconnects has to take its outstanding questions with it, and that is
    # easier to get right when they are not owned by the connection.
    bridge: DeviceBridge
    bridge_waiters: dict[str, asyncio.Future]
    captures: CaptureStore
    device_bridge: BridgeService
    # Fired whenever a new chunk is persisted or a recording is finalized, so
    # the transcription worker can react to fresh work without polling.
    prepare_signal: asyncio.Event
    meeting_language: str | None = None
    meetings_bridge: MeetingsBridgeService | None = None
    notes: NotesWorker | None = None
    final_passes: FinalPassWorker | None = None
    transcriptions: TranscriptionWorker | None = None
    pair_attempts: dict[str, tuple[float, int]] = field(default_factory=dict)


def build(settings: Settings, store: Store, assistant, speech, now, processing_client) -> Hub:
    """The hub over `store`. `assistant` and `speech`, when given, serve every
    configured agent: tests drive the hub with doubles."""
    # Each agent's personality, skills and access groups (Settings › Agents).
    configs = AgentConfigStore(store)
    # Each agent's profile, the primary, and the agents added from Orbit.
    agents = Agents.from_settings(settings, assistant, speech, configs, Roster(store))
    access = AgentAccess(agents, configs)
    store_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="orbit-store")
    turn_inputs = TurnInputTracker(now)
    runtimes = Runtimes(store, agents, now, store_executor, turn_inputs)
    agents.changed = runtimes.agents_changed
    bridge, bridge_waiters, captures = DeviceBridge(), {}, CaptureStore(now=now)
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
    hub = Hub(
        settings,
        store,
        now,
        store_executor,
        configs,
        agents,
        access,
        runtimes,
        SignIns(settings, agents),
        turn_inputs,
        bridge,
        bridge_waiters,
        captures,
        device_bridge,
        asyncio.Event(),
    )
    _meetings(hub, processing_client)
    return hub


def _meetings(hub: Hub, processing_client) -> None:
    """The meetings' transcription, final pass and notes, and their bridge."""
    settings, store, now = hub.settings, hub.store, hub.now
    # Durable meetings use hub-owned STT; Hermes remains interactive-only.
    meeting_transcriber = MeetingTranscriber(settings.meeting_transcription)
    # What a transcript group stored without a language reads as.
    hub.meeting_language = meeting_transcriber.configuration.language
    hub.meetings_bridge = MeetingsBridgeService(
        store,
        settings.meetings_bridge,
        now,
        default_language=hub.meeting_language,
        access=hub.access,
    )
    # Fired when a transcript group finishes, so a note waiting for the last
    # group is written at once instead of on the next tick.
    notes_signal: asyncio.Event = asyncio.Event()
    if processing_client is None and settings.processing is not None:
        processing_client = open_processing_client(settings.processing)
    changed = hub.runtimes.recording_changed
    hub.notes = NotesWorker(
        NoteRepository(store),
        now,
        signal=notes_signal,
        store_executor=hub.store_executor,
        client=processing_client,
        default_language=hub.meeting_language,
        recording_changed=changed,
    )
    hub.final_passes = FinalPassWorker(
        store, meeting_transcriber, settings.diarization, notes_signal, changed, hub.store_executor
    )
    # Dictation is the default agent's: it is input for any agent, and the
    # route names none (contracts/hub-v1.md).
    hub.transcriptions = TranscriptionWorker(
        store,
        hub.agents.default.speech,
        now,
        meetings=meeting_transcriber,
        prepare_signal=hub.prepare_signal,
        store_executor=hub.store_executor,
        group_finished=notes_signal,
        recording_changed=changed,
    )
