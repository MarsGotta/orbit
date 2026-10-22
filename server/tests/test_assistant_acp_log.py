"""An ACP update the hub's SDK cannot read never reaches a log with its text.

The ACP SDK validates each `session/update` before the hub sees it. One it
cannot read (a kind newer than the pinned SDK, here `future_kind`) would raise
a pydantic `ValidationError` inside the SDK's notification task, which the SDK
logs with `logging.exception` on the root logger, its text quoting the
update's input: what the agent was saying. The hub drops such an update before
the SDK reads it and counts it by its code; and whatever a library still logs
on the root logger is written by the hub's formatter, without its text. The
agent here is a few lines of JSON written by the test; the text is invented.
"""

import asyncio
import logging
import sys
import textwrap

from conftest import configured

from orbit_hub import log
from orbit_hub.assistant import acp
from orbit_hub.assistant.backend import MessageComplete, TextBlock, TurnFailed

SAID = "frase privada de la conversación inventada"

AGENT = textwrap.dedent(
    """
    import json, sys
    said = sys.argv[1]
    def send(message):
        sys.stdout.write(json.dumps(message) + "\\n")
        sys.stdout.flush()
    for line in sys.stdin:
        request = json.loads(line)
        method, number = request.get("method"), request.get("id")
        if method == "initialize":
            send({"jsonrpc": "2.0", "id": number,
                  "result": {"protocolVersion": 1, "agentCapabilities": {}}})
        elif method == "session/new":
            send({"jsonrpc": "2.0", "id": number, "result": {"sessionId": "sesion-1"}})
        elif method == "session/prompt":
            send({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": "sesion-1",
                "update": {"sessionUpdate": "future_kind",
                           "content": {"type": "text", "text": said}}}})
            send({"jsonrpc": "2.0", "id": number, "result": {"stopReason": "end_turn"}})
    """
)


async def test_an_update_the_sdk_cannot_read_keeps_its_text_out_of_every_log(
    environment, tmp_path, caplog
):
    settings, _store, _speech = environment
    script = tmp_path / "agente_futuro.py"
    script.write_text(AGENT, encoding="utf-8")
    entry = {
        "id": "futuro",
        "name": "Futuro",
        "kind": "acp",
        "agent": "claude-code",
        "command": [sys.executable, str(script), SAID],
        "orbit_mcp_config": str(tmp_path / "mcp.toml"),
    }
    backend = acp.build_backend(settings, configured(entry))
    events: list = []
    ended = asyncio.Event()

    async def on_event(event):
        events.append(event)
        if isinstance(event, MessageComplete | TurnFailed):
            ended.set()

    connection = backend.connection("equipo-1", "Mac", on_event)
    await connection.open()
    caplog.set_level(logging.DEBUG)
    try:
        opened = await connection.create_session()
        await connection.prompt(opened.session_id, [TextBlock("hola")])
        await asyncio.wait_for(ended.wait(), 30)
    finally:
        await connection.close()
    logged = "\n".join(
        record.getMessage()
        + "\n"
        + (logging.Formatter().formatException(record.exc_info) if record.exc_info else "")
        for record in caplog.records
    )
    assert SAID[-20:] not in logged, [
        (record.name, record.levelname, record.getMessage()) for record in caplog.records
    ]


def test_a_library_record_on_the_root_logger_is_written_without_its_text():
    """What a library logs on the root logger (the ACP SDK's `logging.exception`)
    goes through the hub's formatter: its level and exception class, never its
    message or the exception's text."""
    log.configure()
    root = logging.getLogger()
    formatters = [
        handler.formatter
        for handler in root.handlers
        if isinstance(handler.formatter, log.JsonFormatter)
    ]
    assert formatters, root.handlers
    try:
        raise ValueError(SAID)
    except ValueError:
        record = logging.getLogger("acp.libreria").makeRecord(
            "acp.libreria", logging.ERROR, __file__, 1, "dijo %s", (SAID,), sys.exc_info()
        )
    written = formatters[0].format(record)
    assert SAID[-20:] not in written, written
    assert "ValueError" in written, written
