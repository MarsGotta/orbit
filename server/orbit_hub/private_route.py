"""The bounded HTTP boundary of the hub's signed private routes."""

import inspect

from fastapi import Request
from fastapi.responses import Response

from .errors import PrivateError
from .private_auth import canonical


def json_response(payload: dict, status: int = 200) -> Response:
    return Response(
        canonical(payload),
        status_code=status,
        media_type="application/json",
        headers={"Cache-Control": "no-store"},
    )


def private_route(
    path,
    handler,
    max_bytes,
    *,
    app,
    transport_allowed,
    failure,
    methods,
    requests=None,
    strict_headers=False,
):
    async def endpoint(request: Request):
        acquired = False
        try:
            if not transport_allowed(request.scope):
                raise PrivateError(401, "unauthorized")
            method = "GET" if max_bytes == 0 else "POST"
            if request.method != method:
                raise PrivateError(
                    401 if strict_headers else 400,
                    "invalid_request" if not strict_headers else "unauthorized",
                )
            if request.scope.get("query_string"):
                raise PrivateError(
                    401 if method == "GET" else 400,
                    "unauthorized" if method == "GET" else "invalid_request",
                )
            if strict_headers and (
                request.headers.get("origin") is not None
                or request.headers.get("content-type", "").lower()
                not in {
                    "application/json",
                    "application/json; charset=utf-8",
                    "application/json;charset=utf-8",
                }
            ):
                raise PrivateError(400, "invalid_request")
            if requests is not None:
                if requests.locked():
                    raise PrivateError(429, "busy")
                await requests.acquire()
                acquired = True
            body = bytearray()
            if method == "POST":
                async for chunk in request.stream():
                    size = len(body) + len(chunk)
                    if size > max_bytes:
                        field = "received_bytes" if strict_headers else "bytes"
                        raise PrivateError(
                            400, "invalid_request", {field: size, "max_bytes": max_bytes}
                        )
                    body.extend(chunk)
                if not body:
                    raise PrivateError(400, "invalid_request")
            result = handler(request.scope["headers"], bytes(body))
            if inspect.isawaitable(result):
                result = await result
            return result if isinstance(result, Response) else json_response(result)
        except PrivateError as exc:
            return failure(exc)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            return failure(PrivateError(400, "invalid_request"))
        finally:
            if acquired:
                requests.release()

    endpoint.__name__ = path.strip("/").replace("/", "_").replace("-", "_")
    app.add_api_route(path, endpoint, methods=methods, include_in_schema=False)
