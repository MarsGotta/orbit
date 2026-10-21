"""REF-402: additive hub compatibility at the public boundaries."""

import json
from uuid import uuid4

import pytest
from conftest import FakeAdapter, device
from fastapi.responses import Response
from fastapi.testclient import TestClient
from support_app import client

from orbit_hub.app import create_app
from orbit_hub.config import Settings
from orbit_hub.errors import render
from orbit_hub.protocol import (
    ApprovalParams,
    ClarifyParams,
    DeviceReplyParams,
    DeviceSummary,
    EmptyParams,
    Frame,
    InterruptParams,
    PairRequest,
    RecordingManifest,
    SendParams,
    SessionParams,
    SpeechRequest,
    TrackManifest,
)
from orbit_hub.storage import Store, initialize


@pytest.fixture(scope="module")
def route_client(tmp_path_factory, unit_stt):
    root = tmp_path_factory.mktemp("ref402-routes")
    data, key = root / "data", root / "key"
    initialize(data, key)
    settings = Settings(
        data_dir=data,
        key_file=key,
        hermes_url="https://hermes.invalid",
        hermes_credential_file=root / "never-read",
        meeting_transcription=unit_stt.configuration(),
        allowed_hosts=("localhost",),
    )
    store = Store(data, key)
    try:
        with TestClient(
            create_app(settings, store=store, adapter=FakeAdapter()), base_url="https://localhost"
        ) as http:
            yield http
    finally:
        store.close()


@pytest.mark.parametrize(
    "method,path,code",
    [
        ("POST", "/v1/audio/transcribe", "invalid_payload"),
        ("POST", "/v1/audio/speech", "invalid_payload"),
        ("PUT", f"/v1/captures/{uuid4()}", "invalid_capture"),
        ("PUT", "/v1/chunks/chunk", "invalid_chunk"),
        ("POST", f"/v1/recordings/{uuid4()}/finalize", "invalid_payload"),
        ("POST", "/v1/recordings/invalid/transcript/retry", "invalid_recording"),
    ],
)
def test_registered_http_routes_share_additive_error_shape(route_client, method, path, code):
    assert any(
        route.path_regex.fullmatch(path.split("?", 1)[0])
        and route.methods
        and method in route.methods
        for route in route_client.app.routes
        if hasattr(route, "methods")
    )
    paired = device(route_client.app.state.store)
    response = route_client.request(
        method, path, headers={"Authorization": "Bearer " + paired["token"]}
    )
    assert response.status_code >= 400
    body = response.json()
    assert body["code"] == code, (path, body)
    assert body["refusal"]["code"] == code
    assert "cause" not in body["refusal"]
    assert body["refusal"]["retry"]["kind"] in {"never", "after_fix", "after_ms", "check_outcome"}


@pytest.mark.parametrize(
    "method,path,status,code",
    [
        ("POST", f"/v1/recordings/{uuid4()}/transcript/retry", 404, "recording_not_found"),
        ("POST", "/v1/sessions", 405, "invalid_request"),
    ],
)
def test_authenticated_routes_render_their_own_errors(route_client, method, path, status, code):
    paired = device(route_client.app.state.store)
    response = route_client.request(
        method, path, headers={"Authorization": "Bearer " + paired["token"]}
    )
    assert response.status_code == status, (path, response.text)
    body = response.json()
    assert body["code"] == code
    assert body["refusal"]["code"] == ("not_found" if code == "recording_not_found" else code)
    assert body["refusal"]["retry"]["kind"]
    assert body["message"]


def test_boundary_preserves_existing_refusal_and_error_headers(route_client):
    original = {
        "code": "no_answer",
        "message": "El Mac no respondió dentro del plazo",
        "refusal": {
            "code": "timeout",
            "operation": "computer_action",
            "waited_ms": 30000,
            "deadline_ms": 30000,
            "retry": {"kind": "check_outcome"},
        },
    }
    route_client.app.add_api_route(
        "/ref402-existing-refusal",
        lambda: Response(
            content=json.dumps(original),
            status_code=504,
            media_type="application/json",
            headers={"X-Ref402": "kept"},
        ),
    )
    response = route_client.get("/ref402-existing-refusal", headers={"X-Orbit-Op": "op-ref402-1"})
    assert response.status_code == 504
    # SIM-22: kept as it was, plus the op of the request it answers.
    assert response.json() == {**original, "op": "op-ref402-1"}
    assert response.headers["X-Ref402"] == "kept"


