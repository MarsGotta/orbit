"""Synthetic meetings only; real routes/store/readers, fake providers."""

import json
from uuid import uuid4

import httpx
from conftest import device
from support_app import client
from support_notes import write_note
from support_orbit_mcp import DEFAULT_DEVICE, text_of
from test_deimos_meeting_reads import deimos
from test_meeting_views import meeting

TEXT = "fondos, FONDOS y prefondos; fondos DLT."
TEXTS = {"mic_local": [TEXT, ""], "remote_app": ["fondos", ""]}


def order(http, headers, **payload):
    response = http.post("/v1/meeting-dictionaries/orders", headers=headers, json=payload)
    assert response.status_code == 200, response.text
    return response.json()


def create(http, headers, name="Proyecto"):
    identity = str(uuid4())
    order(http, headers, action="save", dictionary_id=identity, name=name)
    return identity


def select(http, headers, recording, dictionary, always=False):
    return order(
        http,
        headers,
        action="select",
        recording_id=recording,
        dictionary_id=dictionary,
        use_for_series=always,
    )


def correct(http, headers, recording, dictionary, wrong="fondos", right="LedgerBox"):
    return order(
        http,
        headers,
        action="correct",
        recording_id=recording,
        dictionary_id=dictionary,
        wrong=wrong,
        correct=right,
    )


def read(http, headers, recording):
    return [
        t["text"]
        for t in http.get(f"/v1/recordings/{recording}/turns", headers=headers).json()["turns"]
    ]


def test_corrections_are_shared_reversible_whole_words_and_do_not_rewrite_audio_text(environment):
    _, store, _ = environment
    owner = device(store)
    recordings = [meeting(store, owner["device_id"], texts=TEXTS) for _ in range(3)]
    literal = meeting(
        store,
        owner["device_id"],
        texts={"mic_local": ["a+b; aaab", ""], "remote_app": ["", ""]},
    )
    headers = {"Authorization": "Bearer " + owner["token"]}
    with client(environment) as http:
        first, other = create(http, headers), create(http, headers, "Otro")
        for recording, dictionary in zip(recordings, [first, first, other], strict=True):
            select(http, headers, recording, dictionary)
        saved = correct(http, headers, recordings[0], first)
        expected = ["LedgerBox, LedgerBox y prefondos; LedgerBox DLT.", "LedgerBox"]
        assert read(http, headers, recordings[0]) == expected
        assert read(http, headers, recordings[1]) == expected
        assert read(http, headers, recordings[2]) == [TEXT, "fondos"]
        # Longest phrase wins in one pass; replacement output is never input again.
        correct(http, headers, recordings[0], first, "fondos DLT", "NAV")
        assert read(http, headers, recordings[1])[0].endswith("; NAV.")
        entry = saved["terms"][0]["id"]
        order(http, headers, action="remove", dictionary_id=first, entry_id=entry)
        assert read(http, headers, recordings[0])[0] == "fondos, FONDOS y prefondos; NAV."
        with store.read_snapshot() as db:
            rows = db.execute("SELECT device_id,group_id,body FROM group_transcripts").fetchall()
            raw = [
                store.decrypt(r["body"], f"group-transcript:{r['device_id']}:{r['group_id']}")
                for r in rows
            ]
        assert any(TEXT.encode() in value for value in raw)
        order(
            http,
            headers,
            action="remove",
            dictionary_id=first,
            entry_id=http.get("/v1/meeting-dictionaries", headers=headers).json()["dictionaries"][
                0
            ]["terms"][0]["id"],
        )
        assert read(http, headers, recordings[0]) == [TEXT, "fondos"]
        correct(http, headers, literal, other, "a+b", "NAV")
        assert read(http, headers, literal) == ["NAV; aaab"]


