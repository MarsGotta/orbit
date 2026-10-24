"""An assistant for any OpenAI-compatible API, or Anthropic's with a key, run in the hub.

`kind = "openai"` in `[[agents]]`. The model loop is pydantic-ai's
(`pydantic-ai-slim`, pinned exactly in `pyproject.toml`): Chat Completions
for OpenAI-compatible servers (OpenAI, OpenRouter, LiteLLM, Ollama, vLLM…),
Anthropic's own API with `provider = "anthropic"`. Orbit's tools are
`orbit-mcp`'s, launched by the hub over stdio with the `mcp` client the hub
already has; the agent gets no shell and no server files. The conversation is
the one the hub stores: this backend keeps none of its own beyond the live
session, so a resumed session continues from what the Mac reads. The agent's
personality and enabled skills, which the hub keeps (`agent_config`), are
told to the model at every turn; orbit-mcp's `orbit_skill` hands it a
skill's instructions when it asks.
Dictation goes through the hub's meeting transcriber; read-aloud only through
an OpenAI-compatible `/audio/speech` when `[agents.speech]` names one.

The API key is read from a 0600 file and never logged, nor is any message.
"""

import asyncio
import logging
import re
from collections.abc import Sequence
from dataclasses import dataclass, field
from typing import TYPE_CHECKING
from urllib.parse import urlsplit
from uuid import uuid4

from anthropic import AsyncAnthropic
from openai import AsyncOpenAI, Timeout
from pydantic_ai import (
    Agent,
    AgentRunResultEvent,
    BinaryContent,
    FunctionToolCallEvent,
    ModelRequest,
    ModelResponse,
    PartDeltaEvent,
    PartStartEvent,
    TextPart,
    TextPartDelta,
    UsageLimitExceeded,
    UserPromptPart,
)
from pydantic_ai.exceptions import (
    ModelAPIError,
    ModelConnectionError,
    ModelContextWindowExceededError,
    ModelHTTPError,
    ModelRateLimitError,
    ModelTimeoutError,
)
from pydantic_ai.models.anthropic import AnthropicModel
from pydantic_ai.models.openai import OpenAIChatModel
from pydantic_ai.providers.anthropic import AnthropicProvider
from pydantic_ai.providers.openai import OpenAIProvider
from pydantic_ai.usage import UsageLimits

from ..log import clear_op, report
from ..meeting_transcription import MeetingTranscriber
from ..protocol import HubError
from .agent_config import instructions_text, no_instructions
from .backend import (
    Capabilities,
    ClarificationReceipt,
    ContentBlock,
    EventSink,
    ImageBlock,
    MessageChunk,
    MessageComplete,
    OpenedSession,
    PermissionReceipt,
    ResumedSession,
    StatusNotice,
    StoredConversation,
    TextBlock,
    TurnFailed,
)
from .openai_settings import OpenAIAgentSettings, read_api_key
from .openai_speech import OpenAISpeech
from .openai_steps import TurnSteps
from .openai_tools import OrbitTools, ToolTransportFailed

if TYPE_CHECKING:
    from ..config import Settings
    from .agents import AgentConfiguration

KIND = "openai"

# Both SDKs log each whole request, the conversation included, at DEBUG
# (`OPENAI_LOG=debug`, `ANTHROPIC_LOG=debug`). Conversation content stays out
# of the hub's logs whatever its environment says.
for _sdk in ("openai", "anthropic"):
    logging.getLogger(_sdk).setLevel(logging.WARNING)

# Sent as the bearer of an OpenAI-compatible server that needs no key (a local
# Ollama or vLLM): the OpenAI SDK refuses to start without one.
NO_API_KEY = "orbit-sin-clave"