@pytest.mark.parametrize("body", [b"{", b"[1]"])
def test_boundary_preserves_unrecognized_json_error_body_and_headers(route_client, body):
    path = "/ref402-unrecognized-json-" + ("invalid" if body == b"{" else "array")
    route_client.app.add_api_route(
        path,
        lambda: Response(
            content=body,
            status_code=418,
            media_type="application/json",
            headers={"X-Ref402": "kept"},
        ),
    )
    response = route_client.get(path)
    assert response.status_code == 418
    assert response.content == body
    assert response.headers["X-Ref402"] == "kept"


@pytest.mark.parametrize(
    "code,numbers,expected_code,expected_retry",
    [
        ("session_busy", None, "busy", {"kind": "after_fix"}),
        (
            "too_many",
            {"resource": "capture", "count": 5, "max": 4},
            "too_many",
            {"kind": "after_fix"},
        ),
        (
            "speech_too_large",
            {"bytes": 9, "max_bytes": 8},
            "too_large",
            {"kind": "after_fix"},
        ),
        (
            "too_large",
            {"resource": "ipc_request", "bytes": 9, "max_bytes": 8},
            "too_large",
            {"kind": "after_fix"},
        ),
        ("recording_not_found", None, "not_found", {"kind": "after_fix"}),
        (
            "no_answer",
            {"deadline_ms": 30000, "waited_ms": 30000},
            "timeout",
            {"kind": "check_outcome"},
        ),
        ("device_unavailable", None, "mac_disconnected", {"kind": "check_outcome"}),
    ],
)
def test_hub_family_refusals_carry_their_wire_code_and_retry(
    code, numbers, expected_code, expected_retry
):
    payload = render(code, "Causa de prueba", numbers=numbers)
    refusal = payload["refusal"]
    assert payload["code"] == code
    assert refusal["code"] == expected_code
    assert refusal["retry"] == expected_retry
    assert "cause" not in refusal


@pytest.mark.parametrize(
    "code,numbers,expected_retry",
    [
        ("too_large", {"bytes": 9, "max_bytes": 8}, "after_fix"),
        ("speech_too_large", {"bytes": None, "max_bytes": 8}, "after_fix"),
        (
            "speech_too_large",
            {"resource": "unknown", "bytes": 9, "max_bytes": 8},
            "after_fix",
        ),
    ],
)
def test_too_large_without_complete_known_numbers_keeps_legacy_code(code, numbers, expected_retry):
    refusal = render(code, "Causa de prueba", numbers=numbers)["refusal"]
    assert refusal["code"] == code
    assert refusal["retry"] == {"kind": expected_retry}
    assert "bytes" not in refusal or type(refusal["bytes"]) is int
    assert "max_bytes" not in refusal or type(refusal["max_bytes"]) is int
    assert "resource" not in refusal


def test_timeout_without_measured_wait_keeps_legacy_code():
    refusal = render("no_answer", "El Mac no respondió", numbers={"deadline_ms": 30000})["refusal"]
    assert refusal == {
        "code": "no_answer",
        "deadline_ms": 30000,
        "retry": {"kind": "check_outcome"},
    }


@pytest.mark.parametrize(
    "code,numbers,retry",
    [
        ("session_busy", None, {"kind": "after_fix"}),
        ("session_busy", {"retry_after_ms": 17}, {"kind": "after_ms", "ms": 17}),
        ("too_large", {"received_bytes": 2, "max_bytes": 1}, {"kind": "after_fix"}),
        ("outcome_unknown", None, {"kind": "check_outcome"}),
        ("device_unavailable", None, {"kind": "check_outcome"}),
        ("group_not_found", None, {"kind": "after_fix"}),
    ],
)
def test_retry_depends_on_failure_class(code, numbers, retry):
    assert render(code, "Causa de prueba", numbers=numbers)["refusal"]["retry"] == retry


@pytest.mark.parametrize(
    "model,payload",
    [
        (PairRequest, {"code": "a" * 16, "name": "Mac"}),
        (Frame, {"v": 1, "id": str(uuid4()), "method": "ping", "params": {}}),
        (SessionParams, {"session_id": "session"}),
        (InterruptParams, {"session_id": "session", "turn_id": "turn"}),
        (SendParams, {"text": "hola"}),
        (ApprovalParams, {"session_id": "session", "request_id": "request", "approved": True}),
        (ClarifyParams, {"session_id": "session", "request_id": "request", "answer": "sí"}),
        (SpeechRequest, {"text": "hola"}),
        (EmptyParams, {}),
        (DeviceReplyParams, {"request_id": "request", "result": {"windows": []}}),
        (
            TrackManifest,
            {"track": "mic_local", "chunk_count": 1, "last_sequence": 0, "total_frames": 1},
        ),
        (RecordingManifest, {"tracks": [], "gaps": 0, "started_at_ms": 1, "finalized_at_ms": 2}),
    ],
)
def test_mac_input_models_ignore_new_field_but_validate_known_fields(model, payload):
    parsed = model.model_validate({**payload, "future_field": "from a later Mac"})
    assert "future_field" not in parsed.model_dump()
    if payload:
        known = next(iter(payload))
        with pytest.raises(ValueError):
            model.model_validate({**payload, known: object(), "future_field": "ignored"})


