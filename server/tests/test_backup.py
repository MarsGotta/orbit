"""Copies of the hub's database, and putting them back.

The nightly copy goes to a directory off the server without the key; a
restore pairs them again in an empty data directory and refuses another key.
The copy before an activation stays on the server with its key, and an
activation that goes back to the release it was taken from puts it back.
Synthetic data only: invented names, temporary directories.
"""

import hashlib
import json
import re
import sqlite3
import sys
from datetime import UTC, date, datetime, timedelta
from pathlib import Path
from uuid import uuid4

import pytest
from support_transcription import wav_fixture

from orbit_hub import backup
from orbit_hub.cli import main
from orbit_hub.storage import Store, initialize

NOW = datetime(2026, 10, 22, 3, 30, tzinfo=UTC)


SAME_DISK = backup._same_disk


@pytest.fixture(autouse=True)
def another_disk(monkeypatch):
    """The temporary directories share one disk; the copies' destination
    stands for another machine's (the test of that refusal puts the truth back)."""
    monkeypatch.setattr(backup, "_same_disk", lambda one, other: False)


def toml_string(value) -> str:
    return json.dumps(str(value))


def config(path: Path, data: Path, key: Path) -> Path:
    path.write_text(
        "\n".join(
            (
                f"data_dir = {toml_string(data)}",
                f"key_file = {toml_string(key)}",
                'hermes_url = "https://hermes.example.test"',
                f"hermes_credential_file = {toml_string(path.parent / 'credential')}",
                "[meeting_transcription]",
                'endpoint = "https://transcriber.example.test"',
                'model = "fixture-model"',
            )
        )
        + "\n",
        encoding="utf-8",
    )
    return path


def hub(monkeypatch, capsys, configuration: Path, *arguments: str) -> tuple[int, str, str]:
    monkeypatch.setattr(sys, "argv", ["orbit-hub", "--config", str(configuration), *arguments])
    try:
        main()
        code = 0
    except SystemExit as stopped:
        code = stopped.code
    out, err = capsys.readouterr()
    return code, out, err


def a_store_with_a_meeting_and_a_conversation(data: Path, key: Path):
    """An open store with one Mac, one message and one chunk; it stays open,
    its WAL not checkpointed, as the hub runs."""
    initialize(data, key)
    store = Store(data, key)
    owner = store.devices.pair(store.devices.create_code(1000), "Mac de Irene Velasco", 1001)
    session = store.chat.create_session(owner["device_id"], "default", "rt", "st", "ep", 1002)
    store.chat.append_message(session, "user", "Preparar el acta de la Fundación Lumbre", now_ms=1)
    body, recording = wav_fixture(), str(uuid4())
    store.recordings.put_chunk(
        owner["device_id"],
        f"{recording}-mic_local-0",
        recording,
        "mic_local",
        0,
        hashlib.sha256(body).hexdigest(),
        body,
        {"frames": 160},
    )
    return store, owner, session, recording, body


def test_a_nightly_copy_restored_into_a_new_data_dir_opens_with_the_key_and_reads_back(
    tmp_path, monkeypatch, capsys
):
    data, key = tmp_path / "server" / "data", tmp_path / "server" / "key"
    store, owner, session, recording, body = a_store_with_a_meeting_and_a_conversation(data, key)
    destination = tmp_path / "other-machine"
    destination.mkdir()
    try:
        assert (data / "hub.sqlite3-wal").stat().st_size > 0  # the latest rows live in the WAL
        code, out, err = hub(
            monkeypatch,
            capsys,
            config(tmp_path / "hub.toml", data, key),
            "backup",
            str(destination),
        )
        assert (code, err) == (0, ""), err
        # Written while the hub keeps going: a later write does not reach the copy.
        store.chat.append_message(session, "user", "Escrito después de la copia", now_ms=2)
    finally:
        store.close()
    copies = sorted(path for path in destination.rglob("*") if path.is_file())
    names = [path.relative_to(destination).as_posix() for path in copies]
    assert len(names) == 2, names
    assert re.fullmatch(r"daily/hub-\d{4}-\d{2}-\d{2}\.sqlite3", names[0]), names
    assert re.fullmatch(r"weekly/hub-\d{4}-W\d{2}\.sqlite3", names[1]), names
    secret = key.read_bytes()
    for path in copies:
        assert secret not in path.read_bytes(), f"the key travelled in {path}"
        assert path.stat().st_mode & 0o777 == 0o600
    assert "copied" in out

    # Another machine: an empty data directory, and the key the operator kept apart.
    restored, kept = tmp_path / "new" / "data", tmp_path / "new" / "key"
    kept.parent.mkdir()
    kept.write_bytes(secret)
    kept.chmod(0o600)
    code, out, err = hub(
        monkeypatch,
        capsys,
        config(tmp_path / "new.toml", restored, kept),
        "restore",
        str(copies[0]),
    )
    assert (code, err) == (0, ""), err
    assert "opens with the configured key" in out
    reopened = Store(restored, kept)
    try:
        assert reopened.devices.authenticate(owner["token"]) == owner["device_id"]
        page = reopened.chat.session_messages(owner["device_id"], session, None, 10)
        assert [message["text"] for message in page["messages"]] == [
            "Preparar el acta de la Fundación Lumbre"
        ]
        chunk = reopened._db.execute(
            "SELECT body FROM chunks WHERE device_id=? AND recording=?",
            (owner["device_id"], recording),
        ).fetchone()["body"]
        assert (
            reopened.decrypt(chunk, f"chunk:{owner['device_id']}:{recording}-mic_local-0") == body
        )
    finally:
        reopened.close()


