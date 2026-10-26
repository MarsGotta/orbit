"""Exact-body HMAC with bounded in-memory replay protection.

Protocol and path are signed; possessing the key is not a durable grant.
"""

import hashlib
import hmac
import json
import math
import re
from pathlib import Path

from pydantic import BaseModel, ConfigDict, Field, model_validator

from .identifiers import canonical_uuid
from .log import report
from .secret_file import read_secret
from .unix_time import MAX_UNIX_MS

MAX_NONCES = 4096
SKEW_MS = 30_000
RETAIN_SECONDS = 60
HEADER_NAMES = ("key-id", "issued-at-ms", "nonce", "signature")
HEADER_PREFIXES = {
    "orbit_device_bridge_v1": "x-orbit-bridge-",
    "orbit_meetings_bridge_v1": "x-orbit-meetings-",
}
KEY_BYTES = 32
CLOCK_REGRESSION_TOLERANCE_SECONDS = 5
CLOCK_REGRESSIONS_BEFORE_LATCH = 3


def read_key(path, failure):
    """Read a key only when all strict secret-file attributes hold."""
    try:
        key = read_secret(path, KEY_BYTES)
        if len(key) != KEY_BYTES:
            raise ValueError
        return key
    except (OSError, ValueError):
        raise failure(503, "unavailable") from None


def signing_bytes(protocol, path, key_id, issued_at_ms, nonce, body):
    return "\n".join(
        (
            protocol,
            "POST",
            path,
            key_id,
            str(issued_at_ms),
            nonce,
            hashlib.sha256(body).hexdigest(),
        )
    ).encode("utf-8")


def sign_request(protocol, path, key_id, key, body, *, issued_at_ms, nonce):
    prefix = HEADER_PREFIXES[protocol]
    signature = hmac.new(
        key,
        signing_bytes(protocol, path, key_id, issued_at_ms, nonce, body),
        hashlib.sha256,
    ).hexdigest()
    values = (key_id, str(issued_at_ms), nonce, signature)
    return {f"{prefix}{name}": value for name, value in zip(HEADER_NAMES, values, strict=True)}


class Authenticator:
    """Event-loop-owned, with no await between verification and nonce admission."""

    def __init__(self, key_id, key, *, protocol, path, header_prefix, failure, credentials=None):
        self.key_id, self._key = key_id, key
        self._credentials = credentials
        self._protocol, self._path = protocol, path
        self._prefix = header_prefix
        self._failure = failure
        self._nonces = {}
        self._wall = None
        self._monotonic = None
        self._clock_failed = False
        self._clock_regressions = 0

    def check_clock(self, now_ms, monotonic_now):
        """Tolerate one bounded regression, but latch on unsafe or repeated drift."""
        invalid = (
            type(now_ms) is not int
            or not 0 <= now_ms <= MAX_UNIX_MS
            or type(monotonic_now) not in (int, float)
            or not math.isfinite(monotonic_now)
            or monotonic_now < 0
        )
        if self._clock_failed or invalid:
            self._clock_failed = True
            raise self._failure(503, "consumer_clock")
        wall_regression = max(0, (self._wall or now_ms) - now_ms) / 1000
        monotonic_regression = max(0, (self._monotonic or monotonic_now) - monotonic_now)
        regression = max(wall_regression, monotonic_regression)
        if regression:
            self._clock_regressions += 1
            report(
                event="private_clock_regression",
                code="consumer_clock",
                exc_info=False,
            )
            if (
                regression > CLOCK_REGRESSION_TOLERANCE_SECONDS
                or self._clock_regressions >= CLOCK_REGRESSIONS_BEFORE_LATCH
            ):
                self._clock_failed = True
                raise self._failure(503, "consumer_clock")
            return
        self._clock_regressions = 0
        self._wall, self._monotonic = now_ms, monotonic_now

    def admit(self, headers, body, now_ms, monotonic_now):
        values = {}
        for suffix in HEADER_NAMES:
            name = f"{self._prefix}{suffix}".encode("ascii")
            matches = [value for key, value in headers if key.lower() == name]
            if len(matches) != 1 or len(matches[0]) > 256:
                raise self._failure(401, "unauthorized")
            try:
                values[suffix] = matches[0].decode("ascii")
            except UnicodeError:
                raise self._failure(401, "unauthorized") from None
        key_id, issued, nonce, signature = (values[name] for name in HEADER_NAMES)
        key, agent_id = self.credential(key_id)
        try:
            if (
                key is None
                or re.fullmatch(r"[A-Za-z0-9_-]{1,64}", key_id) is None
                or re.fullmatch(r"0|[1-9][0-9]{0,14}", issued) is None
                or int(issued) > MAX_UNIX_MS
                or re.fullmatch(r"[a-f0-9]{64}", signature) is None
            ):
                raise ValueError
            canonical_uuid(nonce)
        except (ValueError, TypeError):
            raise self._failure(401, "unauthorized") from None
        expected = hmac.new(
            key,
            signing_bytes(self._protocol, self._path, key_id, int(issued), nonce, body),
            hashlib.sha256,
        ).hexdigest()
        if not hmac.compare_digest(signature, expected):
            raise self._failure(401, "unauthorized")
        self.check_clock(now_ms, monotonic_now)
        issued = int(issued)
        if abs(now_ms - issued) > SKEW_MS:
            raise self._failure(401, "consumer_clock")
        expired = [
            item
            for item, (timestamp, admitted) in self._nonces.items()
            if now_ms > timestamp + SKEW_MS and monotonic_now - admitted >= RETAIN_SECONDS
        ]
        for item in expired:
            del self._nonces[item]
        if nonce in self._nonces:
            raise self._failure(401, "consumer_replay")
        if len(self._nonces) >= MAX_NONCES:
            raise self._failure(429, "consumer_busy")
        self._nonces[nonce] = (issued, monotonic_now)
        if agent_id is not None and body:
            try:
                claimed = json.loads(body).get("agent_id")
            except (ValueError, AttributeError, UnicodeError, RecursionError):
                raise self._failure(400, "invalid_request") from None
            if claimed is not None and claimed != agent_id:
                raise self._failure(403, "agent_identity_mismatch")
        return agent_id

    def credential(self, key_id):
        if self._credentials is not None:
            bound = self._credentials(key_id, self._protocol)
            if bound is not None:
                return bound
        if key_id == self.key_id:
            return self._key, None
        return None, None


def canonical(value: dict) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode(
        "utf-8"
    )


class PrivateModel(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True, frozen=True)


# Bounds the synchronous action response latency, not execution or receipt retention.
# 2.5 s hides the extra MCP round trip for short actions while slow actions retain
# their existing operation lookup. Operators can choose 0 to return immediately.
ACTION_INLINE_WAIT_MS = 2500


class BridgeConfiguration(PrivateModel):
    action_inline_wait_seconds: float = Field(
        default=ACTION_INLINE_WAIT_MS / 1000, ge=0, le=30, allow_inf_nan=False
    )
    key_id: str = Field(pattern=r"^[A-Za-z0-9_-]{1,64}$")
    credential_file: str = Field(min_length=1, max_length=4096, repr=False)

    @model_validator(mode="after")
    def explicit(self):
        if "\x00" in self.credential_file or not Path(self.credential_file).is_absolute():
            raise ValueError("Bridge requires an explicit key id and private key path")
        return self
