"""An invented ACP agent, run as a subprocess by the tests of `orbit_hub.assistant.acp`.

It speaks the agent side of the same SDK the hub uses as a client, and plays
what Claude Code's and Codex's adapters do at the protocol level: it starts the
MCP servers the client names in `session/new` and calls their tools, asks
permission, honours `session/cancel`, and replays a conversation on
`session/load`. What the prompt says chooses the turn:

    nota      file and terminal requests (refused), an orbit-mcp call, a permission
    trabaja   a thought, two tool calls (one fails), a plan that advances, an answer
    espera    a permission that only a cancellation ends
    inicia    the agent asks to sign in
    muere     the process exits in the middle of the turn
    enorme    one message over the hub's ceiling

Flags: `--record PATH` writes what the agent saw as JSON lines;
`--auth-required` refuses `session/new` the way the adapters do when signed
out; `--exit-before-initialize` exits at once; `--no-load` declares no
`loadSession`.
"""

import asyncio
import json
import os
import sys

import acp
from acp.schema import (
    AgentCapabilities,
    LoadSessionResponse,
    NewSessionResponse,
    PermissionOption,
    PromptResponse,
    ToolCallUpdate,
)
from mcp import ClientSession, StdioServerParameters, stdio_client

ENVIRONMENT_SEEN = (
    "CLAUDE_CONFIG_DIR",
    "CODEX_HOME",
    "CODEX_CONFIG",
    "INITIAL_AGENT_MODE",
    "ORBIT_ACP_TEST_ALLOWED",
    "ORBIT_ACP_TEST_WITHHELD",
)