# The conversation the model receives before the current turn, in characters
# of the messages' text. What it protects: the model's context window, which
# must also hold the instructions, the schemas of orbit-mcp's twelve tools
# (8 705 characters of JSON, measured 2026-10-21), the current turn (up to the
# 32 000 characters `chat.send` admits, a meeting question included), the
# turn's tool results and the answer. Why 64 Ki characters: about 16 000
# tokens at four characters a token, half of a 32 000-token window, a common
# size for local models; larger windows fit it with room. When the stored
# conversation is longer, the oldest messages are left out, and the model and
# the Mac are told how many. It is not a limit on the conversation the hub
# keeps, which stays whole, nor on the current turn, which is sent whole.
HISTORY_MAX_CHARS = 64 * 1024
# Model requests in one turn: each tool round trip is one. What it protects:
# the provider bill and the user's wait against a model that keeps calling
# tools. Why 50: pydantic-ai's own default, more than any task Orbit's tools
# need (a look, an action and its result are three); it does not bound tools
# per request.
MAX_MODEL_REQUESTS_PER_TURN = 50
# One provider request. Connecting: a provider that does not accept the
# connection in 10 s is not there (the hub's other providers use the same).
# Reading: the longest silence between two pieces of the streamed answer; a
# local model on a CPU can take two minutes before its first token. Neither is
# a bound on the whole turn, which streams for as long as the model writes.
MODEL_CONNECT_TIMEOUT_SECONDS = 10
MODEL_READ_TIMEOUT_SECONDS = 120
INSTRUCTIONS = (
    "You are {name}, the user's assistant in Orbit, a companion for their Mac. "
    "Each user message begins with a line such as «[Orbit · <device> · <input>]»: "
    "the Mac it came from and how it was entered (typed text, a transcribed voice "
    "note or a live voice call). It is context, not part of the request. Orbit's "
    "tools (orbit_*) reach that Mac and the user's meetings; when a tool takes a "
    "device, pass that name. Answer in the language the user writes in."
)

# ---------------------------------------------------------------- failures


def provider_url(settings: OpenAIAgentSettings) -> str:
    """The provider as a refusal names it: scheme, host, port and path only.
    The configuration admits no credentials or query in it anyway."""
    url = urlsplit(settings.base_url or "https://api.anthropic.com")
    return f"{url.scheme}://{url.netloc}{url.path}"


_CONTEXT_NUMBERS = re.compile(
    r"maximum context length is (\d+) tokens.*?(?:resulted in|requested) (\d+) tokens",
    re.IGNORECASE | re.DOTALL,
)
_CONTEXT_WORDS = re.compile(
    r"context (?:length|window)|maximum context|prompt is too long", re.IGNORECASE
)


def _leaf(error: BaseException) -> BaseException:
    """The first error inside the task groups that run tools in parallel."""
    while isinstance(error, BaseExceptionGroup) and error.exceptions:
        error = error.exceptions[0]
    return error


# What a provider answers to a request whose image it cannot take: OpenAI's
# `invalid_image` is a 400, a gateway's body ceiling a 413, a server without
# vision 400, 415 or 422. Other statuses keep their own causes.
_IMAGE_REFUSAL_STATUSES = {400, 413, 415, 422}


def image_refusal(
    error: BaseException, settings: OpenAIAgentSettings, images: Sequence[ImageBlock]
) -> HubError | None:
    """`attachment_refused` when the provider refused the turn's first request,
    which carried `images`, for a cause that is not the key, a rate limit or
    the context; None for any other failure."""
    error = _leaf(error)
    if not images or not isinstance(error, ModelHTTPError):
        return None
    if error.status_code not in _IMAGE_REFUSAL_STATUSES or _CONTEXT_WORDS.search(str(error.body)):
        return None
    names = ", ".join(f"«{image.name}»" for image in images)
    what = f"la imagen {names}" if len(images) == 1 else f"las imágenes del mensaje ({names})"
    return HubError(
        "attachment_refused",
        f"El proveedor del asistente ({provider_url(settings)}) no aceptó {what} "
        f"(HTTP {error.status_code}); el modelo no leyó el mensaje",
        numbers={
            "name": ", ".join(image.name for image in images),
            "provider_status": error.status_code,
        },
    )


def _timeout_failure(error: ModelTimeoutError, url: str) -> HubError:
    if error.phase == "connect":
        milliseconds = round(MODEL_CONNECT_TIMEOUT_SECONDS * 1000)
        return HubError(
            "assistant_connect_timeout",
            f"El proveedor del asistente ({url}) no aceptó la conexión en {milliseconds} ms",
            numbers={"connect_timeout_ms": milliseconds},
        )
    milliseconds = round(MODEL_READ_TIMEOUT_SECONDS * 1000)
    return HubError(
        "assistant_timeout",
        f"El proveedor del asistente ({url}) dejó de contestar durante {milliseconds} ms",
        numbers={"timeout_ms": milliseconds},
    )


