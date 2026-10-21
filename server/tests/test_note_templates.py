"""P9-05: Alina's own note templates and their assignment to an event or series.

Routes against the real hub app and store; the note worker is the real one,
driven a job at a time with a scripted model (`support_series`). No provider,
no network and no fixed port.
"""

from pathlib import Path

import pytest
from conftest import device
from support_notes import note_answer
from support_series import SERIES, Bench, hub, recording

from orbit_hub.notes.models import EventContext, NoteInputs
from orbit_hub.notes.templates import BUILTIN_TEMPLATES, MAX_SECTIONS

NOW = 2000.0
ROOT = Path(__file__).resolve().parents[2]


def draft(name="Comité de dirección", sections=("Estado", "Riesgos"), **changes) -> dict:
    return {
        "name": name,
        "context": "La reunión semanal del comité.",
        "sections": [{"name": s, "instructions": f"Lo que toca en {s}."} for s in sections],
        **changes,
    }


# A1: create, edit, reorder and delete a custom template by the real path.


async def test_a1_create_edit_reorder_and_delete_a_custom_template(environment):
    async with hub(environment) as (client, headers, _owner, _app, _model):
        created = await client.put("/v1/note-templates/comite", json=draft(), headers=headers)
        assert created.status_code == 201, created.text
        template = created.json()
        assert template["id"] == "comite" and template["builtin"] is False
        assert [s["name"] for s in template["sections"]] == ["Estado", "Riesgos"]
        assert template["forked_from"] is None

        listing = (await client.get("/v1/note-templates", headers=headers)).json()["templates"]
        ids = [t["id"] for t in listing]
        assert ids[: len(BUILTIN_TEMPLATES)] == [t.id for t in BUILTIN_TEMPLATES]
        assert ids[len(BUILTIN_TEMPLATES) :] == ["comite"]

        reordered = await client.put(
            "/v1/note-templates/comite",
            json=draft(name="Comité", sections=("Riesgos", "Estado", "Próximos pasos")),
            headers=headers,
        )
        assert reordered.status_code == 200, reordered.text
        one = await listed_template(client, headers, "comite")
        assert one["name"] == "Comité"
        assert [s["name"] for s in one["sections"]] == ["Riesgos", "Estado", "Próximos pasos"]

        deleted = await client.delete("/v1/note-templates/comite", headers=headers)
        assert deleted.status_code == 200
        assert deleted.json() == {"template_id": "comite", "deleted": True, "links_released": 0}
        # A retried delete whose receipt was lost is not an error.
        again = await client.delete("/v1/note-templates/comite", headers=headers)
        assert again.json() == {"template_id": "comite", "deleted": False, "links_released": 0}
        assert await listed_template(client, headers, "comite") is None


async def test_a1_a_custom_template_is_stored_encrypted(environment):
    _, store, _ = environment
    async with hub(environment) as (client, headers, _owner, _app, _model):
        secret = "Instrucción sintética P905"
        body = draft(sections=("Estado",))
        body["sections"][0]["instructions"] = secret
        assert (
            await client.put("/v1/note-templates/cifrada", json=body, headers=headers)
        ).status_code == 201
    with store.transaction() as db:
        rows = db.execute("SELECT * FROM note_templates").fetchall()
    assert rows, "the template was not stored"
    for row in rows:
        for value in tuple(row):
            if isinstance(value, bytes | str):
                text = value if isinstance(value, str) else value.decode("latin-1")
                assert secret not in text and "Comité de dirección" not in text


async def test_a1_bounds_are_the_builtin_ones_and_refusals_carry_the_numbers(environment):
    async with hub(environment) as (client, headers, _owner, _app, _model):
        too_many = await client.put(
            "/v1/note-templates/larga",
            json=draft(sections=[f"S{i}" for i in range(MAX_SECTIONS + 1)]),
            headers=headers,
        )
        assert too_many.status_code == 400
        refusal = too_many.json()
        assert refusal["code"] == "invalid_note_template"
        assert "sections" in refusal["message"]
        assert refusal["refusal"]["max"] == MAX_SECTIONS
        assert refusal["refusal"]["actual"] == MAX_SECTIONS + 1

        long_name = await client.put(
            "/v1/note-templates/larga", json=draft(name="x" * 81), headers=headers
        )
        assert long_name.status_code == 400
        assert long_name.json()["refusal"]["max"] == 80

        bad_id = await client.put("/v1/note-templates/Mayúsculas", json=draft(), headers=headers)
        assert bad_id.status_code == 400
        assert bad_id.json()["code"] == "invalid_note_template"


