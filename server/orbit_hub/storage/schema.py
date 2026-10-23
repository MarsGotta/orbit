"""The database's one schema, its migrations and the check of its key.

Every repository's tables are created here, in one order, on one connection:
the order is the one the hub has always opened its database with, and
`PRAGMA user_version` marks the same point in it. Nothing here reads or
writes a row of a domain; the repositories do.
"""

from collections.abc import Callable

from cryptography.exceptions import InvalidTag

from .. import safety
from ..assistant.agent_config import SCHEMA as AGENT_CONFIG_SCHEMA
from ..assistant.handover import SCHEMA as HANDOVER_SCHEMA
from ..assistant.roster import SCHEMA as ROSTER_SCHEMA
from ..attachments import SCHEMA as ATTACHMENTS_SCHEMA
from ..group_storage import SCHEMA as GROUP_SCHEMA
from ..meetings.deletion import SCHEMA as DELETIONS_SCHEMA
from ..meetings.dictionaries import SCHEMA as DICTIONARIES_SCHEMA
from ..meetings.dictionaries import migrate as migrate_dictionaries
from ..meetings.repository import SCHEMA as MEETINGS_SCHEMA
from .reminders import SCHEMA as REMINDERS_SCHEMA

# The key check: one row the hub encrypts with its key when it first opens a
# database, so a database never opens with another key (which would leave it
# with two: old rows unreadable now, new ones unreadable with the old key).
KEY_CHECK_PLAINTEXT = b"orbit.storage.key-check.v1"
KEY_CHECK_CONTEXT = "storage:key-check"
_KEY_CHECK_SCHEMA = """
    CREATE TABLE IF NOT EXISTS storage_key_check (
        id INTEGER PRIMARY KEY CHECK (id = 1), body BLOB NOT NULL);
"""
# Encrypted columns an older database (written before the key check) can be
# verified against: the first row of whichever exists.
_LEGACY_KEY_PROBES = (
    ("devices", "SELECT id, name FROM devices ORDER BY rowid LIMIT 1", "device:{0}"),
)
WRONG_KEY = (
    "The storage key is not the one this database was written with; opening it "
    "would leave it with two keys. Put back the key this data directory was "
    'created with (see docs/self-hosting.md, "Backups")'
)

# The tables of devices, conversations, operation receipts and audio.
_CORE_SCHEMA = """
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
        """


def verify_key(db, decrypt: Callable[[bytes, str], bytes]) -> None:
    """Refuse a key this database was not written with (`WRONG_KEY`).

    A database with the key check is verified against it; an older one
    against its first encrypted device name; an empty one takes this key.
    """
    tables = {row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    try:
        if "storage_key_check" in tables:
            row = db.execute("SELECT body FROM storage_key_check WHERE id=1").fetchone()
            if row is not None:
                if decrypt(row[0], KEY_CHECK_CONTEXT) != KEY_CHECK_PLAINTEXT:
                    raise ValueError(WRONG_KEY)
                return
        for table, query, context in _LEGACY_KEY_PROBES:
            if table in tables:
                row = db.execute(query).fetchone()
                if row is not None:
                    decrypt(row[1], context.format(row[0]))
                    return
    except InvalidTag:
        raise ValueError(WRONG_KEY) from None


def migrate(db, encrypt: Callable[[bytes, str], bytes]) -> None:
    """Create what is missing and add the columns older databases lack."""
    db.executescript(_CORE_SCHEMA)
    _migrate_core(db)
    db.execute("PRAGMA user_version=9")
    db.executescript(GROUP_SCHEMA)
    if "language" not in _columns(db, "transcription_groups"):
        # The language each group was transcribed in. Groups stored
        # before it read as the configured language (`meetings.turns.read`).
        db.execute("ALTER TABLE transcription_groups ADD COLUMN language TEXT")
    db.executescript(safety.SCHEMA)
    db.executescript(MEETINGS_SCHEMA)
    # How far a running final pass got (`meeting_final_pass.status`).
    # Passes stored before it have none until they run again.
    if "progress" not in _columns(db, "recording_final_passes"):
        db.execute("ALTER TABLE recording_final_passes ADD COLUMN progress TEXT")
    # What a completed pass kept of the draft (`meeting_final_pass.status`).
    if "kept_draft" not in _columns(db, "recording_final_passes"):
        db.execute("ALTER TABLE recording_final_passes ADD COLUMN kept_draft TEXT")
    db.executescript(DICTIONARIES_SCHEMA)
    migrate_dictionaries(db)
    # The recordings each device deleted, so that none comes back half.
    db.executescript(DELETIONS_SCHEMA)
    # Each agent's personality, skills and access groups.
    db.executescript(AGENT_CONFIG_SCHEMA)
    # Each agent's profile, the primary, the agents added from Orbit,
    # their accounts, and which conversation another was handed over from.
    db.executescript(ROSTER_SCHEMA)
    db.executescript(HANDOVER_SCHEMA)
    db.executescript(ATTACHMENTS_SCHEMA)
    # «Recordatorio personal», the owner's on every Mac.
    db.executescript(REMINDERS_SCHEMA)
    db.executescript(_KEY_CHECK_SCHEMA)
    db.execute(
        "INSERT OR IGNORE INTO storage_key_check(id, body) VALUES (1, ?)",
        (encrypt(KEY_CHECK_PLAINTEXT, KEY_CHECK_CONTEXT),),
    )


def _migrate_core(db) -> None:
    if "last_seen" not in _columns(db, "devices"):
        db.execute("ALTER TABLE devices ADD COLUMN last_seen REAL NOT NULL DEFAULT 0")
        db.execute("UPDATE devices SET last_seen=created")
    # The agent each conversation belongs to. Conversations stored before
    # agents existed have none and belong to the default agent
    # (`assistant.agents.Agents.owner`).
    if "agent_id" not in _columns(db, "sessions"):
        db.execute("ALTER TABLE sessions ADD COLUMN agent_id TEXT")
    if "title" not in _columns(db, "recording_manifests"):
        db.execute("ALTER TABLE recording_manifests ADD COLUMN title BLOB")
    # When each message was written, whether it was a voice note
    # and the Mac's id for it. Rows from before stay without a time, as text.
    message_columns = _columns(db, "messages")
    for column, definition in (
        ("created_at_ms", "INTEGER"),
        ("kind", "TEXT NOT NULL DEFAULT 'text'"),
        ("duration_ms", "INTEGER"),
        ("client_id", "TEXT"),
        # An answer's summary of what its turn did (`runtime.work`), encrypted.
        # Answers stored before it have none, and show none.
        ("work", "BLOB"),
    ):
        if column not in message_columns:
            db.execute(f"ALTER TABLE messages ADD COLUMN {column} {definition}")
    db.execute("CREATE INDEX IF NOT EXISTS messages_session ON messages(session_id,id)")


def _columns(db, table: str) -> set[str]:
    return {row["name"] for row in db.execute(f"PRAGMA table_info({table})")}
