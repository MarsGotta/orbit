"""Shared loopback hub and configuration fixtures for the orbit_mcp tests.

The hub here is fake, but its two private routes are not: each request is
verified by the real ``Authenticator`` of ``orbit_hub.private_auth``, bound
exactly like the production routes bind it, and the replies use the envelope
shapes of ``bridge_route.py``. No Hermes, no Mac and no external network is ever
involved.
"""

import base64
import hashlib
import os
import stat
import time
from contextlib import asynccontextmanager
from time import monotonic

from fastapi import FastAPI, Request
from fastapi.responses import Response
from support_client_e2e import tcp_server

from orbit_hub.bridge_route import (
    ACCESS_PATH,
    DEVICES_PATH,
    AccessRequest,
    BridgeError,
    BridgeRequest,
    failure,
)
from orbit_hub.bridge_route import HEADER_PREFIX as BRIDGE_HEADER_PREFIX
from orbit_hub.bridge_route import PATH as BRIDGE_PATH
from orbit_hub.bridge_route import PROTOCOL as BRIDGE_PROTOCOL
from orbit_hub.bridge_route import RESULT_PROTOCOL as BRIDGE_RESULT
from orbit_hub.errors import render
from orbit_hub.private_auth import Authenticator, canonical
from orbit_mcp.config import Config
from orbit_mcp.hub import HubClient
from orbit_mcp.server import build_server

BRIDGE_KEY = bytes(range(32))
BRIDGE_KEY_ID = "mcp-bridge-key"
DEFAULT_DEVICE = "portatil"
OTHER_DEVICE = "sobremesa"
DEVICES = {
    DEFAULT_DEVICE: "5f1d0000-0000-4000-8000-000000000001",
    OTHER_DEVICE: "5f1d0000-0000-4000-8000-000000000002",
}

BACKGROUND_WINDOW = {
    "application": "Safari",
    "title": "Lecturas pendientes — Safari",
}
FOREGROUND_WINDOW = {
    "application": "Xcode",
    "title": "orbit — Panel",
}
PNG_BYTES = b"\x89PNG\r\n\x1a\nventana sintetica de Orbit para Deimos"
SCREENSHOT_ANSWER = {
    "application": FOREGROUND_WINDOW["application"],
    "title": FOREGROUND_WINDOW["title"],
    "width": 1280,
    "height": 800,
    "mime_type": "image/png",
    "sha256": hashlib.sha256(PNG_BYTES).hexdigest(),
    "data_base64": base64.b64encode(PNG_BYTES).decode("ascii"),
}
SELECTED_TEXT_ANSWER = {
    "application": FOREGROUND_WINDOW["application"],
    "title": FOREGROUND_WINDOW["title"],
    "text": "Texto seleccionado sintético de la ventana.",
}
# What a Mac says when macOS withholds the screen. There is no switch
# of Orbit left for a refusal to name.
OBSERVATION_REFUSED_NOTICE = (
    "macOS no concede a Orbit el permiso de Grabación de pantalla en este Mac, y sin él "
    "no puede ver la pantalla. Se concede en Ajustes del Sistema › Privacidad y seguridad "
    "› Grabación de pantalla."
)

TOOL_NAMES = (
    "orbit_devices",
    "orbit_windows",
    "orbit_screenshot",
    "orbit_look",
    "orbit_selected_text",
    "orbit_control_tools",
    "orbit_perform_action",
    "orbit_action_result",
    # The only agenda tool; the stored-copy one went with the copy.
    "orbit_calendar",
    "orbit_meetings",
    "orbit_transcript",
    # The note of a recording, as the user sees it.
    "orbit_note",
    # The folders the user gave the assistant, read-only.
    "orbit_files_list",
    "orbit_files_search",
    "orbit_files_read",
    # A file the user attached to a message.
    "orbit_attachment",
)

CONFIG_TEMPLATE = """\
hub_url = "{hub_url}"
default_device = "{default}"

[device_bridge]
key_id = "{bridge_key_id}"
key_file = "{bridge_key_file}"

[devices]
{devices}
"""


class RouteFailure(Exception):
    """What the real routes' error types carry: a status and a fixed code."""

    def __init__(self, status: int, code: str):
        self.status, self.code = status, code
        super().__init__(code)


def answered(result: dict) -> Response:
    return Response(
        content=canonical({"protocol": BRIDGE_RESULT, "state": "answered", "result": result}),
        media_type="application/json",
        headers={"Cache-Control": "no-store"},
    )


def device_refused(notice: str, *, code="permission_not_granted", numbers=None) -> Response:
    if code == "permission_not_granted" and numbers is None:
        numbers = {"permission": "screen_recording"}
    payload = render(
        code,
        notice,
        numbers=numbers,
        payload={
            "protocol": BRIDGE_RESULT,
            "state": "refused",
            "notice": notice,
        },
    )
    payload["refusal"]["detail"] = notice
    return Response(
        content=canonical(payload),
        status_code=403,
        media_type="application/json",
        headers={"Cache-Control": "no-store"},
    )


def hub_rejected(status: int, code: str) -> Response:
    return failure(BridgeError(status, code))


