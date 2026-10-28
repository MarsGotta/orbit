"""Assistant copy is English; meeting content keeps its requested language."""

import json
import re

import pytest
from support_orbit_mcp import FakeHub, serving

from orbit_hub.assistant.activity import public_item
from orbit_hub.assistant.backend import ActivityItem
from orbit_mcp.refusals import error_result


async def test_tool_descriptions_and_argument_help_are_english(tmp_path):
    async with serving(tmp_path, FakeHub()) as (_, server):
        tools = await server.list_tools()
    for tool in tools:
        text = (tool.description or "") + json.dumps(tool.input_schema, ensure_ascii=False)
        assert not re.search(
            r"\b(Lee|Lista|Busca|Carga|Obtiene|Ejecuta|Consulta|Enumera|Nombre|Argumentos)\b", text
        ), text


def test_tool_refusals_are_english_and_keep_the_cause_and_numbers():
    from orbit_mcp.hub import HubClientError

    result = error_result(
        HubClientError("too_many", detail={"refusal": {"code": "too_many", "count": 7, "max": 3}}),
    )
    text = result.content[0].text
    assert result.is_error and "too_many:" in text
    assert "7" in text and "3" in text
    assert "The request contains more items than allowed" in text


def test_activity_has_codes_values_and_an_english_fallback_for_older_macs():
    item = public_item(
        ActivityItem(
            "s",
            "t",
            "tool",
            "started",
            tool="orbit_screenshot",
            arguments={"application": "Sample"},
        )
    )
    assert item["label"] == "Looking at Sample"
    assert item["label_copy"] == {"code": "screenshot_app", "value": "Sample"}
    assert item["subject"] == "the screen"
    assert item["subject_code"] == "screen"


@pytest.mark.parametrize("long_meeting", [False, True])
@pytest.mark.parametrize("language,named", [("es", "Spanish"), ("en", "English")])
async def test_the_note_worker_tells_the_fake_model_the_meetings_language(
    environment, language, named, long_meeting
):
    from conftest import device
    from support_notes import FakeModel, configuration, write_note
    from test_meeting_views import TEXTS, meeting

    _, store, _ = environment
    owner = device(store)["device_id"]
    texts = {
        track: [text * (3000 if long_meeting else 1) for text in values]
        for track, values in TEXTS.items()
    }
    recording = meeting(store, owner, texts=texts, language=language)
    model = FakeModel(configuration=configuration(context_tokens=8192, max_completion_tokens=1024))
    worker = await write_note(store, owner, recording, model=model)
    calls = worker.client.calls
    assert worker.client.purposes()[-1] == ("note_reduce" if long_meeting else "note")
    for call in calls:
        assert f"Write in {named}" in call["messages"][0]["content"]
        prompt = call["messages"][1]["content"]
        assert f"Write the entire note in {named}" in prompt
        assert "Una reunión de trabajo cualquiera" not in prompt
    note = worker.repository.document(owner, recording)["note"]
    assert note["sections"][0]["name"] == ("Resumen" if language == "es" else "Summary")


async def test_privacy_codes_and_english_fallback_reach_a_paired_mac(environment):
    from conftest import device
    from test_meeting_language_and_title import routes_only
    from test_meeting_views import bearer

    from orbit_hub.app import create_app

    settings, store, adapter = environment
    owner = device(store)
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    async with routes_only(app) as http:
        response = await http.get("/v1/settings/ai", headers=bearer(owner))
    assert response.status_code == 200
    functions = {fn["id"]: fn for fn in response.json()["functions"]}
    transcription = functions["meeting_transcription"]
    assert transcription["privacy"] == "The audio stays on your hub."
    assert transcription["privacy_copies"] == [{"code": "hub", "material": "audio"}]


@pytest.mark.parametrize(
    "month,named",
    tuple(
        enumerate(
            ("Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"), 1
        )
    ),
)
def test_calendar_month_names_are_english_and_keep_the_date(month, named):
    from datetime import UTC, datetime

    from orbit_mcp.calendar import render

    start = int(datetime(2026, month, 1, tzinfo=UTC).timestamp() * 1000)
    text = render(
        {
            "requested": {"start_unix_ms": start, "end_unix_ms": start + 86400000},
            "events_returned": 0,
            "events": [],
            "system_timezone": "UTC",
        }
    )
    assert f"1 {named} 2026" in text.splitlines()[0]