def test_a_restore_with_another_key_is_refused_and_leaves_the_data_dir_empty(
    tmp_path, monkeypatch, capsys
):
    data, key = tmp_path / "data", tmp_path / "key"
    store, *_ = a_store_with_a_meeting_and_a_conversation(data, key)
    destination = tmp_path / "copies"
    destination.mkdir()
    try:
        backup.nightly(data, destination, date(2026, 10, 22))
    finally:
        store.close()
    restored, wrong = tmp_path / "new" / "data", tmp_path / "new" / "key"
    wrong.parent.mkdir()
    wrong.write_bytes(bytes(range(32)))
    wrong.chmod(0o600)
    code, _, err = hub(
        monkeypatch,
        capsys,
        config(tmp_path / "new.toml", restored, wrong),
        "restore",
        str(destination / "daily" / "hub-2026-10-22.sqlite3"),
    )
    assert code == 1
    assert "not the one this database was written with" in err
    assert list(restored.iterdir()) == []


def test_a_restore_never_writes_over_a_database(tmp_path, monkeypatch, capsys):
    data, key = tmp_path / "data", tmp_path / "key"
    store, *_ = a_store_with_a_meeting_and_a_conversation(data, key)
    destination = tmp_path / "copies"
    destination.mkdir()
    try:
        backup.nightly(data, destination, date(2026, 10, 22))
        before = (data / "hub.sqlite3").stat().st_ino
        code, _, err = hub(
            monkeypatch,
            capsys,
            config(tmp_path / "hub.toml", data, key),
            "restore",
            str(destination / "daily" / "hub-2026-10-22.sqlite3"),
        )
    finally:
        store.close()
    assert code == 1 and "already holds a database" in err
    assert (data / "hub.sqlite3").stat().st_ino == before


def test_seven_daily_and_four_weekly_copies_are_kept_and_nothing_else_is_touched(tmp_path):
    data, key = tmp_path / "data", tmp_path / "key"
    initialize(data, key)
    Store(data, key).close()
    destination = tmp_path / "copies"
    for name in ("daily", "weekly"):
        (destination / name).mkdir(parents=True)
    # Copies of earlier nights and weeks, and something of the operator's.
    for day in range(1, 10):
        (destination / "daily" / f"hub-2026-10-{day:02d}.sqlite3").write_bytes(b"older")
    for week in range(36, 42):
        (destination / "weekly" / f"hub-2026-W{week}.sqlite3").write_bytes(b"older")
    unrelated = destination / "daily" / "notes-of-the-operator.txt"
    unrelated.write_text("Copias revisadas por Teodora Ruiz", encoding="utf-8")
    said = backup.nightly(data, destination, date(2026, 10, 22))  # a Thursday of W43
    daily = sorted(path.name for path in (destination / "daily").iterdir())
    weekly = sorted(path.name for path in (destination / "weekly").iterdir())
    assert daily == [
        *(f"hub-2026-10-{day:02d}.sqlite3" for day in range(4, 10)),
        "hub-2026-10-22.sqlite3",
        "notes-of-the-operator.txt",
    ]
    assert weekly == [f"hub-2026-W{week}.sqlite3" for week in (39, 40, 41, 43)]
    assert sum(line.startswith("removed") for line in said) == 3 + 3
    # The same night again replaces its copy and keeps the week's.
    backup.nightly(data, destination, date(2026, 10, 22))
    assert sorted(path.name for path in (destination / "weekly").iterdir()) == weekly


def test_a_destination_that_is_not_there_is_refused_and_never_created(tmp_path):
    data, key = tmp_path / "data", tmp_path / "key"
    initialize(data, key)
    Store(data, key).close()
    missing = tmp_path / "mnt" / "other-machine"
    with pytest.raises(backup.BackupError, match="mounted"):
        backup.nightly(data, missing, date(2026, 10, 22))
    assert not missing.exists() and not missing.parent.exists()


def test_a_destination_on_the_hubs_own_disk_is_refused(tmp_path, monkeypatch):
    """A mount point whose disk is not mounted is a directory of the server's."""
    monkeypatch.setattr(backup, "_same_disk", SAME_DISK)
    data, key = tmp_path / "data", tmp_path / "key"
    initialize(data, key)
    Store(data, key).close()
    mount_point = tmp_path / "mnt"
    mount_point.mkdir()
    with pytest.raises(backup.BackupError, match="same disk"):
        backup.nightly(data, mount_point, date(2026, 10, 22))
    assert list(mount_point.iterdir()) == []


