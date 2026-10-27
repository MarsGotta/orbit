"""Write-only credentials and bounded, typed AI settings inputs."""

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

from ..urls import protected_url

Capability = Literal["text", "transcription", "word_timestamps", "speech", "tools"]
FUNCTIONS = {
    "meeting_transcription": "word_timestamps",
    "listening": "transcription",
    "speech": "speech",
    "meeting_notes": "text",
    "vault": "text",
    "assistants": "tools",
}
# Local catalogs are probed, four requests per model. 32 bounds the work;
# cloud catalogs use metadata instead. Neither limits inference requests.
MAX_LOCAL_MODELS = 32
MAX_CLOUD_MODELS = 2048
OPENROUTER_URL = "https://openrouter.ai/api/v1"
MAI = "microsoft/mai-transcribe-2"
# Settings label, URL and encrypted key allocation. Provider keys are normally
# under 200 characters; 4096 leaves margin. These do not limit inference.
# UI copies are compared by contracts/constants.json.
MAX_AI_PROVIDER_NAME_CHARS = 80
MAX_AI_PROVIDER_URL_CHARS = 2048
MAX_AI_PROVIDER_KEY_CHARS = 4096


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True, hide_input_in_errors=True)


class Model(Input):
    id: str = Field(min_length=1, max_length=256)
    capabilities: list[Capability] = Field(max_length=5)


class Provider(Input):
    name: str = Field(min_length=1, max_length=MAX_AI_PROVIDER_NAME_CHARS)
    kind: Literal["local", "openrouter"]
    base_url: str = Field(min_length=1, max_length=MAX_AI_PROVIDER_URL_CHARS)
    api_key: str | None = Field(
        default=None, min_length=1, max_length=MAX_AI_PROVIDER_KEY_CHARS, repr=False
    )
    models: list[Model] | None = Field(default=None, max_length=MAX_CLOUD_MODELS)

    @field_validator("base_url")
    @classmethod
    def url(cls, value):
        protected_url(value, allow_insecure_localhost=True)
        return value.rstrip("/")

    @field_validator("api_key")
    @classmethod
    def token(cls, value):
        if value is not None and any(ord(c) < 33 or ord(c) > 126 for c in value):
            raise ValueError("Key must be a printable token")
        return value


class Choice(Input):
    provider_id: str = Field(min_length=1, max_length=64, pattern=r"^[a-z0-9-]+$")
    model: str = Field(min_length=1, max_length=256)