async def test_a1_a_device_never_sees_another_devices_templates(environment):
    _, store, _ = environment
    async with hub(environment) as (client, headers, _owner, _app, _model):
        mine = await client.put("/v1/note-templates/mia", json=draft(), headers=headers)
        assert mine.status_code == 201, mine.text
        other = device(store, name="otro")
        theirs = {"Authorization": "Bearer " + other["token"]}
        listing = (await client.get("/v1/note-templates", headers=theirs)).json()["templates"]
        assert "mia" not in {t["id"] for t in listing}


async def test_a1_the_number_of_custom_templates_is_bounded_with_its_numbers(environment):
    from orbit_hub.notes.custom_templates import MAX_CUSTOM_TEMPLATES

    async with hub(environment) as (client, headers, _owner, _app, _model):
        for index in range(MAX_CUSTOM_TEMPLATES):
            answer = await client.put(f"/v1/note-templates/t{index}", json=draft(), headers=headers)
            assert answer.status_code == 201, answer.text
        full = await client.put("/v1/note-templates/una-mas", json=draft(), headers=headers)
        assert full.status_code == 409
        assert full.json()["code"] == "note_templates_full"
        assert full.json()["refusal"]["templates"] == MAX_CUSTOM_TEMPLATES
        assert full.json()["refusal"]["max_templates"] == MAX_CUSTOM_TEMPLATES
        # Replacing one that exists is not a new one.
        assert (
            await client.put("/v1/note-templates/t0", json=draft(), headers=headers)
        ).status_code == 200


# A2: editing a built-in creates a custom copy and leaves the built-in alone.


async def test_a2_editing_a_builtin_creates_a_copy_that_names_its_origin(environment):
    async with hub(environment) as (client, headers, _owner, _app, _model):
        before = await listed_template(client, headers, "general")
        assert before["builtin"] is True

        copy = await client.put(
            "/v1/note-templates/mi-general",
            json=draft(name="Mi general", forked_from="general"),
            headers=headers,
        )
        assert copy.status_code == 201, copy.text
        assert copy.json()["forked_from"] == {"id": "general", "name": "General"}
        assert copy.json()["builtin"] is False

        refused = await client.put("/v1/note-templates/general", json=draft(), headers=headers)
        assert refused.status_code == 409
        assert refused.json()["code"] == "note_template_builtin"
        assert refused.json()["refusal"]["template_id"] == "general"
        undeleted = await client.delete("/v1/note-templates/general", headers=headers)
        assert undeleted.status_code == 409
        assert undeleted.json()["code"] == "note_template_builtin"

        after = await listed_template(client, headers, "general")
        assert after == before

        unknown_origin = await client.put(
            "/v1/note-templates/huerfana",
            json=draft(forked_from="no-existe"),
            headers=headers,
        )
        assert unknown_origin.status_code == 404
        assert unknown_origin.json()["code"] == "note_template_not_found"
        assert unknown_origin.json()["refusal"]["template_id"] == "no-existe"


# A3: a recording linked to an event whose series has a template uses it.


async def test_a3_links_are_put_listed_and_deleted_by_series_key(environment):
    async with hub(environment) as (client, headers, _owner, _app, _model):
        await client.put("/v1/note-templates/comite", json=draft(), headers=headers)
        linked = await client.put(
            "/v1/note-template-links",
            json={"calendar_item_id": SERIES, "template_id": "comite"},
            headers=headers,
        )
        assert linked.status_code == 200, linked.text
        assert linked.json()["calendar_item_id"] == SERIES
        assert linked.json()["template_id"] == "comite"
        listing = (await client.get("/v1/note-template-links", headers=headers)).json()
        assert [(link["calendar_item_id"], link["template_id"]) for link in listing["links"]] == [
            (SERIES, "comite")
        ]

        unknown = await client.put(
            "/v1/note-template-links",
            json={"calendar_item_id": SERIES, "template_id": "no-existe"},
            headers=headers,
        )
        assert unknown.status_code == 404
        assert unknown.json()["code"] == "note_template_not_found"

        control = await client.put(
            "/v1/note-template-links",
            json={"calendar_item_id": "a\nb", "template_id": "general"},
            headers=headers,
        )
        assert control.status_code == 400
        assert control.json()["code"] == "invalid_note_template"

        removed = await client.request(
            "DELETE", "/v1/note-template-links", json={"calendar_item_id": SERIES}, headers=headers
        )
        assert removed.json() == {"calendar_item_id": SERIES, "deleted": True}
        assert (await client.get("/v1/note-template-links", headers=headers)).json()["links"] == []


@pytest.fixture
def bench(environment):
    _, store, _ = environment
    owner = device(store)["device_id"]
    bench = Bench(store, owner, NOW)
    yield bench
    bench.close()


def template_draft(name, sections, forked_from=None):
    from orbit_hub.notes.custom_templates import TemplateDraft

    return TemplateDraft.model_validate(
        {
            "name": name,
            "context": "",
            "sections": [{"name": s, "instructions": ""} for s in sections],
            "forked_from": forked_from,
        }
    )