def test_device_summary_rejects_unknown_hub_output_field():
    with pytest.raises(ValueError):
        DeviceSummary.model_validate(
            {
                "id": "device",
                "name": "Mac",
                "paired_at": 1.0,
                "connected": False,
                "last_seen": 1.0,
                "unknown": True,
            }
        )


def test_ready_versions_and_device_version_saved(environment):
    _, store, _ = environment
    paired = device(store)
    with client(environment) as http:
        with http.websocket_connect(
            "wss://localhost/v1/ws",
            headers={
                "Authorization": "Bearer " + paired["token"],
                "X-Orbit-Device-Version": "0.3.12",
            },
        ) as ws:
            ready = ws.receive_json()["data"]
            assert ready["hub_protocol"] == 1
            assert isinstance(ready["hub_version"], str) and ready["hub_version"]
            assert ready["device_version"] == "0.3.12"
            assert http.app.state.device_versions[paired["device_id"]] == "0.3.12"
        with http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + paired["token"]}
        ) as ws:
            assert ws.receive_json()["data"]["device_version"] is None
            assert http.app.state.device_versions[paired["device_id"]] is None


def test_ready_and_health_share_protocol_version(environment):
    _, store, _ = environment
    paired = device(store)
    with client(environment) as http:
        health = http.get("/health").json()
        with http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + paired["token"]}
        ) as ws:
            ready = ws.receive_json()["data"]
        from orbit_hub import protocol

        assert hasattr(protocol, "HUB_PROTOCOL")
        assert health["protocol"] == ready["hub_protocol"] == protocol.HUB_PROTOCOL


def test_oversized_device_version_is_ignored_and_logged(environment, caplog):
    _, store, _ = environment
    paired = device(store)
    with (
        client(environment) as http,
        http.websocket_connect(
            "wss://localhost/v1/ws",
            headers={
                "Authorization": "Bearer " + paired["token"],
                "X-Orbit-Device-Version": "x" * 129,
            },
        ) as ws,
    ):
        assert ws.receive_json()["data"]["device_version"] is None
        assert http.app.state.device_versions[paired["device_id"]] is None
    assert "device_version_ignored" in caplog.text
    assert "x" * 129 not in caplog.text


def test_http_and_websocket_refusals_are_additive(environment):
    _, store, _ = environment
    with client(environment) as http:
        response = http.get("/v1/sessions")
        assert response.status_code == 401
        body = response.json()
        assert body["code"] == "unauthorized"
        assert body["refusal"]["code"]
        assert body["message"]
        assert body["refusal"]["retry"] == {"kind": "after_fix"}
        oversized = http.post("/v1/pair", content=b"x" * 2049)
        assert oversized.json()["refusal"]["bytes"] == 2049
        assert oversized.json()["refusal"]["max_bytes"] == 2048
        for _ in range(9):
            assert http.post("/v1/pair", content=b"").status_code == 400
        limited = http.post("/v1/pair", content=b"")
        assert limited.status_code == 429
        assert limited.json()["refusal"]["retry"] == {"kind": "after_ms", "ms": 60_000}
        assert limited.json()["refusal"]["retry_after_ms"] == 60_000
        paired = device(store)
        with http.websocket_connect(
            "wss://localhost/v1/ws", headers={"Authorization": "Bearer " + paired["token"]}
        ) as ws:
            ws.receive_json()
            ws.send_json({"v": 1, "id": str(uuid4()), "method": "chat.send", "params": {"text": 1}})
            error = ws.receive_json()
            assert error["event"] == "error"
            data = error["data"]
            assert data["code"] == "invalid_request"
            assert data["refusal"]["code"]
            assert data["message"]
            assert data["refusal"]["retry"] == {"kind": "never"}
            ws.send_json(
                {"v": 1, "id": str(uuid4()), "method": "ping", "params": {"future": "x" * 131072}}
            )
            large = ws.receive_json()["data"]["refusal"]
            assert large["code"] == "too_large"
            assert large["bytes"] > large["max_bytes"]