def _connection_failure(error: ModelConnectionError, url: str) -> HubError:
    if error.phase in {"connect", "pool"}:
        return HubError(
            "assistant_unreachable",
            f"No se pudo conectar con el proveedor del asistente ({url}); "
            f"plazo de conexión de {MODEL_CONNECT_TIMEOUT_SECONDS * 1000} ms",
            numbers={"connect_timeout_ms": MODEL_CONNECT_TIMEOUT_SECONDS * 1000},
        )
    return HubError(
        "assistant_connection_lost",
        f"Se cortó la conexión con el proveedor del asistente ({url}) antes de terminar "
        "la respuesta",
    )


def _rate_limited(error: BaseException, url: str) -> HubError:
    numbers: dict[str, int] = {"provider_status": 429}
    message = f"El proveedor del asistente ({url}) limitó las peticiones (HTTP 429)"
    if isinstance(error, ModelAPIError) and error.retry_after is not None:
        numbers["retry_after_ms"] = round(error.retry_after * 1000)
        message += f"; pide esperar {numbers['retry_after_ms']} ms"
    else:
        message += "; no dijo cuánto esperar"
    return HubError("assistant_rate_limited", message, numbers=numbers)


def _context_exceeded(detail: str, history_chars: int) -> HubError:
    numbers = {"history_chars": history_chars, "max_history_chars": HISTORY_MAX_CHARS}
    message = f"El modelo no admite tanto contexto ({history_chars} caracteres de historial"
    if (tokens := _CONTEXT_NUMBERS.search(detail)) is not None:
        numbers["max_tokens"], numbers["tokens"] = int(tokens[1]), int(tokens[2])
        message += f"; {tokens[2]} tokens de un máximo de {tokens[1]}"
    return HubError("assistant_context_exceeded", message + ")", numbers=numbers)


def _provider_failure(error: BaseException, url: str, history_chars: int) -> HubError:
    """What the provider answered, or failed to: its status, limits and context."""
    status = error.status_code if isinstance(error, ModelHTTPError) else None
    if status in {401, 403}:
        return HubError(
            "assistant_unauthorized",
            f"El proveedor del asistente ({url}) rechazó la clave (HTTP {status}); "
            "revisa api_key_file",
            numbers={"provider_status": status},
        )
    if isinstance(error, ModelRateLimitError) or status == 429:
        return _rate_limited(error, url)
    detail = str(error.body) if isinstance(error, ModelAPIError) else ""
    if isinstance(error, ModelContextWindowExceededError) or (
        status == 400 and _CONTEXT_WORDS.search(detail)
    ):
        return _context_exceeded(detail, history_chars)
    if status is not None:
        return HubError(
            "assistant_http_error",
            f"El proveedor del asistente ({url}) devolvió HTTP {status}",
            numbers={"provider_status": status},
        )
    if isinstance(error, ModelAPIError):
        return HubError(
            "assistant_provider_error",
            f"El proveedor del asistente ({url}) falló sin decir por qué",
        )
    return HubError(
        "assistant_failed",
        "El asistente falló antes de terminar la respuesta; no se repetirá",
    )


def describe_failure(
    error: BaseException, settings: OpenAIAgentSettings, history_chars: int
) -> HubError:
    """The cause of a turn that ended with `error`: code, Spanish message, numbers."""
    error = _leaf(error)
    url = provider_url(settings)
    if isinstance(error, ToolTransportFailed):
        return HubError("assistant_tool_failed", error.message, numbers=error.numbers)
    if isinstance(error, UsageLimitExceeded):
        return HubError(
            "assistant_turn_too_long",
            f"El asistente hizo {MAX_MODEL_REQUESTS_PER_TURN} peticiones al modelo en este "
            "turno sin terminar; se detuvo",
            numbers={"max_requests": MAX_MODEL_REQUESTS_PER_TURN},
        )
    if isinstance(error, ModelTimeoutError):
        return _timeout_failure(error, url)
    if isinstance(error, ModelConnectionError):
        return _connection_failure(error, url)
    return _provider_failure(error, url, history_chars)


