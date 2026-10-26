"""Read-time turn regressions using invented words and an encrypted fixture store."""

# ruff: noqa: F811 - imported pytest fixture

import json

import pytest
from conftest import device
from support_turn_readability import compare_recording
from test_meeting_views import (
    NOW,
    all_turns,
    bearer,
    hub,  # noqa: F401 - pytest fixture
    meeting,
    running_client,
    seen_by_deimos,
    signed,
)

from orbit_hub.meetings import repository, turns
from orbit_hub.meetings_bridge import TRANSCRIPT_PATH


def words(text, track, voice, start=0, step=500, duration=400):
    return [
        {
            "start_ms": start + i * step,
            "end_ms": start + i * step + duration,
            "text": word,
            "track": track,
            "voice": voice,
        }
        for i, word in enumerate(text.split())
    ]


def final_recording(store, owner, items):
    identity = meeting(store, owner, texts={"mic_local": ["", ""], "remote_app": ["", ""]})
    value = {"words": items, "audio_ms": max(w["end_ms"] for w in items), "language": "es"}
    context = f"recording-final-transcript:{owner}:{identity}"
    blob = store.encrypt(json.dumps(value).encode(), context)
    with store.transaction() as db:
        db.execute(
            "INSERT INTO recording_final_transcripts VALUES (?,?,?)", (owner, identity, blob)
        )
    return identity, blob


def snapshot(store, owner, recording):
    with store.read_snapshot() as db:
        return turns.read(store, db, owner, recording)


@pytest.mark.parametrize("jitter", [0, -200, 200, 350])
async def test_overlapping_sentences_stay_whole_in_pages_reader_and_deimos(hub, jitter):
    app, store, _, _ = hub
    paired = device(store)
    local, remote = "Hoy revisamos los planos.", "We can send them tomorrow."
    items = words(local, "mic_local", "mic_local") + words(
        remote, "remote_app", "voice_1", start=250 + jitter
    )
    if jitter:
        for i, word in enumerate(items):
            shift = (0, 200, -100, 300)[i % 4]
            word["start_ms"] += shift
            word["end_ms"] += shift
    recording, blob = final_recording(store, paired["device_id"], items)
    async with running_client(app) as http:
        page = await all_turns(http, paired, recording)
        rendered = await seen_by_deimos(http, paired, recording)
        body, headers = signed(
            TRANSCRIPT_PATH,
            {"device_id": paired["device_id"], "recording_id": recording, "format": "segments"},
            issued=int(NOW * 1000),
        )
        response = await http.post(TRANSCRIPT_PATH, content=body, headers=headers)
        assert response.status_code == 200, response.text
    assert [t["text"] for t in page] == [local, remote]
    transcript = snapshot(store, paired["device_id"], recording)
    assert [t.text for t in transcript.turns] == [local, remote]
    assert rendered == turns.render(transcript.lines())
    assert len(rendered.splitlines()) == 2
    assert [json.loads(line)["text"] for line in response.json()["text"].splitlines()] == [
        local,
        remote,
    ]
    with store.read_snapshot() as db:
        assert db.execute("SELECT body FROM recording_final_transcripts").fetchone()[0] == blob


async def test_short_interjection_does_not_split_an_utterance(hub):
    app, store, _, _ = hub
    paired = device(store)
    sentence = "El jardín tendrá flores durante toda la primavera."
    recording, _ = final_recording(
        store,
        paired["device_id"],
        words(sentence, "remote_app", "voice_1") + words("sí", "mic_local", "mic_local", 1250),
    )
    async with running_client(app) as http:
        assert [t["text"] for t in await all_turns(http, paired, recording)] == [sentence, "sí"]


async def test_a_long_pause_taken_by_another_voice_starts_a_turn(hub):
    app, store, _, _ = hub
    paired = device(store)
    recording, _ = final_recording(
        store,
        paired["device_id"],
        words("Primero vemos planos.", "remote_app", "voice_1")
        + words("Después elegimos flores.", "remote_app", "voice_1", 4000)
        + words("Sí, claro.", "mic_local", "mic_local", 2000),
    )
    async with running_client(app) as http:
        assert [t["text"] for t in await all_turns(http, paired, recording)] == [
            "Primero vemos planos.",
            "Sí, claro.",
            "Después elegimos flores.",
        ]


