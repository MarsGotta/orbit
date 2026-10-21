"""Error reports to Sentry, when the operator asks for them, carrying nothing but codes.

Orbit handles meeting audio, transcripts and conversations, and any of them can
end up in an exception's text, a local variable, a request body or an earlier
log line. So the SDK collects as little as it can (no integrations, no
breadcrumbs, no request bodies, no local variables, no source lines, no
tracing), and ``scrub`` then rebuilds every event from an allow-list: the code
as the message, the tags ``op``, ``code``, ``route`` and ``source``, numbers as
data, and file, function and line of each frame. Everything else is dropped,
the exception's text and ``server_name`` included.

Without an ``[error_reports]`` section the SDK is never initialised and
``capture`` does nothing.
"""

from __future__ import annotations

import sentry_sdk
from pydantic import BaseModel, ConfigDict, field_validator
from sentry_sdk.utils import BadDsn, Dsn

TAGS = ("op", "code", "route", "source")
FRAME_FIELDS = ("filename", "function", "lineno")
# Envelope fields Sentry needs to file the event, plus the hub's version.
KEPT = ("event_id", "timestamp", "level", "platform", "release")


class ErrorReportsConfiguration(BaseModel):
    """``[error_reports]`` in orbit.toml: where the operator's Sentry project receives errors."""

    model_config = ConfigDict(extra="forbid", frozen=True, hide_input_in_errors=True)

    dsn: str

    @field_validator("dsn")
    @classmethod
    def sentry_dsn(cls, value: str) -> str:
        try:
            Dsn(value)
        except BadDsn:
            raise ValueError("error_reports.dsn must be a Sentry DSN") from None
        return value


def start(configuration: ErrorReportsConfiguration, release: str) -> None:
    sentry_sdk.init(
        dsn=configuration.dsn,
        # Explicit, so the SDK does not run git to guess one.
        release=release,
        # Only `capture` below sends anything: no logging, HTTP, framework or
        # thread integration collects on its own.
        default_integrations=False,
        auto_enabling_integrations=False,
        integrations=[],
        send_default_pii=False,
        traces_sample_rate=0,
        include_local_variables=False,
        include_source_context=False,
        max_request_body_size="never",
        max_breadcrumbs=0,
        auto_session_tracking=False,
        send_client_reports=False,
        before_send=scrub,
    )


def stop() -> None:
    """Deliver what is queued, within the SDK's shutdown timeout, and stop sending."""
    client = sentry_sdk.get_client()
    if client.is_active():
        client.close()
        sentry_sdk.get_global_scope().set_client(None)


def capture(
    *,
    code: str,
    op: str,
    route: str | None,
    numbers: dict[str, int | float],
    exception: BaseException | None,
) -> None:
    if not sentry_sdk.get_client().is_active():
        return
    with sentry_sdk.new_scope() as scope:
        scope.set_tag("op", op)
        scope.set_tag("code", code)
        scope.set_tag("source", "hub")
        if route is not None:
            scope.set_tag("route", route)
        for key, value in numbers.items():
            scope.set_extra(key, value)
        if exception is not None:
            sentry_sdk.capture_exception(exception)
        else:
            sentry_sdk.capture_message(code, level="error")


def scrub(event: dict, _hint: dict) -> dict | None:
    """The allow-list: rebuild the event from what may leave, drop the rest."""
    tags = event.get("tags") or {}
    code = tags.get("code")
    if not isinstance(code, str):
        # Not raised through `capture`, so nothing says it is safe to send.
        return None
    clean: dict = {key: event[key] for key in KEPT if key in event}
    clean["message"] = code
    clean["tags"] = {key: tags[key] for key in TAGS if isinstance(tags.get(key), str)}
    figures = {
        key: value
        for key, value in (event.get("extra") or {}).items()
        if type(value) in (int, float)
    }
    if figures:
        clean["extra"] = figures
    values = []
    for exception in (event.get("exception") or {}).get("values") or []:
        kept: dict = {}
        if isinstance(exception.get("type"), str):
            # The class name, a code identifier like the function names below.
            kept["type"] = exception["type"]
        frames = (exception.get("stacktrace") or {}).get("frames") or []
        if frames:
            kept["stacktrace"] = {
                "frames": [
                    {key: frame[key] for key in FRAME_FIELDS if key in frame} for frame in frames
                ]
            }
        values.append(kept)
    if values:
        clean["exception"] = {"values": values}
    return clean
