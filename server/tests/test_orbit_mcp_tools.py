from support_orbit_mcp import serving

"""The Orbit tools Deimos calls, fixed against a loopback hub with real auth.

Every request the tools sign is verified by the real ``Authenticator`` of
``orbit_hub.private_auth``; every reply uses the envelope of the production
routes. No Hermes, no Mac, no external network.
"""

import base64
import json

import pytest
from fastapi.responses import Response
from support_orbit_mcp import (
    BACKGROUND_WINDOW,
    BRIDGE_RESULT,
    DEFAULT_DEVICE,
    DEVICES,
    FOREGROUND_WINDOW,
    OBSERVATION_REFUSED_NOTICE,
    OTHER_DEVICE,
    PNG_BYTES,
    SCREENSHOT_ANSWER,
    SELECTED_TEXT_ANSWER,
    FakeHub,
    answered,
    device_refused,
    hub_rejected,
    text_of,
)

WINDOW_START_MS = 1_789_000_000_000
WINDOW_END_MS = 1_789_036_000_000


def images_of(result):
    return [block for block in result.content if getattr(block, "type", None) == "image"]


def typed_refused(refusal, *, code=None, notice=None, status=403):
    payload = {"protocol": BRIDGE_RESULT, "state": "refused", "refusal": refusal}
    payload["code"] = code or refusal["code"]
    if notice is not None:
        payload["refusal"]["detail"] = notice
    return Response(content=json.dumps(payload), status_code=status, media_type="application/json")


