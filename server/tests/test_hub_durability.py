"""The hub's own durability.

Each test reproduces one way the hub loses, hides or refuses what the person
created, or guards a path that already holds the rule (named `..._holds`).
A full disk is simulated with SQLite's own `PRAGMA max_page_count`, which makes
SQLite fail exactly as it does on ENOSPC: `SQLITE_FULL`, "database or disk is
full". Synthetic data only: invented names, no real service.
"""

import asyncio
import hashlib
import os
import sqlite3
from uuid import uuid4

import pytest
from conftest import device
from cryptography.exceptions import InvalidTag
from support_app import client
from support_grouping import put
from support_transcription import wav_fixture

from orbit_hub.storage import Store, initialize

DISK_FULL = "database or disk is full"


def fill_the_disk(store) -> None:
    """From now on SQLite cannot grow the database: what a full disk does."""
    pages = store._db.execute("PRAGMA page_count").fetchone()[0]
    store._db.execute(f"PRAGMA max_page_count={pages}")


def chunk_headers(token, recording, sequence, body):
    frames = (len(body) - 44) // 2  # mono PCM16 after the 44-byte header
    return {
        "Authorization": "Bearer " + token,
        "X-Orbit-Recording": recording,
        "X-Orbit-Track": "mic_local",
        "X-Orbit-Sequence": str(sequence),
        "X-Orbit-SHA256": hashlib.sha256(body).hexdigest(),
        "X-Orbit-First-Sample": str(sequence * frames),
        "X-Orbit-Frames": str(frames),
        "X-Orbit-Sample-Rate": "16000",
        "X-Orbit-Channels": "1",
        "X-Orbit-Monotonic-Ns": str(1000 + sequence),
        "X-Orbit-Started-At-Ms": "2000",
    }


def big_wav(seconds=4):
    from support_audio import wav_bytes

    return wav_bytes(os.urandom(16000 * 2 * seconds))


# --- What already holds -------------------------------------------------------


def test_a_chunk_is_acknowledged_durable_only_after_a_full_commit_holds(environment):
    """The `durable: true` the Mac deletes its audio on comes after a WAL commit
    with `synchronous=FULL` (the WAL is fsynced on every commit)."""
    _, store, _ = environment
    owner = device(store)
    assert store._db.execute("PRAGMA journal_mode").fetchone()[0] == "wal"
    assert store._db.execute("PRAGMA synchronous").fetchone()[0] == 2  # FULL
    trace: list[str] = []
    store._db.set_trace_callback(trace.append)
    original = store.put_chunk

    def observed(*args, **kwargs):
        outcome = original(*args, **kwargs)
        trace.append("<put_chunk returned>")
        return outcome

    store.put_chunk = observed
    body, recording = wav_fixture(), str(uuid4())
    with client(environment) as http:
        response = http.put(
            f"/v1/chunks/{recording}-mic_local-0",
            content=body,
            headers=chunk_headers(owner["token"], recording, 0, body),
        )
    store._db.set_trace_callback(None)
    assert response.status_code == 200 and response.json()["durable"] is True
    insert = next(i for i, sql in enumerate(trace) if sql.startswith("INSERT INTO chunks"))
    commit = next(i for i, sql in enumerate(trace) if i > insert and sql == "COMMIT")
    assert commit < trace.index("<put_chunk returned>")


# --- Disk full: the cause is never said ---------------------------------------


def test_a_full_disk_is_not_masked_by_a_second_rollback(environment):
    """SQLite rolls the transaction back itself on SQLITE_FULL; `Store.transaction`
    then runs ROLLBACK again, which raises "cannot rollback - no transaction is
    active" and replaces the real cause (storage.py:216-219)."""
    _, store, _ = environment
    owner = device(store)["device_id"]
    fill_the_disk(store)
    body, recording = big_wav(), str(uuid4())
    with pytest.raises(sqlite3.OperationalError) as raised:
        store.put_chunk(
            owner,
            f"{recording}-mic_local-0",
            recording,
            "mic_local",
            0,
            hashlib.sha256(body).hexdigest(),
            body,
            {"frames": 64000},
        )
    assert DISK_FULL in str(raised.value), (
        f"the error the hub raises is {str(raised.value)!r}, not the full disk"
    )


def test_a_chunk_the_full_disk_refused_is_said_with_its_cause(environment):
    """A chunk the hub could not store because its disk is full is answered as
    500 `internal_error`, the same as any bug: neither the Mac nor the log
    (which keeps only `exc_type`) can tell that the disk is full."""
    _, store, _ = environment
    owner = device(store)
    body, recording = big_wav(), str(uuid4())
    with client(environment) as http:
        fill_the_disk(store)  # after the lifespan has created the notes' tables
        response = http.put(
            f"/v1/chunks/{recording}-mic_local-0",
            content=body,
            headers=chunk_headers(owner["token"], recording, 0, body),
        )
    refusal = response.json()
    assert (response.status_code, refusal["code"]) != (500, "internal_error"), refusal
    assert response.status_code == 507, refusal


