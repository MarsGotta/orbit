"""Write the sample database with today's hub, only through its commands and routes.

Run by hand, and only on purpose: the file the repository keeps is the one
today's hub wrote, and the kit's tests compare what a hub reads against the
manifest written beside it, which keeps no device token: the kit pairs
its own device on its copy. Every name and text is invented. The vault is a
folder under a bare `/tmp`, removed at the end: its path stays in the
database, and must say nothing of the machine that wrote it.

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
    AGENT_ID,
    DATABASE_NAME,
    GOLDEN,
    MANIFEST,
    MEETING_SECONDS,
    MEETING_TRACK,
    TEST_KEY,
    api_agent,
    chat_turn,
    device_client,
    notes_model,
    pair_device,
    prepare,
    record_meeting,
    run,
    serving,
    transcription_service,
    wait_for_note,
    wait_for_proposals,
    wait_for_transcript,
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
# A function's choice, written after the transcription preference (writing the
# preference takes the transcription choices back). No provider: none can be
# taken back by a route, and while one is kept no route reads the preference.
AI_CHOICE = {"function": "meeting_transcription", "provider_id": "hub", "model": "whisper"}
PERSONALITY = "You are Cedar. You answer briefly and plainly."
SKILL = {
    "name": "cedar-digest",
    "description": "Sums up the Cedar week in three points.",
}
SKILL_FILE = (
    f"---\nname: {SKILL['name']}\ndescription: {SKILL['description']}\n---\n"
    "1. Read the Cedar notes of the week.\n2. Write three short points.\n"
)
# A conversation of the device with the API agent: one turn, which the
# invented agent answers with its text.
TURN = {
    "question": "What is left before the Cedar launch?",
    "answer": "Only the Cedar roadmap review is left before the launch",
}
VAULT_FOLDER = "Cedar notes"
VAULT_NOTE = ("Cedar ideas.md", "# Cedar ideas\n\nA garden for the invented Cedar team.\n")
VAULT = {"kind": "folder", "link_style": "wikilink"}
# A meeting the device records over HTTP, which the invented service
# transcribes, and what the device then writes on it. Each text has a space:
# the person's text the hub keeps.
MEETING = {
    "id": "6a1e4c2b-8d3f-4b5a-9c7e-2f0d1b3a5c4e",
    "started_at_ms": 1767258000000,
    "text": "The Cedar roadmap draft is ready for the weekly sync",
    "title": "Cedar roadmap review",
    "speaker": {"key": MEETING_TRACK, "name": "Rowan Cedar"},
    "private_notes": "Ask about the Cedar launch date",
}
# The meeting's note: the invented model writes it once the transcript is
# ready, the device edits its title, asks for it again with the sample's
# template (which sets that edit aside) and edits the new note's title.
NOTE = {
    "text": "The Cedar roadmap draft goes to the weekly sync",
    "set_aside_title": "Cedar roadmap first notes",
    "title": "Cedar roadmap review notes",
}


def answer(response: httpx.Response) -> dict:
    if response.status_code not in (200, 201, 202):
        raise SystemExit(f"{response.request.method} {response.request.url.path}: {response.text}")
    return response.json()


def write(url: str, code: str, vault: Path) -> dict:
    """Pair the invented device and write one of each kind of value through routes.

    Nothing today's hub deletes or changes on its own, as time passes or when
    it starts: no attachment (one no message holds is deleted after an hour),
    no conversation, transcription or final pass under way: the meeting is
    read once its transcript is ready and its final pass is no longer under way,
    its note once it is written, the vault proposal the hub prepares on its
    own for a written note once it is no longer under way, and the
    conversation, last, once its turn is completed and kept."""
    paired = pair_device(url, code, DEVICE_NAME)
    with device_client(url, paired["token"]) as http:
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
        answer(
            http.put(
                f"/v1/settings/ai/functions/{AI_CHOICE['function']}",
                json={"provider_id": AI_CHOICE["provider_id"], "model": AI_CHOICE["model"]},
            )
        )
        answer(http.put(f"/v1/agents/{AGENT_ID}/personality", json={"text": PERSONALITY}))
        answer(
            http.put(f"/v1/agents/{AGENT_ID}/skills/{SKILL['name']}", json={"content": SKILL_FILE})
        )
        recording = MEETING["id"]
        record_meeting(http, recording, MEETING["started_at_ms"])
        wait_for_transcript(http, recording)
        answer(http.put(f"/v1/recordings/{recording}/title", json={"title": MEETING["title"]}))
        speaker = MEETING["speaker"]
        answer(
            http.put(
                f"/v1/recordings/{recording}/speakers",
                json={"names": {speaker["key"]: speaker["name"]}},
            )
        )
        answer(
            http.put(
                f"/v1/recordings/{recording}/private-notes",
                json={"text": MEETING["private_notes"], "base_revision": 0},
            )
        )
        note_path = f"/v1/recordings/{recording}/note"
        note = wait_for_note(http, recording)
        edit = {"base_revision": note["revision"], "title": NOTE["set_aside_title"]}
        answer(http.put(f"{note_path}/edits", json=edit))
        answer(http.post(note_path, json={"template_id": NOTE_TEMPLATE["id"]}))
        note = wait_for_note(http, recording)
        edit = {"base_revision": note["revision"], "title": NOTE["title"]}
        answer(http.put(f"{note_path}/edits", json=edit))
        # The vault last: the hub prepares a proposal for a written note once,
        # from the meeting's title and speaker as they are then.
        answer(http.put("/v1/vault/config", json={**VAULT, "path": str(vault)}))
        proposals = wait_for_proposals(http, recording)
        if [entry["status"] for entry in proposals] != ["proposed"]:
            raise SystemExit(f"the hub prepared no single proposed vault proposal: {proposals}")
        completed = chat_turn(url, paired["token"], TURN["question"])
        if completed.get("status") != "completed" or completed.get("persisted") is not True:
            raise SystemExit(f"the turn with {AGENT_ID} was not completed and kept: {completed}")
        session = completed["session_id"]
        page = answer(http.get(f"/v1/sessions/{session}/messages"))
        hub_id = answer(http.get("/health"))["hub_id"]
    return {
        "hub_id": hub_id,
        "device": {"id": paired["device_id"], "name": DEVICE_NAME},
        "reminder": {"title_digest": TITLE_DIGEST, **REMINDER},
        "dictionary": DICTIONARY,
        "note_template": {**NOTE_TEMPLATE, "forked_from": None},
        "transcription": {**TRANSCRIPTION, "key_configured": False},
        "ai_choice": AI_CHOICE,
        "agent": {
            "id": AGENT_ID,
            "personality": PERSONALITY,
            "skill": {**SKILL, "content": SKILL_FILE},
        },
        "vault": {**VAULT, "path": str(vault)},
        # Each message as the conversation's page gives it: role, text and,
        # for the agent's answer, the summary of its turn's work.
        "conversation": {
            "id": session,
            "agent_id": page["agent_id"],
            "messages": [
                {
                    "role": message["role"],
                    "text": message["text"],
                    **({"work": message["work"]} if "work" in message else {}),
                }
                for message in page["messages"]
            ],
        },
        "meeting": {**MEETING, "duration_ms": MEETING_SECONDS * 1000},
        "note": {
            **NOTE,
            "template_id": NOTE_TEMPLATE["id"],
            "sections": [section["name"] for section in NOTE_TEMPLATE["sections"]],
        },
        # What the invented model proposed: the transcript's one line quoted
        # whole (at its start, 0 ms) and a new meeting note adding its text.
        "proposal": {
            "id": proposals[0]["id"],
            "summary": NOTE["text"],
            "quote": {"at_ms": 0, "speaker": speaker["name"], "text": MEETING["text"]},
            "note": {
                "path": f"{MEETING['title']}.md",
                "title": MEETING["title"],
                "text": MEETING["text"],
            },
        },
    }


def main() -> None:
    if not TEST_KEY.exists():
        TEST_KEY.write_text(secrets.token_bytes(32).hex() + "\n")
    key = bytes.fromhex(TEST_KEY.read_text().strip())
    # A bare /tmp, not the machine's temporary folder: the vault's path is kept.
    with tempfile.TemporaryDirectory(prefix="orbit-golden-", dir="/tmp") as scratch:
        vault = Path(scratch) / VAULT_FOLDER
        vault.mkdir()
        (vault / VAULT_NOTE[0]).write_text(VAULT_NOTE[1], encoding="utf-8")
        with (
            transcription_service(MEETING["text"]) as service,
            notes_model(NOTE["text"]) as model,
            api_agent(TURN["answer"]) as agent,
        ):
            configuration = prepare(
                Path(scratch) / "hub", key, transcription=service, model=model, agent=agent
            )
            # The first command that opens the store creates it, sealed with the test key.
            code = run(configuration, "pair-code").strip()
            with serving(configuration) as url:
                manifest = write(url, code, vault)
        data = configuration.parent / "data"
        left = sorted(path.name for path in data.glob(f"{DATABASE_NAME}-*"))
        if left:
            raise SystemExit(f"the hub stopped without folding its journal into one file: {left}")
        shutil.copyfile(data / DATABASE_NAME, GOLDEN)
    MANIFEST.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"{GOLDEN} ({GOLDEN.stat().st_size} bytes) and {MANIFEST.name}", file=sys.stderr)


if __name__ == "__main__":
    main()
