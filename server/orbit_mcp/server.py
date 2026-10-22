"""Spanish-described MCP tools backed by the private Orbit hub."""

from __future__ import annotations

import base64
import json
import time
from time import monotonic

from mcp.server.mcpserver import MCPServer
from mcp.types import CallToolResult, ImageContent, TextContent

from orbit_hub.assistant.access import TOOL_GROUP
from orbit_hub.identifiers import canonical_uuid
from orbit_hub.log import bind_op, get_logger
from orbit_hub.meetings_bridge.page import MAX_ANSWER_CHARS

from . import calendar as agenda
from . import files, meetings
from .bridge import ToolAction as GeneralAction
from .config import Config, ConfigError
from .devices import device_selection, hub_selection, seen_ago, validated_devices
from .hub import HubClient, HubClientError, validate_parameters
from .refusals import error_result, mac_sentence, unexpected_result

logger = get_logger("orbit_hub.orbit_mcp.server")

# How long the agent's access groups are taken as read. What it protects: the
# hub from one access read per tool call. Why 10 s: as the device list's
# cache; a change in Settings › Agents reaches this translator within it, and
# the hub refuses a disabled group's request at once regardless.
ACCESS_CACHE_SECONDS = 10.0


def _text(value: str) -> TextContent:
    return TextContent(type="text", text=value)


def _ok(*content: TextContent | ImageContent) -> CallToolResult:
    return CallToolResult(content=list(content), isError=False)


def _screenshot_content(
    result: dict,
    *,
    windows: list[dict] | None = None,
) -> CallToolResult:
    expected = ("application", "title", "width", "height", "mime_type", "data_base64")
    if any(key not in result for key in expected) or result["mime_type"] != "image/png":
        raise HubClientError("invalid_response")
    try:
        base64.b64decode(result["data_base64"], validate=True)
    except (ValueError, TypeError):
        raise HubClientError("invalid_response") from None
    if not isinstance(result["data_base64"], str):
        raise HubClientError("invalid_response")
    details = (
        f"Captura de {result['application']} — {result['title']} "
        f"({result['width']} × {result['height']})."
    )
    if isinstance(result.get("coordinates"), str):
        details += f" {result['coordinates']}"
    if windows is not None:
        details += f" Ventanas disponibles: {json.dumps(windows, ensure_ascii=False)}"
    return _ok(
        ImageContent(type="image", data=result["data_base64"], mimeType="image/png"),
        _text(details),
    )


def _window_parameters(application: str | None, title: str | None) -> dict:
    parameters = {}
    if application is not None:
        parameters["application"] = application
    if title is not None:
        parameters["title"] = title
    return parameters


def _general_action(value: object) -> dict:
    validate_parameters("perform_action", {"action": value})
    return GeneralAction.model_validate(value).model_dump()


def _operation_id(value: str) -> str:
    try:
        canonical_uuid(value)
    except (ValueError, TypeError, AttributeError):
        raise HubClientError("invalid_parameters") from None
    return value


class OrbitMCPServer(MCPServer):
    """orbit-mcp's tools, without those of a group its agent has off."""

    hub_client: HubClient | None = None
    _access: tuple[float, str, dict[str, bool]] | None = None

    async def agent_access(self) -> tuple[str, dict[str, bool]] | None:
        """The agent's id and access groups, or None when the hub could not say.

        Unknown is not "off": every tool is offered then, and the hub, which
        checks each request itself, refuses what is off.
        """
        now = monotonic()
        if self._access is not None and now - self._access[0] < ACCESS_CACHE_SECONDS:
            return self._access[1], self._access[2]
        if self.hub_client is None:
            return None
        try:
            agent_id, groups = await self.hub_client.agent_access()
        except HubClientError as exc:
            logger.info(
                "orbit_mcp_access_unknown",
                extra={"event": "orbit_mcp_access_unknown", "code": exc.code},
            )
            return None
        self._access = (now, agent_id, groups)
        return agent_id, groups

    async def list_tools(self):
        tools = await super().list_tools()
        known = await self.agent_access()
        if known is None:
            return tools
        _agent_id, groups = known
        return [tool for tool in tools if groups.get(TOOL_GROUP.get(tool.name, ""), True)]

    async def call_tool(self, name, arguments, context=None):
        group = TOOL_GROUP.get(name)
        if group is not None:
            known = await self.agent_access()
            if known is not None and not known[1][group]:
                return error_result(
                    HubClientError(
                        "agent_access_disabled",
                        403,
                        {
                            "refusal": {
                                "code": "agent_access_disabled",
                                "agent_id": known[0],
                                "group": group,
                            }
                        },
                    )
                )
        if name == "orbit_perform_action":
            try:
                action = _general_action((arguments or {}).get("action"))
            except HubClientError as exc:
                return error_result(exc)
            arguments = {**arguments, "action": action}
        # MCP 2.2's generated argument model drops extra keys before invoking
        # the function, so capture filters must be checked on the raw call.
        capability = {"orbit_screenshot": "screenshot", "orbit_selected_text": "selected_text"}.get(
            name
        )
        if capability is not None:
            parameters = {key: value for key, value in (arguments or {}).items() if key != "device"}
            try:
                validate_parameters(capability, parameters)
            except HubClientError as exc:
                return error_result(exc)
        return await super().call_tool(name, arguments, context)


