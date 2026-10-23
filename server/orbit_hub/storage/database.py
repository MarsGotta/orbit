"""The hub's one database: its file, its key, its transactions and snapshots.

`Store` owns the connection and the cipher, and nothing of any domain: each
domain reads and writes its rows through its own repository, built here over
this one connection (`store.devices`, `store.chat`, `store.operations`,
`store.recordings`, `store.space`, `store.groups`, `store.attachments`).
"""

import hmac
import os
import secrets
import sqlite3
from collections.abc import Callable
from contextlib import contextmanager
from pathlib import Path
from threading import RLock

from cryptography.hazmat.primitives.ciphers.aead import AESGCM

from ..attachments import AttachmentRepository
from ..group_storage import GroupRepository
from ..log import report
from ..protocol import HubError
from ..secret_file import read_secret
from . import schema
from .chat import ChatRepository
from .devices import DeviceRepository
from .operations import OperationRepository
from .recordings import RecordingRepository
from .space import SpaceRepository, is_storage_full


def initialize(data_dir: Path, key_file: Path) -> None:
    """Called only by the explicit admin init command.

    A data directory that already holds a database is refused: its key may
    be missing or misplaced, and a new key next to it would make the hub
    start on rows it cannot read and write new ones the old key cannot.
    """
    database = data_dir / "hub.sqlite3"
    if database.exists() or database.is_symlink():
        raise FileExistsError(
            f"{data_dir} already holds a store (hub.sqlite3); init would write a new key "
            "next to a database encrypted with another. Put back its key, or restore a "
            'backup into an empty data directory (see docs/self-hosting.md, "Backups")'
        )
    data_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    key_file.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd = os.open(key_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        os.write(fd, AESGCM.generate_key(bit_length=256))
        os.fsync(fd)
    finally:
        os.close(fd)
    directory = os.open(key_file.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


class Store:
    def __init__(
        self,
        data_dir: Path,
        key_file: Path,
        quota_bytes: int | None = None,
        *,
        free_space: Callable[[], int] | None = None,
    ):
        """`quota_bytes` caps the database (`storage_quota_bytes`); without it
        the disk sets the limit (`space.STORAGE_FREE_FLOOR_BYTES`).
        `free_space` measures what the disk has free; tests give their own."""
        key = read_secret(key_file, 32)
        if len(key) != 32:
            raise ValueError("Storage key must contain exactly 32 bytes; run orbit-hub init")
        info = data_dir.stat()
        if info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise ValueError("Data directory must belong to this user and have mode 0700")
        self._cipher = AESGCM(key)
        self._lock = RLock()
        # Counts the commits of this connection: a cache of rows read before
        # the last one (`DeviceRepository.is_active`) is not used again.
        self.commits = 0
        path = data_dir / "hub.sqlite3"
        self._path = path
        if path.is_symlink():
            raise ValueError("Database must not be a symbolic link")
        fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        os.close(fd)
        self._db = sqlite3.connect(path, check_same_thread=False, isolation_level=None)
        self._db.row_factory = sqlite3.Row
        self._db.execute("PRAGMA journal_mode=WAL")
        self._db.execute("PRAGMA synchronous=FULL")
        self._db.execute("PRAGMA foreign_keys=ON")
        # Before any migration: a database opened with another key is left
        # exactly as it was.
        try:
            schema.verify_key(self._db, self.decrypt)
        except BaseException:
            self._db.close()
            raise
        self.space = SpaceRepository(path, quota_bytes, free_space)
        schema.migrate(self._db, self.encrypt)
        self.devices = DeviceRepository(self)
        self.chat = ChatRepository(self)
        self.operations = OperationRepository(
            self, hmac.digest(key, b"orbit.operation.fingerprint.v1", "sha256")
        )
        self.recordings = RecordingRepository(self)
        self.groups = GroupRepository(self)
        self.attachments = AttachmentRepository(self)

    def recover_interrupted(self) -> None:
        """Only hub startup calls this; opening the admin CLI must not alter live turns."""
        with self.transaction() as db:
            # A restart leaves any in-flight turn (submitting, streaming or
            # interrupting) without a known outcome. Its conversation still
            # takes the next message: the turn was cut (`turns`, `runtime.send`).
            db.execute(
                "UPDATE sessions SET status='outcome_unknown'"
                " WHERE status IN ('submitting','streaming','interrupting')"
            )
            db.execute("UPDATE transcription_jobs SET state='pending' WHERE state='running'")
            db.execute("UPDATE transcription_groups SET state='pending' WHERE state='running'")
            db.execute("UPDATE recording_final_passes SET state='queued' WHERE state='running'")

    @contextmanager
    def transaction(self):
        with self._lock:
            self._db.execute("BEGIN IMMEDIATE")
            try:
                yield self._db
            except HubError:
                # A refusal rolls back by design and reaches its caller with
                # its code: it is not a failure to report.
                self._rollback()
                raise
            except BaseException as failure:
                report(
                    event="storage_transaction_failed",
                    code="storage_full" if is_storage_full(failure) else "transaction_rolled_back",
                )
                self._rollback()
                raise
            else:
                self._db.execute("COMMIT")
                # A commit may include any write of this connection, a revocation
                # among them, so a cached revocation signal cannot outlive a
                # successful COMMIT. A revocation from the CLI, in another
                # process, is seen when the cache's TTL runs out.
                self.commits += 1

    @contextmanager
    def reading(self):
        """The live connection for a read, under the writer's lock."""
        with self._lock:
            yield self._db

    def _rollback(self) -> None:
        """Roll back what is open. SQLite has already rolled back a
        transaction it could not grow (`SQLITE_FULL`); a second ROLLBACK would
        raise "no transaction is active" and hide that cause."""
        if self._db.in_transaction:
            self._db.execute("ROLLBACK")

    def close(self) -> None:
        with self._lock:
            # A closed connection must not be reached through a stale cache.
            self.commits += 1
            self._db.close()

    @contextmanager
    def read_snapshot(self):
        """Independent WAL reader: context never holds the live writer/event-loop lock."""
        db = sqlite3.connect(
            self._path.resolve().as_uri() + "?mode=ro", uri=True, isolation_level=None
        )
        db.row_factory = sqlite3.Row
        try:
            db.execute("PRAGMA query_only=ON")
            db.execute("BEGIN")
            yield db
        finally:
            db.close()

    def ensure_capacity(self, payload_bytes: int) -> None:
        """Admit a write of `payload_bytes` inside its transaction, or refuse it
        with the rule that fired (`SpaceRepository.ensure_capacity`). Every
        repository's write that brings content calls it."""
        self.space.ensure_capacity(payload_bytes)

    def encrypt(self, data: bytes, context: str) -> bytes:
        nonce = secrets.token_bytes(12)
        return nonce + self._cipher.encrypt(nonce, data, context.encode())

    def decrypt(self, data: bytes, context: str) -> bytes:
        return self._cipher.decrypt(data[:12], data[12:], context.encode())
