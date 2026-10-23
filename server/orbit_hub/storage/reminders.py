"""«Recordatorio personal»: what the owner wants to remember for a meeting,
kept so that every Mac paired with this hub shows it. It is the owner's, not a
device's (each Mac is the same person), and it never goes into a note.

A reminder is named by its meeting's `title_digest` (`meetings.series`), never
by its title; the row is keyed by an HMAC of that digest with the storage key,
so a copy of the database alone cannot test titles against it, and its text is
encrypted. The latest edit wins, by the time the Mac wrote it: a Mac that was
offline sends its edit when it can, and an older one never replaces a newer.
"""

import hmac

# A reminder is a line or two written before a meeting («Llevar las cifras del
# trimestre»): 1 000 characters is several of them. It protects the row and the
# Mac's read of it; it is not a limit on notes, which have their own. The
# field's copy is `MAX_REMINDER_CHARS` in the interface (contracts/constants.json).
MAX_REMINDER_CHARS = 1_000

SCHEMA = """
    CREATE TABLE IF NOT EXISTS personal_reminders (
        key TEXT PRIMARY KEY, body BLOB NOT NULL, edited_at_ms INTEGER NOT NULL);
"""


class ReminderRepository:
    def __init__(self, store, key_secret: bytes):
        self.store = store
        self._key_secret = key_secret

    def _row_key(self, key: str) -> str:
        return hmac.new(self._key_secret, key.encode(), "sha256").hexdigest()

    def get(self, key: str) -> dict | None:
        """The reminder of the meeting `key`, or None when nobody wrote one."""
        row_key = self._row_key(key)
        with self.store.reading() as db:
            row = db.execute(
                "SELECT body, edited_at_ms FROM personal_reminders WHERE key=?", (row_key,)
            ).fetchone()
        return None if row is None else self._reminder(row_key, row)

    def save(self, key: str, text: str, edited_at_ms: int) -> dict:
        """Keeps this edit unless a later one is kept already; returns the one kept."""
        row_key = self._row_key(key)
        encoded = text.encode()
        with self.store.transaction() as db:
            row = db.execute(
                "SELECT body, edited_at_ms FROM personal_reminders WHERE key=?", (row_key,)
            ).fetchone()
            if row is not None and row["edited_at_ms"] >= edited_at_ms:
                return self._reminder(row_key, row)
            self.store.ensure_capacity(len(encoded) + 128)
            db.execute(
                "INSERT INTO personal_reminders(key, body, edited_at_ms) VALUES (?,?,?)"
                " ON CONFLICT(key) DO UPDATE SET body=excluded.body,"
                " edited_at_ms=excluded.edited_at_ms",
                (row_key, self.store.encrypt(encoded, f"reminder:{row_key}"), edited_at_ms),
            )
        return {"text": text, "edited_at_ms": edited_at_ms}

    def _reminder(self, row_key: str, row) -> dict:
        text = self.store.decrypt(row["body"], f"reminder:{row_key}").decode()
        return {"text": text, "edited_at_ms": row["edited_at_ms"]}
