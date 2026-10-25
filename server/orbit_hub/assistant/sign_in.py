"""Sign in and out with each vendor's official executable.

The CLI alone writes credentials into the agent's private account folder.
Orbit forwards only the sign-in address and one-time code, never credentials
or diagnostic output. Codex signs in in a temporary home: an unsuccessful
attempt must not destroy the existing account. The completed auth.json is
moved whole without being read.
"""

from __future__ import annotations

import asyncio
import os
import re
import shutil
import signal
import time
from contextlib import suppress
from dataclasses import dataclass, field
from pathlib import Path
from typing import TYPE_CHECKING
from uuid import uuid4

from ..errors import HubError
from ..log import report
from .tool_availability import launch_failure
from .tool_settings import HOME_FOLDERS, ToolSettings
from .tool_settings import environment as tool_environment

if TYPE_CHECKING:
    from ..config import Settings
    from .agents import Agent, Agents

# How long one sign-in may wait for the user. What it protects: a process of
# the vendor's tool, which Claude Code would keep forever waiting for a code.
# Why 15 minutes: Codex's one-time code expires then. It does not bound a
# session once signed in.
SIGN_IN_TIMEOUT_SECONDS = 15 * 60
# How long the tool may take to show its address. Not measured against the
# real tools (none runs in development): a generous budget for a Node start
# and one request to the vendor, the same as an agent's start (`official_tools`).
URL_TIMEOUT_SECONDS = 30
# Signing out revokes the account at the vendor: one request, then exit.
SIGN_OUT_TIMEOUT_SECONDS = 30
# One line of the tool's output; a longer one is not an address, and ends the read.
MAX_LINE_BYTES = 64 * 1024
# An address the Mac opens. The vendors' are a few hundred characters.
MAX_URL_CHARS = 2048
# What the Mac sends for Claude Code: the code its callback page shows,
# `<code>#<state>`, each a URL-safe token. Only these characters, so nothing
# else (a line break, another answer) can reach the tool's input.
PASTED_CODE = re.compile(r"[A-Za-z0-9._~-]{1,1024}#[A-Za-z0-9._~-]{1,1024}")
# The terminal colours and links Codex prints even into a pipe.
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")
URL = re.compile(r"https://[^\s\x00-\x1f\x7f]+")
# Codex's one-time code, on the line after the one that announces it.
DEVICE_CODE = re.compile(r"[A-Z0-9]{3,12}(?:-[A-Z0-9]{3,12}){0,3}")
# What a tool's failure says, as a cause the Mac can tell apart. The hub
# keeps only the cause, never the tool's text.
CAUSES = (
    ("not enabled", "device_code_disabled"),
    ("timed out", "expired"),
    ("login is disabled", "login_disabled"),
    ("invalid authorization code", "code_rejected"),
    ("invalid code", "code_rejected"),
)


@dataclass
class Attempt:
    agent_id: str
    service: str
    deadline_ms: int
    process: asyncio.subprocess.Process | None = None
    state: str = "waiting"
    url: str | None = None
    code: str | None = None
    cause: str | None = None
    exit_status: int | None = None
    # Codex's own home for this sign-in, removed when it ends.
    home: Path | None = None
    shown: asyncio.Event = field(default_factory=asyncio.Event)
    task: asyncio.Task | None = None

    def public(self) -> dict:
        data: dict = {"agent_id": self.agent_id, "state": self.state}
        if self.state == "waiting":
            data["expires_at_ms"] = self.deadline_ms
            data["expects_code"] = self.service == "claude-code"
            if self.url is not None:
                data["url"] = self.url
            if self.code is not None:
                data["code"] = self.code
        if self.cause is not None:
            data["cause"] = self.cause
        if self.exit_status is not None:
            data["exit_status"] = self.exit_status
        return data


def tool_settings_of(agent: Agent) -> ToolSettings:
    if agent.kind not in HOME_FOLDERS:
        raise HubError(
            "sign_in_unsupported",
            f"{agent.name} no inicia sesión desde Orbit: lo configura quien gestiona tu hub",
            http_status=409,
            numbers={"agent_id": agent.id, "kind": agent.kind},
        )
    return agent.settings


def login_arguments(settings: ToolSettings) -> list[str]:
    """The official executable, using the subscription's own sign-in."""
    if settings.agent == "claude-code":
        return [*settings.command, "auth", "login", "--claudeai"]
    return [*settings.command, "login", "--device-auth"]


def logout_arguments(settings: ToolSettings) -> list[str]:
    if settings.agent == "claude-code":
        return [*settings.command, "auth", "logout"]
    return [*settings.command, "logout"]


def environment(settings: ToolSettings, home: Path) -> dict[str, str]:
    return tool_environment(settings, home)


