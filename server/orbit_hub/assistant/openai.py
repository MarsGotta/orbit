"""An assistant for any OpenAI-compatible API, or Anthropic's with a key, run in the hub.

`kind = "openai"` in `[[agents]]`. The model loop is pydantic-ai's
(`pydantic-ai-slim`, pinned exactly in `pyproject.toml`): Chat Completions
for OpenAI-compatible servers (OpenAI, OpenRouter, LiteLLM, Ollama, vLLM…),
Anthropic's own API with `provider = "anthropic"`. Orbit's tools are
`orbit-mcp`'s, launched by the hub over stdio with the `mcp` client the hub
already has; the agent gets no shell and no server files. The conversation is
the one the hub stores: this backend keeps none of its own beyond the live
session, so a resumed session continues from what the Mac reads.
Dictation goes through the hub's meeting transcriber; read-aloud only through
an OpenAI-compatible `/audio/speech` when `[agents.speech]` names one.

The API key is read from a 0600 file and never logged, nor is any message.
"""

import asyncio
import base64
import logging
import re
import sys
import unicodedata
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import TYPE_CHECKING, Any, Literal
from urllib.parse import urlsplit
from uuid import uuid4

import httpx
from anthropic import AsyncAnthropic
from mcp import ClientSession, StdioServerParameters, stdio_client
from mcp.shared.exceptions import MCPError
from mcp.types import CONNECTION_CLOSED, REQUEST_TIMEOUT, CallToolResult, ImageContent, TextContent
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
    RunContext,
    TextPart,
    TextPartDelta,
    ToolDefinition,
    ToolFailed,
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
from pydantic_ai.toolsets import AbstractToolset
from pydantic_ai.toolsets.abstract import ToolsetTool
from pydantic_ai.usage import UsageLimits
from pydantic_core import SchemaValidator, core_schema

from ..log import clear_op, report
from ..meeting_transcription import MeetingTranscriber
from ..protocol import HubError
from ..secret_file import read_secret
from ..urls import protected_url
from .backend import (
    Capabilities,
    ClarificationReceipt,
    ContentBlock,
    EventSink,
    MessageChunk,
    MessageComplete,
    OpenedSession,
    PermissionReceipt,
    ResumedSession,
    StatusNotice,
    StoredConversation,
    TurnFailed,
)
from .speech import MAX_SPEECH_CLIP_BYTES, SpeechClip

if TYPE_CHECKING:
    from ..config import Settings
    from .agents import AgentConfiguration

KIND = "openai"

# Both SDKs log each whole request, the conversation included, at DEBUG
# (`OPENAI_LOG=debug`, `ANTHROPIC_LOG=debug`). Conversation content stays out
# of the hub's logs whatever its environment says.
for _sdk in ("openai", "anthropic"):
    logging.getLogger(_sdk).setLevel(logging.WARNING)

# One API key per file. What it protects: the hub's memory against a key path
# that names some other, large file. Why 4 KiB: provider keys are under 200
# bytes; this is far above any. It is not a bound on anything else.
MAX_API_KEY_BYTES = 4 * 1024
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
# orbit-mcp's start: launching Python and answering `initialize` and
# `tools/list`, about one second on the hub's server. 30 s keeps a slow disk
# from failing the first message without holding it for ever.
TOOLS_START_TIMEOUT_SECONDS = 30
# One Orbit tool call. orbit-mcp's longest own wait is the Mac's answer through
# the device bridge (`BRIDGE_TIMEOUT_SECONDS`, 35 s); 60 s leaves it room to
# return its own refusal first. A call that outlives it fails the turn, and its
# outcome stays unknown: it is never repeated.
TOOL_CALL_TIMEOUT_SECONDS = 60
# Read-aloud: the same 180 s provider and 10 s connection budgets as Hermes's.
SPEECH_TIMEOUT_SECONDS = 180
SPEECH_CONNECT_TIMEOUT_SECONDS = 10

INSTRUCTIONS = (
    "You are {name}, the user's assistant in Orbit, a companion for their Mac. "
    "Each user message begins with a line such as «[Orbit · <device> · <input>]»: "
    "the Mac it came from and how it was entered (typed text, a transcribed voice "
    "note or a live voice call). It is context, not part of the request. Orbit's "
    "tools (orbit_*) reach that Mac and the user's meetings; when a tool takes a "
    "device, pass that name. Answer in the language the user writes in."
)

