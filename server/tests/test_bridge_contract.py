from support_orbit_mcp import serving

"""CAL-06: the two sides of the bridge, tested against each other.

`orbit_calendar` shipped in 0.3.11 and never worked once. Alina asked Deimos for
her agenda, twice, and both times got back the same four words with no cause in
them::

    {"error": "invalid_parameters: Los parámetros no son válidos.
     Corrige sus tipos y límites antes de reintentarlo."}

The translator sent ``{"start_ms": str(start_ms), ...}`` and the Mac reads those
two fields with ``as_i64``, which yields nothing for a string, so the refusal
branch was taken on every single call. Both suites were green: the Python tests
answered through a fake bridge that never runs the Mac's parsing, and the Rust
cases built the parameters as numbers by hand. Each side was tested against its
own idea of the contract and nobody held the two ideas side by side.

So the contract stops being an idea. ``contracts/fixtures/device-bridge-request-v1.json``
holds the parameters as they travel, and these cases assert that what leaves the
translator is exactly that document — types included. The Mac's half of the same
assertion is ``crates/orbit-daemon/src/daemon/calendar/bridge_cases.rs``, which
feeds the same document to the real ``device_request``. A change of type on
either side now fails one of the two.

The second half of the spec is why nobody saw the first. The Mac refuses with a
sentence that says exactly what is missing and the translator threw it away,
keeping only the code. Deimos said so itself: «no especifica qué parámetro
rechaza ni por qué». That is not a calendar problem — it is every refusal that
comes back from the Mac over the bridge.
"""

import json
from pathlib import Path

import pytest
from fastapi.responses import Response
from support_orbit_mcp import (
    BRIDGE_RESULT,
    FakeHub,
    answered,
    text_of,
)

from orbit_mcp.hub import MAC_NOTICE_CHARS

FIXTURE = Path(__file__).resolve().parents[2] / "contracts" / "fixtures"
CONTRACT = "orbit.device-bridge-request.v1"

# The exact range Mars asked Deimos about, 24-09-2026: one day, in whole
# milliseconds since 1970. Nothing about it was ever wrong.
HER_START_MS = 1_790_200_800_000
HER_END_MS = 1_790_287_200_000

# What the Mac says when it refuses that read, word for word from
# `vertical.rs`. It names both fields, their type and their order.
MAC_SENTENCE = (
    "Una lectura de agenda necesita start_ms y end_ms, enteros en milisegundos "
    "desde 1970, con end_ms posterior a start_ms."
)


def requests_by_capability() -> dict:
    document = json.loads((FIXTURE / "device-bridge-request-v1.json").read_bytes())
    assert document["schema"] == CONTRACT
    entries = document["requests"]
    assert isinstance(entries, list) and entries
    return {entry["capability"]: entry["parameters"] for entry in entries}


CONTRACTED = requests_by_capability()


def canonical(value) -> str:
    """Types included: `123` and `"123"` are different documents here."""
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def refused_by_the_mac(notice: str, code: str | None = None, **extra) -> Response:
    """The envelope `bridge_route.ask` builds out of the Mac's own refusal.

    `extra` is never sent by the real route; it is here so a case can prove that
    nothing but `notice` is relayed even if something else turns up beside it.
    """
    body = {
        "protocol": BRIDGE_RESULT,
        "state": "refused",
        "refusal": {
            "code": code or "permission_denied",
            "detail": notice,
            "retry": {"kind": "after_fix"},
        },
        **extra,
    }
    return Response(
        content=canonical(body).encode("utf-8"),
        status_code=403,
        media_type="application/json",
        headers={"Cache-Control": "no-store"},
    )


READING = {
    "read_id": "7a1b0000-0000-4000-8000-00000000c0de",
    "requested": {"start_unix_ms": HER_START_MS, "end_unix_ms": HER_END_MS},
    "read_at_unix_ms": HER_START_MS - 1_000,
    "system_timezone": "Europe/Madrid",
    "provider_sync_status": "unknown",
    "enumeration": "bounded_local_read",
    "events": [
        {
            "observation_id": "66666666-6666-4666-8666-666666666666",
            "calendar": {"calendar_id": "local", "source_id": "account"},
            "calendar_item_id": "item-0",
            "start_unix_ms": HER_START_MS + 3_600_000,
            "end_unix_ms": HER_START_MS + 7_200_000,
            "interpreted_timezone": "Europe/Madrid",
            "is_floating": True,
            "is_all_day": False,
            "is_detached": False,
            "title": "Revisión de Orbit",
            "status": "confirmed",
            "invitees_status": "not_provided",
            "event_url_status": "not_provided",
        }
    ],
    "events_returned": 1,
    "omitted": [],
}


def continuations() -> list[dict]:
    """REF-10: how a read goes on, as the fixture pins it.

    Two cases: after whole events, and in the middle of one event whose own
    invitees did not fit a page (``after_invitee``).
    """
    document = json.loads((FIXTURE / "device-bridge-request-v1.json").read_bytes())
    return [
        item["parameters"]
        for item in document["continuations"]
        if item["capability"] == "calendar_read"
    ]