# ---------------------------------------------------------------- sessions


@dataclass
class _Session:
    stored_id: str
    # What the model may still be sent, oldest first: (role, text).
    held: list[tuple[str, str]] = field(default_factory=list)
    # Messages of this conversation before `held`, no longer sent.
    left_out: int = 0
    turn: asyncio.Task | None = None

    def history(self) -> tuple[list[ModelRequest | ModelResponse], int]:
        """The newest held messages within `HISTORY_MAX_CHARS`, and their size."""
        size, start = 0, len(self.held)
        while start > 0 and size + len(self.held[start - 1][1]) <= HISTORY_MAX_CHARS:
            start -= 1
            size += len(self.held[start][1])
        self.left_out += start
        del self.held[:start]
        messages: list[ModelRequest | ModelResponse] = []
        if self.left_out:
            messages.append(
                ModelRequest(
                    parts=[
                        UserPromptPart(
                            f"[Orbit · se dejaron fuera los {self.left_out} mensajes más antiguos "
                            "de esta conversación: el modelo recibe como mucho "
                            f"{HISTORY_MAX_CHARS} caracteres de historial]"
                        )
                    ]
                )
            )
        for role, text in self.held:
            if role == "user":
                messages.append(ModelRequest(parts=[UserPromptPart(text)]))
            else:
                messages.append(ModelResponse(parts=[TextPart(text)]))
        return messages, size


def _prompt(text: str, images: list[ImageBlock]) -> tuple[str | list[str | BinaryContent], str]:
    """The user prompt: the text, then each image whole; and what the
    conversation keeps of it: the text and a line naming each image's
    attachment, so a later turn never sends its bytes again."""
    if not images:
        return text, text
    prompt = [text, *(BinaryContent(data=image.data, media_type=image.mime) for image in images)]
    said = "\n".join(
        [
            text,
            *(
                f"[Orbit · la imagen «{image.name}» (adjunto {image.attachment_id}) iba "
                "en este mensaje y no se vuelve a enviar; para verla otra vez, "
                "orbit_attachment con ese id]"
                for image in images
            ),
        ]
    )
    return prompt, said


def _text_piece(event: object) -> str:
    """The answer's text an event of the stream carries, if any."""
    if isinstance(event, PartStartEvent) and isinstance(event.part, TextPart):
        return event.part.content
    if isinstance(event, PartDeltaEvent) and isinstance(event.delta, TextPartDelta):
        return event.delta.content_delta
    return ""


