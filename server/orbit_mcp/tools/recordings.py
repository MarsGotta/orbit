"""The Mac's recorded meetings, read from the hub: the list, a transcript, a note."""

from __future__ import annotations

from mcp.server.mcpserver import MCPServer
from mcp.types import CallToolResult

from orbit_hub.identifiers import canonical_uuid
from orbit_hub.meetings_bridge.page import MAX_ANSWER_CHARS

from .. import meetings
from ..hub import HubClientError
from .runner import ToolRunner, ok, page_start, text

TRANSCRIPT_DESCRIPTION = (
    "Lee la transcripción de una grabación de Orbit, una línea por intervención: "
    "«[hh:mm:ss] Nombre: texto», con el tiempo desde el inicio de la grabación. Los "
    "nombres son los que ve quien usa Orbit: el micrófono del Mac lleva el de su dueño, "
    "cada voz el que le puso; las voces separadas sin nombre son «Persona 1», «Persona 2»… y el borrador dice «Otra persona». De una "
    "reunión en curso devuelve lo transcrito hasta ahora, marcado como parcial, con "
    "cuánto audio queda por transcribir. Una transcripción larga llega por páginas de "
    f"hasta {MAX_ANSWER_CHARS} caracteres: la primera línea dice cuáles son "
    "y el after_index con el que seguir. format=segments da tiempos at_ms/end_ms, pista y speaker_key para identificar un turno, en una línea JSON por "
    "fragmento transcrito."
)

NOTE_DESCRIPTION = (
    "Lee la nota de una grabación tal como la ve quien usa Orbit: título, resumen "
    "corto, las secciones de su plantilla con sus ediciones aplicadas, las tareas con "
    "responsable, plazo y si están hechas, las decisiones, los temas abiertos, sus "
    "notas privadas, la plantilla, el modelo que la escribió y cuándo. Si la nota "
    "no está escrita, dice en qué estado está y por qué, con sus números (por "
    "ejemplo, cuándo es el siguiente intento). Sólo lectura. Una nota larga llega "
    f"por páginas de hasta {MAX_ANSWER_CHARS} caracteres con el after_index "
    "con el que seguir."
)


def _recording(value, field: str = "recording_id") -> str:
    try:
        return canonical_uuid(value)
    except ValueError:
        raise HubClientError(
            "invalid_parameters",
            detail={"refusal": {"code": "invalid_parameters", "parameter": field}},
        ) from None


def _since_ms(clock, since_days: int | None, before_ms: int | None) -> int:
    days = 7 if since_days is None else since_days
    if type(days) is not int or not 1 <= days <= 365:
        raise HubClientError("invalid_parameters")
    if before_ms is not None and (type(before_ms) is not int or before_ms < 1):
        raise HubClientError("invalid_parameters")
    return max(0, int(clock() * 1000) - days * 24 * 60 * 60 * 1000)


def register(server: MCPServer, runner: ToolRunner) -> None:
    hub_client = runner.hub_client

    @server.tool(
        description=(
            "Lista las reuniones de quien usa Orbit en todos sus Mac, de la que empezó más tarde a la "
            "que empezó antes, incluida la que esté en curso (marcada EN CURSO). Cada una "
            "dice el estado real de su nota —lista con su título y resumen corto, "
            "escribiéndose, esperando la transcripción, en cola, fallida con su causa, sin "
            "pedir o sin modelo configurado— y el de su transcripción. Devuelve "
            f"{meetings.MEETINGS_PER_ANSWER} como máximo; si hay más lo dice y da el "
            "before_ms y before_id con los que seguir. since_days (1–365, 7 por defecto) acota el periodo."
        ),
        structured_output=False,
    )
    async def orbit_meetings(
        device: str | None = None,
        since_days: int | None = None,
        before_ms: int | None = None,
        before_id: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            selection = await select()
            since_ms = _since_ms(runner.clock, since_days, before_ms)
            listing = await hub_client.meetings(
                selection.device_id,
                since_ms,
                meetings.MEETINGS_PER_ANSWER,
                before_ms,
                None if before_id is None else _recording(before_id, "before_id"),
            )
            return ok(text(meetings.render_meetings(listing, since_ms)))

        return await runner.invoke(operation, device)

    @server.tool(
        description=TRANSCRIPT_DESCRIPTION,
        structured_output=False,
    )
    async def orbit_transcript(
        recording_id: str,
        device: str | None = None,
        format: str | None = None,
        after_index: int | None = None,
    ) -> CallToolResult:
        async def operation(select):
            selection = await select()
            selected_format = "text" if format is None else format
            if selected_format not in {"text", "segments"}:
                raise HubClientError("invalid_parameters")
            recording = _recording(recording_id)
            start = page_start(after_index, "after_index")
            # The hub's page holds far more than one answer; the rest is one
            # `after_index` away, so it is never fetched only to be thrown away.
            page = await hub_client.transcript(
                selection.device_id, recording, selected_format, start
            )
            return ok(text(meetings.render_transcript(page, start)))

        return await runner.invoke(operation, device)

    @server.tool(
        description=NOTE_DESCRIPTION,
        structured_output=False,
    )
    async def orbit_note(
        recording_id: str,
        device: str | None = None,
        after_index: int | None = None,
    ) -> CallToolResult:
        async def operation(select):
            selection = await select()
            recording = _recording(recording_id)
            start = page_start(after_index, "after_index")
            document = await hub_client.note(selection.device_id, recording)
            return ok(text(meetings.render_note(document, start)))

        return await runner.invoke(operation, device)
