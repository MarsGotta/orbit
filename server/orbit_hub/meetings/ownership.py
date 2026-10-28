"""Resolve personal-hub meetings without changing their device-keyed storage.

Callers authenticate before using this module. Revocation removes a device's
access, never the person's meetings. Upload and capture do not use this resolver.
"""

from ..errors import HubError


def device_for(db, recording: str, fallback: str) -> str:
    """The original device, including for repeat deletes; unknown ids keep the
    caller's key so discarding a local recording before upload still works.

    A legacy collision is refused rather than reading or editing the wrong
    meeting: older storage permits the same recording UUID on two devices.
    """
    rows = db.execute(
        "SELECT id AS device_id FROM devices WHERE "
        "EXISTS(SELECT 1 FROM recording_manifests m WHERE m.device_id=devices.id AND m.recording=?) "
        "OR EXISTS(SELECT 1 FROM chunks c WHERE c.device_id=devices.id AND c.recording=?) "
        "OR EXISTS(SELECT 1 FROM deleted_recordings t WHERE t.device_id=devices.id AND t.recording=?) "
        "LIMIT 2",
        (recording, recording, recording),
    ).fetchall()
    if len(rows) > 1:
        raise HubError(
            "recording_id_ambiguous",
            "Two recordings have the same identifier on different Macs",
            http_status=409,
            numbers={"matches_at_least": len(rows)},
        )
    return rows[0]["device_id"] if rows else fallback


def recorded_on(store, db, device: str) -> dict:
    """Add provenance when more than one Mac has been paired, revoked ones included."""
    if db.execute("SELECT count(*) FROM devices").fetchone()[0] < 2:
        return {}
    row = db.execute("SELECT name FROM devices WHERE id=?", (device,)).fetchone()
    return {
        "recorded_on": {
            "device_id": device,
            "name": store.decrypt(row["name"], f"device:{device}").decode(),
        }
    }
