"""Write the sample database with today's hub, only through its commands and routes.

Run by hand, and only on purpose: the file the repository keeps is the one
today's hub wrote, and the kit's tests compare what a hub reads against the
manifest written beside it, which keeps no device token: the kit pairs
its own device on its copy. Every name and text is invented.

    server/.venv/bin/python contracts/kit/make_golden.py

(from the repository's root) with the hub's Python, which has `httpx` and the hub.
`ORBIT_KIT_HUB` names another hub to write it, as for the tests. The test key
is kept when it exists, so a regeneration changes only the database and the
manifest.
"""

import hashlib
import json
import secrets
import shutil
import sys
import tempfile
from pathlib import Path

import httpx
from kit_hub import (
    DATABASE_NAME,
    GOLDEN,
    MANIFEST,
    TEST_KEY,
    device_client,
    pair_device,
    prepare,
    run,
    serving,
)

DEVICE_NAME = "Cedar test Mac"
# A reminder is keyed by its meeting's `title_digest`, the SHA-256 of the title.
TITLE_DIGEST = hashlib.sha256(b"Cedar weekly sync").hexdigest()
REMINDER = {
    "text": "Bring the Cedar roadmap draft to the weekly sync",
    "edited_at_ms": 1767225600000,
}
DICTIONARY = {"id": "5d8f2c1e-7a3b-4e6f-9c0d-1b2a3c4d5e6f", "name": "Cedar glossary"}
NOTE_TEMPLATE = {
    "id": "cedar-standup",
    "name": "Cedar standup",
    "context": "A short daily meeting of the invented Cedar team at example.test.",
    "sections": [
        {"name": "Progress", "instructions": "What each person finished since yesterday."},
        {"name": "Blockers", "instructions": "What stops someone, and who can help."},
    ],
}
TRANSCRIPTION = {"model": "whisper"}


def answer(response: httpx.Response) -> dict:
    if response.status_code not in (200, 201):
        raise SystemExit(f"{response.request.method} {response.request.url.path}: {response.text}")
    return response.json()


def write(url: str, code: str) -> dict:
    """Pair the invented device and write one of each kind of value through routes.

    Nothing today's hub deletes or changes on its own, as time passes or when
    it starts: no attachment (one no message holds is deleted after an hour),
    no conversation, transcription or final pass under way."""
    with device_client(url, pair_device(url, code, DEVICE_NAME)) as http:
        answer(http.put(f"/v1/reminders/{TITLE_DIGEST}", json=REMINDER))
        answer(
            http.post(
                "/v1/meeting-dictionaries/orders",
                json={
                    "action": "save",
                    "dictionary_id": DICTIONARY["id"],
                    "name": DICTIONARY["name"],
                },
            )
        )
        template = {key: value for key, value in NOTE_TEMPLATE.items() if key != "id"}
        answer(http.put(f"/v1/note-templates/{NOTE_TEMPLATE['id']}", json=template))
        answer(http.put("/v1/settings/transcription", json=TRANSCRIPTION))
        hub_id = answer(http.get("/health"))["hub_id"]
    return {
        "hub_id": hub_id,
        "reminder": {"title_digest": TITLE_DIGEST, **REMINDER},
        "dictionary": DICTIONARY,
        "note_template": {**NOTE_TEMPLATE, "forked_from": None},
        "transcription": {**TRANSCRIPTION, "key_configured": False},
    }


def main() -> None:
    if not TEST_KEY.exists():
        TEST_KEY.write_text(secrets.token_bytes(32).hex() + "\n")
    key = bytes.fromhex(TEST_KEY.read_text().strip())
    with tempfile.TemporaryDirectory(prefix="orbit-golden-") as scratch:
        configuration = prepare(Path(scratch) / "hub", key)
        # The first command that opens the store creates it, sealed with the test key.
        code = run(configuration, "pair-code").strip()
        with serving(configuration) as url:
            manifest = write(url, code)
        data = configuration.parent / "data"
        left = sorted(path.name for path in data.glob(f"{DATABASE_NAME}-*"))
        if left:
            raise SystemExit(f"the hub stopped without folding its journal into one file: {left}")
        shutil.copyfile(data / DATABASE_NAME, GOLDEN)
    MANIFEST.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"{GOLDEN} ({GOLDEN.stat().st_size} bytes) and {MANIFEST.name}", file=sys.stderr)


if __name__ == "__main__":
    main()
