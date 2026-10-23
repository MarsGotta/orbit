import hashlib
from uuid import uuid4

import pytest
from conftest import device
from cryptography.exceptions import InvalidTag

from orbit_hub.protocol import HubError
from orbit_hub.storage import Store


def test_pairing_single_use_expiry_and_revocation(environment):
    _, store, _ = environment
    code = store.devices.create_code(1000)
    paired = store.devices.pair(code, "synthetic-name", 1001)
    assert store.devices.authenticate(paired["token"]) == paired["device_id"]
    with pytest.raises(HubError, match="Código"):
        store.devices.pair(code, "duplicate", 1001)
    with pytest.raises(HubError):
        store.devices.pair(store.devices.create_code(1000, 30), "expired", 1031)
    store.devices.revoke(paired["device_id"])
    with pytest.raises(HubError):
        store.devices.authenticate(paired["token"])


def test_encrypted_history_ownership_and_restart(environment):
    settings, store, _ = environment
    a, b = device(store), device(store)
    session = store.chat.create_session(
        a["device_id"], "default", "runtime", "stored", "epoch", 1000
    )
    store.chat.append_message(session, "user", "SYNTHETIC-SECRET-CANARY", now_ms=1)
    store.chat.set_status(session, "streaming")
    with pytest.raises(HubError):
        store.chat.session_messages(b["device_id"], session, None, 1)
    for path in settings.data_dir.iterdir():
        if path.is_file():
            assert b"SYNTHETIC-SECRET-CANARY" not in path.read_bytes()
    second = Store(settings.data_dir, settings.key_file)
    assert (
        second.chat.session(a["device_id"], session)["status"] == "streaming"
    )  # admin open is harmless
    second.recover_interrupted()
    assert second.chat.session(a["device_id"], session)["status"] == "outcome_unknown"
    second.close()


def test_cipher_detects_context_substitution_and_modification(environment):
    _, store, _ = environment
    encrypted = store.encrypt(b"fixture", "context-a")
    with pytest.raises(InvalidTag):
        store.decrypt(encrypted, "context-b")
    with pytest.raises(InvalidTag):
        store.decrypt(encrypted[:-1] + bytes([encrypted[-1] ^ 1]), "context-a")


def test_operation_dedup_and_uncertain_receipt(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    op = str(uuid4())
    payload = {"text": "synthetic"}
    assert store.operations.reserve(owner, op, payload) is None
    with pytest.raises(HubError) as error:
        store.operations.reserve(owner, op, payload)
    assert error.value.code == "outcome_unknown"
    store.operations.finish(owner, op, {"event": "chat.session"})
    assert store.operations.reserve(owner, op, payload) == {"event": "chat.session"}
    with pytest.raises(HubError) as error:
        store.operations.reserve(owner, op, {"text": "different"})
    assert error.value.code == "operation_conflict"


def test_chunks_idempotent_conflicting_and_revoked(environment):
    _, store, _ = environment
    owner, chunk, recording = device(store)["device_id"], str(uuid4()), str(uuid4())
    body = b"synthetic audio bytes"
    digest = hashlib.sha256(body).hexdigest()
    result = store.recordings.put_chunk(owner, chunk, recording, "mic_local", 0, digest, body)
    assert result["durable"]
    assert (
        store.recordings.put_chunk(owner, chunk, recording, "mic_local", 0, digest, body) == result
    )
    with pytest.raises(HubError):
        store.recordings.put_chunk(owner, chunk, recording, "remote_app", 0, digest, body)
    with pytest.raises(HubError):
        store.recordings.put_chunk(owner, str(uuid4()), recording, "mic_local", 0, digest, body)
    store.devices.revoke(owner)
    with pytest.raises(HubError):
        store.recordings.put_chunk(owner, str(uuid4()), recording, "mic_local", 1, digest, body)
