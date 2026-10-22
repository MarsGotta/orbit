"""The `[[agents]]` keys of kind `openai`, read while the configuration loads.

Apart from the backend (`openai`) because that module imports pydantic-ai and
both providers' SDKs, over a second together: the administrator's commands
(`pair-code`, `devices`, `revoke`) load the configuration and must not pay it.
Only building an `openai` agent imports the backend (`agents.KINDS`).
"""

import unicodedata
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal

from ..urls import protected_url


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
    # The model accepts images: the images attached to a turn and screenshots
    # from Orbit's tools reach it as such. False by default, because not every
    # OpenAI-compatible server or model takes them; without it the turn names
    # each image for `orbit_attachment` and a screenshot is described in words.
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
    cause; it reads no file, which `openai.build_backend` checks at startup."""
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