def test_a_copy_of_a_database_that_is_being_written_is_consistent(tmp_path):
    """The online backup API copies one committed state, never half a write."""
    data, key = tmp_path / "data", tmp_path / "key"
    store, *_ = a_store_with_a_meeting_and_a_conversation(data, key)
    writer = sqlite3.connect(data / "hub.sqlite3", isolation_level=None)
    try:
        writer.execute("BEGIN IMMEDIATE")
        writer.execute("DELETE FROM messages")  # uncommitted while the copy runs
        target = tmp_path / "copy.sqlite3"
        backup.copy_database(data / "hub.sqlite3", target)
        writer.execute("ROLLBACK")
    finally:
        writer.close()
        store.close()
    copy = sqlite3.connect(target)
    try:
        assert copy.execute("SELECT count(*) FROM messages").fetchone()[0] == 1
        assert copy.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        assert copy.execute("PRAGMA journal_mode").fetchone()[0] == "delete"
    finally:
        copy.close()
    assert not Path(str(target) + "-wal").exists()


# --- Around an activation --------------------------------------------------------


def migrate(data: Path) -> None:
    """What a newer release's first start does to the schema."""
    database = sqlite3.connect(data / "hub.sqlite3", isolation_level=None)
    try:
        database.execute("ALTER TABLE devices ADD COLUMN synthetic_future TEXT")
    finally:
        database.close()


def test_going_back_to_the_release_the_copy_came_from_puts_the_copy_back(tmp_path):
    data, key = tmp_path / "data", tmp_path / "key"
    store, owner, session, *_ = a_store_with_a_meeting_and_a_conversation(data, key)
    store.close()
    backups = tmp_path / "base" / "backups"
    said = backup.activation_copy(data, key, backups, "0.10.1", NOW)
    assert "as 0.10.1 left them" in said[0]
    assert backup.activation_label(backups) == "0.10.1"
    assert (backups / "activation" / "storage.key").read_bytes() == key.read_bytes()
    # The new release starts, migrates and writes.
    migrate(data)
    store = Store(data, key)
    store.chat.append_message(session, "user", "Escrito con la versión nueva", now_ms=3)
    store.close()
    assert backup.activation_plan(data, backups, "0.10.2") == "copy"
    assert backup.activation_plan(data, backups, "0.10.1") == "restore"
    said = backup.activation_restore(data, key, backups, NOW + timedelta(hours=1))
    aside = backups / "set-aside-20261022T043000Z"
    assert str(aside) in said[1]
    assert backup.activation_plan(data, backups, "0.10.1") == "keep"
    restored = Store(data, key)
    try:
        page = restored.chat.session_messages(owner["device_id"], session, None, 10)
        assert [message["text"] for message in page["messages"]] == [
            "Preparar el acta de la Fundación Lumbre"
        ]
    finally:
        restored.close()
    # What the newer release wrote is set aside, not lost.
    kept = Store(aside, key)
    try:
        page = kept.chat.session_messages(owner["device_id"], session, None, 10)
        assert page["messages"][-1]["text"] == "Escrito con la versión nueva"
    finally:
        kept.close()


def test_the_same_schema_keeps_everything_written_since_the_copy(tmp_path):
    data, key = tmp_path / "data", tmp_path / "key"
    store, *_ = a_store_with_a_meeting_and_a_conversation(data, key)
    store.close()
    backups = tmp_path / "backups"
    backup.activation_copy(data, key, backups, "0.10.1", NOW)
    assert backup.activation_plan(data, backups, "0.10.1") == "keep"


def test_a_copy_taken_with_another_key_is_never_put_back(tmp_path):
    data, key = tmp_path / "data", tmp_path / "key"
    store, *_ = a_store_with_a_meeting_and_a_conversation(data, key)
    store.close()
    backups = tmp_path / "backups"
    backup.activation_copy(data, key, backups, "0.10.1", NOW)
    migrate(data)
    (backups / "activation" / "storage.key").write_bytes(bytes(range(32)))
    before = (data / "hub.sqlite3").read_bytes()
    with pytest.raises(backup.BackupError, match="nothing was touched"):
        backup.activation_restore(data, key, backups, NOW)
    assert (data / "hub.sqlite3").read_bytes() == before


def test_a_data_dir_without_a_database_yet_has_nothing_to_copy(tmp_path):
    data = tmp_path / "data"
    data.mkdir(mode=0o700)
    said = backup.activation_copy(data, tmp_path / "key", tmp_path / "backups", "0.10.1", NOW)
    assert said == [f"there is no database at {data / 'hub.sqlite3'} yet: nothing to copy"]
    assert not (tmp_path / "backups").exists()
