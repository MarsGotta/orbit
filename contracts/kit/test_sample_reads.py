"""A hub reads back, through its routes, what today's hub wrote into the sample.

Each test asks a hub serving a copy of `contracts/fixtures/hub-storage/
golden-v1.sqlite3` for what was written and compares, whole, each field the
manifest keeps: the manifest holds what was written, not what a hub answered,
and a hub may add fields to an answer (the contract grows additively), so
fields it does not keep are not compared. The kit reads as a
device it pairs itself on the copy: none of these values belongs to the
device that wrote it.
"""

pytest_plugins = ("kit_hub",)


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


def test_the_transcription_preference_reads_back(sample_hub):
    http, manifest = sample_hub
    read = ok(http.get("/v1/settings/transcription"))
    for field, value in manifest["transcription"].items():
        assert read[field] == value, field
