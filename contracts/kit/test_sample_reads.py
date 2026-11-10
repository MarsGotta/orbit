"""A hub reads back, through its routes, what today's hub wrote into the sample.

Each test asks a hub serving a copy of `contracts/fixtures/hub-storage/
golden-v1.sqlite3` for what was written and compares, whole, each field the
manifest keeps: the manifest holds what was written, not what a hub answered,
and a hub may add fields to an answer (the contract grows additively), so
fields it does not keep are not compared. The kit reads as a
device it pairs itself on the copy: none of these values belongs to the
device that wrote it. A device's name shows only in the device bridge's
list, a private route the kit signs as the contract writes it and calls
without the device's credential; the meetings list and a meeting's
transcript are signed the same way, with the meetings key, for the device
that recorded the sample's meeting, and so is its note. A paired device reads
every conversation, so the kit reads the sample's device's conversation with
its own, and the files its user's message carries: their names in its page,
their text through the assistant's signed read, with the meetings key. The
kit also records a meeting of its own and reads back what its invented
transcription service answered and the note its invented notes model wrote,
and holds a turn of its own with the API agent, which its invented
agent answers.
"""

import json

pytest_plugins = ("kit_hub",)
DEVICES_PATH = "/internal/v1/devices"
MEETINGS_LIST_PATH = "/internal/v1/meetings/list"
MEETINGS_TRANSCRIPT_PATH = "/internal/v1/meetings/transcript"
MEETINGS_NOTE_PATH = "/internal/v1/meetings/note"
ATTACHMENT_READ_PATH = "/internal/v1/attachments/read"
MEETINGS_PROTOCOL = "orbit_meetings_bridge_v1"
# `operator` takes a function's choice back, to the default the hub's
# configuration names (`kit_hub.OPERATOR_TRANSCRIPTION_MODEL`).
TAKEN_BACK = {"provider_id": "operator", "model": "invented-whisper"}


def ok(response) -> dict:
    assert response.status_code == 200, f"{response.status_code}: {response.text}"
    return response.json()


def test_the_hub_is_the_one_that_wrote_the_sample(sample_hub):
    http, manifest = sample_hub
    health = ok(http.get("/health"))
    assert health["ok"] is True and health["protocol"] == 1
    assert health["hub_id"] == manifest["hub_id"]


def test_a_reminder_reads_back_whole(sample_hub):
    http, manifest = sample_hub
    written = manifest["reminder"]
    read = ok(http.get(f"/v1/reminders/{written['title_digest']}"))
    assert read["key"] == written["title_digest"]
    assert read["reminder"]["text"] == written["text"]
    assert read["reminder"]["edited_at_ms"] == written["edited_at_ms"]


def test_a_meeting_dictionary_reads_back_whole(sample_hub):
    http, manifest = sample_hub
    written = manifest["dictionary"]
    read = ok(http.get("/v1/meeting-dictionaries"))["dictionaries"]
    assert [dictionary["id"] for dictionary in read] == [written["id"]]
    assert read[0]["name"] == written["name"]


def test_a_note_template_reads_back_field_by_field(sample_hub):
    http, manifest = sample_hub
    written = manifest["note_template"]
    custom = [
        template
        for template in ok(http.get("/v1/note-templates"))["templates"]
        if not template["builtin"]
    ]
    assert [template["id"] for template in custom] == [written["id"]]
    for field in ("name", "context", "sections", "forked_from"):
        assert custom[0][field] == written[field], field


def test_the_ai_choice_reads_back_and_once_taken_back_the_transcription_preference(sample_hub):
    # One test: while the choice is kept, no route reads the preference, and
    # the order of the tests on the module's hub is not fixed.
    http, manifest = sample_hub
    written = manifest["ai_choice"]
    functions = ok(http.get("/v1/settings/ai"))["functions"]
    chosen = [
        (function["provider_id"], function["model"])
        for function in functions
        if function["id"] == written["function"]
    ]
    assert chosen == [(written["provider_id"], written["model"])]
    ok(http.put(f"/v1/settings/ai/functions/{written['function']}", json=TAKEN_BACK))
    read = ok(http.get("/v1/settings/transcription"))
    for field, value in manifest["transcription"].items():
        assert read[field] == value, field


def test_a_device_name_reads_back_in_the_signed_device_list(sample_hub, sample_signature):
    http, manifest = sample_hub
    written = manifest["device"]
    # Only the signature: the contract asks for nothing else, so the device's
    # own credential is taken off.
    request = http.build_request("GET", DEVICES_PATH, headers=sample_signature(DEVICES_PATH))
    del request.headers["authorization"]
    read = ok(http.send(request))["devices"]
    named = [device["name"] for device in read if device["id"] == written["id"]]
    assert named == [written["name"]]