class OpenAIConnection:
    """One device's conversations with the model, with its own orbit-mcp."""

    def __init__(self, backend: "OpenAIBackend", device: str, on_event: EventSink):
        self.backend, self.device, self.on_event = backend, device, on_event
        self.epoch = ""
        self.live_sessions: set[str] = set()
        self._sessions: dict[str, _Session] = {}
        self._tools = OrbitTools(backend.agent)
        self._closing = False

    @property
    def connected(self) -> bool:
        return bool(self.epoch) and not self._closing and self._tools.alive

    async def open(self) -> None:
        await self._tools.start()
        self.epoch = str(uuid4())

    async def close(self) -> None:
        self._closing = True
        turns = [session.turn for session in self._sessions.values() if session.turn is not None]
        for turn in turns:
            turn.cancel()
        await asyncio.gather(*turns, return_exceptions=True)
        self.live_sessions.clear()
        await self._tools.stop()

    def _open(self, session: _Session) -> str:
        live = str(uuid4())
        self._sessions[live] = session
        self.live_sessions.add(live)
        return live

    async def create_session(self) -> OpenedSession:
        stored = str(uuid4())
        return OpenedSession(self._open(_Session(stored)), stored)

    async def resume_session(
        self, stored_id: str, history: StoredConversation | None = None
    ) -> ResumedSession:
        held = [(message.role, message.text) for message in history.messages] if history else []
        left_out = history.total - len(held) if history else 0
        live = self._open(_Session(stored_id, held, left_out))
        return ResumedSession(live, stored_id, len(held) or None, loading_history=False)

    def _session(self, session_id: str) -> _Session:
        session = self._sessions.get(session_id)
        if session is None or session_id not in self.live_sessions:
            raise HubError("session_runtime_not_found", "La sesión ya no está activa")
        return session

    async def prompt(self, session_id: str, blocks: Sequence[ContentBlock]) -> None:
        session = self._session(session_id)
        if session.turn is not None and not session.turn.done():
            raise HubError("session_busy", "Espera o interrumpe el turno en curso")
        text = "".join(block.text for block in blocks if isinstance(block, TextBlock))
        # The runtime sends images only when `capabilities.images` says so.
        images = [block for block in blocks if isinstance(block, ImageBlock)]
        session.turn = asyncio.create_task(self._turn(session_id, session, text, images))

    async def _turn(
        self, session_id: str, session: _Session, text: str, images: list[ImageBlock]
    ) -> None:
        clear_op()
        chunks: list[str] = []
        separate = False
        history_chars = 0
        steps = TurnSteps(session_id, self.on_event)
        prompt, said = _prompt(text, images)
        # Whether the provider answered anything: only its first answer can be
        # a refusal of the turn's images.
        answered = False
        try:
            history, history_chars = await self._history(session_id, session)
            output = ""
            # Read at each turn: a personality or skill changed in Settings
            # reaches the next turn of every conversation.
            # A skill's instructions come from orbit-mcp's `orbit_skill`.
            current = self.backend.instructions()
            async with self.backend.agent_loop.run_stream_events(
                prompt,
                message_history=history,
                instructions=instructions_text(current),
                toolsets=[self._tools],
                usage_limits=UsageLimits(request_limit=MAX_MODEL_REQUESTS_PER_TURN),
            ) as events:
                async for event in events:
                    answered = answered or isinstance(
                        event, PartStartEvent | PartDeltaEvent | FunctionToolCallEvent
                    )
                    await steps.on(event)
                    piece = _text_piece(event)
                    if isinstance(event, FunctionToolCallEvent):
                        separate = bool(chunks)
                    elif isinstance(event, AgentRunResultEvent):
                        output = event.result.output
                    if piece:
                        if separate:
                            # Text written after a tool call is a new paragraph.
                            piece, separate = "\n\n" + piece, False
                        chunks.append(piece)
                        await self.on_event(MessageChunk(session_id, piece))
            await steps.end_thought()
            answer = "".join(chunks) or output
            session.held += [("user", said), ("assistant", answer)]
            await self.on_event(MessageComplete(session_id, answer, "completed"))
        except asyncio.CancelledError:
            if self._closing:
                raise
            await steps.end_thought()
            answer = "".join(chunks)
            session.held.append(("user", said))
            if answer:
                session.held.append(("assistant", answer))
            await self.on_event(MessageComplete(session_id, answer, "interrupted"))
        except Exception as exc:  # noqa: BLE001 - every failure ends the turn with its cause
            refused = None if answered else image_refusal(exc, self.backend.settings, images)
            failure = refused or describe_failure(exc, self.backend.settings, history_chars)
            report(
                event="assistant_turn_failed", code=failure.code, device=self.device, exc_info=False
            )
            await steps.end_thought()
            # Kept for the next turn unless its image was refused: then the
            # model never read it, as Hermes never receives such a prompt.
            if refused is None:
                session.held.append(("user", said))
            await self.on_event(
                TurnFailed(session_id, failure.code, failure.message, failure.numbers)
            )

    async def _history(
        self, session_id: str, session: _Session
    ) -> tuple[list[ModelRequest | ModelResponse], int]:
        """What the model is sent of the conversation, saying so when some is left out."""
        history, history_chars = session.history()
        if session.left_out:
            await self.on_event(
                StatusNotice(
                    session_id,
                    "warning",
                    f"El asistente no recibe los {session.left_out} mensajes más antiguos "
                    f"de esta conversación: su historial llega a {HISTORY_MAX_CHARS} "
                    "caracteres como mucho",
                )
            )
        return history, history_chars

    async def cancel(self, session_id: str) -> None:
        session = self._session(session_id)
        if session.turn is not None and not session.turn.done():
            session.turn.cancel()

    async def respond_permission(
        self, session_id: str, request_id: str, *, choice: str
    ) -> PermissionReceipt:
        # It never asks: Orbit's tools carry their own consent (the macOS
        # permissions), so no request of this agent is ever pending. One that
        # asked would offer `once` (approve) and `deny` (refuse).
        raise HubError("interaction_not_found", "Este asistente no pide autorizaciones")

    async def respond_clarification(
        self, session_id: str, request_id: str, answer: str, question_id: str | None
    ) -> ClarificationReceipt:
        raise HubError("interaction_not_found", "Este asistente no pide aclaraciones")

    async def turn_stopped(self, session_id: str) -> bool:
        session = self._sessions.get(session_id)
        return session is None or session.turn is None or session.turn.done()