# Arguments arrive as the model's JSON object; orbit-mcp validates them itself
# and answers a refusal the model can read.
_ARGUMENTS = SchemaValidator(
    core_schema.dict_schema(core_schema.str_schema(), core_schema.any_schema())
)


# ---------------------------------------------------------------- settings


@dataclass(frozen=True, repr=False)
class SpeechSettings:
    """An OpenAI-compatible `POST {base_url}/audio/speech`."""

    base_url: str
    model: str
    voice: str
    api_key_file: Path | None


@dataclass(frozen=True, repr=False)
class OpenAIAgentSettings:
    """An `[[agents]]` entry's own keys for kind `openai`; the agent's `id` and
    `name` are the registry's (`agents.AgentConfiguration`)."""

    provider: Literal["openai", "anthropic"]
    # The API root, as the OpenAI SDK takes it (`…/v1`); None is Anthropic's.
    base_url: str | None
    model: str
    api_key_file: Path | None
    # orbit-mcp's own TOML (`ORBIT_MCP_CONFIG`).
    orbit_mcp_config: Path
    # The model accepts images: screenshots from Orbit's tools reach it as such.
    images: bool
    speech: SpeechSettings | None


AGENT_KEYS = {"provider", "base_url", "model", "api_key_file", "orbit_mcp_config", "images", "speech"}  # fmt: skip
SPEECH_KEYS = {"base_url", "model", "voice", "api_key_file"}


def _keys(table: str, raw: Mapping[str, Any], allowed: set[str], required: set[str]) -> None:
    prefix = f"{table}: " if table else ""
    for names, cause in ((required - raw.keys(), "missing"), (raw.keys() - allowed, "unknown")):
        if names:
            raise ValueError(f"{prefix}{cause} keys ({len(names)}): {', '.join(sorted(names))}")


def _text(key: str, value: Any, maximum: int) -> str:
    if not isinstance(value, str):
        # ValueError, as every refusal of the hub's configuration.
        raise ValueError(f"{key}: must be a string, not {type(value).__name__}")  # noqa: TRY004
    if not value.strip():
        raise ValueError(f"{key}: is empty")
    if len(value) > maximum:
        raise ValueError(f"{key}: {len(value)} characters; maximum {maximum}")
    if any(unicodedata.category(character) == "Cc" for character in value):
        raise ValueError(f"{key}: contains control characters")
    return value


def _url(key: str, value: Any) -> str:
    value = _text(key, value, 2048)
    try:
        protected_url(value, allow_insecure_localhost=True)
    except ValueError as exc:
        raise ValueError(f"{key}: {exc}") from None
    return value.rstrip("/")


def _path(key: str, value: Any) -> Path:
    """An absolute path, as written: the file itself is checked at startup."""
    path = Path(_text(key, value, 4096)).expanduser()
    if not path.is_absolute():
        raise ValueError(f"{key}: must be an absolute path, not {value!r}")
    return path


def read_api_key(label: str, key: str, path: Path) -> str:
    """The key in `path`, refused with the cause; the key itself is never in it."""
    try:
        value = read_secret(path, MAX_API_KEY_BYTES).decode("utf-8").strip()
    except FileNotFoundError:
        raise ValueError(f"{label}.{key}: {path} does not exist") from None
    except PermissionError:
        raise ValueError(f"{label}.{key}: {path} cannot be read by the hub's user") from None
    except UnicodeDecodeError:
        raise ValueError(f"{label}.{key}: {path} is not UTF-8 text") from None
    except (OSError, ValueError) as exc:
        cause = exc.strerror if isinstance(exc, OSError) and exc.strerror else str(exc)
        raise ValueError(f"{label}.{key}: {path}: {cause}") from None
    if not value:
        raise ValueError(f"{label}.{key}: {path} is empty")
    return value


def _speech(raw: Any) -> SpeechSettings:
    if not isinstance(raw, Mapping):
        raise ValueError(f"speech: must be a table, not {type(raw).__name__}")  # noqa: TRY004
    _keys("speech", raw, SPEECH_KEYS, {"base_url", "model", "voice"})
    return SpeechSettings(
        base_url=_url("speech.base_url", raw["base_url"]),
        model=_text("speech.model", raw["model"], 256),
        voice=_text("speech.voice", raw["voice"], 64),
        api_key_file=(
            _path("speech.api_key_file", raw["api_key_file"]) if "api_key_file" in raw else None
        ),
    )


