"""Read the hub's vault directly: no Mac selection or provider access."""

import json

from orbit_hub.meetings_bridge.page import MAX_ANSWER_CHARS
from orbit_hub.vault.models import MAX_PAGE_SIZE, MAX_RESPONSE_BYTES, Read
from orbit_hub.vault.routes import BRIDGE_PATH

from ..hub import HubClientError
from .runner import ok, page_start, text


def rendered(result, after_index):
    body = json.dumps(result, ensure_ascii=False, indent=2)
    start = page_start(after_index, "after_index")
    if start > len(body):
        raise HubClientError(
            "invalid_parameters",
            detail={
                "refusal": {
                    "code": "invalid_parameters",
                    "after_index": start,
                    "total_chars": len(body),
                }
            },
        )
    end = min(len(body), start + MAX_ANSWER_CHARS)
    next_index = str(end) if end < len(body) else "fin"
    return ok(
        text(
            f"Caracteres {start}–{end} de {len(body)}; next_after_index: {next_index}\n"
            + body[start:end]
        )
    )


async def read(runner, parameters, after_index=0):
    runner.current_config()
    try:
        request = Read.model_validate(parameters)
    except ValueError:
        raise HubClientError(
            "vault_invalid_request",
            detail={"refusal": {"code": "vault_invalid_request", "max_page_size": MAX_PAGE_SIZE}},
        ) from None
    result = await runner.hub_client._meetings_call(
        BRIDGE_PATH, request.model_dump(exclude={"agent_id"}), None
    )
    if not isinstance(result, dict) or len(json.dumps(result).encode()) > MAX_RESPONSE_BYTES:
        raise HubClientError("invalid_response")
    if request.operation in {"search", "list", "backlinks", "outlinks"}:
        if (
            not isinstance(result.get("entries"), list)
            or len(result["entries"]) > request.page_size
        ):
            raise HubClientError("invalid_response")
    elif not isinstance(result.get("body"), str):
        raise HubClientError("invalid_response")
    return rendered(result, after_index)


def register(server, runner):
    @server.tool(
        description="Busca por nombre y contenido en tu vault de Notas del hub. Solo lectura. Devuelve rutas citables y next_page para continuar; no necesita un Mac conectado.",
        structured_output=False,
    )
    async def orbit_notes_search(
        query: str, content: bool = True, page: int = 1, page_size: int = 50, after_index: int = 0
    ):
        async def operation(_select):
            return await read(
                runner,
                {
                    "operation": "search",
                    "query": query,
                    "content": content,
                    "page": page,
                    "page_size": page_size,
                },
                after_index,
            )

        return await runner.invoke(operation)

    @server.tool(
        description="Lee una nota Markdown del vault, con frontmatter y enlaces internos resueltos. Cita su path. after_index continúa una respuesta larga. Solo lectura, sin acceso al disco del Mac.",
        structured_output=False,
    )
    async def orbit_note_read(path: str, after_index: int = 0):
        async def operation(_select):
            return await read(runner, {"operation": "read", "path": path}, after_index)

        return await runner.invoke(operation)

    @server.tool(
        description="Lee index.md de tu vault de Notas; si falta, sintetiza un índice paginado. next_page y next_after_index permiten continuar. Solo lectura: nunca crea el archivo.",
        structured_output=False,
    )
    async def orbit_notes_index(page: int = 1, page_size: int = 50, after_index: int = 0):
        async def operation(_select):
            return await read(
                runner, {"operation": "index", "page": page, "page_size": page_size}, after_index
            )

        return await runner.invoke(operation)
