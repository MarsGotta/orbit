from support_orbit_mcp import serving

"""The engine catalogue reaches Deimos in pages, and silence is over.

Every earlier test of this path declared a toy catalogue of a few hundred
bytes, which is exactly what let the real size reach the Mac twice. The
catalogue here is of the order the real engine publishes — 147 909 bytes,
measured on a real Mac on 2026-09-23 — and it crosses the same private route,
the same signed envelope and the same translator the Mac's answer crosses.
"""

import json

import pytest
from support_orbit_mcp import (
    DEFAULT_DEVICE,
    DEVICES,
    FakeHub,
    answered,
    device_refused,
    text_of,
)

pytestmark = pytest.mark.anyio

# The link the Mac answers through refuses any frame over this, and the
# catalogue page budget on the Mac clears it with its envelope on top. Both are
# restated rather than imported: this side must fail if the Mac's numbers drift
# away from the ones the wire contract names.
MAX_HUB_FRAME_BYTES = 128 * 1024
REAL_CATALOGUE_BYTES = 147_909


def realistic_tool(index: int) -> dict:
    """One tool declared the way a real engine declares it."""
    return {
        "name": f"engine_tool_{index:03d}",
        "description": (
            f"Herramienta {index} del motor de control, con su descripción completa tal "
            "como el motor la publica en su catálogo."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                f"parameter_{field}": {
                    "type": "string",
                    "title": f"Parámetro {field}",
                    "description": (
                        f"Argumento {field} de la herramienta {index}, descrito por el motor "
                        "con el detalle que necesita quien la llama para no adivinar su forma."
                    ),
                    "enum": ["ninguno", "primero", "segundo", "tercero"],
                }
                for field in range(10)
            },
            "required": ["parameter_0", "parameter_1"],
            "additionalProperties": False,
        },
    }


def encoded(value) -> int:
    return len(json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))


def catalogue_of(target: int) -> list[dict]:
    """A catalogue of at least `target` bytes, grown one realistic tool at a time."""
    tools: list[dict] = []
    while encoded(tools) < target:
        tools.append(realistic_tool(len(tools)))
    return tools


async def test_a_real_sized_catalogue_reaches_deimos_entire_in_pages(tmp_path):
    catalogue = catalogue_of(150 * 1024)
    catalogue_bytes = encoded(catalogue)
    assert REAL_CATALOGUE_BYTES <= catalogue_bytes <= REAL_CATALOGUE_BYTES * 2, (
        f"the test uses a catalogue of the real size, not a toy one: {catalogue_bytes} bytes"
    )
    assert catalogue_bytes > MAX_HUB_FRAME_BYTES, (
        "the whole catalogue does not fit one frame: that is why it is asked in pages"
    )

    # The translator consumes pages; the real Mac paginator is exercised in
    # computer_control/catalogue_tests.rs, not reimplemented by this fixture.
    split = len(catalogue) // 2
    pages = [
        {
            "tools": catalogue[:split],
            "after_index": 0,
            "next_after_index": split,
            "total": len(catalogue),
        },
        {"tools": catalogue[split:], "after_index": split, "total": len(catalogue)},
    ]
    assert len(pages) == 2, "the real catalogue takes two round trips, not a dozen"
    for page in pages:
        # What the Mac has to put on the link for this page, envelope included.
        frame = {
            "v": 1,
            "id": "x" * 36,
            "method": "device.reply",
            "params": {"request_id": "x" * 36, "result": page},
        }
        assert encoded(frame) <= MAX_HUB_FRAME_BYTES, (
            f"a page has to fit the hub's frame: {encoded(frame)} bytes"
        )

    hub = FakeHub(device_replies=[answered(page) for page in pages])
    context = serving(tmp_path, hub)
    _client, server = await context.__aenter__()
    try:
        listed = await server.call_tool("orbit_control_tools", {"device": DEFAULT_DEVICE})
    finally:
        await context.__aexit__(None, None, None)

    assert not listed.is_error, text_of(listed)
    # Deimos asked once; the extra round trips stayed between the translator
    # and the Mac.
    assert [request["capability"] for request in hub.device_requests] == [
        "control_tools",
        "control_tools",
    ]
    assert [request["parameters"] for request in hub.device_requests] == [
        {},
        {"after_index": pages[0]["next_after_index"]},
    ]
    assert all(request["device_id"] == DEVICES[DEFAULT_DEVICE] for request in hub.device_requests)

    # The whole catalogue arrived, in order, with the schemas Deimos needs
    # to name a tool and give it arguments.
    assembled = json.loads(text_of(listed))
    assert assembled == catalogue
    assert assembled[-1]["name"] == catalogue[-1]["name"], "the last page arrives too"
    assert assembled[-1]["input_schema"]["properties"], "the schemas arrive intact"


