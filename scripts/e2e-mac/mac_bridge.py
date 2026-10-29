"""Owner-local test bridge client, imported by mac.py; Python 3.9."""

from __future__ import annotations

import json
import socket
import time
from pathlib import Path

from mac_controls import MacFailure


class TestBridge:
    """One bounded JSON line to the in-app Pruebas bridge (Python 3.9).

    A lost press receipt is an uncertain outcome: this client never retries.
    The twelve-second transport bound covers reading, the view and writing
    (4 + 2 + 4 seconds), with two seconds for local scheduling. It does not bound audio or the daemon's operations.
    """

    def __init__(self, path: Path, max_bytes: int):
        self.path = path
        self.max_bytes = max_bytes

    def call(self, request: dict) -> dict:
        line = (json.dumps(request, ensure_ascii=False) + "\n").encode()
        if len(line) > self.max_bytes:
            raise MacFailure(
                f"test bridge request: {len(line)} bytes exceeds {self.max_bytes} bytes"
            )
        try:
            reply = self._exchange(line)
        except socket.timeout:  # noqa: UP041 -- socket.timeout is distinct on the Mac's Python 3.9.
            raise MacFailure(
                f"test bridge transport_timeout after 12 seconds for {request.get('command')}; "
                "outcome unknown; no retry"
            ) from None
        except OSError as cause:
            raise MacFailure(
                f"test bridge transport failed for {request.get('command')}: "
                f"errno {cause.errno}, {cause.strerror}; no retry"
            ) from None
        if len(reply) > self.max_bytes:
            raise MacFailure(
                f"test bridge reply: {len(reply)} bytes exceeds {self.max_bytes} bytes"
            )
        if not reply.endswith(b"\n"):
            raise MacFailure("test bridge closed without a complete reply")
        try:
            result = json.loads(reply)
        except ValueError:
            raise MacFailure(f"test bridge invalid_json_reply ({len(reply)} bytes)") from None
        if not isinstance(result, dict):
            raise MacFailure(f"test bridge reply is {type(result).__name__}, expected an object")
        if not result.get("ok"):
            raise MacFailure("test bridge refused: " + json.dumps(result, ensure_ascii=False))
        return result

    def _exchange(self, line: bytes) -> bytes:
        deadline = time.monotonic() + 12
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(12)
            connection.connect(str(self.path))
            connection.settimeout(max(0.001, deadline - time.monotonic()))
            connection.sendall(line)
            return self._reply(connection, deadline)

    def _reply(self, connection: socket.socket, deadline: float) -> bytes:
        reply = bytearray()
        while len(reply) <= self.max_bytes:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise MacFailure("test bridge reply_timeout after 12 seconds")
            connection.settimeout(remaining)
            part = connection.recv(self.max_bytes + 1 - len(reply))
            if not part:
                break
            reply.extend(part)
            if b"\n" in reply:
                return bytes(reply).split(b"\n", 1)[0] + b"\n"
        return bytes(reply)
