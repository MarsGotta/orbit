"""Configuration of the one model that processes meeting material.

The model is chosen by the operator, not by the device: any OpenAI-compatible
`chat/completions` endpoint that honours `response_format: json_schema`. The
credential stays on the hub, in a private file, and never travels to the Mac.
"""

from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator

from ..urls import protected_url

# Upper bound of one call. It protects the single processing slot from a
# provider that accepts the request and never answers: while it waits, no other
# note is generated. A four-hour meeting measured well under a minute on
# qwen3.8-flash-next so 900 s is a ceiling
# for slow or remote providers, not an expected duration.
MAX_TIMEOUT_SECONDS = 900
# Ceiling of the answer the model may write. A note of a two-hour meeting used
# about 1 100 completion tokens without reasoning; 32 768 leaves room for
# reasoning models and long templates without letting one answer exhaust memory.
MAX_COMPLETION_TOKENS = 32_768

Reasoning = Literal["provider_default", "off", "on"]


class ProcessingConfiguration(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, hide_input_in_errors=True)

    endpoint: str = Field(min_length=1, max_length=512)
    model: str = Field(min_length=1, max_length=128, pattern=r"^[A-Za-z0-9][A-Za-z0-9_.:/@-]*$")
    credential_file: Path | None
    # "off" asks templates that support it (llama.cpp, vLLM) not to reason
    # first: measured on Qwen, a two-hour note took 30 s instead of 75 s and
    # stayed valid. "provider_default" sends nothing, for providers that reject
    # unknown fields.
    reasoning: Reasoning = "provider_default"
    timeout_seconds: float = Field(default=300.0, gt=0, le=MAX_TIMEOUT_SECONDS)
    max_completion_tokens: int = Field(default=8192, ge=256, le=MAX_COMPLETION_TOKENS)
    # The model's context window, in tokens. Material that would not fit is
    # split before it is sent instead of being refused by the provider.
    context_tokens: int = Field(default=131_072, ge=4096, le=2_000_000)
    allow_insecure_localhost: bool = False

    @field_validator("credential_file")
    @classmethod
    def absolute_credential(cls, value: Path | None) -> Path | None:
        if value is None:
            return None
        value = value.expanduser()
        if not value.is_absolute():
            raise ValueError("processing.credential_file must be an absolute path")
        return value

    @model_validator(mode="after")
    def explicit_destination(self):
        url = protected_url(self.endpoint, allow_insecure_localhost=self.allow_insecure_localhost)
        if not url.path.endswith("/chat/completions"):
            raise ValueError("processing.endpoint must end in /chat/completions")
        return self
