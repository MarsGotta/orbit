"""Paired devices and the codes that pair them."""

import hashlib
import json
import secrets
import time
from uuid import uuid4

from ..device_groups import handled
from ..protocol import HubError


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


class DeviceRepository:
    def __init__(self, store):
        self.store = store
        # One query per delta flooded the loop with `SELECT revoked` against
        # SQLite. A short TTL absorbs bursts without outliving an explicit revoke:
        # an entry read before the last commit is not used (`Store.commits`).
        self.active_ttl = 1.0
        self._active: dict[str, tuple[float, int, bool]] = {}

    def create_code(self, now: float, ttl: int = 300) -> str:
        if not 30 <= ttl <= 600:
            raise ValueError("Pairing TTL must be between 30 and 600 seconds")
        code = secrets.token_urlsafe(24)
        with self.store.transaction() as db:
            self.store.ensure_capacity(1024)
            db.execute("DELETE FROM pair_codes WHERE expires < ? OR used=1", (now,))
            db.execute(
                "INSERT INTO pair_codes(hash,expires) VALUES (?,?)", (digest(code), now + ttl)
            )
        return code

    def pair(
        self,
        code: str,
        name: str,
        now: float,
        *,
        kind: str = "mac",
        capabilities: list[str] | None = None,
        installation_id: str | None = None,
    ) -> dict:
        """Pair a device with what it said it is (`protocol.PairRequest`
        validated it). `capabilities` None is every group, kept as NULL; a
        list is kept as the device said it, `[]` included."""
        device_id, token = str(uuid4()), secrets.token_urlsafe(32)
        with self.store.transaction() as db:
            changed = db.execute(
                "UPDATE pair_codes SET used=1 WHERE hash=? AND used=0 AND expires>?",
                (digest(code), now),
            ).rowcount
            if not changed:
                raise HubError("invalid_pairing", "Invalid, used or expired code")
            self.store.ensure_capacity(len(name.encode()) + 1024)
            db.execute(
                "INSERT INTO devices(id,name,token_hash,created,last_seen,kind,capabilities,"
                "installation_id) VALUES (?,?,?,?,?,?,?,?)",
                (
                    device_id,
                    self.store.encrypt(name.encode(), f"device:{device_id}"),
                    digest(token),
                    now,
                    now,
                    kind,
                    None if capabilities is None else json.dumps(capabilities),
                    installation_id,
                ),
            )
        return {"device_id": device_id, "token": token}

    def authenticate(self, token: str) -> str:
        if not token or len(token) > 160:
            raise HubError("unauthorized", "Invalid device credential")
        with self.store.reading() as db:
            row = db.execute(
                "SELECT id FROM devices WHERE token_hash=? AND revoked=0", (digest(token),)
            ).fetchone()
        if row is None:
            raise HubError("unauthorized", "Invalid device credential")
        return row["id"]

    def is_active(self, device_id: str) -> bool:
        cached = self._active.get(device_id)
        if cached is not None:
            observed, commits, value = cached
            if commits == self.store.commits and time.monotonic() - observed < self.active_ttl:
                return value
        commits = self.store.commits
        with self.store.reading() as db:
            row = db.execute("SELECT revoked FROM devices WHERE id=?", (device_id,)).fetchone()
        value = row is not None and not row["revoked"]
        self._active[device_id] = (time.monotonic(), commits, value)
        return value

    def revoke(self, device_id: str) -> None:
        with self.store.transaction() as db:
            if not db.execute("UPDATE devices SET revoked=1 WHERE id=?", (device_id,)).rowcount:
                raise HubError("not_found", "Unknown device")

    def all(self) -> list[dict]:
        """Every device ever paired, revoked ones too, as the CLI lists them."""
        with self.store.reading() as db:
            rows = db.execute("SELECT id,created,revoked FROM devices ORDER BY created").fetchall()
        return [dict(row) for row in rows]

    def active(self) -> list[dict]:
        """The devices not revoked, with what each is and handles; never
        their installation's id, which nobody reads yet."""
        with self.store.reading() as db:
            rows = db.execute(
                "SELECT id,name,created,COALESCE(last_seen,created) AS last_seen,kind,capabilities"
                " FROM devices WHERE revoked=0 ORDER BY created,id"
            ).fetchall()
        return [
            {
                "id": row["id"],
                "name": self.store.decrypt(row["name"], f"device:{row['id']}").decode("utf-8"),
                "paired_at": row["created"],
                "last_seen": row["last_seen"],
                "kind": row["kind"],
                "capabilities": _groups(row["capabilities"]),
            }
            for row in rows
        ]

    def handling(self, device_id: str) -> tuple[str, list[str]] | None:
        """What `device_id` is and the groups it handles; None when unknown."""
        with self.store.reading() as db:
            row = db.execute(
                "SELECT kind,capabilities FROM devices WHERE id=?", (device_id,)
            ).fetchone()
        return None if row is None else (row["kind"], _groups(row["capabilities"]))

    def mark_seen(self, device_id: str, seen_at: float) -> None:
        with self.store.transaction() as db:
            db.execute(
                "UPDATE devices SET last_seen=MAX(last_seen,?) WHERE id=? AND revoked=0",
                (seen_at, device_id),
            )


def _groups(stored: str | None) -> list[str]:
    """The groups a row keeps: NULL, a device that said none, is every group."""
    return handled(None if stored is None else json.loads(stored))