def build_server(config: Config, hub_client: HubClient, *, clock=time.time) -> MCPServer:
    server = OrbitMCPServer(
        name="orbit",
        instructions="Herramientas privadas de Orbit para el perfil Deimos.",
    )
    server.hub_client = hub_client

    def current_config() -> Config:
        nonlocal config
        try:
            refreshed = config.reload_if_changed()
        except ConfigError:
            raise HubClientError("config_invalid") from None
        if refreshed is not config:
            config = refreshed
            hub_client.update_config(config)
        return config

    async def invoke(operation, device=None):
        started = monotonic()
        bind_op(None)
        selection = None

        async def select():
            nonlocal selection
            selection = await device_selection(current_config(), hub_client, device)
            return selection

        try:
            result = await operation(select)
        except HubClientError as exc:
            result = error_result(exc, selection)
        except Exception:  # noqa: BLE001 -- MCP must return every failure as a tool result
            selected = selection.requested_name if selection is not None else device
            result = unexpected_result(selected, started)
        if selection is not None and selection.notice is not None:
            result.content.append(_text(selection.notice))
        return result

    @server.tool(
        description="Enumera los Mac conocidos por el hub, su conexión y cuándo se vieron.",
        structured_output=False,
    )
    async def orbit_devices() -> CallToolResult:
        async def operation(select):
            active_config = current_config()
            try:
                devices = validated_devices(await hub_client.devices())
            except HubClientError as exc:
                if exc.status != 404 or not active_config.devices:
                    raise
                lines = [
                    f"- {name}{' (predeterminado)' if name == active_config.default_device else ''}"
                    " — configurado en el fichero"
                    for name in active_config.devices
                ]
                return _ok(_text("Dispositivos de Orbit:\n" + "\n".join(lines)))
            default_id = None
            if active_config.default_device in active_config.devices:
                default_id = active_config.devices[active_config.default_device]
            elif devices:
                try:
                    default_id = hub_selection(devices, active_config.default_device).device_id
                except HubClientError:
                    pass
            lines = []
            for device in devices:
                aliases = [
                    name
                    for name, device_id in active_config.devices.items()
                    if device_id == device.id
                ]
                markers = []
                if aliases:
                    markers.append("fichero: " + ", ".join(aliases))
                if device.id == default_id:
                    markers.append("predeterminado")
                suffix = f" — {', '.join(markers)}" if markers else ""
                lines.append(
                    f"- {device.name} — conectado {'sí' if device.connected else 'no'} — "
                    f"visto por última vez {seen_ago(device.last_seen)}{suffix}"
                )
            return _ok(_text("Dispositivos de Orbit:\n" + ("\n".join(lines) or "Ninguno.")))

        return await invoke(operation)

    @server.tool(
        description="Lista las ventanas visibles de un Mac mediante Orbit.",
        structured_output=False,
    )
    async def orbit_windows(device: str | None = None) -> CallToolResult:
        async def operation(select):
            selection = await select()
            result = await hub_client.device_request(selection.device_id, "screen_sources", {})
            windows = result.get("windows")
            if not isinstance(windows, list):
                raise HubClientError("invalid_response")
            return _ok(_text(json.dumps(windows, ensure_ascii=False)))

        return await invoke(operation, device)

    @server.tool(
        description=(
            "Obtiene una captura PNG; application y title eligen una ventana, sin ambos usa "
            "la ventana en primer plano, y full_screen=true pide el escritorio completo al motor."
        ),
        structured_output=False,
    )
    async def orbit_screenshot(
        device: str | None = None,
        application: str | None = None,
        title: str | None = None,
        full_screen: bool = False,
    ) -> CallToolResult:
        async def operation(select):
            selection = await select()
            parameters = _window_parameters(application, title)
            if full_screen:
                if parameters:
                    raise HubClientError("invalid_parameters")
                parameters["full_screen"] = True
            result = await hub_client.device_request(selection.device_id, "screenshot", parameters)
            return _screenshot_content(result)

        return await invoke(operation, device)

    @server.tool(
        description="Mira la pantalla: lista las ventanas y captura la ventana en primer plano.",
        structured_output=False,
    )
    async def orbit_look(device: str | None = None) -> CallToolResult:
        async def operation(select):
            selection = await select()
            inventory = await hub_client.device_request(selection.device_id, "screen_sources", {})
            windows = inventory.get("windows")
            if not isinstance(windows, list):
                raise HubClientError("invalid_response")
            screenshot = await hub_client.device_request(selection.device_id, "screenshot", {})
            return _screenshot_content(screenshot, windows=windows)

        return await invoke(operation, device)

    @server.tool(
        description="Lee el texto seleccionado en una ventana concreta o en la ventana en primer plano.",
        structured_output=False,
    )
    async def orbit_selected_text(
        device: str | None = None,
        application: str | None = None,
        title: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            selection = await select()
            result = await hub_client.device_request(
                selection.device_id, "selected_text", _window_parameters(application, title)
            )
            if not all(
                isinstance(result.get(key), str) for key in ("application", "title", "text")
            ):
                raise HubClientError("invalid_response")
            return _ok(
                _text(
                    f"Texto seleccionado en {result['application']} — {result['title']}:\n"
                    f"{result['text']}"
                )
            )

        return await invoke(operation, device)

    @server.tool(
        description=(
            "Lista las herramientas de control que el motor declara en ese Mac, con su "
            "descripción y su esquema de argumentos. Es la lista autoritativa: "
            "orbit_perform_action sólo acepta nombres que salgan de aquí. No ejecuta nada."
        ),
        structured_output=False,
    )
    async def orbit_control_tools(device: str | None = None) -> CallToolResult:
        async def operation(select):
            selection = await select()
            # The Mac answers in pages; the walk happens inside the client so
            # that Deimos asks once and receives the catalogue entire.
            tools = await hub_client.control_tools(selection.device_id)
            if not all(
                isinstance(tool, dict) and isinstance(tool.get("name"), str) for tool in tools
            ):
                raise HubClientError("invalid_response")
            if not tools:
                return _ok(_text("El motor de ese Mac no declara ninguna herramienta."))
            return _ok(_text(json.dumps(tools, ensure_ascii=False)))

        return await invoke(operation, device)

    @server.tool(
        description=(
            "Ejecuta una herramienta del motor en ese Mac. El nombre y los argumentos son "
            "los que declara orbit_control_tools; Orbit los pasa tal cual. Quien usa Orbit ya "
            "concedió en macOS el uso de su ordenador: nadie tiene que decir que sí otra vez. Si la "
            "acción sigue en marcha responde «in_progress» con su operation_id —quiere decir "
            "en curso, no que espere a nadie— y su desenlace se lee con orbit_action_result. "
            "Sin motor disponible responde que no lo está, con la causa, y no lo intenta por "
            "otro camino."
        ),
        structured_output=False,
    )
    async def orbit_perform_action(
        action: GeneralAction,
        device: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            parameters = {"action": action.model_dump()}
            selection = await select()
            result = await hub_client.device_request(
                selection.device_id, "perform_action", parameters
            )
            return _ok(_text(json.dumps(result, ensure_ascii=False)))

        return await invoke(operation, device)

    @server.tool(
        description=(
            "Consulta el desenlace de una acción pedida con orbit_perform_action. "
            "Distingue en curso, ejecución, resultado desconocido y "
            "fallo. «in_progress» significa que sigue ejecutándose en el Mac. "
            "También lee los recibos históricos guardados por versiones anteriores."
        ),
        structured_output=False,
    )
    async def orbit_action_result(
        operation_id: str,
        device: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            identity = _operation_id(operation_id)
            selection = await select()
            result = await hub_client.action_result(selection.device_id, identity)
            if result.get("outcome") in {"failed", "unknown"}:
                code = result.get("code")
                if isinstance(code, str):
                    detail = {
                        "refusal": result.get("refusal"),
                        "mac_notice": result.get("notice"),
                        "device_version": result.get("device_version"),
                    }
                    result["descripcion"] = "".join(
                        block.text
                        for block in error_result(HubClientError(code, detail=detail)).content
                        if isinstance(block, TextContent)
                    )
                    if "notice" in result:
                        result["notice"] = mac_sentence(detail)
            return _ok(_text(json.dumps(result, ensure_ascii=False)))

        return await invoke(operation, device)

    @server.tool(
        description=(
            "La agenda de quien usa Orbit, leída de su Mac en el momento de preguntar, de todos sus "
            "calendarios. Acepta cualquier rango en una sola llamada: un día, el mes que "
            "viene, diciembre, el año pasado o un año entero; Orbit lo lee entero por dentro "
            "y devuelve todos los eventos. Es la herramienta que hay que usar para cualquier "
            "pregunta sobre su agenda. start_ms y end_ms son milisegundos desde 1970 (UTC), "
            "con end_ms posterior a start_ms."
        ),
        structured_output=False,
    )
    async def orbit_calendar(
        start_ms: int,
        end_ms: int,
        device: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            if not isinstance(start_ms, int) or not isinstance(end_ms, int):
                raise HubClientError("invalid_parameters")
            if end_ms <= start_ms:
                raise HubClientError("invalid_parameters")
            selection = await select()
            # Whole milliseconds, as numbers, held to
            # `contracts/fixtures/device-bridge-request-v1.json`. The whole
            # range goes to the Mac, which answers it in pages; the walk
            # joins them, so this is one call however long the range is.
            reading = await hub_client.calendar_read(selection.device_id, start_ms, end_ms)
            return _ok(_text(agenda.render(reading)))

        return await invoke(operation, device)

    def _present(**values) -> dict:
        return {key: value for key, value in values.items() if value is not None}

    @server.tool(
        description=(
            "Lista las carpetas del Mac que quien usa Orbit dio al asistente y, con path, lo "
            "que hay dentro de una de ellas: carpetas, archivos con su tamaño y enlaces. Sin "
            "path dice cuáles son y si se pueden leer. Sólo lectura. Una carpeta grande llega "
            "por páginas: la respuesta da el after_index con el que seguir."
        ),
        structured_output=False,
    )
    async def orbit_files_list(
        path: str | None = None,
        after_index: int | None = None,
        device: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            parameters = _present(path=path, after_index=after_index)
            validate_parameters("files_list", parameters)
            selection = await select()
            result = await hub_client.device_request(selection.device_id, "files_list", parameters)
            return _ok(_text(files.render_list(result)))

        return await invoke(operation, device)

    @server.tool(
        description=(
            "Busca en las carpetas del Mac que quien usa Orbit dio al asistente: por nombre de "
            "archivo o carpeta, sin distinguir mayúsculas ni acentos, y con content=true también "
            "dentro de los archivos de texto. path limita la búsqueda a una carpeta. Tiene "
            "techos de resultados, de entradas recorridas y de bytes leídos, y dice cuál la "
            "detuvo. Sólo lectura."
        ),
        structured_output=False,
    )
    async def orbit_files_search(
        query: str,
        path: str | None = None,
        content: bool | None = None,
        device: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            parameters = _present(query=query, path=path, content=content)
            validate_parameters("files_search", parameters)
            selection = await select()
            result = await hub_client.device_request(
                selection.device_id, "files_search", parameters
            )
            return _ok(_text(files.render_search(result)))

        return await invoke(operation, device)

    @server.tool(
        description=(
            "Lee un archivo de las carpetas del Mac que quien usa Orbit dio al asistente: texto "
            "o Markdown por páginas (after_index sigue donde acabó la anterior), el texto de un "
            "PDF por páginas (after_page y after_index) o una imagen, que llega como imagen. "
            "path es la ruta absoluta que dan orbit_files_list u orbit_files_search. Nada "
            "fuera de esas carpetas, tampoco a través de enlaces o «..». Sólo lectura."
        ),
        structured_output=False,
    )
    async def orbit_files_read(
        path: str,
        after_index: int | None = None,
        after_page: int | None = None,
        device: str | None = None,
    ) -> CallToolResult:
        async def operation(select):
            parameters = _present(path=path, after_index=after_index, after_page=after_page)
            validate_parameters("files_read", parameters)
            selection = await select()
            result = await hub_client.device_request(selection.device_id, "files_read", parameters)
            if result["content"] == "image":
                data = result.get("data_base64")
                if not isinstance(data, str) or result.get("mime_type") != "image/png":
                    raise HubClientError("invalid_response")
                try:
                    base64.b64decode(data, validate=True)
                except (ValueError, TypeError):
                    raise HubClientError("invalid_response") from None
                return _ok(
                    ImageContent(type="image", data=data, mimeType="image/png"),
                    _text(files.render_read(result)),
                )
            return _ok(_text(files.render_read(result)))

        return await invoke(operation, device)

    def _index(value, name: str) -> int:
        if value is None:
            return 0
        if type(value) is not int or not 0 <= value < 2**63:
            raise HubClientError(
                "invalid_parameters",
                detail={"refusal": {"code": "invalid_parameters", "parameter": name}},
            )
        return value

    def _recording(value) -> str:
        try:
            return canonical_uuid(value)
        except ValueError:
            raise HubClientError(
                "invalid_parameters",
                detail={"refusal": {"code": "invalid_parameters", "parameter": "recording_id"}},
            ) from None

    @server.tool(
        description=(
            "Lista las reuniones grabadas en el Mac de quien usa Orbit, de la que empezó más tarde a la "
            "que empezó antes, incluida la que esté en curso (marcada EN CURSO). Cada una "
            "dice el estado real de su nota —lista con su título y resumen corto, "
            "escribiéndose, esperando la transcripción, en cola, fallida con su causa, sin "
            "pedir o sin modelo configurado— y el de su transcripción. Devuelve "
            f"{meetings.MEETINGS_PER_ANSWER} como máximo; si hay más lo dice y da el "
            "before_ms con el que seguir. since_days (1–365, 7 por defecto) acota el periodo."
        ),
        structured_output=False,
    )
    async def orbit_meetings(
        device: str | None = None,
        since_days: int | None = None,
        before_ms: int | None = None,
    ) -> CallToolResult:
        async def operation(select):
            selection = await select()
            days = 7 if since_days is None else since_days
            if type(days) is not int or not 1 <= days <= 365:
                raise HubClientError("invalid_parameters")
            if before_ms is not None and (type(before_ms) is not int or before_ms < 1):
                raise HubClientError("invalid_parameters")
            since_ms = max(0, int(clock() * 1000) - days * 24 * 60 * 60 * 1000)
            listing = await hub_client.meetings(
                selection.device_id, since_ms, meetings.MEETINGS_PER_ANSWER, before_ms
            )
            return _ok(_text(meetings.render_meetings(listing, since_ms)))

        return await invoke(operation, device)

    @server.tool(
        description=(
            "Lee la transcripción de una grabación de Orbit, una línea por intervención: "
            "«[hh:mm:ss] Nombre: texto», con el tiempo desde el inicio de la grabación. Los "
            "nombres son los que ve quien usa Orbit: el micrófono del Mac lleva el de su dueño, "
            "cada voz el que le puso; las voces separadas sin nombre son «Persona 1», «Persona 2»… y el borrador dice «Otra persona». De una "
            "reunión en curso devuelve lo transcrito hasta ahora, marcado como parcial, con "
            "cuánto audio queda por transcribir. Una transcripción larga llega por páginas de "
            f"hasta {MAX_ANSWER_CHARS} caracteres: la primera línea dice cuáles son "
            "y el after_index con el que seguir. format=segments da tiempos at_ms/end_ms, pista y speaker_key para identificar un turno, en una línea JSON por "
            "fragmento transcrito."
        ),
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
            start = _index(after_index, "after_index")
            # The hub's page holds far more than one answer; the rest is one
            # `after_index` away, so it is never fetched only to be thrown away.
            page = await hub_client.transcript(
                selection.device_id, recording, selected_format, start
            )
            return _ok(_text(meetings.render_transcript(page, start)))

        return await invoke(operation, device)

    @server.tool(
        description=(
            "Lee la nota de una grabación tal como la ve quien usa Orbit: título, resumen "
            "corto, las secciones de su plantilla con sus ediciones aplicadas, las tareas con "
            "responsable, plazo y si están hechas, las decisiones, los temas abiertos, sus "
            "notas privadas, la plantilla, el modelo que la escribió y cuándo. Si la nota "
            "no está escrita, dice en qué estado está y por qué, con sus números (por "
            "ejemplo, cuándo es el siguiente intento). Sólo lectura. Una nota larga llega "
            f"por páginas de hasta {MAX_ANSWER_CHARS} caracteres con el after_index "
            "con el que seguir."
        ),
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
            start = _index(after_index, "after_index")
            document = await hub_client.note(selection.device_id, recording)
            return _ok(_text(meetings.render_note(document, start)))

        return await invoke(operation, device)

    return server