class InventedAgent:
    def __init__(self, record_path, *, auth_required, load):
        self.record_path = record_path
        self.auth_required = auth_required
        self.load = load
        self.client = None
        self.sessions = {}
        self.cancelled = {}

    def on_connect(self, conn):
        self.client = conn

    def record(self, kind, **fields):
        if self.record_path:
            with open(self.record_path, "a", encoding="utf-8") as handle:
                handle.write(json.dumps({"kind": kind, **fields}, ensure_ascii=False) + "\n")

    async def initialize(self, protocol_version, client_capabilities=None, client_info=None, **_):
        self.record(
            "initialize",
            protocol_version=protocol_version,
            client_capabilities=client_capabilities.model_dump(
                mode="json", by_alias=True, exclude_none=True
            ),
            client_info=client_info.model_dump(mode="json") if client_info else None,
            environment={name: os.environ.get(name) for name in ENVIRONMENT_SEEN},
            cwd=os.getcwd(),
        )
        return acp.InitializeResponse(
            protocol_version=1,
            agent_capabilities=AgentCapabilities(load_session=self.load),
            auth_methods=[],
        )

    async def new_session(self, cwd, additional_directories=None, mcp_servers=None, **meta):
        servers = [
            server.model_dump(mode="json", by_alias=True, exclude_none=True)
            for server in mcp_servers or []
        ]
        self.record("session/new", cwd=cwd, mcp_servers=servers, meta=meta)
        if self.auth_required:
            raise acp.RequestError.auth_required()
        session_id = f"sesion-{len(self.sessions) + 1}-{os.getpid()}"
        self.sessions[session_id] = servers
        return NewSessionResponse(session_id=session_id)

    async def load_session(
        self, cwd, session_id, mcp_servers=None, additional_directories=None, **meta
    ):
        servers = [
            server.model_dump(mode="json", by_alias=True, exclude_none=True)
            for server in mcp_servers or []
        ]
        self.record("session/load", cwd=cwd, session_id=session_id, meta=meta)
        self.sessions[session_id] = servers
        # A load replays the conversation before it answers.
        await self.client.session_update(
            session_id, acp.update_user_message_text("Pregunta de antes")
        )
        await self.client.session_update(
            session_id, acp.update_agent_message_text("Respuesta de antes")
        )
        return LoadSessionResponse()

    async def cancel(self, session_id, **_):
        self.record("session/cancel", session_id=session_id)
        self.cancelled.setdefault(session_id, asyncio.Event()).set()

    async def say(self, session_id, text):
        await self.client.session_update(session_id, acp.update_agent_message_text(text))

    async def ask(self, session_id, title):
        return await self.client.request_permission(
            session_id=session_id,
            tool_call=ToolCallUpdate(tool_call_id="llamada-1", title=title),
            options=[
                PermissionOption(option_id="si-una-vez", name="Permitir", kind="allow_once"),
                PermissionOption(option_id="siempre", name="Siempre", kind="allow_always"),
                PermissionOption(option_id="no", name="Rechazar", kind="reject_once"),
            ],
        )

    async def refused(self, kind, call):
        try:
            await call
        except acp.RequestError as exc:
            self.record(kind, code=exc.code, message=str(exc), data=exc.data)
        else:
            self.record(kind, code=None)

    async def call_orbit(self, session_id):
        [server] = [s for s in self.sessions[session_id] if s["name"] == "orbit"]
        parameters = StdioServerParameters(
            command=server["command"],
            args=server["args"],
            env={variable["name"]: variable["value"] for variable in server["env"]},
        )
        async with (
            stdio_client(parameters) as (read_stream, write_stream),
            ClientSession(read_stream, write_stream) as session,
        ):
            await session.initialize()
            result = await session.call_tool("orbit_devices", {})
        text = "".join(block.text for block in result.content if block.type == "text")
        self.record("orbit_devices", is_error=result.is_error, text=text)

    async def prompt(self, session_id, prompt, **_):
        text = "".join(block.text for block in prompt if block.type == "text")
        self.record("session/prompt", session_id=session_id, text=text)
        if "nota" in text:
            await self.refused(
                "fs/read_text_file",
                self.client.read_text_file(session_id=session_id, path="/etc/hostname"),
            )
            await self.refused(
                "terminal/create",
                self.client.create_terminal(session_id=session_id, command="/bin/true"),
            )
            await self.client.session_update(
                session_id, acp.start_tool_call("llamada-0", "orbit_devices", kind="fetch")
            )
            try:
                await self.call_orbit(session_id)
            except Exception as exc:
                self.record("orbit_devices", error=repr(exc))
                raise
            await self.client.session_update(
                session_id,
                acp.update_plan([acp.plan_entry("Mirar los equipos"), acp.plan_entry("Crear")]),
            )
            await self.say(session_id, "Voy a crear la nota. ")
            answer = await self.ask(session_id, "Crear la nota «Plan de la excursión»")
            self.record(
                "permission",
                outcome=answer.outcome.model_dump(mode="json", by_alias=True, exclude_none=True),
            )
            if getattr(answer.outcome, "option_id", None) == "si-una-vez":
                await self.say(session_id, "Hecha.")
            else:
                await self.say(session_id, "No la creo.")
            return PromptResponse(stop_reason="end_turn")
        if "trabaja" in text:
            await self.client.session_update(
                session_id, acp.update_agent_thought_text("Miro primero la pantalla, ")
            )
            await self.client.session_update(
                session_id, acp.update_agent_thought_text("luego el calendario.")
            )
            look = acp.start_tool_call(
                "llamada-mirar", "mcp__orbit__orbit_look", kind="read", raw_input={}
            )
            # claude-agent-acp names the tool in its own meta field.
            look.field_meta = {"claudeCode": {"toolName": "mcp__orbit__orbit_look"}}
            await self.client.session_update(session_id, look)
            await self.client.session_update(
                session_id,
                acp.update_plan(
                    [
                        acp.plan_entry("Mirar la pantalla", status="in_progress"),
                        acp.plan_entry("Leer el calendario"),
                    ]
                ),
            )
            await self.client.session_update(
                session_id, acp.update_tool_call("llamada-mirar", status="completed")
            )
            await self.client.session_update(
                session_id,
                acp.start_tool_call(
                    "llamada-agenda",
                    "orbit_calendar",
                    kind="read",
                    raw_input={"start_ms": 0, "end_ms": 1},
                ),
            )
            await self.client.session_update(
                session_id,
                acp.update_tool_call(
                    "llamada-agenda",
                    status="failed",
                    content=[acp.tool_content(acp.text_block("Sin permiso de calendario"))],
                ),
            )
            await self.client.session_update(
                session_id,
                acp.update_plan(
                    [
                        acp.plan_entry("Mirar la pantalla", status="completed"),
                        acp.plan_entry("Leer el calendario", status="completed"),
                    ]
                ),
            )
            await self.say(session_id, "No pude leer el calendario.")
            return PromptResponse(stop_reason="end_turn")
        if "espera" in text:
            cancelled = self.cancelled.setdefault(session_id, asyncio.Event())
            await self.say(session_id, "Pensando… ")
            answer = await self.ask(session_id, "Borrar la nota «Compras del sábado»")
            self.record(
                "permission",
                outcome=answer.outcome.model_dump(mode="json", by_alias=True, exclude_none=True),
            )
            await cancelled.wait()
            return PromptResponse(stop_reason="cancelled")
        if "inicia" in text:
            raise acp.RequestError.auth_required()
        if "muere" in text:
            await self.say(session_id, "Empiezo… ")
            os._exit(3)
        if "enorme" in text:
            await self.say(session_id, "x" * (8 * 1024 * 1024 + 1))
            return PromptResponse(stop_reason="end_turn")
        await self.say(session_id, "Hola.")
        return PromptResponse(stop_reason="end_turn")


def main():
    arguments = sys.argv[1:]
    if "--exit-before-initialize" in arguments:
        sys.exit(7)
    record_path = None
    if "--record" in arguments:
        record_path = arguments[arguments.index("--record") + 1]
    agent = InventedAgent(
        record_path,
        auth_required="--auth-required" in arguments,
        load="--no-load" not in arguments,
    )
    asyncio.run(acp.run_agent(agent))


if __name__ == "__main__":
    main()