async def test_internal_error_keeps_its_server_message_when_cause_matches_a_native_code(tmp_path):
    hub = FakeHub(
        device_replies=[
            typed_refused(
                {
                    "code": "internal_error",
                    "cause": "native_command_invalid",
                    "retry": {"kind": "after_fix"},
                }
            )
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_control_tools", {"device": DEFAULT_DEVICE})
    body = text_of(result)
    assert result.is_error, body
    assert "Orbit encontró un fallo interno al preparar la respuesta" in body, body
    assert "Orbit no pudo construir" not in body, body
    assert "causa: native_command_invalid" in body, body


@pytest.mark.parametrize(
    "detail",
    [
        "start_ms y end_ms deben ser enteros",
        "after_index debe ser un entero no negativo",
        "full_screen debe ser verdadero o falso",
    ],
)
async def test_modern_invalid_parameter_keeps_the_specific_field(tmp_path, detail):
    hub = FakeHub(
        device_replies=[
            typed_refused(
                {
                    "code": "invalid_parameter",
                    "parameter": "request",
                    "expected": {"kind": "valid_selection"},
                    "detail": detail,
                    "retry": {"kind": "after_fix"},
                }
            )
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_control_tools", {"device": DEFAULT_DEVICE})
    assert result.is_error
    assert detail in text_of(result)


@pytest.mark.parametrize(
    "original,family,numbers,phrase",
    [
        (
            "no_answer",
            "timeout",
            {"operation": "interaction", "waited_ms": 31000, "deadline_ms": 30000},
            "El Mac no contestó",
        ),
    ],
)
async def test_hub_code_survives_its_refusal_family(
    tmp_path,
    original,
    family,
    numbers,
    phrase,
):
    hub = FakeHub(
        device_replies=[
            typed_refused(
                {
                    "code": family,
                    **numbers,
                    "retry": {"kind": "check_outcome"},
                },
                code=original,
            )
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_control_tools", {"device": DEFAULT_DEVICE})
    assert result.is_error
    assert text_of(result).startswith(original + ":")
    assert phrase in text_of(result)


async def test_screenshot_with_names_returns_the_hubs_image_and_names_the_window(tmp_path):
    hub = FakeHub(device_replies=[answered(SCREENSHOT_ANSWER)])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_screenshot",
            {
                "device": DEFAULT_DEVICE,
                "application": FOREGROUND_WINDOW["application"],
                "title": FOREGROUND_WINDOW["title"],
            },
        )
        assert not result.is_error
        images = images_of(result)
        assert len(images) == 1
        assert images[0].mime_type == "image/png"
        assert base64.b64decode(images[0].data) == PNG_BYTES
        text = text_of(result)
        assert FOREGROUND_WINDOW["application"] in text
        assert FOREGROUND_WINDOW["title"] in text
        assert str(SCREENSHOT_ANSWER["width"]) in text
        assert str(SCREENSHOT_ANSWER["height"]) in text
        # One signed request only, accepted by the real Authenticator, addressed
        # to the device the name resolves to.
        assert [request["capability"] for request in hub.device_requests] == ["screenshot"]
        assert hub.device_requests[0]["device_id"] == DEVICES[DEFAULT_DEVICE]
        assert hub.device_requests[0]["parameters"] == {
            "application": FOREGROUND_WINDOW["application"],
            "title": FOREGROUND_WINDOW["title"],
        }


@pytest.mark.parametrize("tool", ["orbit_screenshot", "orbit_selected_text"])
async def test_unknown_capture_filter_is_refused_before_hub(tmp_path, tool):
    hub = FakeHub(device_replies=[answered(SCREENSHOT_ANSWER)])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(tool, {"window": "Safari"})
    body = text_of(result)
    assert result.is_error, body
    assert "invalid_parameters" in body
    assert "window" in body
    assert "application" in body and "title" in body
    assert not hub.device_requests


async def test_uncertain_remote_action_keeps_its_retirement_cause_for_deimos(tmp_path):
    operation_id = "10000000-0000-4000-8000-000000000001"
    hub = FakeHub(
        device_replies=[
            typed_refused(
                {
                    "code": "outcome_unknown",
                    "operation_id": operation_id,
                    "detail": "Cambió el dispositivo; el control anterior se retiró.",
                    "retry": {"kind": "check_outcome"},
                }
            )
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_perform_action",
            {"device": DEFAULT_DEVICE, "action": {"tool": "click", "arguments": {}}},
        )
    body = text_of(result)
    assert result.is_error, body
    assert "outcome_unknown" in body
    assert operation_id in body
    assert "Cambió el dispositivo" in body
    assert "Todo está parado" not in body


async def test_look_without_parameters_inventories_then_captures_without_parameters(tmp_path):
    hub = FakeHub(
        device_replies=[
            answered({"windows": [BACKGROUND_WINDOW, FOREGROUND_WINDOW]}),
            answered(SCREENSHOT_ANSWER),
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_look", {})
        assert not result.is_error
        assert [request["capability"] for request in hub.device_requests] == [
            "screen_sources",
            "screenshot",
        ]
        # The default device was resolved from the configuration, not named.
        assert all(
            request["device_id"] == DEVICES[DEFAULT_DEVICE] for request in hub.device_requests
        )
        assert hub.device_requests[1]["parameters"] == {}
        images = images_of(result)
        assert len(images) == 1
        assert base64.b64decode(images[0].data) == PNG_BYTES
        text = text_of(result)
        assert BACKGROUND_WINDOW["application"] in text
        assert BACKGROUND_WINDOW["title"] in text
        assert FOREGROUND_WINDOW["application"] in text
        assert FOREGROUND_WINDOW["title"] in text


async def test_rejections_and_timeouts_are_tool_results_and_service_continues(tmp_path):
    hub = FakeHub(
        device_replies=[
            hub_rejected(404, "device_unavailable"),
            device_refused(OBSERVATION_REFUSED_NOTICE),
            hub_rejected(504, "no_answer"),
            answered(SCREENSHOT_ANSWER),
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        arguments = {
            "device": DEFAULT_DEVICE,
            "application": FOREGROUND_WINDOW["application"],
            "title": FOREGROUND_WINDOW["title"],
        }

        offline = await server.call_tool("orbit_screenshot", arguments)
        assert offline.is_error
        assert "device_unavailable" in text_of(offline)
        assert "El Mac no está conectado a Orbit" in text_of(offline)

        refused = await server.call_tool("orbit_screenshot", arguments)
        assert refused.is_error
        # The current Mac names the permission; its sentence still travels with it.
        assert "permission_not_granted" in text_of(refused)
        assert "Grabación de pantalla" in text_of(refused)
        assert "no autorizado" not in text_of(refused).lower()
        assert "Grabación de pantalla" in text_of(refused)

        late = await server.call_tool("orbit_screenshot", arguments)
        assert late.is_error
        assert "no_answer" in text_of(late)
        assert "El Mac no contestó a tiempo" in text_of(late)

        # The process never falls over on the hub's say-so: the next call is served.
        recovered = await server.call_tool("orbit_screenshot", arguments)
        assert not recovered.is_error
        assert base64.b64decode(images_of(recovered)[0].data) == PNG_BYTES


@pytest.mark.parametrize("requested", ["Sample Mac", "SÁMPLE"], ids=["exact", "unique-prefix"])
async def test_name_resolution_ignores_case_and_accents(tmp_path, requested):
    device_id = "5f1d0000-0000-4000-8000-000000000041"
    hub = FakeHub(
        devices=[
            {
                "id": device_id,
                "name": "Sámple Mac",
                "paired_at": 1_000.0,
                "connected": True,
                "last_seen": 1_100.0,
            }
        ]
    )
    async with serving(tmp_path, hub, default_device="Sámple Mac", devices={}) as (_client, server):
        result = await server.call_tool("orbit_windows", {"device": requested})

    assert not result.is_error
    assert len(hub.device_requests) == 1
    assert hub.device_requests[0]["device_id"] == device_id


async def test_duplicate_name_uses_the_newest_connected_identity_and_says_so(tmp_path):
    older_id = "5f1d0000-0000-4000-8000-000000000042"
    newer_id = "5f1d0000-0000-4000-8000-000000000043"
    hub = FakeHub(
        devices=[
            {
                "id": older_id,
                "name": "Sample Mac",
                "paired_at": 1_000.0,
                "connected": True,
                "last_seen": 1_300.0,
            },
            {
                "id": newer_id,
                "name": "Sample Mac",
                "paired_at": 1_200.0,
                "connected": True,
                "last_seen": 1_250.0,
            },
        ]
    )
    async with serving(tmp_path, hub, default_device="Sample Mac", devices={}) as (_client, server):
        result = await server.call_tool("orbit_windows", {"device": "Sample Mac"})

    assert not result.is_error
    assert hub.device_requests[0]["device_id"] == newer_id
    text = text_of(result).lower()
    assert "varias identidades" in text
    assert "conectada más reciente" in text


async def test_devices_names_connection_and_last_seen_without_ids(tmp_path):
    connected_id = "5f1d0000-0000-4000-8000-000000000044"
    disconnected_id = "5f1d0000-0000-4000-8000-000000000045"
    hub = FakeHub(
        devices=[
            {
                "id": connected_id,
                "name": "Sample Mac",
                "paired_at": 1_000.0,
                "connected": True,
                "last_seen": 1_300.0,
            },
            {
                "id": disconnected_id,
                "name": "Mac de viaje",
                "paired_at": 900.0,
                "connected": False,
                "last_seen": 1_100.0,
            },
        ]
    )
    async with serving(
        tmp_path,
        hub,
        default_device="favorito",
        devices={"favorito": connected_id},
    ) as (_client, server):
        result = await server.call_tool("orbit_devices", {})

    assert not result.is_error
    text = text_of(result)
    assert "Sample Mac — conectado sí — visto por última vez hace" in text
    assert "Mac de viaje — conectado no — visto por última vez hace" in text
    assert "fichero: favorito" in text
    assert connected_id not in text and disconnected_id not in text


async def test_recent_bridge_activity_is_described_as_less_than_a_minute(tmp_path, monkeypatch):
    now = 2_000.0
    monkeypatch.setattr("orbit_mcp.devices.time.time", lambda: now)
    hub = FakeHub(
        devices=[
            {
                "id": DEVICES[DEFAULT_DEVICE],
                "name": "Sample Mac",
                "paired_at": 1_000.0,
                "connected": True,
                "last_seen": now - 10,
            }
        ]
    )
    async with serving(tmp_path, hub, default_device="Sample Mac", devices={}) as (_client, server):
        result = await server.call_tool("orbit_devices", {})

    assert not result.is_error
    assert "visto por última vez hace menos de un minuto" in text_of(result)


# The sharing switch is gone and the agenda now shares itself,
# so no message may send the user to press something that is no longer there. The
# autonomy switches are named as Ajustes actually labels them.
RETIRED_CONTROLS = (
    "Compartir con Deimos",
    "vuelve a compartir",
    "volver a compartirla",
    "volver a compartir",
    "selecciona el calendario",
    "Deimos ve mi pantalla",
    "Deimos puede actuar",
    "Deimos ve mis reuniones",
)


def test_no_message_sends_the_user_to_a_control_that_no_longer_exists():
    from orbit_mcp.refusals import ERROR_MESSAGES

    for code, message in ERROR_MESSAGES.items():
        for phrase in RETIRED_CONTROLS:
            assert phrase.lower() not in message.lower(), (
                f"{code} still sends the user to «{phrase}», which the app no longer has"
            )


async def test_unknown_device_names_known_macs_and_marks_the_default(tmp_path):
    hub = FakeHub(
        devices=[
            {
                "id": DEVICES[DEFAULT_DEVICE],
                "name": "Sample Mac",
                "paired_at": 1_000.0,
                "connected": True,
                "last_seen": 1_300.0,
            },
            {
                "id": DEVICES["sobremesa"],
                "name": "Mac de viaje",
                "paired_at": 900.0,
                "connected": False,
                "last_seen": 1_100.0,
            },
        ]
    )
    async with serving(tmp_path, hub, default_device="Sample Mac", devices={}) as (_client, server):
        result = await server.call_tool("orbit_windows", {"device": "Mac inexistente"})

    assert result.is_error
    text = text_of(result)
    assert "unknown_device" in text
    assert "Conozco: Sample Mac (predeterminado), Mac de viaje" in text
    assert hub.device_requests == []


async def test_revoked_identity_is_not_listed_or_resolved_by_name(tmp_path):
    hub = FakeHub(
        devices=[
            {
                "id": DEVICES[DEFAULT_DEVICE],
                "name": "Sample Mac",
                "paired_at": 1_000.0,
                "connected": True,
                "last_seen": 1_300.0,
            }
        ]
    )
    async with serving(tmp_path, hub, default_device="Sample Mac", devices={}) as (_client, server):
        listing = await server.call_tool("orbit_devices", {})
        retired = await server.call_tool("orbit_windows", {"device": "Mac retirado"})

    text = text_of(listing)
    assert "Sample Mac — conectado sí" in text
    assert "predeterminado" in text
    assert "Mac retirado" not in text
    assert retired.is_error
    assert "unknown_device" in text_of(retired)
    assert hub.device_requests == []


async def test_a_full_screen_tells_deimos_what_its_coordinates_are(tmp_path):
    """The Mac fits a full screen into what a capture can carry and
    says, with both sizes, that desktop coordinates are given on that image.
    The sentence survives the hub's reply contract and reaches Deimos."""
    from orbit_mcp.bridge import ScreenshotResult

    coordinates = (
        "Esta imagen mide 1280 × 831 píxeles y muestra la pantalla entera, que mide 3024 × 1964."
    )
    answer = {
        **SCREENSHOT_ANSWER,
        "application": "Escritorio",
        "title": "Pantalla completa",
        "width": 1280,
        "height": 831,
        "coordinates": coordinates,
    }
    wire = {key: value for key, value in answer.items() if key != "data_base64"}
    wire |= {"capture_id": "c", "sha256": "a" * 64}
    dumped = ScreenshotResult.model_validate(wire).model_dump(exclude_none=True)
    assert dumped["coordinates"] == coordinates
    hub = FakeHub(device_replies=[answered(answer)])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_screenshot", {"device": DEFAULT_DEVICE, "full_screen": True}
        )
    assert not result.is_error, text_of(result)
    assert coordinates in text_of(result)


@pytest.mark.parametrize(
    "active,connected,default_connected,explicit,expected,reason",
    [
        (True, True, False, None, OTHER_DEVICE, "turno"),
        (True, True, True, None, OTHER_DEVICE, "turno"),
        (False, True, False, None, OTHER_DEVICE, "único"),
        (False, False, False, None, DEFAULT_DEVICE, "fichero"),
        (False, True, True, None, DEFAULT_DEVICE, "fichero"),
        (True, True, False, DEFAULT_DEVICE, DEFAULT_DEVICE, None),
    ],
)
async def test_unnamed_capture_uses_the_speaking_mac_then_connection_then_config(
    tmp_path, active, connected, default_connected, explicit, expected, reason
):
    hub = FakeHub(
        devices=[
            {
                "id": DEVICES[DEFAULT_DEVICE],
                "name": DEFAULT_DEVICE,
                "paired_at": 1000.0,
                "connected": default_connected,
                "last_seen": 1001.0,
            },
            {
                "id": DEVICES[OTHER_DEVICE],
                "name": OTHER_DEVICE,
                "paired_at": 1000.0,
                "connected": connected,
                "last_seen": 1001.0,
                **({"assistant_turn_active": True} if active else {}),
            },
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_screenshot", {"device": explicit} if explicit else {}
        )
    assert not result.is_error, text_of(result)
    assert hub.device_requests[0]["device_id"] == DEVICES[expected]
    if reason:
        assert expected in text_of(result)
        assert reason in text_of(result)


async def test_orbit_mcp_does_not_name_one_agent_to_every_agent(tmp_path):
    """orbit-mcp serves every agent of the hub, whatever its name: neither its
    instructions nor the Mac it chose for the turn in progress say «Deimos»."""
    hub = FakeHub(
        devices=[
            {
                "id": DEVICES[OTHER_DEVICE],
                "name": OTHER_DEVICE,
                "paired_at": 1000.0,
                "connected": True,
                "last_seen": 1001.0,
                "assistant_turn_active": True,
            },
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_screenshot", {})
    assert not result.is_error, text_of(result)
    assert "turno" in text_of(result)
    assert "Deimos" not in text_of(result)
    assert "Deimos" not in (server.instructions or "")


async def test_a_changed_window_tells_deimos_to_request_it_again(tmp_path):
    hub = FakeHub(device_replies=[device_refused("", code="window_identity_changed")])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_screenshot", {})
    assert result.is_error
    text = text_of(result).lower()
    assert "ventana cambió" in text
    assert "vuelve a pedir" in text
    assert "no conoce" not in text


async def test_automatic_selection_refreshes_the_inventory_while_named_reads_keep_the_cache(
    tmp_path, monkeypatch
):
    from types import SimpleNamespace

    from orbit_mcp import hub as hub_module

    # Keep the cache inside its window without changing the I/O loop's clock.
    monkeypatch.setattr(
        hub_module, "time", SimpleNamespace(monotonic=lambda: 0, time=hub_module.time.time)
    )
    hub = FakeHub()
    async with serving(tmp_path, hub) as (_client, server):
        first = await server.call_tool("orbit_screenshot", {})
        assert not first.is_error, text_of(first)
        assert hub.device_requests[-1]["device_id"] == DEVICES[DEFAULT_DEVICE]
        hub.devices[0]["connected"] = False
        hub.devices[1]["connected"] = True
        hub.devices[1]["assistant_turn_active"] = True
        # A named call and a listing retain the ordinary cache.
        named = await server.call_tool("orbit_screenshot", {"device": DEFAULT_DEVICE})
        listing = await server.call_tool("orbit_devices", {})
        assert not named.is_error and not listing.is_error
        assert hub.device_list_requests == 1
        second = await server.call_tool("orbit_screenshot", {})
        assert not second.is_error, text_of(second)
        assert hub.device_requests[-1]["device_id"] == DEVICES[OTHER_DEVICE]
        assert hub.device_list_requests == 2


async def test_action_schema_describes_tool_and_default_empty_arguments(tmp_path):
    async with serving(tmp_path, FakeHub()) as (_client, server):
        tools = {tool.name: tool for tool in await server.list_tools()}
    schema = tools["orbit_perform_action"].input_schema
    action = schema["properties"]["action"]
    if "$ref" in action:
        action = schema["$defs"][action["$ref"].rsplit("/", 1)[1]]
    assert action["required"] == ["tool"]
    assert action["properties"]["tool"]["type"] == "string"
    assert action["properties"]["arguments"]["type"] == "object"
    assert action["properties"]["arguments"]["default"] == {}
    assert action["additionalProperties"] is False


@pytest.mark.parametrize("tool", ["get_screen_size", "click"])
async def test_action_without_arguments_reaches_hub_with_empty_object(tmp_path, tool):
    hub = FakeHub()
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_perform_action", {"action": {"tool": tool}})
    assert not result.is_error, text_of(result)
    assert hub.device_requests[0]["parameters"]["action"] == {
        "tool": tool,
        "arguments": {},
    }


@pytest.mark.parametrize(
    "action,field,received,expected",
    [
        ({"name": "PRIVATE_VALUE"}, "action", "name", "tool"),
        (
            {"tool": "PRIVATE_VALUE", "arguments": [], "extra": "PRIVATE_VALUE"},
            "action",
            "extra",
            "arguments",
        ),
        ({"tool": "PRIVATE VALUE"}, "action.tool", "tool", "128"),
        ({"tool": ""}, "action.tool", "tool", "128"),
        (
            {"tool": "click", "arguments": "PRIVATE_VALUE"},
            "action.arguments",
            "arguments",
            "objeto",
        ),
        ([], "action", "list", "objeto"),
    ],
)
async def test_action_refusal_names_field_keys_and_shape_without_values(
    tmp_path, action, field, received, expected
):
    hub = FakeHub()
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_perform_action", {"action": action})
    body = text_of(result)
    assert result.is_error, body
    assert "invalid_parameters" in body
    assert field in body and received in body and expected in body, body
    assert "PRIVATE_VALUE" not in body and "PRIVATE VALUE" not in body
    assert not hub.device_requests and not hub.device_list_requests


async def test_untranslated_code_says_phrase_missing_and_keeps_mac_detail(tmp_path):
    hub = FakeHub(
        device_replies=[
            typed_refused(
                {"code": "new_native_cause", "retry": {"kind": "never"}},
                code="new_native_cause",
                notice="Comprobación nativa: 3 ventanas, 0 coincidencias.",
            )
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_selected_text", {})
    body = text_of(result)
    assert result.is_error
    assert "new_native_cause" in body and "no tiene frase" in body
    assert "3 ventanas, 0 coincidencias" in body
    assert "revisa sus versiones" not in body


@pytest.mark.parametrize(
    "code,phrase",
    [
        ("selected_text_empty", "rango de selección vacío"),
        ("selected_text_not_exposed", "no se sabe si hay texto seleccionado"),
        ("selected_text_unavailable", "no se sabe si hay texto seleccionado"),
    ],
)
async def test_selection_absence_exposure_and_read_failure_have_distinct_phrases(
    tmp_path, code, phrase
):
    hub = FakeHub(
        device_replies=[
            typed_refused(
                {"code": "helper_refused", "helper_code": code, "retry": {"kind": "after_fix"}}
            )
        ]
    )
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool("orbit_selected_text", {})
    assert result.is_error
    assert code in text_of(result) and phrase in text_of(result)


async def test_selected_text_uses_its_own_capability(tmp_path):
    hub = FakeHub(device_replies=[answered(SELECTED_TEXT_ANSWER)])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_selected_text", {"device": DEFAULT_DEVICE, "application": "Calculadora"}
        )
    assert not result.is_error
    assert hub.device_requests[0]["capability"] == "selected_text"


@pytest.mark.parametrize("outcome", ["performed", "failed", "unknown"])
async def test_perform_action_accepts_inline_outcomes_in_the_lookup_shape(tmp_path, outcome):
    from fastapi.responses import JSONResponse

    from orbit_hub.bridge_outcomes import RESULT_PROTOCOL

    payload = {
        "protocol": RESULT_PROTOCOL,
        "state": "answered",
        "operation_id": "00000000-0000-4000-8000-000000000001",
        "outcome": outcome,
        "receipt": {"state": outcome, "tool": "fixture_action"},
    }
    hub = FakeHub(device_replies=[JSONResponse(payload)])
    async with serving(tmp_path, hub) as (_client, server):
        result = await server.call_tool(
            "orbit_perform_action", {"action": {"tool": "fixture_action"}}
        )
    assert not result.is_error, text_of(result)
    decoded = json.loads(result.content[0].text)
    assert decoded == {"outcome": outcome, "receipt": payload["receipt"]}
    assert len(hub.device_requests) == 1