class FakeHub:
    """Loopback stand-in for the hub's private routes and their Mac."""

    def __init__(
        self,
        device_replies=None,
        devices=None,
        access=None,
    ):
        self.app = FastAPI()
        # agent id (None: the default) -> its access groups; None serves no
        # access route, as a hub that could not say.
        self.access = access
        self.access_requests = []
        self.device_requests = []
        self.device_list_requests = 0
        # The X-Orbit-Op of every request, in arrival order.
        self.ops = []
        self.device_replies = list(device_replies or [])
        self.devices = list(
            devices
            if devices is not None
            else [
                {
                    "id": DEVICES[DEFAULT_DEVICE],
                    "name": DEFAULT_DEVICE,
                    "paired_at": 1_000.0,
                    "connected": True,
                    "last_seen": time.time(),
                },
                {
                    "id": DEVICES[OTHER_DEVICE],
                    "name": OTHER_DEVICE,
                    "paired_at": 900.0,
                    "connected": False,
                    "last_seen": time.time() - 120,
                },
            ]
        )
        self.bridge_auth = Authenticator(
            BRIDGE_KEY_ID,
            BRIDGE_KEY,
            protocol=BRIDGE_PROTOCOL,
            path=BRIDGE_PATH,
            header_prefix=BRIDGE_HEADER_PREFIX,
            failure=RouteFailure,
        )
        self.devices_auth = Authenticator(
            BRIDGE_KEY_ID,
            BRIDGE_KEY,
            protocol=BRIDGE_PROTOCOL,
            path=DEVICES_PATH,
            header_prefix=BRIDGE_HEADER_PREFIX,
            failure=RouteFailure,
        )

        @self.app.post(BRIDGE_PATH)
        async def device_request(request: Request) -> Response:
            self.ops.append(request.headers.get("x-orbit-op"))
            body = await request.body()
            try:
                self.bridge_auth.admit(
                    request.scope["headers"], body, int(time.time() * 1000), monotonic()
                )
                parsed = BridgeRequest.model_validate_json(body)
            except RouteFailure as exc:
                return hub_rejected(exc.status, exc.code)
            except (ValueError, TypeError, UnicodeError, RecursionError):
                return hub_rejected(400, "invalid_request")
            self.device_requests.append(parsed.model_dump())
            if self.device_replies:
                return self.device_replies.pop(0)
            return self._default_reply(parsed.capability)

        self.access_auth = Authenticator(
            BRIDGE_KEY_ID,
            BRIDGE_KEY,
            protocol=BRIDGE_PROTOCOL,
            path=ACCESS_PATH,
            header_prefix=BRIDGE_HEADER_PREFIX,
            failure=RouteFailure,
        )

        @self.app.post(ACCESS_PATH)
        async def agent_access(request: Request) -> Response:
            body = await request.body()
            if self.access is None:
                return hub_rejected(503, "unavailable")
            try:
                self.access_auth.admit(
                    request.scope["headers"], body, int(time.time() * 1000), monotonic()
                )
                parsed = AccessRequest.model_validate_json(body)
            except RouteFailure as exc:
                return hub_rejected(exc.status, exc.code)
            self.access_requests.append(parsed.agent_id)
            agent_id = parsed.agent_id or "default"
            return Response(
                content=canonical(
                    {
                        "protocol": BRIDGE_RESULT,
                        "state": "answered",
                        "agent_id": agent_id,
                        "groups": self.access[agent_id],
                    }
                ),
                media_type="application/json",
            )

        @self.app.get(DEVICES_PATH)
        async def device_list(request: Request) -> Response:
            self.ops.append(request.headers.get("x-orbit-op"))
            try:
                self.devices_auth.admit(
                    request.scope["headers"], b"", int(time.time() * 1000), monotonic()
                )
            except RouteFailure as exc:
                return hub_rejected(exc.status, exc.code)
            self.device_list_requests += 1
            return Response(
                content=canonical({"devices": self.devices}),
                status_code=200,
                media_type="application/json",
                headers={"Cache-Control": "no-store"},
            )

    def _default_reply(self, capability: str) -> Response:
        # The exact shapes a connected Mac answers with. Screen sources do not
        # expose bindings or a foreground marker.
        if capability == "screen_sources":
            return answered({"windows": [BACKGROUND_WINDOW, FOREGROUND_WINDOW]})
        if capability == "screenshot":
            return answered(SCREENSHOT_ANSWER)
        if capability == "selected_text":
            return answered(SELECTED_TEXT_ANSWER)
        if capability == "perform_action":
            return Response(
                content=canonical(
                    {
                        "protocol": BRIDGE_RESULT,
                        "state": "pending",
                        "outcome": "in_progress",
                        "operation_id": "00000000-0000-4000-8000-000000000001",
                    }
                ),
                status_code=202,
                media_type="application/json",
            )
        return answered({"state": "unknown"})


def write_key(directory, name: str, material: bytes):
    """A key file with the attributes `read_key` demands of a real one."""
    path = directory / name
    path.write_bytes(material)
    os.chmod(path, 0o600)
    assert stat.S_IMODE(path.stat().st_mode) == 0o600
    return path


def write_mcp_config(
    directory,
    hub_url: str,
    *,
    bridge_key_file=None,
    default_device=DEFAULT_DEVICE,
    devices=None,
):
    bridge_key_file = bridge_key_file or write_key(directory, "device-bridge.key", BRIDGE_KEY)
    configured_devices = DEVICES if devices is None else devices
    device_lines = "\n".join(
        f'{name} = "{device_id}"' for name, device_id in configured_devices.items()
    )
    path = directory / "mcp.toml"
    path.write_text(
        CONFIG_TEMPLATE.format(
            hub_url=hub_url,
            default=default_device,
            bridge_key_id=BRIDGE_KEY_ID,
            bridge_key_file=bridge_key_file,
            devices=device_lines,
        ),
        encoding="utf-8",
    )
    return path


def text_of(result) -> str:
    return "".join(block.text for block in result.content if getattr(block, "type", None) == "text")


@asynccontextmanager
async def serving(tmp_path, hub, **configuration):
    async with tcp_server(hub.app) as url:
        config = Config.load(write_mcp_config(tmp_path, url, **configuration))
        hub_client = HubClient(config)
        yield hub_client, build_server(config, hub_client)