def signed_meetings(http, sign, path: str, payload: dict) -> dict:
    """A meetings bridge read with only the signature, as for the device list."""
    body = json.dumps(payload).encode()
    headers = {"content-type": "application/json", **sign(path, body, MEETINGS_PROTOCOL)}
    request = http.build_request("POST", path, content=body, headers=headers)
    del request.headers["authorization"]
    return ok(http.send(request))


def test_the_signed_meetings_list_holds_the_samples_meeting(sample_hub, sample_signature):
    http, manifest = sample_hub
    written = manifest["meeting"]
    payload = {"device_id": manifest["device"]["id"], "since_ms": 0, "limit": 100}
    read = signed_meetings(http, sample_signature, MEETINGS_LIST_PATH, payload)["meetings"]
    # The list may hold other meetings (the kit's own).
    listed = [meeting for meeting in read if meeting["recording_id"] == written["id"]]
    assert len(listed) == 1, read
    for field in ("title", "started_at_ms", "duration_ms"):
        assert listed[0][field] == written[field], field


def test_the_signed_transcript_names_the_speaker_and_their_words(sample_hub, sample_signature):
    http, manifest = sample_hub
    written = manifest["meeting"]
    payload = {
        "device_id": manifest["device"]["id"],
        "recording_id": written["id"],
        "format": "text",
    }
    read = signed_meetings(http, sample_signature, MEETINGS_TRANSCRIPT_PATH, payload)
    assert f"{written['speaker']['name']}: {written['text']}" in read["text"], read["text"]


def test_a_meeting_reads_back_field_by_field(sample_hub):
    http, manifest = sample_hub
    written = manifest["meeting"]
    read = ok(http.get(f"/v1/recordings/{written['id']}"))
    for field in ("title", "started_at_ms", "duration_ms"):
        assert read[field] == written[field], field
    assert read["private_notes"]["text"] == written["private_notes"]
    speaker = written["speaker"]
    # The whole list: a speaker the sample does not hold is read wrong too.
    named = [(item["key"], item["name"]) for item in read["speakers"]]
    assert named == [(speaker["key"], speaker["name"])]
    turns = ok(http.get(f"/v1/recordings/{written['id']}/turns"))["turns"]
    assert [(turn["speaker_key"], turn["text"]) for turn in turns] == [
        (speaker["key"], written["text"])
    ]


def test_a_meeting_the_kit_records_is_transcribed_by_its_service(
    sample_hub, kit_meeting, kit_transcription
):
    http, _ = sample_hub
    _, text = kit_transcription
    turns = ok(http.get(f"/v1/recordings/{kit_meeting}/turns"))["turns"]
    assert [turn["text"] for turn in turns] == [text]


def assert_written_by_the_model(note: dict, text: str, sections: list[str]) -> None:
    """Each text the model wrote in `note` is `text`, in each section named."""
    assert note["short_summary"] == text
    assert [(section["name"], section["markdown"]) for section in note["sections"]] == [
        (name, text) for name in sections
    ]
    assert note["decisions"] == [text]
    assert note["open_topics"] == [text]
    assert [item["text"] for item in note["action_items"]] == [text]


def test_a_meetings_note_reads_back_field_by_field(sample_hub):
    http, manifest = sample_hub
    written, meeting = manifest["note"], manifest["meeting"]["id"]
    read = ok(http.get(f"/v1/recordings/{meeting}/note"))
    assert read["state"] == "ready"
    assert read["template_id"] == written["template_id"]
    note = read["note"]
    assert note["template_id"] == written["template_id"]
    assert note["title"] == written["text"]
    assert_written_by_the_model(note, written["text"], written["sections"])
    assert read["edits"]["title"] == written["title"]
    assert read["superseded_edits"]["title"] == written["set_aside_title"]


def test_the_signed_note_shows_the_users_edits(sample_hub, sample_signature):
    http, manifest = sample_hub
    written, meeting = manifest["note"], manifest["meeting"]
    payload = {"device_id": manifest["device"]["id"], "recording_id": meeting["id"]}
    read = signed_meetings(http, sample_signature, MEETINGS_NOTE_PATH, payload)
    assert read["state"] == "ready"
    assert read["note"]["title"] == written["title"]
    assert read["note"]["short_summary"] == written["text"]
    assert read["superseded_edits"] is True
    assert read["private_notes"] == meeting["private_notes"]


def test_a_meeting_the_kit_records_gets_its_note_from_its_model(sample_hub, kit_note, kit_model):
    _, manifest = sample_hub
    _, text = kit_model
    # Another text than the sample's: a hub that answered the sample's note
    # for every meeting would not pass.
    assert text != manifest["note"]["text"]
    note = kit_note["note"]
    sections = [section["name"] for section in note["sections"]]
    assert sections
    assert note["title"] == text
    assert_written_by_the_model(note, text, sections)


def test_an_agent_personality_reads_back_whole(sample_hub):
    http, manifest = sample_hub
    written = manifest["agent"]
    read = ok(http.get(f"/v1/agents/{written['id']}/config"))
    assert read["personality"]["text"] == written["personality"]


