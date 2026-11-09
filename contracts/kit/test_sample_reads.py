"""A hub reads back, through its routes, what today's hub wrote into the sample.

Each test asks a hub serving a copy of `contracts/fixtures/hub-storage/
golden-v1.sqlite3` for what was written and compares, whole, each field the
manifest keeps: the manifest holds what was written, not what a hub answered,
and a hub may add fields to an answer (the contract grows additively), so
fields it does not keep are not compared. The kit reads as a
device it pairs itself on the copy: none of these values belongs to the
device that wrote it. A device's name shows only in the device bridge's
list, a private route the kit signs as the contract writes it and calls
without the device's credential; the meetings list is signed the same way,
with the meetings key.
"""

import json

pytest_plugins = ("kit_hub",)
DEVICES_PATH = "/internal/v1/devices"
MEETINGS_LIST_PATH = "/internal/v1/meetings/list"
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


def test_the_signed_meetings_list_of_the_sample_is_empty(sample_hub, sample_signature):
    http, manifest = sample_hub
    body = json.dumps({"device_id": manifest["device"]["id"], "since_ms": 0, "limit": 10}).encode()
    headers = {
        "content-type": "application/json",
        **sample_signature(MEETINGS_LIST_PATH, body, MEETINGS_PROTOCOL),
    }
    # Only the signature, as for the device list.
    request = http.build_request("POST", MEETINGS_LIST_PATH, content=body, headers=headers)
    del request.headers["authorization"]
    assert ok(http.send(request)) == {"meetings": [], "more": False}


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


def test_the_vault_configuration_reads_back_field_by_field(sample_hub):
    http, manifest = sample_hub
    read = ok(http.get("/v1/vault/config"))
    assert read["configured"] is True
    for field, value in manifest["vault"].items():
        assert read[field] == value, field