def test_private_notes_the_full_disk_refused_are_said_with_their_cause(environment):
    """The person's private notes on a meeting, refused by a full disk, come back
    as 500 `internal_error` without the cause."""
    _, store, _ = environment
    owner = device(store)
    recording = str(uuid4())
    put(store, owner["device_id"], recording, "mic_local", 0)
    with client(environment) as http:
        fill_the_disk(store)  # after the lifespan has created the notes' tables
        response = http.put(
            f"/v1/recordings/{recording}/private-notes",
            json={"text": "Notas privadas de Alina Ferrer. " * 600, "base_revision": 0},
            headers={"Authorization": "Bearer " + owner["token"]},
        )
    refusal = response.json()
    assert (response.status_code, refusal["code"]) != (500, "internal_error"), refusal


# --- A refused chat message burns its id --------------------------------------


def chat_frame(operation, recording=None, text="¿Qué se decidió sobre el presupuesto?"):
    params = {"text": text, "client_id": operation}
    if recording is not None:
        params["meeting"] = {"recording_id": recording, "minutes": 5}
    return {"v": 1, "id": operation, "method": "chat.send", "params": params}


def answer_to(ws, operation):
    while True:
        frame = ws.receive_json()
        if frame.get("id") == operation and frame["event"] in {"chat.session", "error"}:
            return frame