def continuation() -> dict:
    """The continuation after whole events."""
    (entry,) = [item for item in continuations() if "after_invitee" not in item]
    return entry


def fragment_continuation() -> dict:
    """The continuation in the middle of one event's invitees."""
    (entry,) = [item for item in continuations() if "after_invitee" in item]
    return entry


async def test_ref10_a2_the_continuation_leaves_the_translator_as_the_contract_says(tmp_path):
    """The second request of a walk is the fixture's continuation, types included."""
    following = continuation()
    first = {
        **READING,
        "read_id": following["read_id"],
        "next_after_ms": following["after_ms"],
        "next_after_index": following["after_index"],
    }
    hub = FakeHub(device_replies=[answered(first), answered({**READING, "events": []})])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar", {"start_ms": HER_START_MS, "end_ms": HER_END_MS}
        )
    assert not result.is_error, text_of(result)
    assert canonical(hub.device_requests[1]["parameters"]) == canonical(following)


async def test_ref10_a3_a_continuation_inside_one_event_leaves_the_translator_as_the_contract_says(
    tmp_path,
):
    """REF-10 A3: the Mac stopped inside one event's invitees; the walk goes on there.

    The index stays on the same event and ``after_invitee`` names the first
    invitee not sent yet, so the cursor still moves forward and the Mac finds
    its place.
    """
    following = fragment_continuation()
    first = {
        **READING,
        "events": [{**READING["events"][0], "invitees_continued": True, "invitees": []}],
        "read_id": following["read_id"],
        "next_after_ms": following["after_ms"],
        "next_after_index": following["after_index"],
        "next_after_invitee": following["after_invitee"],
    }
    hub = FakeHub(
        device_replies=[
            answered(
                {**first, "next_after_index": following["after_index"], "next_after_invitee": 1}
            ),
            answered(first),
            answered({**READING, "events": [{**READING["events"][0], "invitees": []}]}),
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar", {"start_ms": HER_START_MS, "end_ms": HER_END_MS}
        )
    assert not result.is_error, text_of(result)
    assert canonical(hub.device_requests[2]["parameters"]) == canonical(following)


async def test_a1_a2_a6_the_range_leaves_the_translator_as_the_contract_says(tmp_path):
    """A1/A2/A6: Alina's own two numbers, from the tool to the wire.

    This is the whole of defect one in one assertion. Before the fix the
    parameters recorded here were ``{"start_ms": "1790200800000", ...}`` and the
    Mac refused every one of them.
    """
    hub = FakeHub(device_replies=[answered(READING)])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar", {"start_ms": HER_START_MS, "end_ms": HER_END_MS}
        )

    body = text_of(result)
    assert not result.is_error, body
    sent = hub.device_requests[0]
    assert sent["capability"] == "calendar_read"
    assert canonical(sent["parameters"]) == canonical(CONTRACTED["calendar_read"])
    # A6: and the events come back, which is what she asked for.
    assert "Revisión de Orbit" in body


async def test_a2_the_screenshot_parameters_also_match_the_contract(tmp_path):
    """A2/A7: the other capabilities whose parameters the Mac parses by type."""
    hub = FakeHub()
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_screenshot", {"full_screen": True})

    assert not result.is_error, text_of(result)
    assert canonical(hub.device_requests[0]["parameters"]) == canonical(CONTRACTED["screenshot"])


async def test_a2_the_catalogue_page_number_also_matches_the_contract(tmp_path):
    """A2/A7: `after_index` is read with `as_u64`; it has to arrive as a number."""
    after = CONTRACTED["control_tools"]["after_index"]
    hub = FakeHub(
        device_replies=[
            answered(
                {
                    "tools": [{"name": "click"}, {"name": "type"}],
                    "after_index": 0,
                    "total": 3,
                    "next_after_index": after,
                }
            ),
            answered({"tools": [{"name": "scroll"}], "after_index": after, "total": 3}),
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_control_tools", {})

    assert not result.is_error, text_of(result)
    assert hub.device_requests[0]["parameters"] == {}
    assert canonical(hub.device_requests[1]["parameters"]) == canonical(CONTRACTED["control_tools"])


async def test_a3_the_macs_own_sentence_reaches_whoever_asked(tmp_path):
    """A3: the explanation exists; it stops being thrown away on the way back.

    The fixed table sentence stays — it names the code — and the Mac's sentence
    arrives beside it, naming the two fields the table cannot know about.
    """
    hub = FakeHub(device_replies=[refused_by_the_mac(MAC_SENTENCE, "invalid_parameters")])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar", {"start_ms": HER_START_MS, "end_ms": HER_END_MS}
        )

    body = text_of(result)
    assert result.is_error, body
    assert "invalid_parameters" in body
    assert MAC_SENTENCE in body, body
    # Not only the code, and not only the fixed sentence of the table.
    assert "start_ms" in body and "end_ms" in body, body


async def test_a4_any_capability_refused_by_the_mac_carries_its_explanation(tmp_path):
    """A4: this is the bridge's rule, not the calendar's.

    `selected_text` refuses through the same typed envelope; its cause and
    sanitized detail must travel just the same.
    """
    notice = "macOS no concede Grabación de pantalla a Orbit; se concede en Ajustes del Sistema."
    hub = FakeHub(device_replies=[refused_by_the_mac(notice)])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_selected_text", {})

    body = text_of(result)
    assert result.is_error, body
    assert notice in body, body


async def test_oversized_refusal_detail_names_the_field_without_forwarding_content(tmp_path):
    """A5: a Mac that answers with a book does not get a book relayed."""
    notice = "Ñ" * (MAC_NOTICE_CHARS * 4)
    hub = FakeHub(device_replies=[refused_by_the_mac(notice, "calendar_busy")])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar", {"start_ms": HER_START_MS, "end_ms": HER_END_MS}
        )

    body = text_of(result)
    assert result.is_error, body
    assert "device_reply_invalid" in body
    assert "refusal.detail" in body
    assert "Ñ" not in body


