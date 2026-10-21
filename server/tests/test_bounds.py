import hashlib
import json
from uuid import uuid4

import pytest
from conftest import device, said
from support_transcription import wav_fixture

from orbit_hub.protocol import HubError
from orbit_hub.storage import SESSION_MESSAGES_PAGE_BYTES, SESSION_MESSAGES_PAGE_LIMIT, Store


def test_a_messages_page_decrypts_only_what_it_returns(environment, monkeypatch):
    _, store, _ = environment
    owner = device(store)["device_id"]
    session = store.create_session(owner, "runtime", "stored", "epoch", 1000)
    for index in range(205):
        store.append_message(session, "user", str(index), now_ms=index)
    decrypt = store.decrypt
    calls = []

    def observed_decrypt(body, context):
        calls.append(context)
        return decrypt(body, context)

    monkeypatch.setattr(store, "decrypt", observed_decrypt)
    page = store.session_messages(owner, session, None, SESSION_MESSAGES_PAGE_LIMIT)
    assert len(calls) == 200
    assert [message["text"] for message in page["messages"]] == [str(i) for i in range(5, 205)]
    assert page["next"] == page["messages"][0]["id"]


def test_a_messages_page_bounds_encoded_json_without_truncating_messages(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    session = store.create_session(owner, "runtime", "stored", "epoch", 1000)
    text = "\0" * 32000  # JSON escaping is six times larger than its UTF-8 bytes.
    for _ in range(10):
        store.append_message(session, "assistant", text, now_ms=1)
    page = store.session_messages(owner, session, None, SESSION_MESSAGES_PAGE_LIMIT)
    assert 0 < len(page["messages"]) < 10
    assert all(message["text"] == text for message in page["messages"])
    assert page["next"] is not None
    encoded = json.dumps(page, ensure_ascii=False, separators=(",", ":")).encode()
    assert len(encoded) <= SESSION_MESSAGES_PAGE_BYTES
    store.append_message(session, "assistant", "x" * (512 * 1024 + 1), now_ms=2)
    with pytest.raises(HubError) as refused:
        store.session_messages(owner, session, None, SESSION_MESSAGES_PAGE_LIMIT)
    assert refused.value.code == "session_message_too_large"
    assert refused.value.numbers["bytes"] > refused.value.numbers["max_bytes"]


def test_full_storage_blocks_new_payloads_but_preserves_reads_retries_and_revoke(environment):
    settings, store, _ = environment
    owner = device(store)["device_id"]
    session = store.create_session(owner, "runtime", "stored", "epoch", 1000)
    store.append_message(session, "user", "PRESERVED", now_ms=1)
    operation = str(uuid4())
    store.reserve_operation(owner, operation, {"text": "fixture"})
    store.finish_operation(owner, operation, {"event": "saved"})
    recording, chunk = str(uuid4()), str(uuid4())
    body = wav_fixture()
    digest = hashlib.sha256(body).hexdigest()
    receipt = store.put_chunk(
        owner, chunk, recording, "mic_local", 0, digest, body, {"frames": 160}
    )
    job = store.claim_transcription(1000)
    constrained = Store(settings.data_dir, settings.key_file, quota_bytes=1)
    try:
        assert constrained.physical_bytes() > 1
        actions = [
            lambda: constrained.create_code(1000),
            lambda: constrained.create_session(owner, "new", "new", "epoch", 1000),
            lambda: constrained.append_message(session, "assistant", "BLOCKED", now_ms=2),
            lambda: constrained.reserve_operation(owner, str(uuid4()), {}),
            # The quota is enforced at the reservation, before the
            # operation is applied; finish_operation no longer consults it.
            lambda: constrained.reserve_operation(owner, str(uuid4()), {"event": "BLOCKED"}),
            lambda: constrained.put_chunk(
                owner, str(uuid4()), recording, "mic_local", 1, digest, body, {"frames": 1}
            ),
            lambda: constrained.finish_transcription(job, "BLOCKED_TRANSCRIPT"),
            lambda: constrained.finalize_recording(
                owner,
                recording,
                {
                    "tracks": [
                        {
                            "track": "mic_local",
                            "chunk_count": 1,
                            "last_sequence": 0,
                            "total_frames": 160,
                        }
                    ],
                    "gaps": 0,
                    "started_at_ms": 0,
                    "finalized_at_ms": 1,
                },
            ),
        ]
        for action in actions:
            with pytest.raises(HubError) as error:
                action()
            assert error.value.code == "quota_exceeded"
        assert said(constrained, owner, session) == [{"role": "user", "text": "PRESERVED"}]
        assert constrained.reserve_operation(owner, operation, {"text": "fixture"}) == {
            "event": "saved"
        }
        assert (
            constrained.put_chunk(
                owner, chunk, recording, "mic_local", 0, digest, body, {"frames": 160}
            )
            == receipt
        )
        constrained.revoke(owner)
        assert not constrained.is_active(owner)
    finally:
        constrained.close()


def test_admission_reserves_growth_not_only_current_file_size(environment):
    settings, store, _ = environment
    owner = device(store)["device_id"]
    session = store.create_session(owner, "runtime", "stored", "epoch", 1000)
    constrained = Store(
        settings.data_dir, settings.key_file, quota_bytes=store.physical_bytes() + 1024
    )
    try:
        with pytest.raises(HubError) as error:
            constrained.append_message(session, "user", "tiny but no reserved headroom", now_ms=1)
        assert error.value.code == "quota_exceeded"
        assert said(constrained, owner, session) == []
    finally:
        constrained.close()
