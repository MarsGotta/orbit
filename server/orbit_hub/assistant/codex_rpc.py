"""Bounded JSON-RPC stdio transport for the official Codex app-server."""

import asyncio
import json
import os
import signal
from contextlib import suppress

from ..errors import HubError
from .official_tools import MAX_AGENT_MESSAGE_BYTES, START_TIMEOUT_SECONDS
from .tool_availability import launch_failure

# A broken server must not accumulate tasks waiting for human input. Sixteen
# concurrent callbacks allow batched questions across a turn; not a turn cap.
MAX_SERVER_REQUESTS = 16
# Grace for the CLI and its MCP children after stdin closes, before signals.
STOP_GRACE_SECONDS = 2


class RpcError(Exception):
    def __init__(self, error):
        self.code = error.get("code")
        self.data = error.get("data")
        # Never put the vendor's error message in exception text or a log.
        super().__init__("Codex refused a protocol request")


class CodexRpc:
    def __init__(self, backend, notification, request, jail_prefix):
        self.backend, self.notification, self.request = backend, notification, request
        self.jail_prefix = jail_prefix
        self.process = None
        self.reader = None
        self.pending = {}
        self.callbacks = set()
        self.serial = 0
        self.sending = asyncio.Lock()
        self.failure = None
        self.closed = False

    async def open(self):
        try:
            self.process = await asyncio.create_subprocess_exec(
                *self.jail_prefix,
                *self.backend.settings.command,
                "app-server",
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.DEVNULL,
                cwd=self.backend.workspaces.current(),
                env=self.backend.environment(),
                start_new_session=True,
                limit=MAX_AGENT_MESSAGE_BYTES,
            )
        except OSError as exc:
            raise launch_failure("codex", self.backend.settings.command[0], exc) from None
        if self.closed:
            await self.stop_process()
            raise HubError("agent_stopped", "Codex se cerró durante su arranque")
        self.reader = asyncio.create_task(self.read())
        from ..app import hub_version

        await self.call(
            "initialize",
            {
                "clientInfo": {
                    "name": "orbit-hub",
                    "title": "Orbit",
                    "version": hub_version(),
                },
                "capabilities": {"experimentalApi": True},
            },
        )
        await self.send({"method": "initialized"})

    async def send(self, message):
        async with self.sending:
            self.process.stdin.write((json.dumps(message) + "\n").encode())
            await self.process.stdin.drain()

    async def call(self, method, params, timeout=START_TIMEOUT_SECONDS):
        if self.closed:
            raise HubError("agent_stopped", "Codex ya está cerrado")
        if self.failure:
            raise self.failure
        self.serial += 1
        request_id = self.serial
        future = asyncio.get_running_loop().create_future()
        self.pending[request_id] = future
        try:
            await self.send({"id": request_id, "method": method, "params": params})
            return await asyncio.wait_for(future, timeout)
        except TimeoutError:
            raise HubError(
                "agent_timeout",
                f"Codex no respondió a {method} en {timeout} s",
                numbers={"timeout_ms": timeout * 1000},
            ) from None
        finally:
            self.pending.pop(request_id, None)

    async def read(self):
        try:
            while (message := await self.receive()) is not None:
                if not isinstance(message, dict):
                    raise TypeError
                if "method" not in message:
                    future = self.pending.get(message.get("id"))
                    if future is not None and not future.done():
                        if "error" in message:
                            future.set_exception(RpcError(message["error"]))
                        else:
                            future.set_result(message.get("result"))
                elif "id" in message:
                    if len(self.callbacks) >= MAX_SERVER_REQUESTS:
                        raise HubError("agent_protocol", "Codex superó 16 peticiones simultáneas")
                    task = asyncio.create_task(self.respond(message))
                    self.callbacks.add(task)
                    task.add_done_callback(self.callbacks.discard)
                else:
                    await self.notification(message["method"], message.get("params", {}))
        except asyncio.CancelledError:
            raise
        except (ValueError, TypeError, KeyError):
            self.failure = HubError("agent_protocol", "Codex envió un mensaje JSON inválido")
        except HubError as exc:
            self.failure = exc
        except Exception:  # noqa: BLE001 - vendor input must end the request without leaking diagnostics
            self.failure = HubError("agent_protocol", "No se pudo leer la salida de Codex")
        finally:
            await self.stop_process()
            self.failure = self.failure or HubError(
                "agent_stopped",
                f"Codex terminó con código {self.process.returncode}",
                numbers={"exit_status": self.process.returncode},
            )
            for future in self.pending.values():
                if not future.done():
                    future.set_exception(self.failure)
            await self.notification("orbit/transportClosed", {})

    async def receive(self):
        try:
            line = await self.process.stdout.readuntil(b"\n")
        except asyncio.IncompleteReadError as exc:
            line = exc.partial
            if not line:
                return None
        except asyncio.LimitOverrunError:
            raise HubError(
                "agent_stopped",
                f"Codex envió un mensaje mayor de {MAX_AGENT_MESSAGE_BYTES} bytes",
                numbers={"max_bytes": MAX_AGENT_MESSAGE_BYTES},
            ) from None
        return json.loads(line)

    async def respond(self, message):
        try:
            result = await self.request(message["method"], message.get("params", {}))
            await self.send({"id": message["id"], "result": result})
        except asyncio.CancelledError:
            raise
        except Exception:  # noqa: BLE001 - vendor input must end the request without leaking diagnostics
            with suppress(ConnectionError, OSError):
                await self.send(
                    {
                        "id": message["id"],
                        "error": {
                            "code": -32601,
                            "message": "Orbit does not support this request",
                        },
                    }
                )

    async def close(self):
        self.closed = True
        for task in self.callbacks:
            task.cancel()
        await asyncio.gather(*self.callbacks, return_exceptions=True)
        await self.stop_process()
        if self.reader is not None:
            self.reader.cancel()
            await asyncio.gather(self.reader, return_exceptions=True)

    async def stop_process(self):
        if self.process is None:
            return
        with suppress(OSError, RuntimeError):
            self.process.stdin.close()
        try:
            await asyncio.wait_for(self.process.wait(), STOP_GRACE_SECONDS)
        except TimeoutError:
            await self.terminate()
        # Reach only descendants of the process this transport started.
        with suppress(ProcessLookupError, PermissionError):
            os.killpg(self.process.pid, signal.SIGTERM)

    async def terminate(self):
        for sig in (signal.SIGTERM, signal.SIGKILL):
            with suppress(ProcessLookupError, PermissionError):
                os.killpg(self.process.pid, sig)
            try:
                await asyncio.wait_for(self.process.wait(), STOP_GRACE_SECONDS)
                return
            except TimeoutError:
                continue
