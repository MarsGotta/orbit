"""Both sides consume each bridge fixture."""

import json
from pathlib import Path

import pytest
from pydantic import ValidationError

from orbit_hub.protocol import DeviceReplyParams
from orbit_mcp.bridge import PARAMETER_MODELS, RESULT_MODELS, BridgeRefusal

ROOT = Path(__file__).resolve().parents[2] / "contracts" / "fixtures" / "bridge"


def names(folder):
    files = list(folder.iterdir())
    assert all(path.suffix == ".json" for path in files)
    return {path.stem for path in files}


def read(path):
    return json.loads(path.read_text())


def test_every_capability_has_a_request_and_both_reply_shapes():
    capabilities = set(PARAMETER_MODELS)
    assert names(ROOT / "requests") == capabilities
    assert names(ROOT / "replies") == {
        f"{capability}-{suffix}" for capability in capabilities for suffix in ("result", "refusal")
    }


@pytest.mark.parametrize("capability", sorted(PARAMETER_MODELS))
def test_request_fixture_is_consumed_by_mcp_parameter_model(capability):
    raw = read(ROOT / "requests" / f"{capability}.json")
    assert raw["event"] == "device.request"
    parameters = PARAMETER_MODELS[capability].model_validate(raw["data"]["parameters"])
    # Every field Rust emits must be understood by the consumer (CAL-08).
    assert parameters.model_dump(exclude_none=True) == raw["data"]["parameters"]


@pytest.mark.parametrize("capability", sorted(PARAMETER_MODELS))
@pytest.mark.parametrize("suffix", ["result", "refusal"])
def test_reply_fixture_is_consumed_by_mcp_reply_model(capability, suffix):
    raw = read(ROOT / "replies" / f"{capability}-{suffix}.json")
    assert raw["method"] == "device.reply"
    reply = DeviceReplyParams.model_validate(raw["params"])
    assert (reply.result is not None) == (suffix == "result")
    assert (reply.refusal is not None) == (suffix == "refusal")
    typed = (
        RESULT_MODELS[capability].model_validate(reply.result)
        if reply.result is not None
        else BridgeRefusal.model_validate(reply.refusal)
    )
    assert typed.model_dump(exclude_none=True) == raw["params"][suffix]
    if reply.result is not None:
        assert type(typed).__name__.removesuffix("Result").lower() == capability.replace("_", "")


def test_calendar_result_fixture_exercises_nested_events_and_cursor():
    reply = read(ROOT / "replies" / "calendar_read-result.json")
    result = reply["params"]["result"]
    assert result["events"][0]["invitees"][0]["participant_url"]
    assert result["oversized"][0]["field"] == "title"
    assert result["next_after_ms"] > result["events"][0]["start_unix_ms"]
    parsed = DeviceReplyParams.model_validate(reply["params"])
    assert parsed.result is not None
    assert (
        RESULT_MODELS["calendar_read"]
        .model_validate(parsed.result)
        .model_dump(exclude_none=True)["oversized"][0]["field"]
        == "title"
    )


@pytest.mark.parametrize("field", ["event", "invitee", "oversized"])
def test_calendar_fixture_nested_types_reject_wrong_fields(field):
    reply = read(ROOT / "replies" / "calendar_read-result.json")
    result = reply["params"]["result"]
    if field == "event":
        result["events"][0]["start_unix_ms"] = "ayer"
    elif field == "invitee":
        result["events"][0]["invitees"][0]["is_current_user"] = "no"
    else:
        result["oversized"][0]["bytes"] = "grande"
    with pytest.raises(ValidationError):
        RESULT_MODELS["calendar_read"].model_validate(reply["params"]["result"])