async def test_one_speaker_monologue_keeps_the_minute_window(hub):
    app, store, _, _ = hub
    paired = device(store)
    items = words(" ".join(f"word{i}" for i in range(130)), "mic_local", "mic_local")
    recording, _ = final_recording(store, paired["device_id"], items)
    async with running_client(app) as http:
        page = await all_turns(http, paired, recording)
    assert [t["at_ms"] for t in page] == [0, 60_000]
    assert [t["text"] for t in page] == [
        " ".join(w["text"] for w in items[:120]),
        " ".join(w["text"] for w in items[120:]),
    ]


async def test_stored_legacy_names_turn_interval_merge_and_correction_keep_their_words(hub):
    app, store, _, _ = hub
    paired = device(store)
    owner = paired["device_id"]
    items = words("Traigo mapas nuevos hoy.", "remote_app", "voice_1")
    items += words("Uno dos tres cuatro.", "mic_local", "mic_local", 250)
    items += words("Más mapas mañana.", "remote_app", "voice_2", 6000)
    recording, blob = final_recording(store, owner, items)
    # Exact persisted shape from the old reader: one formerly fragmented word
    # was named, whole voices were renamed/merged. No turn ordinal is stored.
    changes = {
        "names": {"voice_1": "Elena", "turn_remote_app_500_900": "Pablo"},
        "same_as": {"voice_2": "voice_1"},
        "turns": [
            {
                "track": "remote_app",
                "start_ms": 500,
                "end_ms": 900,
                "speaker_key": "turn_remote_app_500_900",
            }
        ],
    }
    context = f"recording-speakers:{owner}:{recording}"
    sealed = store.encrypt(json.dumps(changes).encode(), context)
    with store.transaction() as db:
        db.execute(
            "INSERT INTO recording_speakers VALUES (?,?,?,?)", (owner, recording, sealed, NOW)
        )
    # The correction API stores heard-form substitutions in dictionaries, not
    # turn indices. Save it and open a fresh reader to prove persistence.
    async with running_client(app) as http:
        dictionary = "00000000-0000-4000-8000-000000000123"
        response = await http.post(
            "/v1/meeting-dictionaries/orders",
            headers=bearer(paired),
            json={
                "action": "correct",
                "recording_id": recording,
                "dictionary_id": dictionary,
                "name": "Jardín",
                "wrong": "mapas",
                "correct": "planos",
            },
        )
        assert response.status_code == 200, response.text
        page = await all_turns(http, paired, recording)
        rendered = await seen_by_deimos(http, paired, recording)
    transcript = snapshot(store, owner, recording)
    remote = [s for s in transcript.segments if s.track == "remote_app"]
    assert [(s.text, transcript.name(s.speaker_key)) for s in remote] == [
        ("Traigo", "Elena"),
        ("mapas", "Pablo"),
        ("nuevos", "Elena"),
        ("hoy.", "Elena"),
        ("Más", "Elena"),
        ("mapas", "Elena"),
        ("mañana.", "Elena"),
    ]
    assert [t["text"] for t in page] == [
        "Traigo",
        "Uno dos tres cuatro.",
        "planos",
        "nuevos hoy.",
        "Más planos mañana.",
    ]
    assert rendered == turns.render(transcript.lines())
    with store.read_snapshot() as db:
        assert repository.speaker_changes(store, db, owner, recording) == changes
        assert db.execute("SELECT body FROM recording_speakers").fetchone()[0] == sealed
        assert db.execute("SELECT body FROM recording_final_transcripts").fetchone()[0] == blob


async def test_live_draft_joins_groups_despite_other_track(hub):
    app, store, _, _ = hub
    paired = device(store)
    recording = meeting(
        store,
        paired["device_id"],
        texts={
            "mic_local": ["Primera parte.", "Segunda parte."],
            "remote_app": ["Sí.", "De acuerdo."],
        },
        finalize=False,
    )
    # Close the pending last group by the ordinary draft deadline.
    while job := store.groups.claim(1031):
        store.groups.finish(
            job, "Segunda parte." if job["manifest"]["track"] == "mic_local" else "De acuerdo."
        )
    async with running_client(app) as http:
        page = await all_turns(http, paired, recording)
    assert [t["text"] for t in page] == ["Primera parte. Segunda parte.", "Sí. De acuerdo."]


