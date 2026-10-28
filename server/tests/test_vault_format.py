"""Vault-defined roles and Tasks syntax through the authenticated reading API."""

import httpx
import pytest
from conftest import device

from orbit_hub.app import create_app


@pytest.fixture
async def vault(environment, tmp_path):
    settings, store, adapter = environment
    root = tmp_path / "garden"
    root.mkdir()
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app),
        base_url="https://localhost",
        headers={"Authorization": f"Bearer {device(store)['token']}"},
    ) as client:
        response = await client.put("/v1/vault/config", json={"kind": "folder", "path": str(root)})
        assert response.status_code == 200
        yield client, root


def put(root, path, content):
    target = root / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content)


async def read(client, operation, **values):
    response = await client.post("/v1/vault/read", json={"operation": operation, **values})
    assert response.status_code == 200, response.text
    return response.json()


async def test_defaults_without_schema(vault):
    client, root = vault
    for kind in ("person", "project", "meeting", "topic", "source", "journal", "odd"):
        put(root, f"{kind}.md", f"---\ntype: {kind}\nstage: active\nstart: 2026-10-28\n---\nText")
    put(root, "plain.md", "Text")
    catalog = await read(client, "catalog")
    by_type = {n["type"]: n for n in catalog["notes"]}
    assert by_type["project"]["role"] == "project"
    assert by_type["project"]["start"] == "2026-10-28"
    assert by_type["project"]["state"] == "active"
    assert by_type["odd"]["role"] == "note"
    assert by_type["note"]["role"] == "note"
    assert catalog["sources_count"] == 1
    assert set(catalog["schema"]["types"]) == {
        "person",
        "project",
        "meeting",
        "topic",
        "source",
        "journal",
    }


SCHEMA = """---
type: schema
types:
  contact: {role: person, folder: People/}
  venture: {role: project, folder: Projects/}
  client: {role: project, folder: Clients/}
  evidence: {role: source}
fields:
  start: kickoff
  stage: phase
---
Free prose; `Wrong/` · `type: phantom` is not configuration.
"""


async def test_custom_roles_folders_fields_and_unknown_type(vault):
    client, root = vault
    put(root, "SCHEMA.md", SCHEMA)
    put(root, "People/ada.md", "---\ntitle: Ada Vale\n---\nGardener")
    for path in ("Projects/garden.md", "Clients/orchard.md"):
        put(
            root,
            path,
            "---\nkickoff: 2026-11-01\nphase: active\ndescription: Garden\ngenerated: {by: orbit/1.0, at: '2026-10-28T12:00:00Z'}\n---\n[[ada]]",
        )
    put(root, "odd.md", "---\ntype: peculiar\n---\nFree")
    put(root, "source.md", "---\ntype: evidence\n---\nTranscript")
    put(root, "Projects/explicit.md", "---\ntype: peculiar\n---\nExplicit wins")
    catalog = await read(client, "catalog")
    notes = {n["path"]: n for n in catalog["notes"]}
    assert notes["People/ada.md"]["type"] == "contact"
    assert notes["People/ada.md"]["role"] == "person"
    for path in ("Projects/garden.md", "Clients/orchard.md"):
        assert notes[path]["role"] == "project"
        assert notes[path]["start"] == "2026-11-01"
        assert notes[path]["state"] == "active"
        assert notes[path]["modified_at"] == "2026-10-28T12:00:00Z"
    assert notes["Projects/explicit.md"]["role"] == "note"
    assert notes["odd.md"]["type"] == "peculiar"
    assert notes["odd.md"]["role"] == "note"
    assert catalog["sources_count"] == 1
    assert "phantom" not in catalog["schema"]["folders"]
    note = await read(client, "read", path="Projects/garden.md")
    assert note["role"] == "project"
    assert note["properties"]["start"] == "2026-11-01"
    assert "kickoff:" in note["raw"]


@pytest.mark.parametrize(
    "schema,location",
    [
        ("---\ntypes: [\n---\nText", "línea 3"),
        ("---\ntypes:\n  client: {role: potato}\n---\nText", "types.client.role"),
        ("---\nfields: {start: []}\n---\nText", "fields.start"),
        (
            "---\ntypes: {client: {role: project, folder: ../escape/}}\n---\nText",
            "types.client.folder",
        ),
    ],
)
async def test_bad_schema_is_advisory_and_defaults_apply(vault, schema, location):
    client, root = vault
    put(root, "SCHEMA.md", schema)
    put(root, "garden.md", "---\ntype: project\nstart: 2026-11-01\n---\nGarden")
    catalog = await read(client, "catalog")
    assert catalog["notes"][0]["role"] == "project"
    assert any(location in warning for warning in catalog["schema"]["warnings"])
    assert "person" in catalog["schema"]["types"]
    note = await read(client, "read", path="SCHEMA.md")
    assert note["raw"] == schema
    assert any(location in warning for warning in note["schema_warnings"])