def parse_settings(raw: Mapping[str, Any]) -> OpenAIAgentSettings:
    """An `[[agents]]` entry's own keys for kind `openai` (see `agents.KINDS`):
    all but `id`, `name` and `kind`. Every refusal names the key and the
    cause; it reads no file, which `build_backend` checks at startup."""
    provider = raw.get("provider", "openai")
    if provider not in {"openai", "anthropic"}:
        raise ValueError(f'provider: must be "openai" or "anthropic", not {provider!r}')
    required = {"model", "orbit_mcp_config"}
    if provider == "openai":
        required.add("base_url")
    else:
        required.add("api_key_file")
    _keys("", raw, AGENT_KEYS, required)
    images = raw.get("images", False)
    if type(images) is not bool:
        raise ValueError(f"images: must be true or false, not {images!r}")
    return OpenAIAgentSettings(
        provider=provider,
        base_url=_url("base_url", raw["base_url"]) if "base_url" in raw else None,
        model=_text("model", raw["model"], 256),
        api_key_file=_path("api_key_file", raw["api_key_file"]) if "api_key_file" in raw else None,
        orbit_mcp_config=_path("orbit_mcp_config", raw["orbit_mcp_config"]),
        images=images,
        speech=_speech(raw["speech"]) if "speech" in raw else None,
    )


# ---------------------------------------------------------------- failures


def provider_url(settings: OpenAIAgentSettings) -> str:
    """The provider as a refusal names it: scheme, host, port and path only.
    The configuration admits no credentials or query in it anyway."""
    url = urlsplit(settings.base_url or "https://api.anthropic.com")
    return f"{url.scheme}://{url.netloc}{url.path}"


class ToolTransportFailed(Exception):
    """An Orbit tool call that orbit-mcp never answered: the turn ends with it."""

    def __init__(self, tool: str, message: str, numbers: dict[str, int] | None = None):
        super().__init__(message)
        self.tool, self.message, self.numbers = tool, message, numbers


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
    if isinstance(error, ModelConnectionError):
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
    status = error.status_code if isinstance(error, ModelHTTPError) else None
    if status in {401, 403}:
        return HubError(
            "assistant_unauthorized",
            f"El proveedor del asistente ({url}) rechazó la clave (HTTP {status}); "
            "revisa api_key_file",
            numbers={"provider_status": status},
        )
    if isinstance(error, ModelRateLimitError) or status == 429:
        numbers: dict[str, int] = {"provider_status": 429}
        message = f"El proveedor del asistente ({url}) limitó las peticiones (HTTP 429)"
        if isinstance(error, ModelAPIError) and error.retry_after is not None:
            numbers["retry_after_ms"] = round(error.retry_after * 1000)
            message += f"; pide esperar {numbers['retry_after_ms']} ms"
        else:
            message += "; no dijo cuánto esperar"
        return HubError("assistant_rate_limited", message, numbers=numbers)
    detail = str(error.body) if isinstance(error, ModelAPIError) else ""
    if isinstance(error, ModelContextWindowExceededError) or (
        status == 400 and _CONTEXT_WORDS.search(detail)
    ):
        numbers = {"history_chars": history_chars, "max_history_chars": HISTORY_MAX_CHARS}
        message = f"El modelo no admite tanto contexto ({history_chars} caracteres de historial"
        if (tokens := _CONTEXT_NUMBERS.search(detail)) is not None:
            numbers["max_tokens"], numbers["tokens"] = int(tokens[1]), int(tokens[2])
            message += f"; {tokens[2]} tokens de un máximo de {tokens[1]}"
        return HubError("assistant_context_exceeded", message + ")", numbers=numbers)
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


# ---------------------------------------------------------------- Orbit's tools