async def test_a5_nothing_but_the_sentence_the_mac_meant_to_send_is_relayed(tmp_path):
    """A5: typed refusal detail is the channel; unrelated payload fields are not.

    A refusal that started relaying whatever else arrived beside it would be a
    way for the Mac's contents to travel out under the name of an error.
    """
    hub = FakeHub(
        device_replies=[
            refused_by_the_mac(
                "El Mac está ocupado leyendo la agenda.",
                "calendar_busy",
                result={"title": "Cena con Ana en el puerto"},
                detail={"invitees": ["ana@example.invalid"]},
                debug="/Users/sample/Library/Calendars",
            )
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar", {"start_ms": HER_START_MS, "end_ms": HER_END_MS}
        )

    body = text_of(result)
    assert result.is_error, body
    assert "El Mac está ocupado leyendo la agenda." in body
    assert "Cena con Ana" not in body, body
    assert "ana@example.invalid" not in body, body
    assert "/Users/mars" not in body, body


@pytest.mark.parametrize(
    "notice",
    ["", "   ", "\n\t ", "\x07\x00"],
    ids=["empty", "blank", "whitespace", "control"],
)
async def test_a5_an_empty_or_unprintable_notice_leaves_the_fixed_message_alone(tmp_path, notice):
    hub = FakeHub(device_replies=[refused_by_the_mac(notice, "calendar_busy")])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar", {"start_ms": HER_START_MS, "end_ms": HER_END_MS}
        )

    body = text_of(result)
    assert result.is_error, body
    assert body.startswith("calendar_busy: ")
    assert "\x00" not in body and "\x07" not in body
    assert body.strip() == body.rstrip()


async def test_a5_a_multiline_sentence_arrives_as_one_line(tmp_path):
    hub = FakeHub(
        device_replies=[refused_by_the_mac("Primera línea.\n\nSegunda línea.", "calendar_busy")]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_calendar", {"start_ms": HER_START_MS, "end_ms": HER_END_MS}
        )

    body = text_of(result)
    assert result.is_error, body
    assert "Primera línea. Segunda línea." in body, body


def test_generated_refusal_and_helper_codes_have_a_sentence_for_deimos():
    import re

    from orbit_mcp.server import ERROR_MESSAGES

    generated = Path(__file__).resolve().parents[2] / "apps/desktop/src/bridge/generated"
    for name in ("RefusalCode", "HelperCode"):
        codes = set(re.findall(r'"([a-z_]+)"', (generated / f"{name}.ts").read_text()))
        assert codes, f"no codes read from {name}"
        assert not codes - ERROR_MESSAGES.keys(), (name, codes - ERROR_MESSAGES.keys())


async def test_an_unnamed_transcript_keeps_the_counted_page_body_separate_from_the_mac(
    environment, tmp_path
):
    import re

    from conftest import device
    from test_deimos_meeting_reads import NOW, deimos, finished_meeting

    from orbit_hub.meetings.repository import set_owner_name

    _, store, _ = environment
    owner = device(store)["device_id"]
    recording = finished_meeting(
        store,
        owner,
        text_by_track={"mic_local": "Texto local. " * 5000, "remote_app": "Texto remoto. " * 5000},
    )
    set_owner_name(store, owner, "Alina", NOW)
    async with deimos(environment, tmp_path, owner) as server:
        result = await server.call_tool("orbit_transcript", {"recording_id": recording})
    assert not result.is_error, text_of(result)
    assert len(result.content) == 2
    assert all(block.type == "text" for block in result.content)
    header, separator, body = result.content[0].text.partition("\n")
    bounds = re.search(r"caracteres (\d+)–(\d+) de (\d+)", header)
    assert bounds and separator
    start, end, total = map(int, bounds.groups())
    assert len(body) == end - start
    assert len(body) == 48_000
    assert total > end
    assert header.startswith("Página: caracteres ")
    assert "portatil" in result.content[1].text and "fichero" in result.content[1].text