def test_interleaved_corrections_preserve_both_entries(environment, monkeypatch):
    from contextlib import contextmanager

    from orbit_hub.meetings.dictionaries import Correct, execute

    _, store, _ = environment
    owner = device(store)
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    headers = {"Authorization": "Bearer " + owner["token"]}
    with client(environment) as http:
        dictionary = create(http, headers)
    pending = Correct(
        action="correct",
        recording_id=recording,
        dictionary_id=dictionary,
        wrong="DLT",
        correct="NAV",
    )
    transaction = store.transaction

    @contextmanager
    def interleaved_transaction():
        nonlocal pending
        # Complete the other order just before this writer can begin. A read
        # taken outside that transaction must not overwrite its correction.
        if pending is not None:
            next_order, pending = pending, None
            execute(store, owner["device_id"], next_order)
        with transaction() as db:
            yield db

    monkeypatch.setattr(store, "transaction", interleaved_transaction)
    execute(
        store,
        owner["device_id"],
        Correct(
            action="correct",
            recording_id=recording,
            dictionary_id=dictionary,
            wrong="fondos",
            correct="LedgerBox",
        ),
    )
    with client(environment) as http:
        saved = http.get("/v1/meeting-dictionaries", headers=headers).json()["dictionaries"][0]
        assert {e["correct"] for e in saved["terms"]} == {"NAV", "LedgerBox"}
        assert read(http, headers, recording) == [
            "LedgerBox, LedgerBox y prefondos; LedgerBox NAV.",
            "LedgerBox",
        ]


def test_two_series_share_dictionary_override_and_inheritance(environment):
    _, store, _ = environment
    owner = device(store)
    recordings = [
        meeting(
            store,
            owner["device_id"],
            texts=TEXTS,
            event={"calendar_item_id": series, "title": title},
        )
        for series, title in [("daily", "Daily"), ("refine", "Refinamiento"), ("daily", "Daily")]
    ]
    without_series = meeting(store, owner["device_id"], texts=TEXTS)
    headers = {"Authorization": "Bearer " + owner["token"]}
    with client(environment) as http:
        shared, other = create(http, headers), create(http, headers, "Otro")
        for recording in recordings[:2]:
            select(http, headers, recording, shared, True)
        correct(http, headers, recordings[0], shared)
        corrected_document = http.get(f"/v1/recordings/{recordings[0]}", headers=headers).json()
        assert corrected_document["dictionary"]["source"] == "series"
        assert "LedgerBox" in read(http, headers, recordings[2])[0]
        listing = http.get("/v1/meeting-dictionaries", headers=headers).json()["dictionaries"]
        assert {s["calendar_item_id"] for s in listing[0]["series"]} == {"daily", "refine"}
        order(http, headers, action="save", dictionary_id=shared, name="Proyecto renombrado")
        assert (
            http.get("/v1/meeting-dictionaries", headers=headers).json()["dictionaries"][0]["name"]
            == "Proyecto renombrado"
        )
        select(http, headers, recordings[2], other)
        assert read(http, headers, recordings[2])[0] == TEXT
        select(http, headers, recordings[2], None)
        assert "LedgerBox" in read(http, headers, recordings[2])[0]
        document = http.get(f"/v1/recordings/{recordings[2]}", headers=headers).json()
        assert document["dictionary"]["dictionary_id"] == shared
        assert document["dictionary"]["source"] == "series"
        select(http, headers, recordings[2], other, True)
        assert read(http, headers, recordings[0]) == [TEXT, "fondos"]
        assert "LedgerBox" in read(http, headers, recordings[1])[0]
        refusal = http.post(
            "/v1/meeting-dictionaries/orders",
            headers=headers,
            json={
                "action": "select",
                "recording_id": without_series,
                "dictionary_id": shared,
                "use_for_series": True,
            },
        )
        assert refusal.status_code == 409
        assert refusal.json()["code"] == "dictionary_series_missing"
        assert "series" not in refusal.json()["refusal"]