def private_folder(path: Path) -> None:
    """A folder of the hub's alone, `0700`; a link or a file in its place is refused."""
    if path.is_symlink() or (path.exists() and not path.is_dir()):
        raise HubError(
            "agent_folder_invalid",
            f"La carpeta del agente no es un directorio: {path.name}",
            http_status=500,
        )
    if not path.exists():
        path.mkdir(mode=0o700)
        path.chmod(0o700)


def cause_of(line: str) -> str | None:
    lowered = line.lower()
    return next((cause for phrase, cause in CAUSES if phrase in lowered), None)


class SignIns:
    """One sign-in at a time per agent, and its outcome until the next one."""

    def __init__(self, settings: Settings, agents: Agents, now=time.time):
        self.settings, self.agents, self.now = settings, agents, now
        self.attempts: dict[str, Attempt] = {}

    def folders(self, agent: Agent, service: str) -> tuple[Path, Path]:
        folder = self.settings.data_dir / "agents" / agent.id
        return folder, folder / HOME_FOLDERS[service]

    async def start(self, agent_id: str) -> dict:
        agent = self.agents.get(agent_id)
        settings = tool_settings_of(agent)
        current = self.attempts.get(agent.id)
        if current is not None and current.state == "waiting":
            return current.public()
        folder, home = self.folders(agent, settings.agent)
        for path in (folder.parent, folder, home):
            private_folder(path)
        attempt = Attempt(
            agent.id, settings.agent, round((self.now() + SIGN_IN_TIMEOUT_SECONDS) * 1000)
        )
        if settings.agent == "codex":
            attempt.home = folder / f"sign-in-{uuid4().hex}"
            private_folder(attempt.home)
        self.attempts[agent.id] = attempt
        try:
            attempt.process = await asyncio.create_subprocess_exec(
                *login_arguments(settings),
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                env=environment(settings, attempt.home or home),
                cwd=folder,
                # Its own process group, so cancelling reaches what it started.
                start_new_session=True,
                limit=MAX_LINE_BYTES,
            )
        except OSError as exc:
            self._end(attempt, "failed", "not_started")
            raise launch_failure(settings.agent, settings.command[0], exc) from None
        attempt.task = asyncio.create_task(self._run(attempt, home))
        try:
            async with asyncio.timeout(URL_TIMEOUT_SECONDS):
                await attempt.shown.wait()
        except TimeoutError:
            await self._stop(attempt, "failed", "no_url")
        return attempt.public()

    def status(self, agent_id: str) -> dict:
        agent = self.agents.get(agent_id)
        tool_settings_of(agent)
        attempt = self.attempts.get(agent.id)
        if attempt is None:
            raise HubError(
                "sign_in_not_found",
                f"No hay ningún inicio de sesión de {agent.name}",
                http_status=404,
                numbers={"agent_id": agent.id},
            )
        return attempt.public()

    async def submit_code(self, agent_id: str, code: str) -> dict:
        """Hand the tool the code its page showed; the outcome is read with `status`."""
        attempt = self.attempts.get(agent_id)
        process = attempt.process if attempt is not None else None
        if (
            attempt is None
            or attempt.state != "waiting"
            or attempt.service != "claude-code"
            or process is None
            or process.stdin is None
        ):
            raise HubError(
                "sign_in_not_waiting_for_code",
                "Este inicio de sesión no espera ningún código",
                http_status=409,
                numbers={"agent_id": agent_id},
            )
        if PASTED_CODE.fullmatch(code) is None:
            raise HubError(
                "sign_in_code_invalid",
                "El código no es el que da la página de Claude: cópialo entero, con su «#»",
                http_status=422,
                numbers={"agent_id": agent_id, "characters": len(code)},
            )
        with suppress(ConnectionError, RuntimeError):
            process.stdin.write(code.encode("ascii") + b"\n")
            await process.stdin.drain()
        return attempt.public()

    async def cancel(self, agent_id: str) -> dict | None:
        """Stop a sign-in under way; the agent's account stays as it was."""
        attempt = self.attempts.get(agent_id)
        if attempt is None:
            return None
        if attempt.state == "waiting":
            await self._stop(attempt, "cancelled", None)
        return attempt.public()

    async def sign_out(self, agent_id: str) -> None:
        """The vendor's own sign-out, which revokes the account and deletes
        its credentials from the agent's folder."""
        agent = self.agents.get(agent_id)
        settings = tool_settings_of(agent)
        await self.cancel(agent.id)
        folder, home = self.folders(agent, settings.agent)
        if not home.is_dir():
            self.agents.observe(agent.id, "signed_out")
            return
        try:
            process = await asyncio.create_subprocess_exec(
                *logout_arguments(settings),
                stdin=asyncio.subprocess.DEVNULL,
                stdout=asyncio.subprocess.DEVNULL,
                stderr=asyncio.subprocess.DEVNULL,
                env=environment(settings, home),
                cwd=folder,
                start_new_session=True,
            )
        except OSError as exc:
            raise HubError(
                "sign_out_failed",
                f"No arrancó el cierre de sesión de {agent.name} en el hub ({exc.strerror})",
                http_status=502,
                numbers={"agent_id": agent.id},
            ) from None
        try:
            async with asyncio.timeout(SIGN_OUT_TIMEOUT_SECONDS):
                code = await process.wait()
        except TimeoutError:
            await _terminate(process)
            raise HubError(
                "sign_out_timeout",
                f"El cierre de sesión de {agent.name} no terminó en {SIGN_OUT_TIMEOUT_SECONDS} s",
                http_status=504,
                numbers={"agent_id": agent.id, "timeout_ms": SIGN_OUT_TIMEOUT_SECONDS * 1000},
            ) from None
        if code != 0:
            report("sign_out_failed", "sign_out_failed", numbers={"exit_status": code}, exc_info=False)  # fmt: skip
            raise HubError(
                "sign_out_failed",
                f"{agent.name} no pudo cerrar la sesión (código {code})",
                http_status=502,
                numbers={"agent_id": agent.id, "exit_status": code},
            )
        self.agents.observe(agent.id, "signed_out")

    async def close(self) -> None:
        for attempt in list(self.attempts.values()):
            if attempt.state == "waiting":
                await self._stop(attempt, "cancelled", None)

    async def _run(self, attempt: Attempt, home: Path) -> None:
        process = attempt.process
        try:
            async with asyncio.timeout(SIGN_IN_TIMEOUT_SECONDS):
                await asyncio.gather(
                    self._read(attempt, process.stdout, shows=True),
                    self._read(attempt, process.stderr, shows=False),
                )
                code = await process.wait()
        except TimeoutError:
            await self._stop(attempt, "failed", "timed_out")
            return
        if attempt.state != "waiting":
            return
        if code != 0:
            attempt.exit_status = code
            cause = attempt.cause or "exited"
            report("sign_in_failed", cause, numbers={"exit_status": code}, exc_info=False)
            self._end(attempt, "failed", cause)
            return
        if attempt.home is not None:
            # Codex's finished sign-in: its credentials, moved whole without
            # being read, over the agent's own.
            source = attempt.home / "auth.json"
            if not source.is_file() or source.is_symlink():
                self._end(attempt, "failed", "no_credentials")
                return
            os.replace(source, home / "auth.json")
        self._end(attempt, "done", None)
        self.agents.observe(attempt.agent_id, "signed_in")

    async def _read(self, attempt: Attempt, stream, *, shows: bool) -> None:
        """Each line once, for the address, the code or a failure's cause;
        the line itself is dropped."""
        announced = False
        while True:
            try:
                raw = await stream.readline()
            except ValueError:
                return
            if not raw:
                return
            line = ANSI.sub("", raw.decode("utf-8", "replace")).strip()
            if not shows:
                attempt.cause = cause_of(line) or attempt.cause
                continue
            if attempt.url is None:
                found = URL.search(line)
                if found is not None and len(found.group()) <= MAX_URL_CHARS:
                    attempt.url = found.group()
                    if attempt.service == "claude-code":
                        attempt.shown.set()
            elif attempt.service == "codex" and attempt.code is None:
                if "code" in line.lower():
                    announced = True
                elif announced and DEVICE_CODE.fullmatch(line):
                    attempt.code = line
                    attempt.shown.set()

    async def _stop(self, attempt: Attempt, state: str, cause: str | None) -> None:
        if attempt.state != "waiting":
            return
        attempt.state, attempt.cause = state, cause
        attempt.shown.set()
        if attempt.process is not None:
            await _terminate(attempt.process)
        if attempt.task is not None and attempt.task is not asyncio.current_task():
            await asyncio.gather(attempt.task, return_exceptions=True)
        self._end(attempt, state, cause)

    def _end(self, attempt: Attempt, state: str, cause: str | None) -> None:
        """The attempt's outcome; Codex's own home for it goes, whatever is in it."""
        attempt.state, attempt.cause = state, cause
        attempt.shown.set()
        if attempt.home is not None:
            shutil.rmtree(attempt.home, ignore_errors=True)


async def _terminate(process: asyncio.subprocess.Process) -> None:
    """The tool and whatever it started, by its process group."""
    if process.returncode is not None:
        return
    with suppress(ProcessLookupError, PermissionError):
        os.killpg(process.pid, signal.SIGTERM)
    try:
        async with asyncio.timeout(2):
            await process.wait()
    except TimeoutError:
        with suppress(ProcessLookupError, PermissionError):
            os.killpg(process.pid, signal.SIGKILL)
        await process.wait()
