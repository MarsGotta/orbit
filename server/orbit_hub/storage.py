"""Durable bindings and encrypted payloads. No network or provider dependencies."""

import errno
import hashlib
import hmac
import json
import os
import secrets
import shutil
import sqlite3
import time
from collections.abc import Callable
from contextlib import contextmanager
from pathlib import Path
from threading import RLock
from uuid import uuid4

from cryptography.exceptions import InvalidTag
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

from . import safety
from .assistant.agent_config import SCHEMA as AGENT_CONFIG_SCHEMA
from .assistant.handover import SCHEMA as HANDOVER_SCHEMA
from .assistant.roster import SCHEMA as ROSTER_SCHEMA
from .attachments import SCHEMA as ATTACHMENTS_SCHEMA
from .attachments import AttachmentRepository
from .group_storage import SCHEMA as GROUP_SCHEMA
from .group_storage import GroupRepository
from .log import report
from .meetings.dictionaries import SCHEMA as DICTIONARIES_SCHEMA
from .meetings.dictionaries import migrate as migrate_dictionaries
from .meetings.repository import SCHEMA as MEETINGS_SCHEMA
from .protocol import HubError
from .secret_file import read_secret

# An operation's durable receipt is bounded public frame data (the
# largest interaction acknowledgement stays a few KiB). Its headroom is
# reserved when the operation is admitted, so recording the outcome of an
# already-applied operation can never fail on the content quota.
OPERATION_RECEIPT_RESERVE_BYTES = 16 * 1024

# The conversations `/v1/sessions` lists for a Mac, the most recent
# ones. The hub keeps every conversation ever opened and each restart opens new
# ones, so the whole list only grows.
# What it protects: the Mac that reads the list, which keeps at most 128 of
# them (`MAX_SESSIONS` in crates/orbit-daemon/src/hub.rs).
# Why 128: it is what the daemon keeps, and it is far more than the app's list
# of recent conversations shows. The two are kept equal by hand.
# What it is not: a retention limit. Nothing is deleted; the route says how
# many there are (`total`), and older conversations stay stored.
SESSION_LIST_LIMIT = 128

# One page of a conversation's messages (`GET /v1/sessions/{id}/messages`).
# What it protects: the Mac that reads it, whose HTTP reader stops at 1 MiB
# (`MAX_HUB_HTTP_BODY_BYTES`) and which passes the page on to the app inside
# one 640 KiB read (`SessionMessages`, crates/orbit-core/src/reads.rs). Why
# 512 KiB: it is what `chat.history` returned before it, and it leaves the
# daemon room for what it adds to each message. What it is not: a limit on a
# conversation. Older messages come with `next`; only a single message larger
# than a page is refused, with its size.
SESSION_MESSAGES_PAGE_BYTES = 512 * 1024
# The most messages one page decrypts: the 200 `chat.history` read, which
# brings most conversations whole in one page. The bytes are bounded apart.
SESSION_MESSAGES_PAGE_LIMIT = 200
# The bytes of a page around its messages: `{"messages":[`, `],"next":`, a
# message id, `,"total":`, a count and `,"agent_id":` with the session's agent.
# Measured as 116 with two 19-digit numbers and a 32-character agent id; 128
# leaves room.
_PAGE_ENVELOPE_BYTES = 128


# What the hub's database may grow to when no `storage_quota_bytes` is set:
# half of what it and the disk's free space add up to, once this floor is
# left free. The other half is room for one whole copy of the database beside
# it: `scripts/hub/activate.sh` takes a consistent copy on the server before
# every activation, and a database grown into that room could never be copied
# again. The floor protects the rest of the server, which keeps writing after
# the hub stops admitting audio: a full root filesystem stops journald,
# package updates and logins. Why 5 GiB: journald alone keeps up to 4 GiB by
# default (`SystemMaxUse`, 10 % of the filesystem capped at 4 GiB), and 1 GiB
# more covers a package update. What it is not: a retention rule. Nothing is
# ever deleted to stay under it; the hub refuses new content with its numbers
# and the Mac keeps its copy.
STORAGE_FREE_FLOOR_BYTES = 5 * 1024**3
# From this share of the limit on, the hub tells the Macs that it is nearly
# full (`ready.storage`, `storage.status`). What it protects: time to act
# before meetings stop reaching the hub. Why 80 %: Mars's choice (2026-10-22);
# at the production pace measured that day (the database grew about 0.8 GB in
# a day) the last fifth of an 80 GB limit is about three weeks.
STORAGE_WARNING_PERCENT = 80

# The key check: one row the hub encrypts with its key when it first opens a
# database, so a database never opens with another key (which would leave it
# with two: old rows unreadable now, new ones unreadable with the old key).
_KEY_CHECK_PLAINTEXT = b"orbit.storage.key-check.v1"
_KEY_CHECK_CONTEXT = "storage:key-check"
_KEY_CHECK_SCHEMA = """
    CREATE TABLE IF NOT EXISTS storage_key_check (
        id INTEGER PRIMARY KEY CHECK (id = 1), body BLOB NOT NULL);
"""
# Encrypted columns an older database (written before the key check) can be
# verified against: the first row of whichever exists.
_LEGACY_KEY_PROBES = (
    ("devices", "SELECT id, name FROM devices ORDER BY rowid LIMIT 1", "device:{0}"),
)
_WRONG_KEY = (
    "The storage key is not the one this database was written with; opening it "
    "would leave it with two keys. Put back the key this data directory was "
    'created with (see docs/self-hosting.md, "Backups")'
)