async def test_deimos_reads_the_correction_through_orbit_mcp(environment, tmp_path):
    _, store, _ = environment
    owner = device(store)
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    headers = {"Authorization": "Bearer " + owner["token"]}
    with client(environment) as http:
        dictionary = create(http, headers)
        correct(http, headers, recording, dictionary)
    async with deimos(environment, tmp_path, owner["device_id"]) as server:
        result = await server.call_tool(
            "orbit_transcript", {"device": DEFAULT_DEVICE, "recording_id": recording}
        )
    assert not result.is_error, text_of(result)
    assert "LedgerBox, LedgerBox y prefondos; LedgerBox DLT." in text_of(result)


async def test_note_gets_terms_and_an_existing_note_stays_until_rewritten(environment):
    _, store, _ = environment
    owner = device(store)
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    before = await write_note(store, owner["device_id"], recording)
    old = before.repository.document(owner["device_id"], recording)["note"]
    headers = {"Authorization": "Bearer " + owner["token"]}
    with client(environment) as http:
        dictionary = str(uuid4())
        order(
            http,
            headers,
            action="correct",
            recording_id=recording,
            dictionary_id=dictionary,
            wrong="fondos",
            correct="LedgerBox",
            name="Primer diccionario",
        )
    assert before.repository.document(owner["device_id"], recording)["note"] == old
    after = await write_note(store, owner["device_id"], recording)
    prompt = json.dumps(after.client.calls[0]["messages"], ensure_ascii=False)
    assert "Términos del diccionario" in prompt and "LedgerBox" in prompt


def test_bounds_ownership_encryption_and_no_terms_in_logs(environment, caplog):
    _, store, _ = environment
    owner, stranger = device(store), device(store)
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    headers = {"Authorization": "Bearer " + owner["token"]}
    with client(environment) as http:
        dictionary = create(http, headers, "Nombre privado QZX94")
        correct(http, headers, recording, dictionary, "forma privada", "Término privado QZX94")
        from orbit_hub.meetings.dictionaries import MAX_TERM_CHARS, MAX_TERMS

        response = http.post(
            "/v1/meeting-dictionaries/orders",
            headers=headers,
            json={
                "action": "correct",
                "recording_id": recording,
                "dictionary_id": dictionary,
                "wrong": "x",
                "correct": "x" * (MAX_TERM_CHARS + 1),
            },
        )
        assert response.status_code == 413
        assert response.json()["refusal"]["characters"] == MAX_TERM_CHARS + 1
        assert response.json()["refusal"]["max_characters"] == MAX_TERM_CHARS
        for i in range(MAX_TERMS - 1):
            correct(http, headers, recording, dictionary, f"wrong{i}", f"right{i}")
        response = http.post(
            "/v1/meeting-dictionaries/orders",
            headers=headers,
            json={
                "action": "correct",
                "recording_id": recording,
                "dictionary_id": dictionary,
                "wrong": "new",
                "correct": "extra",
            },
        )
        assert response.status_code == 409
        assert response.json()["refusal"]["terms"] == MAX_TERMS
        assert response.json()["refusal"]["max_terms"] == MAX_TERMS
        # Another wrong spelling is still accepted when the term list is full.
        correct(http, headers, recording, dictionary, "otra forma", "Término privado QZX94")
        foreign = {"Authorization": "Bearer " + stranger["token"]}
        assert http.get("/v1/meeting-dictionaries", headers=foreign).json()["dictionaries"] == []
        response = http.post(
            "/v1/meeting-dictionaries/orders",
            headers=foreign,
            json={"action": "save", "dictionary_id": dictionary, "name": "Otra"},
        )
        assert response.status_code == 200  # Same id belongs to a different device.
        with store.read_snapshot() as db:
            blobs = [bytes(r[0]) for r in db.execute("SELECT body FROM meeting_dictionaries")]
        assert all(b"QZX94" not in blob and b"wrong" not in blob for blob in blobs)
    assert "Término privado QZX94" not in caplog.text
    assert "Nombre privado QZX94" not in caplog.text