def test_a_chat_message_refused_before_it_was_stored_can_be_sent_again(environment):
    """A question about a meeting whose audio has not reached the hub yet is
    refused (`meeting_not_found`) and nothing is stored, but ws.py:297 already
    reserved its id: when the Mac sends the same message again once the audio
    is there, the hub answers `outcome_unknown` ("no se repetirá") for a message
    it never kept."""
    _, store, _ = environment
    owner = device(store)
    operation, recording = str(uuid4()), str(uuid4())
    headers = {"Authorization": "Bearer " + owner["token"]}
    with (
        client(environment) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        ws.send_json(chat_frame(operation, recording))
        first = answer_to(ws, operation)
        assert first["data"]["code"] == "meeting_not_found"
        put(store, owner["device_id"], recording, "mic_local", 0)
        ws.send_json(chat_frame(operation, recording))
        again = answer_to(ws, operation)
    assert again["event"] == "chat.session", again["data"]


def test_a_chat_message_the_full_disk_refused_is_said_with_its_cause(environment, monkeypatch):
    """The person's message could not be stored (disk full): the Mac is told
    `internal_error` "No se pudo completar la operación; no se repetirá"."""
    _, store, _ = environment
    owner = device(store)
    original = store.append_message
    full = [1]

    def append_message(*args, **kwargs):
        if full[0]:
            full[0] -= 1
            raise sqlite3.OperationalError(DISK_FULL)
        return original(*args, **kwargs)

    monkeypatch.setattr(store, "append_message", append_message)
    operation = str(uuid4())
    headers = {"Authorization": "Bearer " + owner["token"]}
    with (
        client(environment) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        ws.send_json(chat_frame(operation, text="Mensaje de Bruno Saavedra"))
        first = answer_to(ws, operation)
    assert first["event"] == "error"
    assert first["data"]["code"] != "internal_error", first["data"]


def test_a_chat_message_the_full_disk_refused_can_be_sent_again(environment, monkeypatch):
    """Once the disk has room, the same message (same id) is refused for good as
    `outcome_unknown` although the hub never stored it."""
    _, store, _ = environment
    owner = device(store)
    original = store.append_message
    full = [1]

    def append_message(*args, **kwargs):
        if full[0]:
            full[0] -= 1
            raise sqlite3.OperationalError(DISK_FULL)
        return original(*args, **kwargs)

    monkeypatch.setattr(store, "append_message", append_message)
    operation = str(uuid4())
    headers = {"Authorization": "Bearer " + owner["token"]}
    with (
        client(environment) as http,
        http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws,
    ):
        assert ws.receive_json()["event"] == "ready"
        ws.send_json(chat_frame(operation, text="Mensaje de Bruno Saavedra"))
        assert answer_to(ws, operation)["event"] == "error"
        ws.send_json(chat_frame(operation, text="Mensaje de Bruno Saavedra"))
        again = answer_to(ws, operation)
    assert again["event"] == "chat.session", again["data"]


# --- The storage key ----------------------------------------------------------


def stored_conversation(data, key):
    """A store with one device, one conversation and one message; closed."""
    store = Store(data, key)
    owner = store.pair(store.create_code(1000), "Mac de Alina", 1001)
    session = store.create_session(owner["device_id"], "default", "rt", "st", "ep", 1002)
    store.append_message(session, "user", "Recordar el acta del comité", now_ms=1_002_000)
    store.close()
    return owner, session


def test_init_refuses_a_data_dir_that_already_holds_a_store(tmp_path):
    """`orbit-hub init` only refuses an existing key file (O_EXCL). With the key
    missing (misplaced, not restored with the data), init writes a new key next
    to a database encrypted with the old one, and the hub starts on it."""
    data, key = tmp_path / "data", tmp_path / "key"
    initialize(data, key)
    stored_conversation(data, key)
    key.unlink()
    with pytest.raises((FileExistsError, ValueError)):
        initialize(data, key)


def test_a_store_opened_with_a_key_that_is_not_its_own_refuses_to_start(tmp_path):
    """`Store` never checks that the key is the one the database was written
    with. With another key it starts, authenticates the Mac (token hashes are
    not encrypted), acknowledges new chunks as durable with the new key, and
    every older meeting, note and message fails later as `InvalidTag`
    (`internal_error` to the Mac). Putting the right key back then makes the
    newly acknowledged chunks unreadable instead."""
    data, key, wrong = tmp_path / "data", tmp_path / "key", tmp_path / "wrong"
    initialize(data, key)
    owner, session = stored_conversation(data, key)
    wrong.write_bytes(bytes(range(32)))
    wrong.chmod(0o600)
    opened = None
    facts = {}
    try:
        opened = Store(data, wrong)
    except ValueError:
        pass
    else:
        facts["authenticated"] = opened.authenticate(owner["token"]) == owner["device_id"]
        body, recording = wav_fixture(), str(uuid4())
        facts["new_chunk"] = opened.put_chunk(
            owner["device_id"],
            f"{recording}-mic_local-0",
            recording,
            "mic_local",
            0,
            hashlib.sha256(body).hexdigest(),
            body,
            {"frames": 160},
        )
        try:
            opened.session_messages(owner["device_id"], session, None, 10)
        except InvalidTag:
            facts["old_messages"] = "InvalidTag"
        opened.close()
    assert opened is None, f"the hub started with a key that is not its own: {facts}"


# --- Note corrections set aside twice ----------------------------------------


async def test_corrections_set_aside_by_a_regeneration_survive_the_next_one(environment):
    """Regenerating a note sets the person's text corrections aside
    (`superseded`) so they can be restored. A second regeneration overwrites
    them with the newer ones (notes/repository.py:148-151,
    `superseded=coalesce(?, superseded)`): the first corrections are gone and
    nothing says so."""
    from concurrent.futures import ThreadPoolExecutor

    from support_notes import FakeModel
    from test_notes import recording as meeting
    from test_notes import step

    from orbit_hub.notes import NoteRepository, NotesWorker
    from orbit_hub.notes.models import EditRequest

    _, store, _ = environment
    owner = device(store)["device_id"]
    executor = ThreadPoolExecutor(max_workers=1)
    clock = {"now": 2000.0}
    worker = NotesWorker(
        NoteRepository(store),
        lambda: clock["now"],
        signal=asyncio.Event(),
        store_executor=executor,
        client=FakeModel(),
    )
    worker.repository.initialize()
    try:
        identity = meeting(store, owner)
        notes = worker.repository
        notes.request(owner, identity, 2000.0, True)
        await step(worker, clock)
        revision = notes.document(owner, identity)["revision"]
        notes.edit(
            owner, identity, EditRequest(base_revision=revision, title="Comité de Dalia"), 2000.0
        )
        notes.request(owner, identity, 2000.0, True, template_id="project")
        await step(worker, clock)
        revision = notes.document(owner, identity)["revision"]
        notes.edit(
            owner, identity, EditRequest(base_revision=revision, title="Otro título"), 2000.0
        )
        document = notes.request(owner, identity, 2000.0, True)
    finally:
        executor.shutdown()
    kept = repr((document["edits"], document["superseded_edits"]))
    assert "Comité de Dalia" in kept, f"the first corrections are gone: {kept}"


# --- The quota refusal says "never retry" -------------------------------------


def test_a_chunk_the_quota_refused_is_not_marked_never_to_retry(environment):
    """The quota refusal says "conserva la copia local" with its numbers, but
    its disposition is `retry: never` (errors.py:153-162: neither
    `quota_exceeded` nor `internal_error` is in any retry family), although
    the same chunk is admitted as soon as an administrator frees room."""
    _, store, _ = environment
    owner = device(store)
    body, recording = wav_fixture(), str(uuid4())
    with client(environment) as http:
        store._quota = 1
        response = http.put(
            f"/v1/chunks/{recording}-mic_local-0",
            content=body,
            headers=chunk_headers(owner["token"], recording, 0, body),
        )
        refusal = response.json()["refusal"]
        store._quota = 10 * 1024**3
        admitted = http.put(
            f"/v1/chunks/{recording}-mic_local-0",
            content=body,
            headers=chunk_headers(owner["token"], recording, 0, body),
        )
    assert response.status_code == 507 and admitted.json()["durable"] is True
    assert refusal["retry"]["kind"] != "never", refusal
