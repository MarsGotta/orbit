"""Recording finalization and the transcript retry."""

from fastapi import Request
from pydantic import ValidationError

from ..errors import HubError
from ..identifiers import canonical_uuid
from ..log import in_store
from ..notes.models import EventContext
from ..protocol import RecordingManifest

# A closing manifest: the tracks' counts, the title and the
# calendar event the meeting was started from. The event is what makes it
# large: up to 200 invitees of 160-character names and 254-character
# addresses, a 512-character title and the series key, about 83 000
# characters; with every character escaped as `\uXXXX` (six bytes) that is
# just under 500 KiB. 512 KiB admits the largest valid manifest and refuses
# anything else with its size before it is parsed.
MAX_MANIFEST_BYTES = 512 * 1024


def mount(
    app,
    store,
    identity,
    bounded_body,
    store_executor,
    prepare_signal,
    now,
    notes,
    recording_changed,
    diarization_configured,
):
    @app.post("/v1/recordings/{recording_id}/finalize")
    async def finalize_recording(recording_id: str, request: Request):
        """Close a recording; a meeting's note is asked for at once."""
        device = identity(request)
        try:
            recording_id = canonical_uuid(recording_id)
            manifest = RecordingManifest.model_validate_json(
                await bounded_body(request, MAX_MANIFEST_BYTES, "recording")
            )
            if manifest.event is not None:
                manifest = manifest.model_copy(
                    update={
                        "event": EventContext.model_validate(manifest.event).model_dump(mode="json")
                    }
                )
        except (ValidationError, ValueError) as exc:
            raise HubError("invalid_manifest", "Manifiesto de cierre no válido") from exc
        stored = manifest.model_dump(mode="json")
        # A track with nothing missing is stored as a 0.10.1 Mac wrote it, so
        # its retried closure matches the closure already kept.
        for track in stored["tracks"]:
            if not track.get("missing_sequences"):
                track.pop("missing_sequences", None)
        # Likewise a closure never muted, as a Mac before «Silenciar» wrote it.
        if not stored["muted_spans"]:
            stored.pop("muted_spans")
        outcome = await in_store(
            store_executor,
            lambda: store.recordings.finalize(
                device,
                recording_id,
                stored,
                final_pass=True,
                final_pass_error=None
                if diarization_configured
                else {"code": "diarization_not_configured", "phase": "prepare"},
            ),
        )
        prepare_signal.set()
        if manifest.kind == "meeting":
            await in_store(
                store_executor,
                lambda: notes.repository.request(
                    device, recording_id, now(), notes.configured, regenerate=False
                ),
            )
            notes.signal.set()
        recording_changed(device, recording_id)
        return outcome

    @app.post("/v1/recordings/{recording_id}/transcript/retry")
    async def retry_recording_transcript(recording_id: str, request: Request):
        device = identity(request)
        try:
            recording_id = canonical_uuid(recording_id)
        except ValueError as exc:
            raise HubError("invalid_recording", "ID de grabación no válido") from exc
        retry_at = now()
        requeued = await in_store(
            store_executor, store.groups.retry, device, recording_id, retry_at
        )
        if requeued:
            prepare_signal.set()
        return {"requeued": requeued}