async def test_draft_and_every_final_window_send_the_recent_terms_and_retranscription_keeps_them(
    environment, tmp_path
):
    import asyncio
    from email.parser import BytesParser

    from support_waits import wait_for
    from test_final_pass import fake_separator, final_worker

    from orbit_hub.meeting_final_pass import FinalPassRepository
    from orbit_hub.meeting_transcription import MeetingTranscriber
    from orbit_hub.transcription import TranscriptionWorker

    _, store, adapter = environment
    owner = device(store)
    # Empty drafts avoid making the final-pass integrity comparison depend on
    # a fake Whisper's vocabulary. This test checks the actual multipart wire.
    event = {"calendar_item_id": "daily", "title": "Daily"}
    previous = meeting(
        store,
        owner["device_id"],
        texts={"mic_local": ["", ""], "remote_app": ["", ""]},
        event=event,
    )
    recording = meeting(
        store,
        owner["device_id"],
        texts={"mic_local": ["", ""], "remote_app": ["", ""]},
        finalize=False,
    )
    headers = {"Authorization": "Bearer " + owner["token"]}
    with client(environment) as http:
        dictionary = create(http, headers)
        select(http, headers, previous, dictionary, True)
        for wrong, right in [
            ("a", "Antiguo" * 15),
            ("d", "Ajuste" * 7),
            ("b", "Reciente" * 15),
            ("c", "NAV"),
        ]:
            correct(http, headers, previous, dictionary, wrong, right)
        correct(http, headers, previous, dictionary, "otra a", "Antiguo" * 15)
    from orbit_hub.meetings.dictionaries import MAX_PROMPT_CHARS

    expected = "Glosario: " + "Antiguo" * 15 + ", NAV."
    assert len(expected) <= MAX_PROMPT_CHARS
    # The older 42-character term would fit only if the prefix and final
    # period were wrongly excluded from the budget (154 versus 165).
    assert len(expected) + 2 + len("Ajuste" * 7) == 165
    requests = []

    async def respond(request):
        mime = BytesParser().parsebytes(
            f"Content-Type: {request.headers['content-type']}\r\nMIME-Version: 1.0\r\n\r\n".encode()
            + request.content
        )
        fields = {
            p.get_param("name", header="content-disposition"): p.get_payload(decode=True)
            for p in mime.get_payload()
        }
        requests.append(fields)
        return httpx.Response(200, json={"text": "", "language": "es", "words": []})

    transcriber = MeetingTranscriber(
        {"endpoint": "http://localhost:0", "model": "fake"},
        client_factory=lambda: httpx.AsyncClient(transport=httpx.MockTransport(respond)),
    )

    async def transcribe_drafts(expected_prompt):
        requests.clear()
        with store.transaction() as db:
            total = db.execute(
                "SELECT count(*) FROM transcription_groups WHERE recording=?", (recording,)
            ).fetchone()[0]
            db.execute(
                "UPDATE transcription_groups SET state='pending',next_attempt=0 WHERE recording=?",
                (recording,),
            )
        done = asyncio.Event()
        completed = []

        def changed(*_):
            completed.append(True)
            if len(completed) == total:
                done.set()

        worker = TranscriptionWorker(
            store, adapter, lambda: 1001, meetings=transcriber, recording_changed=changed
        )
        worker.start()
        try:
            await wait_for(done.wait(), "all draft groups transcribed")
        finally:
            await worker.close()
        assert len(requests) == total
        for request in requests:
            assert request.get("prompt") == expected_prompt

    await transcribe_drafts(None)
    from test_meeting_views import STARTED

    store.recordings.finalize(
        owner["device_id"],
        recording,
        {
            "tracks": [
                {"track": track, "chunk_count": 3, "last_sequence": 2, "total_frames": 480_000}
                for track in ("mic_local", "remote_app")
            ],
            "gaps": 0,
            "started_at_ms": STARTED,
            "finalized_at_ms": STARTED + 30_000,
            "event": event,
        },
    )
    await transcribe_drafts(expected.encode())
    requests.clear()
    repo = FinalPassRepository(store)
    final = final_worker(store, transcriber, fake_separator(tmp_path))
    for _ in range(2):
        repo.request(owner["device_id"], recording)
        assert await final.run_once()
        with store.read_snapshot() as db:
            assert (
                db.execute("SELECT state FROM recording_final_passes").fetchone()[0] == "completed"
            )
    assert len(requests) == 4  # Two tracks, twice, through the final pass.
    assert all(r["prompt"].decode() == expected for r in requests)
    assert all(r["timestamp_granularities[]"] == b"word" for r in requests)