async def test_catalogue_without_required_counters_names_the_missing_field(tmp_path):
    """The hub model used to require the counters; MCP now owns that check."""
    catalogue = catalogue_of(2 * 1024)
    hub = FakeHub(device_replies=[answered({"tools": catalogue})])
    async with serving(tmp_path, hub) as (_client, server):
        listed = await server.call_tool("orbit_control_tools", {"device": DEFAULT_DEVICE})
    assert listed.is_error
    assert "device_reply_invalid" in text_of(listed)
    assert "result.after_index" in text_of(listed)
    assert "required field" in text_of(listed)
    assert len(hub.device_requests) == 1


async def test_a_refused_frame_reaches_the_caller_with_its_cause(tmp_path):
    """The link refusing a reply is an answer, not a wait that times out."""
    hub = FakeHub(
        device_replies=[
            device_refused(
                "Orbit preparó una respuesta de 151600 bytes y el enlace con el hub sólo "
                "admite 131072. Pídela por partes.",
                code="too_large",
                numbers={"bytes": 151600, "max_bytes": 131072},
            )
        ]
    )
    context = serving(tmp_path, hub)
    _client, server = await context.__aenter__()
    try:
        result = await server.call_tool("orbit_control_tools", {"device": DEFAULT_DEVICE})
    finally:
        await context.__aexit__(None, None, None)

    assert result.is_error
    # Not `no_answer`: the difference between "the Mac said no" and "the Mac
    # said nothing" is what decides whether anyone retries forever.
    assert "no_answer" not in text_of(result)
    assert "too_large" in text_of(result)
    assert "Pídela por partes" in text_of(result)
    assert "151600" in text_of(result) and "131072" in text_of(result)
    assert "not_authorized" not in text_of(result)


@pytest.mark.parametrize(
    "second",
    [
        # A page that answers a different index than the one asked for.
        {"tools": [], "after_index": 99, "total": 2},
        # A continuation that never advances would walk the same page forever.
        {"tools": [], "after_index": 1, "next_after_index": 1, "total": 2},
        # A last page that ends the walk short of what the Mac declared.
        {"tools": [], "after_index": 1, "total": 2},
    ],
)
async def test_a_walk_that_cannot_be_trusted_is_refused_rather_than_assembled(tmp_path, second):
    first = {
        "tools": [realistic_tool(0)],
        "after_index": 0,
        "next_after_index": 1,
        "total": 2,
    }
    hub = FakeHub(device_replies=[answered(first), answered(second)])
    context = serving(tmp_path, hub)
    _client, server = await context.__aenter__()
    try:
        result = await server.call_tool("orbit_control_tools", {"device": DEFAULT_DEVICE})
    finally:
        await context.__aexit__(None, None, None)

    assert result.is_error
    assert "invalid_response" in text_of(result)


async def test_a_catalogue_that_changes_mid_walk_is_named_not_mixed(tmp_path):
    first = {
        "tools": [realistic_tool(0)],
        "after_index": 0,
        "next_after_index": 1,
        "total": 2,
    }
    # The engine restarted with a different catalogue between the two pages.
    second = {"tools": [realistic_tool(7)], "after_index": 1, "total": 5}
    hub = FakeHub(device_replies=[answered(first), answered(second)])
    context = serving(tmp_path, hub)
    _client, server = await context.__aenter__()
    try:
        result = await server.call_tool("orbit_control_tools", {"device": DEFAULT_DEVICE})
    finally:
        await context.__aexit__(None, None, None)

    assert result.is_error
    assert "catalogue_changed" in text_of(result)
    assert "catalogue" in text_of(result).lower()
