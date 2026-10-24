"""ACP's newline-delimited JSON over an agent process's stdio."""

import asyncio
import json
from contextlib import suppress
from typing import Any

from acp.schema import SessionNotification
from pydantic import ValidationError

from ..log import report


class LineTransport:
    """ACP's newline-delimited JSON over an agent's stdio, with a ceiling per message.

    The SDK's own transport reads a line of any length; this one stops at
    `MAX_AGENT_MESSAGE_BYTES` (the stream's limit) and ends the connection,
    recording why.

    It also drops a `session/update` the SDK could not read (a kind newer than
    its schema): the SDK would log the validation error on the root logger,
    and that error quotes the update, which is what the agent said. Here it is
    counted by its code alone.
    """

    def __init__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
        self.reader, self.writer = reader, writer
        self.overrun = False
        self._sending = asyncio.Lock()

    async def send(self, message: dict[str, Any]) -> None:
        data = (json.dumps(message, separators=(",", ":")) + "\n").encode()
        async with self._sending:
            self.writer.write(data)
            await self.writer.drain()

    async def receive(self) -> dict[str, Any] | None:
        while True:
            try:
                line = await self.reader.readuntil(b"\n")
            except asyncio.LimitOverrunError:
                self.overrun = True
                return None
            except asyncio.IncompleteReadError as exc:
                line = exc.partial
                if not line.strip():
                    return None
            if not line.strip():
                continue
            try:
                message = json.loads(line)
            except ValueError:
                report("acp_message_unreadable", "upstream_protocol", exc_info=False)
                continue
            if not isinstance(message, dict):
                continue
            if message.get("method") == "session/update" and "id" not in message:
                try:
                    SessionNotification.model_validate(message.get("params"))
                except ValidationError:
                    report("acp_update_unreadable", "upstream_protocol", exc_info=False)
                    continue
            return message

    async def close(self) -> None:
        with suppress(OSError, RuntimeError):
            if not self.writer.is_closing():
                self.writer.close()
