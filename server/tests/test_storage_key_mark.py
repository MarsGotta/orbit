"""Encrypted values may carry, in front, a mark of the key that sealed them.

A later hub will write that mark; this one reads both forms and still writes
the one every earlier hub reads, so activating an earlier hub keeps the
database whole. The marked form is built here by hand, from its description in
contracts/hub-storage-v1.md, never with the hub's own code:

    "OHK1" (4 bytes) | key id (8) | nonce (12) | AES-256-GCM ciphertext and tag

The key id is the first 8 bytes of HMAC-SHA256(storage key,
"orbit.storage.key-id.v1"). The associated data is the 12-byte header (mark
and key id) followed by the value's context, so neither can be swapped.
Today's form is the nonce (12) and the ciphertext, with the context alone.
"""

import hmac
import os
import shutil
import sqlite3

import pytest
from cryptography.exceptions import InvalidTag
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

from orbit_hub.storage import Store
from orbit_hub.storage.chat import SESSION_MESSAGES_PAGE_LIMIT
from orbit_hub.storage.schema import KEY_CHECK_CONTEXT, KEY_CHECK_PLAINTEXT

MARK = b"OHK1"
REMINDER_KEY = "ab" * 32


def key_id(key: bytes) -> bytes:
    return hmac.digest(key, b"orbit.storage.key-id.v1", "sha256")[:8]


def sealed_with_mark(key: bytes, plaintext: bytes, context: str) -> bytes:
    header = MARK + key_id(key)
    nonce = os.urandom(12)
    return header + nonce + AESGCM(key).encrypt(nonce, plaintext, header + context.encode())


def sealed_today(key: bytes, plaintext: bytes, context: str, nonce: bytes | None = None) -> bytes:
    nonce = os.urandom(12) if nonce is None else nonce
    return nonce + AESGCM(key).encrypt(nonce, plaintext, context.encode())


def opened_today(key: bytes, blob: bytes, context: str) -> bytes:
    return AESGCM(key).decrypt(blob[:12], blob[12:], context.encode())


def copy_of(storage_template, folder):
    template_data, template_key = storage_template
    folder.mkdir()
    data, key = folder / "data", folder / "key"
    shutil.copytree(template_data, data)
    shutil.copy2(template_key, key)
    return data, key


@pytest.fixture
def opened(tmp_path, storage_template):
    data, key = copy_of(storage_template, tmp_path / "hub")
    store = Store(data, key)
    yield store, data, key.read_bytes()
    store.close()


def test_a_value_sealed_with_the_mark_of_this_key_is_read(opened):
    store, _, key = opened
    words = b"Llevar las cifras de Vega"
    blob = sealed_with_mark(key, words, "reminder:lyra")
    assert store.decrypt(blob, "reminder:lyra") == words
    # Bound to its context and to its header.
    with pytest.raises(InvalidTag):
        store.decrypt(blob, "reminder:orion")
    tampered = blob[:4] + bytes([blob[4] ^ 1]) + blob[5:]
    with pytest.raises(InvalidTag):
        store.decrypt(tampered, "reminder:lyra")
    header, nonce = MARK + key_id(key), os.urandom(12)
    unbound = header + nonce + AESGCM(key).encrypt(nonce, words, b"reminder:lyra")
    with pytest.raises(InvalidTag):
        store.decrypt(unbound, "reminder:lyra")


def test_a_value_sealed_today_is_still_read(opened):
    store, _, key = opened
    words = "Preguntar a Orión por la entrega".encode()
    assert store.decrypt(sealed_today(key, words, "message:s:user"), "message:s:user") == words


def test_an_unmarked_value_whose_nonce_looks_like_a_mark_is_still_read(opened):
    """A nonce is random: one in 2**96 begins with the mark and this key's id.
    Such a value is today's form all the same, and it is read."""
    store, _, key = opened
    words = b"Nota de Lyra"
    blob = sealed_today(key, words, "device:lyra", nonce=MARK + key_id(key))
    assert store.decrypt(blob, "device:lyra") == words


