"""Paragraph pauses cut turns only when another voice actually occupies the gap."""

# ruff: noqa: F811 - imported pytest fixture

import pytest
from conftest import device
from support_turn_readability import legacy_turns, measurements
from test_meeting_views import (
    all_turns,
    hub,  # noqa: F401 - pytest fixture
    running_client,
    seen_by_deimos,
)
from test_turn_utterances import final_recording, snapshot, words

from orbit_hub.meetings import turns


def pause_fixtures():
    """Invented, reproducible inputs used by the regression and numeric report."""
    remote = lambda text, at: words(text, "remote_app", "voice_1", at)
    local = lambda text, at: words(text, "mic_local", "mic_local", at)
    paragraph = remote("Hoy vemos planos nuevos.", 0)
    paragraph += remote("Luego elegimos flores azules.", 10_000)
    paragraph += remote("Finalmente plantamos junto al camino.", 25_000)
    jittered = local("Hoy revisamos los planos.", 0) + remote("We can send them tomorrow.", 450)
    for i, word in enumerate(jittered):
        shift = (0, 200, -100, 300)[i % 4]
        word["start_ms"] += shift
        word["end_ms"] += shift
    return {
        "paused_monologue": (
            local("Uno dos", 0)
            + local("tres cuatro", 10_000)
            + local("cinco seis", 25_000)
            + local("siete ocho.", 65_000),
            ["Uno dos tres cuatro cinco seis", "siete ocho."],
        ),
        "few_changes": (
            paragraph + local("Cierro la revisión.", 40_000),
            [
                (
                    "Hoy vemos planos nuevos. Luego elegimos flores azules. "
                    "Finalmente plantamos junto al camino."
                ),
                "Cierro la revisión.",
            ],
        ),
        "overlap_before_pause": (
            remote("Uno dos", 0) + remote("tres cuatro.", 4_000) + local("sí", 250),
            ["Uno dos tres cuatro.", "sí"],
        ),
        "speech_during_pause": (
            remote("Uno dos", 0) + remote("tres cuatro.", 4_000) + local("sí", 1_500),
            ["Uno dos", "sí", "tres cuatro."],
        ),
        "speech_spanning_pause_start": (
            remote("Uno dos", 0)
            + remote("tres cuatro.", 4_000)
            + words("sí", "mic_local", "mic_local", 250, duration=2_000),
            ["Uno dos", "sí", "tres cuatro."],
        ),
        "speech_at_resume": (
            remote("Uno dos", 0) + remote("tres cuatro.", 4_000) + local("sí", 4_000),
            ["Uno dos tres cuatro.", "sí"],
        ),
        "speech_ends_at_pause_start": (
            remote("Uno dos", 0)
            + remote("tres cuatro.", 4_000)
            + words("sí", "mic_local", "mic_local", 250, duration=650),
            ["Uno dos tres cuatro.", "sí"],
        ),
        "overlap": (
            remote("We can send them tomorrow.", 250) + local("Hoy revisamos los planos.", 0),
            ["Hoy revisamos los planos.", "We can send them tomorrow."],
        ),
        "jitter": (
            jittered,
            ["Hoy revisamos los planos.", "We can send them tomorrow."],
        ),
        "interjection": (
            remote("El jardín tendrá flores durante toda la primavera.", 0) + local("sí", 1_250),
            ["El jardín tendrá flores durante toda la primavera.", "sí"],
        ),
    }


@pytest.mark.parametrize("fixture", pause_fixtures())
async def test_paragraphs_and_real_intervening_speech_agree_in_all_readers(hub, fixture):
    app, store, _, _ = hub
    paired = device(store)
    items, expected = pause_fixtures()[fixture]
    recording, blob = final_recording(store, paired["device_id"], items)
    async with running_client(app) as http:
        page = await all_turns(http, paired, recording)
        rendered = await seen_by_deimos(http, paired, recording)
    transcript = snapshot(store, paired["device_id"], recording)
    assert [turn["text"] for turn in page] == expected
    assert [turn.text for turn in transcript.turns] == expected
    assert rendered == turns.render(transcript.lines())
    if fixture == "paused_monologue":
        # The original global-consecutive rule, including its minute window,
        # is the compatibility reference when only one voice speaks.
        legacy = legacy_turns(transcript.segments)
        assert [(t.at_ms, t.text) for t in transcript.turns] == [(t.at_ms, t.text) for t in legacy]
        assert measurements(transcript.turns, transcript.audio_ms) == measurements(
            legacy, transcript.audio_ms
        )
    with store.read_snapshot() as db:
        assert db.execute("SELECT body FROM recording_final_transcripts").fetchone()[0] == blob


def fixture_measurements():
    """The four numbers for every synthetic input, with no transcript in output."""
    report = {}
    for name, (items, _) in pause_fixtures().items():
        segments = [
            turns.Segment(w["start_ms"], w["track"], w["voice"], w["text"], 0, 0, "es", w["end_ms"])
            for w in items
        ]
        report[name] = measurements(turns.merge_segments(segments), max(w["end_ms"] for w in items))
    return report