def test_an_agent_skill_reads_back_whole(sample_hub):
    http, manifest = sample_hub
    agent, written = manifest["agent"]["id"], manifest["agent"]["skill"]
    items = ok(http.get(f"/v1/agents/{agent}/config"))["skills"]["items"]
    assert [item["name"] for item in items] == [written["name"]]
    assert items[0]["description"] == written["description"]
    read = ok(http.get(f"/v1/agents/{agent}/skills/{written['name']}"))
    assert read["name"] == written["name"]
    assert read["description"] == written["description"]
    assert read["content"] == written["content"]


def assert_the_proposal_written(read: dict, manifest: dict) -> None:
    """`read` is the sample's vault proposal, field by field as the manifest keeps it."""
    written = manifest["proposal"]
    assert read["id"] == written["id"]
    assert read["status"] == "proposed"
    assert read["meeting_id"] == manifest["meeting"]["id"]
    assert read["title"] == manifest["meeting"]["title"]
    assert read["summary"] == written["summary"]
    quote = written["quote"]
    assert [(item["at_ms"], item["speaker"], item["text"]) for item in read["quotes"]] == [
        (quote["at_ms"], quote["speaker"], quote["text"])
    ]
    note = written["note"]
    assert [(change["path"], change["title"]) for change in read["changes"]] == [
        (note["path"], note["title"])
    ]
    assert [line["after"] for line in read["changes"][0]["lines"]] == [note["text"]]
    assert read["new_source"]["summary"] == written["summary"]


def test_the_vault_proposal_reads_back_by_its_meeting_and_by_its_id(sample_hub):
    http, manifest = sample_hub
    meeting = manifest["meeting"]["id"]
    listed = ok(http.get("/v1/vault/distillations", params={"meeting_id": meeting}))["entries"]
    assert len(listed) == 1, listed
    assert_the_proposal_written(listed[0], manifest)
    read = ok(http.get(f"/v1/vault/distillations/{manifest['proposal']['id']}"))
    assert_the_proposal_written(read, manifest)


def test_the_vault_configuration_reads_back_field_by_field(sample_hub):
    http, manifest = sample_hub
    read = ok(http.get("/v1/vault/config"))
    assert read["configured"] is True
    for field, value in manifest["vault"].items():
        assert read[field] == value, field


def test_the_conversation_reads_back_message_by_message(sample_hub):
    http, manifest = sample_hub
    written = manifest["conversation"]
    listed = ok(http.get("/v1/sessions"))["sessions"]
    held = [session for session in listed if session["id"] == written["id"]]
    assert len(held) == 1, listed
    assert held[0]["agent_id"] == written["agent_id"]
    page = ok(http.get(f"/v1/sessions/{written['id']}/messages"))
    assert page["agent_id"] == written["agent_id"]
    read = page["messages"]
    assert [message["role"] for message in read] == [
        message["role"] for message in written["messages"]
    ]
    for got, kept in zip(read, written["messages"], strict=True):
        assert got["text"] == kept["text"], kept["role"]
        assert got.get("work") == kept.get("work"), kept["role"]


def test_the_files_of_the_users_message_read_back_by_name_and_text(sample_hub, sample_signature):
    http, manifest = sample_hub
    written = manifest["conversation"]
    page = ok(http.get(f"/v1/sessions/{written['id']}/messages"))
    user = page["messages"][0]
    assert user["role"] == "user"
    carried = [(file["id"], file["name"], file["kind"]) for file in user.get("attachments", [])]
    assert carried == [(file["id"], file["name"], file["kind"]) for file in written["attachments"]]
    for kept in written["attachments"]:
        payload = {"device_id": manifest["device"]["id"], "attachment_id": kept["id"]}
        read = signed_meetings(http, sample_signature, ATTACHMENT_READ_PATH, payload)
        assert read["attachment"]["name"] == kept["name"], kept["kind"]
        assert read["text"] == kept["text"], kept["kind"]
        assert read["next_after_index"] is None, kept["kind"]


def test_a_turn_the_kit_holds_is_answered_by_its_agent(sample_hub, kit_turn, kit_agent):
    http, manifest = sample_hub
    _, text = kit_agent
    # Another text than the sample's: a hub that answered the sample's
    # conversation for every turn would not pass.
    answers = [m["text"] for m in manifest["conversation"]["messages"] if m["role"] == "assistant"]
    assert text not in answers
    assert kit_turn["status"] == "completed" and kit_turn["persisted"] is True, kit_turn
    assert kit_turn["text"] == text
    session = kit_turn["session_id"]
    assert session != manifest["conversation"]["id"]
    read = ok(http.get(f"/v1/sessions/{session}/messages"))["messages"]
    assert [(message["role"], message["text"]) for message in read][-1] == ("assistant", text)