def test_this_hub_still_writes_only_what_earlier_hubs_read(opened):
    store, data, key = opened
    words = b"Resumen para Vega"
    assert opened_today(key, store.encrypt(words, "note:lyra"), "note:lyra") == words
    lyra = store.devices.pair(store.devices.create_code(1000), "Mac de Lyra", 1001)
    session = store.chat.create_session(lyra["device_id"], "atlas", "r", "s", "e", 1002)
    store.chat.append_message(session, "user", "¿Qué dijo Vega?", now_ms=1_002_000)
    store.reminders.save(REMINDER_KEY, "Llevar el informe de Orión", 1_003_000)
    header = MARK + key_id(key)
    database = sqlite3.connect(data / "hub.sqlite3")
    try:
        query = "SELECT name FROM sqlite_master WHERE type='table'"
        tables = [row[0] for row in database.execute(query)]
        tables = [name for name in tables if not name.startswith("sqlite_")]
        marked = [
            table
            for table in tables
            for row in database.execute(f"SELECT * FROM {table}")
            for value in row
            if isinstance(value, bytes) and value.startswith(header)
        ]
    finally:
        database.close()
    assert marked == []


def test_a_database_whose_key_check_carries_the_mark_opens(tmp_path, storage_template):
    data, key_file = copy_of(storage_template, tmp_path / "marked")
    key = key_file.read_bytes()
    marked = sealed_with_mark(key, KEY_CHECK_PLAINTEXT, KEY_CHECK_CONTEXT)
    with sqlite3.connect(data / "hub.sqlite3") as database:
        database.execute("UPDATE storage_key_check SET body=? WHERE id=1", (marked,))
    database.close()
    Store(data, key_file).close()
    # Marked with another key, it is still a database this key did not write.
    other_data, other_key_file = copy_of(storage_template, tmp_path / "other")
    foreign = sealed_with_mark(bytes(range(32)), KEY_CHECK_PLAINTEXT, KEY_CHECK_CONTEXT)
    with sqlite3.connect(other_data / "hub.sqlite3") as database:
        database.execute("UPDATE storage_key_check SET body=? WHERE id=1", (foreign,))
    database.close()
    with pytest.raises(ValueError, match="not the one this database was written with"):
        Store(other_data, other_key_file)
    database = sqlite3.connect(other_data / "hub.sqlite3")
    try:
        kept = database.execute("SELECT body FROM storage_key_check WHERE id=1").fetchone()[0]
    finally:
        database.close()
    assert kept == foreign


def test_rows_sealed_with_the_mark_are_read_through_the_repositories(tmp_path, storage_template):
    """A database a later hub wrote into holds both forms side by side."""
    data, key_file = copy_of(storage_template, tmp_path / "mixed")
    key = key_file.read_bytes()
    store = Store(data, key_file)
    lyra = store.devices.pair(store.devices.create_code(1000), "Mac de Lyra", 1001)
    store.devices.pair(store.devices.create_code(1000), "Mac de Orión", 1002)
    session = store.chat.create_session(lyra["device_id"], "atlas", "r", "s", "e", 1003)
    store.chat.append_message(session, "user", "¿Qué dijo Vega?", now_ms=1_003_000)
    store.reminders.save(REMINDER_KEY, "Llevar el informe de Orión", 1_004_000)
    store.close()

    def remark(database, table, column, where, context, *arguments):
        query = f"SELECT {column} FROM {table} WHERE {where}"
        blob = database.execute(query, arguments).fetchone()[0]
        plaintext = opened_today(key, blob, context)
        database.execute(
            f"UPDATE {table} SET {column}=? WHERE {where}",
            (sealed_with_mark(key, plaintext, context), *arguments),
        )

    database = sqlite3.connect(data / "hub.sqlite3")
    try:
        device = lyra["device_id"]
        remark(database, "devices", "name", "id=?", f"device:{device}", device)
        remark(database, "messages", "body", "session_id=?", f"message:{session}:user", session)
        row_key = database.execute("SELECT key FROM personal_reminders").fetchone()[0]
        remark(database, "personal_reminders", "body", "key=?", f"reminder:{row_key}", row_key)
        database.commit()
    finally:
        database.close()

    store = Store(data, key_file)
    try:
        assert sorted(paired["name"] for paired in store.devices.active()) == [
            "Mac de Lyra",
            "Mac de Orión",
        ]
        page = store.chat.session_messages(device, session, None, SESSION_MESSAGES_PAGE_LIMIT)
        assert [message["text"] for message in page["messages"]] == ["¿Qué dijo Vega?"]
        assert store.reminders.get(REMINDER_KEY)["text"] == "Llevar el informe de Orión"
    finally:
        store.close()
