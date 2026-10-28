"""The supplied, invented design vault exercised through the public hub API."""

import hashlib
import shutil
from pathlib import Path

import httpx
import pytest
from conftest import device

from orbit_hub.app import create_app


@pytest.fixture
async def design_vault(environment, tmp_path):
    settings, store, adapter = environment
    root = tmp_path / "synthetic-vault"
    shutil.copytree(Path(__file__).parent / "fixtures/vault", root)
    before = {
        str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in root.rglob("*.md")
    }
    owner = device(store)
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app),
        base_url="https://localhost",
        headers={"Authorization": f"Bearer {owner['token']}"},
    ) as client:
        assert (
            await client.put("/v1/vault/config", json={"kind": "folder", "path": str(root)})
        ).status_code == 200
        yield client, root, store
    assert before == {
        str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in root.rglob("*.md")
    }


async def read(client, operation, **values):
    response = await client.post("/v1/vault/read", json={"operation": operation, **values})
    assert response.status_code == 200, response.text
    return response.json()


async def test_indexes_design_notes_citations_tasks_dependencies_and_log(design_vault):
    client, _, store = design_vault
    overview = await read(client, "catalog")
    titles = {n["title"] for n in overview["notes"]}
    assert {"Ada Vale", "Garden Pilot"} <= titles
    person = await read(client, "read", path="People/ada-vale.md")
    assert person["raw"].startswith("---\n")
    assert person["citations"][0]["id"] == "r1021"
    assert person["citations"][0]["number"] == 1
    assert person["queries"][0]["kind"] == "tasks"
    assert person["queries"][0]["lines"] == ["not done", "description includes Ada Vale"]
    assert any(link["path"] == "People/cora-moss.md" for link in person["outlinks"])
    tasks = await read(client, "tasks", query="Ada Vale")
    assert any(task["done"] and task["block_id"] == "cache-rates" for task in tasks["tasks"])
    assert all(task["who"] == "Ada Vale" for task in tasks["tasks"])
    project = await read(client, "read", path="Projects/garden-pilot.md")
    assert len(project["tasks"]) == 5
    assert any(
        dep["prerequisite"].endswith("#cache-rates") and dep["dependent"].endswith("#airport-demo")
        for dep in project["dependencies"]
    )
    assert any(dep["relation"] == "after" for dep in project["dependencies"])
    assert overview["okf_version"] == "0.2"
    assert overview["schema"]["path"] == "SCHEMA.md"
    assert overview["recent"][0]["action"] == "apply"
    assert (await read(client, "catalog", page_size=1))["next_page"] == 2
    with store.read_snapshot() as db:
        assert db.execute("SELECT COUNT(*) FROM vault_working_copy").fetchone()[0] > 0


async def test_persists_format_and_job_model_without_running_any_job(design_vault):
    client, root, _ = design_vault
    response = await client.put(
        "/v1/vault/config",
        json={
            "kind": "folder",
            "path": str(root),
            "link_style": "wikilink",
            "maintenance_model": "fixture/provider-model",
        },
    )
    assert response.status_code == 200, response.text
    config = (await client.get("/v1/vault/config")).json()
    assert config["link_style"] == "wikilink"
    assert config["maintenance_model"] == "fixture/provider-model"
    assert config["maintenance_enabled"] is False


async def test_working_copy_reuses_unchanged_notes_but_never_returns_deleted_content(
    design_vault, monkeypatch
):
    from orbit_hub.vault.folder import Folder

    client, root, _ = design_vault
    original = Folder.read
    calls = []

    def tracked(reader, path):
        calls.append(path)
        return original(reader, path)

    monkeypatch.setattr(Folder, "read", tracked)
    await read(client, "catalog")
    first = len(calls)
    await read(client, "read", path="People/ada-vale.md")
    assert len(calls) == first
    # Work on a new synthetic note: the supplied fixture stays byte-identical.
    temporary = root / "temporary-note.md"
    temporary.write_text("# Temporary\nOne.")
    assert (await read(client, "read", path=temporary.name))["body"].endswith("One.")
    temporary.write_text("# Temporary\nTwo, changed.")
    assert (await read(client, "read", path=temporary.name))["body"].endswith("Two, changed.")
    temporary.unlink()
    response = await client.post(
        "/v1/vault/read", json={"operation": "read", "path": temporary.name}
    )
    assert response.status_code == 404


def test_method_defaults_and_long_fiche_warning():
    from orbit_hub.vault import METHOD_SCHEMA, validate_note

    assert set(METHOD_SCHEMA["types"]) == {
        "person",
        "project",
        "meeting",
        "topic",
        "source",
        "journal",
    }
    result = validate_note(
        {"type": "project", "title": "Garden", "stage": "active", "start": "2026-10-27"},
        "word " * 401,
    )
    assert result["errors"] == []
    assert result["warnings"] == [{"code": "word_limit", "words": 401, "max_words": 400}]


@pytest.mark.parametrize(
    "fields,code",
    [
        ({"type": {}}, "invalid_type"),
        ({"type": "person", "tags": 45}, "invalid_tags"),
        ({"type": "project", "start": "other"}, "invalid_start"),
        ({"type": "source", "sources": -1}, "invalid_sources"),
    ],
)
def test_method_validates_typed_properties_without_throwing_on_input(fields, code):
    from orbit_hub.vault import validate_note

    assert code in validate_note(fields, "")["errors"]


async def test_source_properties_from_safe_frontmatter_validate_without_coercing_the_file(
    design_vault,
):
    from orbit_hub.vault import validate_note

    client, _, _ = design_vault
    path = "Sources/Meetings/2026-10-21-garden-review.md"
    note = await read(client, "read", path=path)
    assert validate_note(note["frontmatter"], note["body"], path=path)["errors"] == []


async def test_dependencies_resolve_across_notes_and_wikilink_owners(design_vault):
    client, root, _ = design_vault
    note = root / "Projects/synthetic-links.md"
    note.write_text(
        "---\ntype: project\n---\n- [ ] Plan [[Ada Vale|Ada]] 📅 2026-10-30 🆔 plan ⛔ cache-rates ^plan\n"
    )
    try:
        result = await read(client, "read", path="Projects/synthetic-links.md")
        assert result["tasks"][0]["who"] == "Ada"
        assert result["tasks"][0]["who_path"] == "People/ada-vale.md"
        assert result["dependencies"] == [
            {
                "prerequisite": "Projects/garden-pilot.md#cache-rates",
                "dependent": "Projects/synthetic-links.md#plan",
                "relation": "requires",
            }
        ]
    finally:
        note.unlink()