async def test_tasks_markers_owners_cross_note_dependencies_and_queries(vault):
    client, root = vault
    put(root, "SCHEMA.md", SCHEMA)
    put(root, "People/ada.md", "---\ntitle: Ada Vale\n---\nPerson")
    put(root, "People/ben.md", "---\ntitle: Ben Reed\n---\nPerson")
    put(root, "prep.md", "---\ntype: note\n---\n- [x] Prepare ✅ 2026-10-27 🆔 prep ^block-prep")
    put(
        root,
        "Projects/garden.md",
        """---
kickoff: 2026-11-01
sources: [{id: minutes, resource: /prep.md}]
---
- [ ] [[prep]] Ask [[ada|Ada]] then [Ben](/People/ben.md) #garden 📅 2026-11-03 ⏳ 2026-11-02 🛫 2026-11-01 ⏫ 🔁 every week 🆔 plant ⛔ prep [^minutes] ^block-plant
- [ ] [Ada](/People/ada.md) Train 🔼 🆔 train ⛔ plant,prep
- [ ] [[ben]] Visit 🔽 📅 2026-11-04
```tasks
not done
due after 2026-11-01
due before 2026-11-04
due on 2026-11-03
path includes Projects/
description includes Ask
tags include #garden
sort by due
```
```tasks
done
```
""",
    )
    page = await read(client, "tasks")
    tasks = page["tasks"]
    plant = next(t for t in tasks if t.get("id") == "plant")
    assert plant["who"] == "Ada"
    assert plant["who_path"] == "People/ada.md"
    assert plant["due"] == "2026-11-03"
    assert plant["scheduled"] == "2026-11-02"
    assert plant["start"] == "2026-11-01"
    assert plant["priority"] == "⏫"
    assert plant["recurrence"] == "every week"
    assert plant["depends_on"] == ["prep"]
    assert plant["project"] == "Projects/garden.md"
    assert plant["source"] == "/prep.md"
    assert "📅" not in plant["text"]
    assert next(t for t in tasks if t.get("id") == "prep")["done_at"] == "2026-10-27"
    assert {t["priority"] for t in tasks} >= {"⏫", "🔼", "🔽"}
    note = await read(client, "read", path="Projects/garden.md")
    assert {
        "prerequisite": "prep.md#prep",
        "dependent": "Projects/garden.md#plant",
        "relation": "requires",
    } in note["dependencies"]
    assert {
        "prerequisite": "Projects/garden.md#milestone-start",
        "dependent": "Projects/garden.md#train",
        "relation": "after",
    } in note["dependencies"]
    assert [t["id"] for t in note["queries"][0]["tasks"]] == ["plant"]
    assert note["queries"][0]["warnings"] == ["Esta línea de la consulta no se aplica: sort by due"]
    assert [t["id"] for t in note["queries"][1]["tasks"]] == ["prep"]


@pytest.mark.parametrize(
    "schema",
    ["", "# Free text\n`Wrong/` · `type: phantom`", "---\ntype: schema\n---\nFree text"],
)
async def test_missing_schema_keys_use_english_defaults(vault, schema):
    client, root = vault
    if schema:
        put(root, "SCHEMA.md", schema)
    put(root, "project.md", "---\ntype: project\nstart: 2026-11-01\n---\nProject")
    catalog = await read(client, "catalog")
    assert catalog["schema"]["warnings"] == []
    assert catalog["schema"]["fields"]["start"] == "start"
    assert catalog["schema"]["folders"] == {}
    assert catalog["notes"][0]["role"] == "project"


async def test_nearest_folder_and_no_type_name_special_cases(vault):
    client, root = vault
    put(
        root,
        "SCHEMA.md",
        "---\ntype: schema\ntypes:\n  project: {role: person, folder: Team/}\n  person: {role: project, folder: Team/Work/}\n---\nText",
    )
    put(root, "Team/ada.md", "---\ntitle: Ada Vale\n---\nPerson")
    put(root, "Team/Work/garden.md", "---\nstart: 2026-11-01\n---\n- [ ] Ask [[ada]] 🆔 ask")
    catalog = await read(client, "catalog")
    notes = {n["path"]: n for n in catalog["notes"]}
    assert notes["Team/Work/garden.md"]["type"] == "person"
    assert notes["Team/Work/garden.md"]["role"] == "project"
    assert notes["Team/ada.md"]["role"] == "person"
    task = (await read(client, "tasks"))["tasks"][0]
    assert task["who_path"] == "Team/ada.md"
    assert task["project"] == "Team/Work/garden.md"


async def test_schema_failure_discards_all_overrides(vault):
    client, root = vault
    put(
        root,
        "SCHEMA.md",
        "---\ntype: schema\ntypes: {client: {role: project, folder: Clients/}}\nfields: {start: []}\n---\nText",
    )
    put(root, "Clients/garden.md", "---\nstart: 2026-11-01\n---\nGarden")
    catalog = await read(client, "catalog")
    assert catalog["schema"]["folders"] == {}
    assert catalog["notes"][0]["type"] == "note"
    assert catalog["notes"][0]["start"] == "2026-11-01"


async def test_tasks_after_metadata_and_completed_due_dates(vault):
    client, root = vault
    put(root, "ada.md", "---\ntype: person\n---\nPerson")
    put(
        root,
        "garden.md",
        "---\ntype: project\n---\n- [x] Ask 📅 2026-11-01 ✅ 2026-10-28 [[ada]] 🆔 ask\n- [ ] Train ⏳ 2026-11-02 ⛔ ask [[ada]]",
    )
    tasks = (await read(client, "tasks"))["tasks"]
    assert all(t["who_path"] == "ada.md" for t in tasks)
    assert tasks[0]["done_at"] == "2026-10-28"
    assert tasks[0]["due"] == "2026-11-01"
    note = await read(client, "read", path="garden.md")
    assert not any(edge["relation"] == "after" for edge in note["dependencies"])