class OpenAIBackend:
    """A model behind an OpenAI-compatible API, or Anthropic's, with Orbit's tools."""

    def __init__(self, agent: "AgentConfiguration", model, instructions=no_instructions):
        self.agent, self.settings = agent, agent.settings
        # The personality and enabled skills the hub keeps for this agent.
        self.instructions = instructions
        # Images: the turn's own and the tools' screenshots, when
        # `settings.images` says the model reads them. History: the hub's own
        # (see the module). Voice: read-aloud configured.
        self.capabilities = Capabilities(
            images=self.settings.images,
            resume=True,
            history=False,
            voice=self.settings.speech is not None,
        )
        self.agent_loop = Agent(
            model, instructions=INSTRUCTIONS.format(name=agent.name), output_type=str
        )

    def connection(self, device: str, device_name: str, on_event: EventSink) -> OpenAIConnection:
        return OpenAIConnection(self, device, on_event)


def _check_files(agent: "AgentConfiguration") -> str | None:
    """The agent's files as the hub's user reads them, refused at startup with
    the key and the cause; the API key, if there is one."""
    settings, label = agent.settings, f'agent "{agent.id}"'
    if not settings.orbit_mcp_config.is_file():
        raise ValueError(
            f"{label}.orbit_mcp_config: {settings.orbit_mcp_config} does not exist or is not a file"
        )
    if settings.speech is not None and settings.speech.api_key_file is not None:
        read_api_key(f"{label}.speech", "api_key_file", settings.speech.api_key_file)
    if settings.api_key_file is None:
        return None
    return read_api_key(label, "api_key_file", settings.api_key_file)


def build_backend(
    hub: "Settings", agent: "AgentConfiguration", instructions=no_instructions
) -> OpenAIBackend:
    """The agent's backend (`agents.KINDS`). `hub` is the hub's own
    configuration, which every kind's factory takes; this one needs none of it.
    `instructions` reads the agent's personality and enabled skills."""
    key = _check_files(agent)
    settings = agent.settings
    timeout = Timeout(MODEL_READ_TIMEOUT_SECONDS, connect=MODEL_CONNECT_TIMEOUT_SECONDS)
    # Retries are off: a refusal reaches the user with its cause at once, and a
    # rate limit with the wait the provider asked for, instead of a silent wait.
    if settings.provider == "anthropic":
        client = AsyncAnthropic(
            api_key=key, base_url=settings.base_url, max_retries=0, timeout=timeout
        )
        model = AnthropicModel(settings.model, provider=AnthropicProvider(anthropic_client=client))
    else:
        client = AsyncOpenAI(
            api_key=key or NO_API_KEY, base_url=settings.base_url, max_retries=0, timeout=timeout
        )
        model = OpenAIChatModel(settings.model, provider=OpenAIProvider(openai_client=client))
    return OpenAIBackend(agent, model, instructions)


# ---------------------------------------------------------------- voice


def build_speech(hub: "Settings", agent: "AgentConfiguration") -> OpenAISpeech:
    """The agent's speech (`agents.KINDS`): dictation always (the meeting
    transcriber is mandatory); read-aloud only with `[agents.speech]`, which
    `capabilities.voice` reports."""
    return OpenAISpeech(agent, MeetingTranscriber(hub.meeting_transcription))