async def test_final_word_fragments_are_corrected_as_a_phrase_in_app_and_deimos_views(
    environment, tmp_path
):
    from orbit_hub.meeting_final_pass import FinalPassRepository

    _, store, _ = environment
    owner = device(store)
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    headers = {"Authorization": "Bearer " + owner["token"]}
    with client(environment) as http:
        dictionary = create(http, headers)
        correct(http, headers, recording, dictionary, "fondos DLT", "LedgerBox")
    repository = FinalPassRepository(store)
    repository.request(owner["device_id"], recording)
    job = repository.claim()
    assert job is not None
    repository.complete(
        job,
        {
            "spans": [],
            "words": [
                {
                    "start_ms": 0,
                    "end_ms": 100,
                    "track": "mic_local",
                    "voice": "mic_local",
                    "text": "fondos",
                },
                {
                    "start_ms": 100,
                    "end_ms": 200,
                    "track": "mic_local",
                    "voice": "mic_local",
                    "text": " DLT",
                },
            ],
            "audio_ms": 30_000,
            "language": "es",
        },
    )
    with client(environment) as http:
        assert read(http, headers, recording) == ["LedgerBox"]
    async with deimos(environment, tmp_path, owner["device_id"]) as server:
        for format in ("text", "segments"):
            result = await server.call_tool(
                "orbit_transcript",
                {"device": DEFAULT_DEVICE, "recording_id": recording, "format": format},
            )
            assert not result.is_error, text_of(result)
            assert "LedgerBox" in text_of(result)
            assert "fondos" not in text_of(result)


def test_one_dictionary_is_refused_before_it_outgrows_a_library_page(environment):
    _, store, _ = environment
    owner = device(store)
    recording = meeting(store, owner["device_id"], texts=TEXTS)
    headers = {"Authorization": "Bearer " + owner["token"]}
    with client(environment) as http:
        dictionary = create(http, headers)
        from orbit_hub.meetings.dictionaries import MAX_DICTIONARY_BYTES

        # Four-byte terms and forms exercise the serialized byte bound, not
        # the independent term count or character bounds.
        refused = None
        for i in range(64):
            for j in range(8):
                response = http.post(
                    "/v1/meeting-dictionaries/orders",
                    headers=headers,
                    json={
                        "action": "correct",
                        "recording_id": recording,
                        "dictionary_id": dictionary,
                        "wrong": f"{i}-{j}" + "🪐" * 120,
                        "correct": f"{i}" + "🌒" * 120,
                    },
                )
                if response.status_code != 200:
                    refused = response
                    break
            if refused is not None:
                break
        assert refused is not None
        assert refused.status_code == 413, refused.text
        assert refused.json()["code"] == "dictionary_too_large"
        numbers = refused.json()["refusal"]
        assert numbers["bytes"] > MAX_DICTIONARY_BYTES
        assert numbers["max_bytes"] == MAX_DICTIONARY_BYTES
        saved = http.get("/v1/meeting-dictionaries", headers=headers).json()["dictionaries"][0]
        assert (
            len(json.dumps(saved, ensure_ascii=False, separators=(",", ":")).encode())
            <= MAX_DICTIONARY_BYTES
        )