def link(series, template_id):
    from orbit_hub.notes.custom_templates import LinkRequest

    return LinkRequest(calendar_item_id=series, template_id=template_id)


async def test_a3_the_assigned_template_writes_the_note_unless_one_is_chosen(bench):
    templates = bench.worker.templates
    templates.put(bench.owner, "comite", template_draft("Comité", ("Estado", "Riesgos")), NOW)
    templates.link(bench.owner, link(SERIES, "comite"), NOW)
    bench.model.script("note", note_answer(sections=2), note_answer(sections=5))

    assigned = recording(bench.store, bench.owner)
    bench.request(assigned, event=bench.event(1_000_000))
    chosen = recording(bench.store, bench.owner)
    bench.request(chosen, template_id="project", event=bench.event(2_000_000))
    await bench.drain()

    written = bench.worker.repository.document(bench.owner, assigned)
    assert written["state"] == "ready", written["error"]
    assert written["template_id"] is None  # the request named none: the hub chose
    assert written["note"]["template_id"] == "comite"
    assert written["note"]["template_name"] == "Comité"
    assert written["note"]["template_source"] == "series"
    assert [s["name"] for s in written["note"]["sections"]] == ["Estado", "Riesgos"]
    prompt = bench.model.calls[0]["messages"][1]["content"]
    assert "Plantilla: Comité" in prompt

    manual = bench.worker.repository.document(bench.owner, chosen)
    assert manual["state"] == "ready", manual["error"]
    assert manual["note"]["template_id"] == "project"
    assert manual["note"]["template_source"] == "requested"


async def test_a3_without_a_link_or_an_event_the_note_uses_general(bench):
    plain = recording(bench.store, bench.owner)
    bench.request(plain)
    other_series = recording(bench.store, bench.owner)
    bench.request(other_series, event=bench.event(1_000_000, series="otra-serie"))
    await bench.drain()
    for identity in (plain, other_series):
        note = bench.worker.repository.document(bench.owner, identity)["note"]
        assert note["template_id"] == "general" and note["template_source"] == "default"


def test_a3_old_requests_stay_valid_and_the_series_key_is_bounded():
    from orbit_hub.notes.series_key import MAX_SERIES_KEY_BYTES

    old = NoteInputs.model_validate(
        {"template_id": "general", "event": {"title": "Daily", "starts_at_ms": 5, "attendees": []}}
    )
    assert old.event.calendar_item_id is None and old.template_id == "general"
    assert NoteInputs().template_id is None
    assert EventContext(calendar_item_id="x" * MAX_SERIES_KEY_BYTES).calendar_item_id
    for wrong in ("", "x" * (MAX_SERIES_KEY_BYTES + 1), "con\tcontrol", "ñ" * 600):
        with pytest.raises(ValueError):
            EventContext(calendar_item_id=wrong)


# A4: deleting an assigned template returns those series to General.


async def test_a4_deleting_an_assigned_template_returns_its_series_to_general(bench):
    templates = bench.worker.templates
    templates.put(bench.owner, "comite", template_draft("Comité", ("Estado", "Riesgos")), NOW)
    templates.link(bench.owner, link(SERIES, "comite"), NOW)
    bench.model.script("note", note_answer(sections=2))
    before = recording(bench.store, bench.owner)
    bench.request(before, event=bench.event(1_000_000))
    await bench.drain()
    written = bench.worker.repository.document(bench.owner, before)["note"]

    released = templates.delete(bench.owner, "comite")
    assert released == {"template_id": "comite", "deleted": True, "links_released": 1}
    assert templates.links(bench.owner) == {"links": []}

    after = recording(bench.store, bench.owner)
    bench.request(after, event=bench.event(2_000_000))
    await bench.drain()
    assert bench.worker.repository.document(bench.owner, after)["note"]["template_id"] == "general"
    # The note written with the deleted template keeps its name and sections.
    assert bench.worker.repository.document(bench.owner, before)["note"] == written
    assert written["template_name"] == "Comité"


async def test_a4_a_note_that_names_a_deleted_template_says_which(bench):
    templates = bench.worker.templates
    templates.put(bench.owner, "efimera", template_draft("Efímera", ("Una",)), NOW)
    templates.delete(bench.owner, "efimera")
    identity = recording(bench.store, bench.owner)
    bench.request(identity, template_id="efimera")
    await bench.drain()
    error = bench.worker.repository.document(bench.owner, identity)["error"]
    assert error == {"code": "note_template_not_found", "template_id": "efimera", "attempt": 1}


async def listed_template(client, headers, identity):
    response = await client.get("/v1/note-templates", headers=headers)
    assert response.status_code == 200, response.text
    return next((item for item in response.json()["templates"] if item["id"] == identity), None)
