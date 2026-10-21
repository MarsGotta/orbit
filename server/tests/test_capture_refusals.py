from support_orbit_mcp import serving

"""A failed capture upload keeps its cause through the MCP tool."""

import json

import pytest
from fastapi.responses import Response
from support_orbit_mcp import (
    DEFAULT_DEVICE,
    FakeHub,
    answered,
    text_of,
)

from orbit_hub.bridge_route import RESULT_PROTOCOL

pytestmark = pytest.mark.anyio


def capture_upload_refused() -> Response:
    """The exact envelope `bridge_route` builds for a refusal that names itself."""
    payload = {
        "protocol": RESULT_PROTOCOL,
        "state": "refused",
        "refusal": {"code": "upstream_failed", "service": "hub", "retry": {"kind": "after_fix"}},
    }
    return Response(
        content=json.dumps(payload, ensure_ascii=False, sort_keys=True, separators=(",", ":")),
        status_code=403,
        media_type="application/json",
        headers={"Cache-Control": "no-store"},
    )


@pytest.mark.parametrize("tool", ["orbit_screenshot", "orbit_look"])
async def test_a5_a_capture_that_could_not_be_uploaded_says_so(tmp_path, tool):
    hub = FakeHub(
        device_replies=([answered({"windows": []})] if tool == "orbit_look" else [])
        + [capture_upload_refused()]
    )
    context = serving(tmp_path, hub)
    _client, server = await context.__aenter__()
    try:
        result = await server.call_tool(tool, {"device": DEFAULT_DEVICE})
    finally:
        await context.__aexit__(None, None, None)

    assert result.is_error
    rendered = text_of(result)
    # Not a timeout, and not the generic "revisa el código y la configuración".
    assert "no_answer" not in rendered
    assert "entregar la captura" in rendered
    assert "upstream_failed" in rendered and "servicio: hub" in rendered