def is_storage_full(failure: BaseException | None) -> bool:
    """Whether a failure, or one it was raised from, is the disk being full.

    SQLite says so with `SQLITE_FULL` ("database or disk is full"), which a
    failure re-raised from Python code may carry only as its text; the file
    system with ENOSPC or EDQUOT.
    """
    seen = 0
    while failure is not None and seen < 8:
        if isinstance(failure, sqlite3.Error):
            code = getattr(failure, "sqlite_errorcode", None)
            if (code is not None and code & 0xFF == sqlite3.SQLITE_FULL) or (
                "database or disk is full" in str(failure)
            ):
                return True
        if isinstance(failure, OSError) and failure.errno in (errno.ENOSPC, errno.EDQUOT):
            return True
        failure = failure.__cause__ or failure.__context__
        seen += 1
    return False


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


def disk_free(path: Path) -> int:
    """What the disk holding `path` has free: what sets the hub's limit when no
    quota does. The tests replace it, so the host's own disk never decides one."""
    return shutil.disk_usage(path).free


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
        the disk sets the limit (`STORAGE_FREE_FLOOR_BYTES`). `free_space`
        measures what the disk has free; tests give their own."""
        key = read_secret(key_file, 32)
        if len(key) != 32:
            raise ValueError("Storage key must contain exactly 32 bytes; run orbit-hub init")
        info = data_dir.stat()
        if info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise ValueError("Data directory must belong to this user and have mode 0700")
        self._cipher = AESGCM(key)
        self._fingerprint_key = hmac.digest(key, b"orbit.operation.fingerprint.v1", "sha256")
        self._lock = RLock()
        self._quota = quota_bytes
        self._free_space = free_space or (lambda: disk_free(data_dir))
        # One query per delta flooded the loop with `SELECT revoked` against
        # SQLite. A short TTL absorbs bursts without outliving an explicit revoke.
        self._active_ttl = 1.0
        self._active_cache: dict[str, tuple[float, bool]] = {}
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
            self._verify_key()
        except BaseException:
            self._db.close()
            raise
        self._db.executescript("""
            CREATE TABLE IF NOT EXISTS devices (
                id TEXT PRIMARY KEY, name BLOB NOT NULL, token_hash TEXT UNIQUE NOT NULL,
                created REAL NOT NULL, last_seen REAL NOT NULL DEFAULT 0,
                revoked INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS pair_codes (
                hash TEXT PRIMARY KEY, expires REAL NOT NULL, used INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY, device_id TEXT NOT NULL REFERENCES devices(id),
                runtime_id TEXT NOT NULL, stored_id TEXT NOT NULL, epoch TEXT NOT NULL,
                status TEXT NOT NULL, created REAL NOT NULL, agent_id TEXT);
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id),
                role TEXT NOT NULL, body BLOB NOT NULL, created_at_ms INTEGER,
                kind TEXT NOT NULL DEFAULT 'text', duration_ms INTEGER, client_id TEXT);
            CREATE TABLE IF NOT EXISTS operations (
                id TEXT NOT NULL, device_id TEXT NOT NULL REFERENCES devices(id),
                fingerprint TEXT NOT NULL, status TEXT NOT NULL, response BLOB,
                PRIMARY KEY(device_id,id));
            CREATE TABLE IF NOT EXISTS chunks (
                id TEXT NOT NULL, device_id TEXT NOT NULL REFERENCES devices(id),
                recording TEXT NOT NULL, track TEXT NOT NULL, sequence INTEGER NOT NULL,
                sha256 TEXT NOT NULL, body BLOB NOT NULL, size INTEGER NOT NULL,
                PRIMARY KEY(device_id,id), UNIQUE(device_id,recording,track,sequence));
            CREATE TABLE IF NOT EXISTS transcription_jobs (
                device_id TEXT NOT NULL, chunk_id TEXT NOT NULL, metadata BLOB NOT NULL,
                state TEXT NOT NULL DEFAULT 'pending', attempts INTEGER NOT NULL DEFAULT 0,
                next_attempt REAL NOT NULL DEFAULT 0, error_code TEXT,
                PRIMARY KEY(device_id,chunk_id),
                FOREIGN KEY(device_id,chunk_id) REFERENCES chunks(device_id,id));
            CREATE TABLE IF NOT EXISTS transcripts (
                device_id TEXT NOT NULL, chunk_id TEXT NOT NULL, body BLOB NOT NULL,
                PRIMARY KEY(device_id,chunk_id),
                FOREIGN KEY(device_id,chunk_id) REFERENCES chunks(device_id,id));
            CREATE TABLE IF NOT EXISTS recording_manifests (
                device_id TEXT NOT NULL REFERENCES devices(id), recording TEXT NOT NULL,
                body BLOB NOT NULL, title BLOB, PRIMARY KEY(device_id,recording));
        """)
        device_columns = {row["name"] for row in self._db.execute("PRAGMA table_info(devices)")}
        if "last_seen" not in device_columns:
            self._db.execute("ALTER TABLE devices ADD COLUMN last_seen REAL NOT NULL DEFAULT 0")
            self._db.execute("UPDATE devices SET last_seen=created")
        # The agent each conversation belongs to. Conversations stored before
        # agents existed have none and belong to the default agent
        # (`assistant.agents.Agents.owner`).
        session_columns = {row["name"] for row in self._db.execute("PRAGMA table_info(sessions)")}
        if "agent_id" not in session_columns:
            self._db.execute("ALTER TABLE sessions ADD COLUMN agent_id TEXT")
        manifest_columns = {
            row["name"] for row in self._db.execute("PRAGMA table_info(recording_manifests)")
        }
        if "title" not in manifest_columns:
            self._db.execute("ALTER TABLE recording_manifests ADD COLUMN title BLOB")
        # When each message was written, whether it was a voice note
        # and the Mac's id for it. Rows from before stay without a time, as text.
        message_columns = {row["name"] for row in self._db.execute("PRAGMA table_info(messages)")}
        for column, definition in (
            ("created_at_ms", "INTEGER"),
            ("kind", "TEXT NOT NULL DEFAULT 'text'"),
            ("duration_ms", "INTEGER"),
            ("client_id", "TEXT"),
        ):
            if column not in message_columns:
                self._db.execute(f"ALTER TABLE messages ADD COLUMN {column} {definition}")
        self._db.execute("CREATE INDEX IF NOT EXISTS messages_session ON messages(session_id,id)")
        self._db.execute("PRAGMA user_version=9")
        self._db.executescript(GROUP_SCHEMA)
        group_columns = {
            row["name"] for row in self._db.execute("PRAGMA table_info(transcription_groups)")
        }
        if "language" not in group_columns:
            # The language each group was transcribed in. Groups stored
            # before it read as the configured language (`meetings.turns.read`).
            self._db.execute("ALTER TABLE transcription_groups ADD COLUMN language TEXT")
        self._db.executescript(safety.SCHEMA)
        self._db.executescript(MEETINGS_SCHEMA)
        # How far a running final pass got (`meeting_final_pass.status`).
        # Passes stored before it have none until they run again.
        pass_columns = {
            row["name"] for row in self._db.execute("PRAGMA table_info(recording_final_passes)")
        }
        if "progress" not in pass_columns:
            self._db.execute("ALTER TABLE recording_final_passes ADD COLUMN progress TEXT")
        # What a completed pass kept of the draft (`meeting_final_pass.status`).
        if "kept_draft" not in pass_columns:
            self._db.execute("ALTER TABLE recording_final_passes ADD COLUMN kept_draft TEXT")
        self._db.executescript(DICTIONARIES_SCHEMA)
        migrate_dictionaries(self._db)
        # Each agent's personality, skills and access groups.
        self._db.executescript(AGENT_CONFIG_SCHEMA)
        # Each agent's profile, the primary, the agents added from Orbit,
        # their accounts, and which conversation another was handed over from.
        self._db.executescript(ROSTER_SCHEMA)
        self._db.executescript(HANDOVER_SCHEMA)
        self._db.executescript(ATTACHMENTS_SCHEMA)
        self._db.executescript(_KEY_CHECK_SCHEMA)
        self._db.execute(
            "INSERT OR IGNORE INTO storage_key_check(id, body) VALUES (1, ?)",
            (self.encrypt(_KEY_CHECK_PLAINTEXT, _KEY_CHECK_CONTEXT),),
        )
        self.groups = GroupRepository(self)
        self.attachments = AttachmentRepository(self)

    def _verify_key(self) -> None:
        """Refuse a key this database was not written with (`_WRONG_KEY`).

        A database with the key check is verified against it; an older one
        against its first encrypted device name; an empty one takes this key.
        """
        tables = {
            row[0] for row in self._db.execute("SELECT name FROM sqlite_master WHERE type='table'")
        }
        try:
            if "storage_key_check" in tables:
                row = self._db.execute("SELECT body FROM storage_key_check WHERE id=1").fetchone()
                if row is not None:
                    if self.decrypt(row[0], _KEY_CHECK_CONTEXT) != _KEY_CHECK_PLAINTEXT:
                        raise ValueError(_WRONG_KEY)
                    return
            for table, query, context in _LEGACY_KEY_PROBES:
                if table in tables:
                    row = self._db.execute(query).fetchone()
                    if row is not None:
                        self.decrypt(row[1], context.format(row[0]))
                        return
        except InvalidTag:
            raise ValueError(_WRONG_KEY) from None

    def storage_status(self) -> dict:
        """What the database uses, its limit and what is left, with whether
        it is nearly full (`STORAGE_WARNING_PERCENT`) and what sets the limit:
        `disk` (`STORAGE_FREE_FLOOR_BYTES`) or `quota` (`storage_quota_bytes`)."""
        used = self.physical_bytes()
        disk_limit = max(0, (used + self._free_space() - STORAGE_FREE_FLOOR_BYTES) // 2)
        if self._quota is not None and self._quota <= disk_limit:
            limit, source = self._quota, "quota"
        else:
            limit, source = disk_limit, "disk"
        return {
            "used_bytes": used,
            "max_bytes": limit,
            "remaining_bytes": max(0, limit - used),
            "limit": source,
            "nearly_full": used * 100 >= limit * STORAGE_WARNING_PERCENT,
        }

    def storage_full(self) -> HubError:
        """The refusal for a write the disk itself refused, with its numbers."""
        try:
            free = self._free_space()
        except OSError:
            free = None
        numbers = {"used_bytes": self.physical_bytes()}
        if free is not None:
            numbers["free_bytes"] = free
        return HubError(
            "storage_full",
            "El disco del servidor está lleno y no se guardó nada; libera espacio en el servidor",
            numbers=numbers,
        )

    def recover_interrupted(self) -> None:
        """Only hub startup calls this; opening the admin CLI must not alter live turns."""
        with self.transaction() as db:
            # A restart leaves any in-flight turn (submitting, streaming or
            # interrupting) without a known outcome.
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
                # among them, so the cached revocation signal cannot outlive a
                # successful COMMIT. A revocation from the CLI, in another
                # process, is seen when the cache's TTL runs out.
                self._active_cache.clear()

    def _rollback(self) -> None:
        """Roll back what is open. SQLite has already rolled back a
        transaction it could not grow (`SQLITE_FULL`); a second ROLLBACK would
        raise "no transaction is active" and hide that cause."""
        if self._db.in_transaction:
            self._db.execute("ROLLBACK")

    def close(self) -> None:
        with self._lock:
            # A closed connection must not be reached through a stale cache.
            self._active_cache.clear()
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

    def physical_bytes(self) -> int:
        """Observed SQLite, WAL and shared-memory bytes, not just audio payloads."""
        total = 0
        for path in (self._path, Path(str(self._path) + "-wal"), Path(str(self._path) + "-shm")):
            try:
                total += path.stat().st_size
            except FileNotFoundError:
                continue  # SQLite can remove a checkpointed sidecar.
        return total

    def ensure_capacity(self, payload_bytes: int) -> None:
        """Conservative admission check, called inside the writer transaction.

        SQLite page/index/checkpoint growth is not a precise payload-size function.
        Reserve three copies plus 256 KiB; a filesystem quota is still required for
        a strict physical ceiling. Revocation and existing-job state changes remain
        available at the admission ceiling; no content is deleted or rotated.
        The limit is `storage_status`'s: the configured quota, or what the disk
        leaves (`STORAGE_FREE_FLOOR_BYTES`).
        """
        payload_bytes = max(0, payload_bytes)
        reserve = payload_bytes * 3 + 256 * 1024
        status = self.storage_status()
        used, limit = status["used_bytes"], status["max_bytes"]
        if used + reserve > limit:
            # The refusal says the rule that fired: what the write brought, what
            # admission reserves for it, what the database already uses, the
            # limit, what is left of it and what sets it (`disk` or `quota`).
            message = (
                "Cuota del servidor alcanzada; sube storage_quota_bytes o libera espacio. "
                if status["limit"] == "quota"
                else "Espacio del servidor agotado; libera espacio en el servidor. "
            )
            raise HubError(
                "quota_exceeded",
                message + "El Mac conserva su copia",
                numbers={
                    "bytes": payload_bytes,
                    "reserved_bytes": reserve,
                    "used_bytes": used,
                    "max_bytes": limit,
                    "remaining_bytes": status["remaining_bytes"],
                    "limit": status["limit"],
                },
            )

    @staticmethod
    def digest(value: str) -> str:
        return hashlib.sha256(value.encode()).hexdigest()

    def encrypt(self, data: bytes, context: str) -> bytes:
        nonce = secrets.token_bytes(12)
        return nonce + self._cipher.encrypt(nonce, data, context.encode())

    def decrypt(self, data: bytes, context: str) -> bytes:
        return self._cipher.decrypt(data[:12], data[12:], context.encode())

    def create_code(self, now: float, ttl: int = 300) -> str:
        if not 30 <= ttl <= 600:
            raise ValueError("Pairing TTL must be between 30 and 600 seconds")
        code = secrets.token_urlsafe(24)
        with self.transaction() as db:
            self.ensure_capacity(1024)
            db.execute("DELETE FROM pair_codes WHERE expires < ? OR used=1", (now,))
            db.execute(
                "INSERT INTO pair_codes(hash,expires) VALUES (?,?)", (self.digest(code), now + ttl)
            )
        return code

    def pair(self, code: str, name: str, now: float) -> dict:
        device_id, token = str(uuid4()), secrets.token_urlsafe(32)
        with self.transaction() as db:
            changed = db.execute(
                "UPDATE pair_codes SET used=1 WHERE hash=? AND used=0 AND expires>?",
                (self.digest(code), now),
            ).rowcount
            if not changed:
                raise HubError("invalid_pairing", "Código inválido, utilizado o caducado")
            self.ensure_capacity(len(name.encode()) + 1024)
            db.execute(
                "INSERT INTO devices(id,name,token_hash,created,last_seen) VALUES (?,?,?,?,?)",
                (
                    device_id,
                    self.encrypt(name.encode(), f"device:{device_id}"),
                    self.digest(token),
                    now,
                    now,
                ),
            )
        return {"device_id": device_id, "token": token}

    def authenticate(self, token: str) -> str:
        if not token or len(token) > 160:
            raise HubError("unauthorized", "Credencial de dispositivo no válida")
        with self._lock:
            row = self._db.execute(
                "SELECT id FROM devices WHERE token_hash=? AND revoked=0", (self.digest(token),)
            ).fetchone()
        if row is None:
            raise HubError("unauthorized", "Credencial de dispositivo no válida")
        return row["id"]

    def is_active(self, device_id: str) -> bool:
        cached = self._active_cache.get(device_id)
        if cached is not None:
            observed, value = cached
            if time.monotonic() - observed < self._active_ttl:
                return value
        with self._lock:
            row = self._db.execute(
                "SELECT revoked FROM devices WHERE id=?", (device_id,)
            ).fetchone()
        value = row is not None and not row["revoked"]
        self._active_cache[device_id] = (time.monotonic(), value)
        return value

    def revoke(self, device_id: str) -> None:
        with self.transaction() as db:
            if not db.execute("UPDATE devices SET revoked=1 WHERE id=?", (device_id,)).rowcount:
                raise HubError("not_found", "Dispositivo desconocido")

    def devices(self) -> list[dict]:
        with self._lock:
            rows = self._db.execute(
                "SELECT id,created,revoked FROM devices ORDER BY created"
            ).fetchall()
        return [dict(row) for row in rows]

    def active_devices(self) -> list[dict]:
        with self._lock:
            rows = self._db.execute(
                "SELECT id,name,created,COALESCE(last_seen,created) AS last_seen"
                " FROM devices WHERE revoked=0 ORDER BY created,id"
            ).fetchall()
        return [
            {
                "id": row["id"],
                "name": self.decrypt(row["name"], f"device:{row['id']}").decode("utf-8"),
                "paired_at": row["created"],
                "last_seen": row["last_seen"],
            }
            for row in rows
        ]

    def mark_device_seen(self, device_id: str, seen_at: float) -> None:
        with self.transaction() as db:
            db.execute(
                "UPDATE devices SET last_seen=MAX(last_seen,?) WHERE id=? AND revoked=0",
                (seen_at, device_id),
            )

    def create_session(
        self, device: str, agent: str, runtime: str, stored: str, epoch: str, now: float
    ) -> str:
        session_id = str(uuid4())
        with self.transaction() as db:
            self.ensure_capacity(1024)
            db.execute(
                "INSERT INTO sessions(id,device_id,runtime_id,stored_id,epoch,status,created,"
                "agent_id) VALUES (?,?,?,?,?,?,?,?)",
                (
                    session_id,
                    device,
                    runtime,
                    stored,
                    epoch,
                    "idle",
                    now,
                    agent,
                ),
            )
        return session_id

    def session(self, device: str, session_id: str) -> dict:
        with self._lock:
            row = self._db.execute(
                "SELECT * FROM sessions WHERE device_id=? AND id=?", (device, session_id)
            ).fetchone()
        if row is None:
            raise HubError("session_not_found", "Sesión no disponible para este dispositivo")
        return dict(row)

    def sessions_after(
        self, device: str, before: str | None, limit: int = SESSION_LIST_LIMIT
    ) -> tuple[list[dict], int, str | None]:
        """One page of the device's conversations, newest first, after `before`.

        Each row says when it started, when its last message was written (its
        start while it has none, or none with a time), how many it holds and
        its agent (None: stored before agents existed).
        """
        with self._lock:
            if before is None:
                anchor = None
            else:
                anchor = self._db.execute(
                    "SELECT created,id FROM sessions WHERE device_id=? AND id=?",
                    (device, before),
                ).fetchone()
                if anchor is None:
                    raise HubError(
                        "session_not_found", "Sesión no disponible para este dispositivo"
                    )
            # The extra row proves that another page exists. No stored row is cut.
            rows = self._db.execute(
                "SELECT s.id,s.created,s.agent_id,"
                " (SELECT created_at_ms FROM messages m WHERE m.session_id=s.id"
                "  ORDER BY m.id DESC LIMIT 1) AS last_ms,"
                " (SELECT COUNT(*) FROM messages m WHERE m.session_id=s.id) AS count"
                " FROM sessions s WHERE s.device_id=? "
                "AND (? IS NULL OR s.created < ? OR (s.created = ? AND s.id > ?)) "
                "ORDER BY s.created DESC, s.id ASC LIMIT ?",
                (
                    device,
                    before,
                    anchor["created"] if anchor is not None else None,
                    anchor["created"] if anchor is not None else None,
                    before,
                    limit + 1,
                ),
            ).fetchall()
            total = int(
                self._db.execute(
                    "SELECT COUNT(*) AS total FROM sessions WHERE device_id=?", (device,)
                ).fetchone()["total"]
            )
        page = [
            {
                "id": row["id"],
                "created_at_ms": round(row["created"] * 1000),
                "last_at_ms": row["last_ms"]
                if row["last_ms"] is not None
                else round(row["created"] * 1000),
                "message_count": row["count"],
                "agent_id": row["agent_id"],
            }
            for row in rows[:limit]
        ]
        next_cursor = page[-1]["id"] if len(rows) > limit else None
        return page, total, next_cursor

    def status(self, session_id: str, status: str) -> None:
        with self.transaction() as db:
            db.execute("UPDATE sessions SET status=? WHERE id=?", (status, session_id))

    def resume_session(
        self, device: str, session_id: str, runtime: str, stored: str, epoch: str
    ) -> None:
        with self.transaction() as db:
            cursor = db.execute(
                "UPDATE sessions SET runtime_id=?, stored_id=?, epoch=? WHERE device_id=? AND id=?",
                (runtime, stored, epoch, device, session_id),
            )
            if cursor.rowcount != 1:
                raise HubError("session_not_found", "Sesión no disponible para este dispositivo")

    def append_message(
        self,
        session_id: str,
        role: str,
        text: str,
        *,
        now_ms: int,
        kind: str = "text",
        duration_ms: int | None = None,
        client_id: str | None = None,
        attachments: list[str] | tuple[str, ...] = (),
    ) -> None:
        """Append one message; `attachments`, the ids its turn named, are bound
        to it in the same transaction (`AttachmentRepository.bind`)."""
        encoded = text.encode()
        with self.transaction() as db:
            self.ensure_capacity(len(encoded) + 256)
            cursor = db.execute(
                "INSERT INTO messages(session_id,role,body,created_at_ms,kind,duration_ms,"
                "client_id) VALUES (?,?,?,?,?,?,?)",
                (
                    session_id,
                    role,
                    self.encrypt(encoded, f"message:{session_id}:{role}"),
                    now_ms,
                    kind,
                    duration_ms,
                    client_id,
                ),
            )
            if attachments:
                self.attachments.bind(db, session_id, cursor.lastrowid, list(attachments))

    def session_messages(
        self, device: str, session_id: str, before: int | None, limit: int
    ) -> dict:
        """One page of a conversation, oldest first, ending before `before`.

        Newest messages first fill the page up to `limit` and
        `SESSION_MESSAGES_PAGE_BYTES` of JSON, whole; `next` reads the ones
        before and `total` counts the conversation's. A conversation of
        another device is `session_not_found`. `agent_id` is the session's
        (None: stored before agents existed).
        """
        session = self.session(device, session_id)
        budget = SESSION_MESSAGES_PAGE_BYTES - _PAGE_ENVELOPE_BYTES
        messages: list[dict] = []
        with self._lock:
            # Sizes first: a long conversation never decrypts more than a page.
            total = self._db.execute(
                "SELECT COUNT(*) FROM messages WHERE session_id=?", (session_id,)
            ).fetchone()[0]
            rows = self._db.execute(
                "SELECT id,role,length(body) AS size,created_at_ms,kind,duration_ms,client_id"
                " FROM messages WHERE session_id=? AND (? IS NULL OR id < ?)"
                " ORDER BY id DESC LIMIT ?",
                (session_id, before, before, limit + 1),
            ).fetchall()
            # Each message's attachments, listed (not their content).
            attached = self.attachments.of_messages(self._db, [row["id"] for row in rows[:limit]])
            for row in rows[:limit]:
                body = self._db.execute(
                    "SELECT body FROM messages WHERE id=?", (row["id"],)
                ).fetchone()[0]
                message = {
                    "id": row["id"],
                    "role": row["role"],
                    "text": self.decrypt(body, f"message:{session_id}:{row['role']}").decode(),
                    "at_ms": row["created_at_ms"],
                    "kind": row["kind"],
                }
                if row["duration_ms"] is not None:
                    message["duration_ms"] = row["duration_ms"]
                if row["client_id"] is not None:
                    message["client_id"] = row["client_id"]
                if row["id"] in attached:
                    message["attachments"] = attached[row["id"]]
                # As the route writes it (FastAPI: compact, UTF-8), plus its comma.
                size = (
                    len(json.dumps(message, ensure_ascii=False, separators=(",", ":")).encode()) + 1
                )
                if size > budget:
                    if not messages:
                        raise HubError(
                            "session_message_too_large",
                            "Un mensaje de la conversación no cabe en una página",
                            numbers={
                                "bytes": size,
                                "max_bytes": SESSION_MESSAGES_PAGE_BYTES - _PAGE_ENVELOPE_BYTES,
                            },
                        )
                    break
                messages.append(message)
                budget -= size
        older = len(messages) < len(rows)
        messages.reverse()
        return {
            "messages": messages,
            "next": messages[0]["id"] if older and messages else None,
            "total": total,
            "agent_id": session["agent_id"],
        }

    def reserve_operation(self, device: str, operation_id: str, payload: dict) -> dict | None:
        fingerprint = self.operation_fingerprint(payload)
        with self.transaction() as db:
            if db.execute(
                "SELECT 1 FROM safety_operations WHERE device_id=? AND operation_id=?",
                (device, operation_id),
            ).fetchone():
                raise HubError("operation_conflict", "El ID ya pertenece a un control de seguridad")
            existing = db.execute(
                "SELECT * FROM operations WHERE device_id=? AND id=?", (device, operation_id)
            ).fetchone()
            if existing:
                if existing["fingerprint"] != fingerprint:
                    raise HubError("operation_conflict", "El ID ya pertenece a otra operación")
                if existing["response"]:
                    return json.loads(
                        self.decrypt(existing["response"], f"operation:{device}:{operation_id}")
                    )
                raise HubError(
                    "outcome_unknown", "Operación ya recibida; no se repetirá automáticamente"
                )
            self.ensure_capacity(OPERATION_RECEIPT_RESERVE_BYTES)
            db.execute(
                "INSERT INTO operations(id,device_id,fingerprint,status) VALUES (?,?,?,'pending')",
                (operation_id, device, fingerprint),
            )
        return None

    def release_operation(self, device: str, operation_id: str) -> None:
        """Give back the id of an operation refused before it kept or sent
        anything, so the same message can be sent again (`ws.py`). Only a
        reservation still pending: a receipt is never removed."""
        with self.transaction() as db:
            db.execute(
                "DELETE FROM operations WHERE device_id=? AND id=? AND status='pending'",
                (device, operation_id),
            )

    def operation_fingerprint(self, payload: dict) -> str:
        canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"))
        return hmac.new(self._fingerprint_key, canonical.encode(), "sha256").hexdigest()

    def finish_operation(self, device: str, operation_id: str, response: dict) -> None:
        """Record an operation's receipt; the quota was reserved at admission.

        The operation may already have been applied upstream: refusing here
        would tell the client the operation failed when it did not, and lose
        the receipt of an effect that cannot be retried.
        """
        body = self.encrypt(json.dumps(response).encode(), f"operation:{device}:{operation_id}")
        with self.transaction() as db:
            db.execute(
                "UPDATE operations SET status='completed',response=? WHERE device_id=? AND id=?",
                (body, device, operation_id),
            )

    def operation(self, device: str, operation_id: str) -> dict | None:
        with self._lock:
            row = self._db.execute(
                "SELECT status,response FROM operations WHERE device_id=? AND id=?",
                (device, operation_id),
            ).fetchone()
        if row is None:
            return None
        response = (
            json.loads(self.decrypt(row["response"], f"operation:{device}:{operation_id}"))
            if row["response"]
            else None
        )
        return {"status": row["status"], "response": response}

    def put_chunk(
        self,
        device: str,
        chunk_id: str,
        recording: str,
        track: str,
        sequence: int,
        digest: str,
        body: bytes,
        metadata: dict | None = None,
        *,
        received_at: float = 0,
    ) -> dict:
        if hashlib.sha256(body).hexdigest() != digest:
            raise HubError("hash_mismatch", "El hash no coincide con los bytes recibidos")
        with self.transaction() as db:
            active = db.execute("SELECT revoked FROM devices WHERE id=?", (device,)).fetchone()
            if active is None or active["revoked"]:
                raise HubError("revoked", "Dispositivo revocado")
            row = db.execute(
                "SELECT * FROM chunks WHERE device_id=? AND id=?", (device, chunk_id)
            ).fetchone()
            if (
                row is None
                and db.execute(
                    "SELECT 1 FROM recording_manifests WHERE device_id=? AND recording=?",
                    (device, recording),
                ).fetchone()
            ):
                raise HubError(
                    "recording_finalized", "La captura está cerrada; no admite nuevos fragmentos"
                )
            if row:
                job = db.execute(
                    "SELECT metadata FROM transcription_jobs WHERE device_id=? AND chunk_id=?",
                    (device, chunk_id),
                ).fetchone()
                old_metadata = (
                    json.loads(self.decrypt(job["metadata"], f"metadata:{device}:{chunk_id}"))
                    if job
                    else None
                )
                if old_metadata != metadata:
                    raise HubError(
                        "chunk_conflict", "Metadatos de captura distintos para el mismo fragmento"
                    )
                if (row["sha256"], row["recording"], row["track"], row["sequence"]) != (
                    digest,
                    recording,
                    track,
                    sequence,
                ):
                    raise HubError(
                        "chunk_conflict",
                        "El fragmento ya existe con contenido o metadatos distintos",
                    )
            else:
                self.ensure_capacity(len(body) + len(json.dumps(metadata).encode()) + 1024)
                try:
                    db.execute(
                        "INSERT INTO chunks VALUES (?,?,?,?,?,?,?,?)",
                        (
                            chunk_id,
                            device,
                            recording,
                            track,
                            sequence,
                            digest,
                            self.encrypt(body, f"chunk:{device}:{chunk_id}"),
                            len(body),
                        ),
                    )
                except sqlite3.IntegrityError as exc:
                    raise HubError(
                        "chunk_conflict", "La secuencia ya pertenece a otro fragmento"
                    ) from exc
                if metadata is not None:
                    db.execute(
                        "INSERT INTO transcription_jobs(device_id,chunk_id,metadata) VALUES (?,?,?)",
                        (
                            device,
                            chunk_id,
                            self.encrypt(
                                json.dumps(metadata, sort_keys=True).encode(),
                                f"metadata:{device}:{chunk_id}",
                            ),
                        ),
                    )
                    db.execute(
                        "INSERT INTO chunk_receipts VALUES (?,?,?)", (device, chunk_id, received_at)
                    )
        return {"chunk_id": chunk_id, "sha256": digest, "durable": True}

    def claim_transcription(self, now: float) -> dict | None:
        return self.groups.claim(now)

    def finish_transcription(self, job: dict, text: str, language: str | None = None) -> None:
        self.groups.finish(job, text, language)

    def fail_transcription(self, job: dict, code: str, now: float) -> None:
        self.groups.fail(job, code, now)

    def retry_transcription(self, device: str, recording: str, now: float) -> int:
        return self.groups.retry(device, recording, now)

    def finalize_recording(
        self,
        device: str,
        recording: str,
        manifest: dict,
        *,
        final_pass: bool = False,
        final_pass_error: dict | None = None,
    ) -> dict:
        # The HTTP close asks for the final pass atomically with the manifest.
        # Importers/fixtures of historical recordings retain their draft unless
        # explicitly requested.
        tracks = {track["track"]: track for track in manifest["tracks"]}
        if (
            len(tracks) != len(manifest["tracks"])
            or manifest["finalized_at_ms"] < manifest["started_at_ms"]
        ):
            raise HubError(
                "invalid_manifest", "Pistas duplicadas o reloj de cierre anterior al inicio"
            )
        with self.transaction() as db:
            active = db.execute("SELECT revoked FROM devices WHERE id=?", (device,)).fetchone()
            if active is None or active["revoked"]:
                raise HubError("revoked", "Dispositivo revocado")
            previous = db.execute(
                "SELECT body FROM recording_manifests WHERE device_id=? AND recording=?",
                (device, recording),
            ).fetchone()
            if previous:
                old = json.loads(self.decrypt(previous["body"], f"manifest:{device}:{recording}"))
                # A field the manifest does not carry reads as its default, so
                # a retried closure matches whichever of them it left out.
                for field, default in (("title", None), ("kind", "meeting"), ("event", None)):
                    old.setdefault(field, default)
                    manifest.setdefault(field, default)
                if old != manifest:
                    raise HubError(
                        "manifest_conflict",
                        "La grabación ya tiene un manifiesto de cierre distinto",
                    )
            else:
                rows = db.execute(
                    """SELECT c.id,c.track,c.sequence,j.metadata FROM chunks c
                    LEFT JOIN transcription_jobs j ON j.device_id=c.device_id AND j.chunk_id=c.id
                    WHERE c.device_id=? AND c.recording=? ORDER BY c.track,c.sequence""",
                    (device, recording),
                ).fetchall()
                if {row["track"] for row in rows} != set(tracks):
                    raise HubError(
                        "manifest_mismatch",
                        "Las pistas del manifiesto no coinciden con los fragmentos durables",
                    )
                for name, declared in tracks.items():
                    actual = [row for row in rows if row["track"] == name]
                    count = declared["chunk_count"]
                    # The hub holds every sequence from 0 to the last one,
                    # except those the Mac declared it will never get.
                    missing = set(declared.get("missing_sequences") or ())
                    if declared["last_sequence"] + 1 - len(missing) != count:
                        raise HubError(
                            "manifest_mismatch",
                            "Recuento o secuencias no coinciden; faltan fragmentos confirmados",
                        )
                    expected = [
                        sequence
                        for sequence in range(declared["last_sequence"] + 1)
                        if sequence not in missing
                    ]
                    if len(actual) != count or [row["sequence"] for row in actual] != expected:
                        raise HubError(
                            "manifest_mismatch",
                            "Recuento o secuencias no coinciden; faltan fragmentos confirmados",
                        )
                    frames = 0
                    for row in actual:
                        metadata = (
                            json.loads(
                                self.decrypt(row["metadata"], f"metadata:{device}:{row['id']}")
                            )
                            if row["metadata"]
                            else {}
                        )
                        if not isinstance(metadata.get("frames"), int):
                            raise HubError(
                                "manifest_mismatch",
                                "No se puede acreditar el número de frames de un fragmento",
                            )
                        frames += metadata["frames"]
                    if frames != declared["total_frames"]:
                        raise HubError("manifest_mismatch", "El total de frames no coincide")
                self.ensure_capacity(len(json.dumps(manifest).encode()) + 1024)
                db.execute(
                    "INSERT INTO recording_manifests(device_id,recording,body) VALUES (?,?,?)",
                    (
                        device,
                        recording,
                        self.encrypt(
                            json.dumps(manifest, sort_keys=True).encode(),
                            f"manifest:{device}:{recording}",
                        ),
                    ),
                )
                if final_pass:
                    db.execute(
                        "INSERT OR IGNORE INTO recording_final_passes(device_id,recording,state,error) "
                        "VALUES (?,?,?,?)",
                        (
                            device,
                            recording,
                            "failed" if final_pass_error else "queued",
                            json.dumps(final_pass_error) if final_pass_error else None,
                        ),
                    )
        transcript = self.groups.capture(device, recording)
        return {
            "recording_id": recording,
            "finalized": True,
            "durable": True,
            "capture_complete": transcript["capture_complete"],
            "transcript_ready": transcript["transcript_ready"],
        }
