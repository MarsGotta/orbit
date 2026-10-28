"""Device-authenticated settings/reads and signed, access-gated assistant reads."""

from functools import partial
from threading import Lock
from time import monotonic

from fastapi import Request
from fastapi.responses import JSONResponse
from starlette.concurrency import run_in_threadpool

from ..errors import HubError, PrivateError, render
from ..private_auth import Authenticator, read_key
from ..private_route import json_response, private_route
from .models import MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, Configuration, Read, parse, refusal
from .service import Vault

BRIDGE_PATH = "/internal/v1/vault/read"


def mount(app, store, identity, bounded_body, meetings_bridge, transport_allowed):
    vault = Vault(store)

    @app.get("/v1/vault/config")
    async def configuration(request: Request):
        identity(request)
        return JSONResponse(
            await run_in_threadpool(vault.public), headers={"Cache-Control": "no-store"}
        )

    @app.put("/v1/vault/config")
    async def configure(request: Request):
        identity(request)
        config = parse(Configuration, await bounded_body(request, MAX_REQUEST_BYTES))
        return JSONResponse(
            await run_in_threadpool(vault.configure, config), headers={"Cache-Control": "no-store"}
        )

    @app.delete("/v1/vault/config")
    async def disconnect(request: Request):
        identity(request)
        return JSONResponse(await run_in_threadpool(vault.disconnect))

    @app.post("/v1/vault/read")
    async def read(request: Request):
        identity(request)
        read = parse(Read, await bounded_body(request, MAX_REQUEST_BYTES))
        return reply(await run_in_threadpool(vault.read, read))

    signed = SignedVault(vault, meetings_bridge)
    private_route(
        BRIDGE_PATH,
        signed.read,
        MAX_REQUEST_BYTES,
        app=app,
        transport_allowed=transport_allowed,
        strict_headers=True,
        methods=["POST", "GET", "PUT", "DELETE"],
        failure=partial(_failure),
    )


def _failure(exc):
    return json_response(
        render(exc.code, "Could not read the vault.", numbers=exc.numbers), exc.status
    )


def reply(payload):
    response = json_response(payload)
    size = len(response.body)
    if size > MAX_RESPONSE_BYTES:
        raise refusal(
            "vault_response_too_large",
            f"The Notes response is {size} bytes; the maximum is {MAX_RESPONSE_BYTES}.",
            413,
            bytes=size,
            max_bytes=MAX_RESPONSE_BYTES,
        )
    return response


class SignedVault:
    def __init__(self, vault, bridge):
        self.vault, self.bridge, self.auth = vault, bridge, None
        self.auth_lock = Lock()

    def authenticate(self, headers, body):
        with self.auth_lock:
            return self._authenticate(headers, body)

    def _authenticate(self, headers, body):
        self.bridge.check()
        if self.auth is None:
            config = self.bridge.configuration
            self.auth = Authenticator(
                path=BRIDGE_PATH,
                protocol="orbit_meetings_bridge_v1",
                header_prefix="x-orbit-meetings-",
                key_id=config.key_id if config else None,
                key=read_key(config.credential_file, PrivateError) if config else None,
                failure=PrivateError,
                credentials=self.bridge.access.bridge_credential if self.bridge.access else None,
            )
        return self.auth.admit(headers, body, int(self.bridge.now() * 1000), monotonic())

    async def read(self, headers, body):
        # File/key and all connector I/O run off the event loop.
        return await run_in_threadpool(self._read, headers, body)

    def _read(self, headers, body):
        identity = self.authenticate(headers, body)
        try:
            request = parse(Read, body)
            if self.bridge.access:
                self.bridge.access.require(identity or request.agent_id, "notes")
            return reply(self.vault.read(request))
        except HubError as exc:
            return json_response(
                render(
                    exc.code,
                    exc.message,
                    numbers=exc.numbers,
                    payload=exc.display_metadata(),
                ),
                exc.http_status,
            )