def test_comparison_returns_only_numbers(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    recording, _ = final_recording(
        store,
        owner,
        words("Uno dos tres cuatro.", "mic_local", "mic_local")
        + words("Five six seven eight.", "remote_app", "voice_1", 250),
    )
    result = compare_recording(store, owner, recording)
    assert result["before"]["turn_count"] == 8
    assert result["after"] == {
        "turn_count": 2,
        "short_turn_count": 0,
        "median_words_per_turn": 4,
        "speaker_changes_per_minute": 60_000 / 2150,
    }
    assert all(
        isinstance(number, (int, float)) for values in result.values() for number in values.values()
    )


async def test_legacy_flat_names_and_new_turn_naming_use_word_intervals(hub):
    app, store, _, _ = hub
    paired = device(store)
    owner = paired["device_id"]
    sentence = "El vivero abre mañana."
    items = words(sentence, "remote_app", "voice_1")
    items += words("We will visit soon.", "mic_local", "mic_local", 250)
    items += words("Llevaré semillas nuevas.", "remote_app", "voice_2", 6000)
    recording, blob = final_recording(store, owner, items)
    context = f"recording-speakers:{owner}:{recording}"
    flat_names = store.encrypt(json.dumps({"voice_1": "Elena"}).encode(), context)
    with store.transaction() as db:
        db.execute(
            "INSERT INTO recording_speakers VALUES (?,?,?,?)", (owner, recording, flat_names, NOW)
        )
    async with running_client(app) as http:
        page = await all_turns(http, paired, recording)
        assert page[0]["text"] == sentence
        with store.read_snapshot() as db:
            assert db.execute("SELECT body FROM recording_speakers").fetchone()[0] == flat_names
        path = f"/v1/recordings/{recording}/speakers"
        renamed = await http.put(
            path,
            headers=bearer(paired),
            json={"names": {"voice_1": "Pablo"}, "turn": {"at_ms": 0, "speaker_key": "voice_1"}},
        )
        assert renamed.status_code == 200, renamed.text
        merged = await http.put(path, headers=bearer(paired), json={"names": {"voice_2": "Elena"}})
        assert merged.status_code == 200, merged.text
        page = await all_turns(http, paired, recording)
        rendered = await seen_by_deimos(http, paired, recording)
    transcript = snapshot(store, owner, recording)
    assert [(transcript.name(t.speaker_key), t.text) for t in transcript.turns] == [
        ("Pablo", sentence),
        ("Micrófono local", "We will visit soon."),
        ("Elena", "Llevaré semillas nuevas."),
    ]
    assert rendered == turns.render(transcript.lines())
    with store.read_snapshot() as db:
        changes = repository.speaker_changes(store, db, owner, recording)
        assert changes["same_as"] == {"voice_2": "voice_1"}
        assert changes["turns"] == [
            {
                "track": "remote_app",
                "start_ms": 0,
                "end_ms": 1900,
                "speaker_key": "turn_remote_app_0_1900",
            }
        ]
        assert db.execute("SELECT body FROM recording_final_transcripts").fetchone()[0] == blob


@pytest.mark.parametrize("pause, expected", [(1000, ["Uno dos"]), (1001, ["Uno", "dos"])])
async def test_pause_threshold_is_between_word_end_and_next_start(hub, pause, expected):
    app, store, _, _ = hub
    paired = device(store)
    recording, _ = final_recording(
        store,
        paired["device_id"],
        words("Uno", "mic_local", "mic_local", duration=4000)
        + words("dos", "mic_local", "mic_local", start=4000 + pause)
        + words("Sí", "remote_app", "voice_1", start=4500, duration=100),
    )
    async with running_client(app) as http:
        page = await all_turns(http, paired, recording)
        assert [t["text"] for t in page if t["speaker_key"] == "mic_local"] == expected
        assert [t["text"] for t in page if t["speaker_key"] == "voice_1"] == ["Sí"]