class OrbitTools(AbstractToolset[None]):
    """orbit-mcp, launched over stdio from this hub's own environment.

    One process per device connection. A task of its own owns the stdio
    client: the `mcp` client runs in task groups that must be entered and
    left by the same task, and the connection opens and closes from others.
    """

    def __init__(self, agent: "AgentConfiguration"):
        self.agent, self.settings = agent, agent.settings
        self._session: ClientSession | None = None
        self._tools: dict[str, ToolsetTool[None]] = {}
        self._owner: asyncio.Task | None = None
        self._stop = asyncio.Event()

    @property
    def id(self) -> str:
        return "orbit"

    @property
    def alive(self) -> bool:
        return self._session is not None and self._owner is not None and not self._owner.done()

    def parameters(self) -> StdioServerParameters:
        return StdioServerParameters(
            command=sys.executable,
            args=["-m", "orbit_mcp.cli"],
            # Added to the `mcp` client's short list of safe variables (PATH,
            # HOME…): nothing else of the hub's environment reaches it.
            env={
                "ORBIT_MCP_CONFIG": str(self.settings.orbit_mcp_config),
                # Which agent asks, so orbit-mcp can offer each its own tools;
                # the same variable an ACP agent's orbit-mcp receives.
                "ORBIT_MCP_AGENT": self.agent.id,
            },
        )

    async def start(self) -> None:
        ready: asyncio.Future[None] = asyncio.get_running_loop().create_future()
        self._owner = asyncio.create_task(self._own(ready))
        try:
            await asyncio.wait_for(asyncio.shield(ready), TOOLS_START_TIMEOUT_SECONDS)
        except TimeoutError:
            await self.stop()
            raise HubError(
                "assistant_tools_unavailable",
                f"orbit-mcp no arrancó en {TOOLS_START_TIMEOUT_SECONDS * 1000} ms; "
                "el asistente no tiene las herramientas de Orbit",
                numbers={"timeout_ms": TOOLS_START_TIMEOUT_SECONDS * 1000},
            ) from None
        except Exception as exc:
            await self.stop()
            report(event="orbit_tools_start_failed", code="assistant_tools_unavailable")
            raise HubError(
                "assistant_tools_unavailable",
                "orbit-mcp terminó al arrancar (su causa queda en el registro del hub); "
                "el asistente no tiene las herramientas de Orbit",
            ) from exc

    async def _own(self, ready: asyncio.Future) -> None:
        clear_op()
        try:
            async with (
                stdio_client(self.parameters()) as (read_stream, write_stream),
                ClientSession(read_stream, write_stream) as session,
            ):
                await session.initialize()
                listing = await session.list_tools()
                self._tools = {
                    tool.name: ToolsetTool(
                        toolset=self,
                        tool_def=ToolDefinition(
                            name=tool.name,
                            description=tool.description or "",
                            parameters_json_schema=tool.input_schema,
                        ),
                        max_retries=1,
                        args_validator=_ARGUMENTS,
                    )
                    for tool in listing.tools
                }
                self._session = session
                ready.set_result(None)
                await self._stop.wait()
        except Exception as exc:  # noqa: BLE001 - the start reports it, or `alive` turns false
            if not ready.done():
                ready.set_exception(exc)
            else:
                report(event="orbit_tools_lost", code="assistant_tools_lost")
        finally:
            self._session = None
            if not ready.done():
                ready.set_exception(RuntimeError("orbit-mcp stopped before it was ready"))

    async def stop(self) -> None:
        """Close orbit-mcp's stdin and wait for it; 5 s, then cancel its owner."""
        self._stop.set()
        owner = self._owner
        if owner is None:
            return
        done, _ = await asyncio.wait({owner}, timeout=5)
        if not done:
            owner.cancel()
            await asyncio.gather(owner, return_exceptions=True)

    async def get_tools(self, ctx: RunContext[None]) -> dict[str, ToolsetTool[None]]:
        return self._tools

    async def call_tool(
        self, name: str, tool_args: dict[str, Any], ctx: RunContext[None], tool: ToolsetTool[None]
    ) -> Any:
        session = self._session
        if session is None or not self.alive:
            raise ToolTransportFailed(
                name, f"La herramienta {name} no se pudo usar: orbit-mcp ya no está en marcha"
            )
        try:
            result = await session.call_tool(
                name, tool_args, read_timeout_seconds=TOOL_CALL_TIMEOUT_SECONDS
            )
        except MCPError as exc:
            if exc.code == REQUEST_TIMEOUT:
                milliseconds = round(TOOL_CALL_TIMEOUT_SECONDS * 1000)
                raise ToolTransportFailed(
                    name,
                    f"La herramienta {name} no contestó en {milliseconds} ms; su resultado es "
                    "incierto y no se repetirá",
                    {"timeout_ms": milliseconds},
                ) from exc
            if exc.code == CONNECTION_CLOSED or not self.alive:
                raise ToolTransportFailed(
                    name, f"La herramienta {name} falló: orbit-mcp terminó durante la llamada"
                ) from exc
            raise ToolTransportFailed(
                name,
                f"orbit-mcp rechazó la llamada a {name} (código {exc.code})",
                {"mcp_code": exc.code},
            ) from exc
        if not isinstance(result, CallToolResult):
            raise ToolTransportFailed(
                name, f"La herramienta {name} pidió datos que este asistente no sabe dar"
            )
        parts: list[str | BinaryContent] = []
        for block in result.content:
            if isinstance(block, TextContent):
                parts.append(block.text)
            elif isinstance(block, ImageContent) and self.settings.images:
                parts.append(
                    BinaryContent(data=base64.b64decode(block.data), media_type=block.mime_type)
                )
            elif isinstance(block, ImageContent):
                parts.append(
                    "[Orbit: la imagen de esta herramienta no se envía porque este modelo no "
                    "admite imágenes (images = false)]"
                )
        text_only = all(isinstance(part, str) for part in parts)
        if result.is_error:
            # orbit-mcp's refusals are written for the model to read and correct.
            raise ToolFailed("\n".join(part for part in parts if isinstance(part, str)))
        if text_only:
            return "\n".join(parts)
        return parts


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
        text = "".join(block.text for block in blocks)
        session.turn = asyncio.create_task(self._turn(session_id, session, text))

    async def _turn(self, session_id: str, session: _Session, text: str) -> None:
        clear_op()
        chunks: list[str] = []
        separate = False
        history_chars = 0
        try:
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
            output = ""
            async with self.backend.agent_loop.run_stream_events(
                text,
                message_history=history,
                toolsets=[self._tools],
                usage_limits=UsageLimits(request_limit=MAX_MODEL_REQUESTS_PER_TURN),
            ) as events:
                async for event in events:
                    piece = ""
                    if isinstance(event, PartStartEvent) and isinstance(event.part, TextPart):
                        piece = event.part.content
                    elif isinstance(event, PartDeltaEvent) and isinstance(
                        event.delta, TextPartDelta
                    ):
                        piece = event.delta.content_delta
                    elif isinstance(event, FunctionToolCallEvent):
                        separate = bool(chunks)
                        await self.on_event(
                            StatusNotice(session_id, "tool", f"Usando {event.part.tool_name}")
                        )
                    elif isinstance(event, AgentRunResultEvent):
                        output = event.result.output
                    if piece:
                        if separate:
                            # Text written after a tool call is a new paragraph.
                            piece, separate = "\n\n" + piece, False
                        chunks.append(piece)
                        await self.on_event(MessageChunk(session_id, piece))
            answer = "".join(chunks) or output
            session.held += [("user", text), ("assistant", answer)]
            await self.on_event(MessageComplete(session_id, answer, "completed"))
        except asyncio.CancelledError:
            if self._closing:
                raise
            answer = "".join(chunks)
            session.held.append(("user", text))
            if answer:
                session.held.append(("assistant", answer))
            await self.on_event(MessageComplete(session_id, answer, "interrupted"))
        except Exception as exc:  # noqa: BLE001 - every failure ends the turn with its cause
            failure = describe_failure(exc, self.backend.settings, history_chars)
            report(
                event="assistant_turn_failed", code=failure.code, device=self.device, exc_info=False
            )
            session.held.append(("user", text))
            await self.on_event(
                TurnFailed(session_id, failure.code, failure.message, failure.numbers)
            )

    async def cancel(self, session_id: str) -> None:
        session = self._session(session_id)
        if session.turn is not None and not session.turn.done():
            session.turn.cancel()

    async def respond_permission(
        self, session_id: str, request_id: str, allow: bool
    ) -> PermissionReceipt:
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

    def __init__(self, agent: "AgentConfiguration", model):
        self.agent, self.settings = agent, agent.settings
        # Images: no turn carries one (`TextBlock` only); screenshots from the
        # tools reach the model as images when `settings.images` says it reads them.
        # History: the hub's own (see the module). Voice: read-aloud configured.
        self.capabilities = Capabilities(
            images=False, resume=True, history=False, voice=self.settings.speech is not None
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


def build_backend(hub: "Settings", agent: "AgentConfiguration") -> OpenAIBackend:
    """The agent's backend (`agents.KINDS`). `hub` is the hub's own
    configuration, which every kind's factory takes; this one needs none of it."""
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
    return OpenAIBackend(agent, model)


# ---------------------------------------------------------------- voice


class OpenAISpeech:
    """Dictation by the hub's meeting transcriber; read-aloud by `speech`, if any."""

    def __init__(self, agent: "AgentConfiguration", transcriber: MeetingTranscriber):
        self.agent, self.settings, self.transcriber = agent, agent.settings, transcriber

    async def transcribe(self, audio: bytes) -> str:
        return (await self.transcriber.transcribe(audio)).text

    async def synthesize(self, text: str) -> SpeechClip:
        speech = self.settings.speech
        if speech is None:
            raise HubError(
                "speech_not_configured",
                f"{self.agent.name} («{self.agent.id}») no tiene voz para leer en voz alta: "
                "falta [agents.speech] en su configuración",
                http_status=503,
                numbers={"agent_id": self.agent.id},
            )
        headers = {}
        if speech.api_key_file is not None:
            key = read_api_key(
                f'agent "{self.agent.id}".speech', "api_key_file", speech.api_key_file
            )
            headers["Authorization"] = f"Bearer {key}"
        try:
            async with (
                httpx.AsyncClient(
                    timeout=httpx.Timeout(
                        SPEECH_TIMEOUT_SECONDS, connect=SPEECH_CONNECT_TIMEOUT_SECONDS
                    ),
                    follow_redirects=False,
                ) as client,
                client.stream(
                    "POST",
                    speech.base_url + "/audio/speech",
                    headers=headers,
                    json={
                        "model": speech.model,
                        "voice": speech.voice,
                        "input": text,
                        "response_format": "mp3",
                    },
                ) as response,
            ):
                if response.status_code != 200:
                    raise HubError(
                        "speech_http_error",
                        f"El servicio de voz ({speech.base_url}) devolvió HTTP "
                        f"{response.status_code}",
                        http_status=502,
                        numbers={"provider_status": response.status_code},
                    )
                mime_type = response.headers.get("content-type", "").split(";")[0].strip()
                if not mime_type.startswith("audio/") or len(mime_type) > 128:
                    raise HubError(
                        "speech_not_audio",
                        f"El servicio de voz ({speech.base_url}) no devolvió audio",
                        http_status=502,
                    )
                body = bytearray()
                async for chunk in response.aiter_bytes():
                    if len(chunk) > MAX_SPEECH_CLIP_BYTES - len(body):
                        raise HubError(
                            "speech_too_large",
                            "El clip de voz supera el límite de 4 MiB",
                            numbers={
                                "bytes": len(body) + len(chunk),
                                "max_bytes": MAX_SPEECH_CLIP_BYTES,
                            },
                        )
                    body.extend(chunk)
        except HubError:
            raise
        except httpx.TimeoutException as exc:
            report(event="assistant_speech_failed", code="speech_timeout")
            connecting = isinstance(exc, httpx.ConnectTimeout)
            seconds = SPEECH_CONNECT_TIMEOUT_SECONDS if connecting else SPEECH_TIMEOUT_SECONDS
            raise HubError(
                "speech_timeout",
                f"El servicio de voz ({speech.base_url}) no "
                f"{'aceptó la conexión' if connecting else 'contestó'} en {seconds * 1000} ms",
                http_status=504,
                numbers={"timeout_ms": seconds * 1000},
            ) from exc
        except httpx.HTTPError as exc:
            report(event="assistant_speech_failed", code="speech_unreachable")
            raise HubError(
                "speech_unreachable",
                f"No se pudo conectar con el servicio de voz ({speech.base_url})",
                http_status=503,
            ) from exc
        if not body:
            raise HubError(
                "speech_empty", "El servicio de voz entregó un clip vacío", http_status=502
            )
        # The Mac shows where a clip came from: the speech model, as printable ASCII.
        provider = "".join(c if " " <= c <= "~" else "?" for c in speech.model)[:128]
        return SpeechClip(mime_type, provider, bytes(body))


def build_speech(hub: "Settings", agent: "AgentConfiguration") -> OpenAISpeech:
    """The agent's speech (`agents.KINDS`): dictation always (the meeting
    transcriber is mandatory); read-aloud only with `[agents.speech]`, which
    `capabilities.voice` reports."""
    return OpenAISpeech(agent, MeetingTranscriber(hub.meeting_transcription))
