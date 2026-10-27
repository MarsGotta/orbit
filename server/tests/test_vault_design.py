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
    assert {"Javier Ortega", "Piloto Hoteles Ribera"} <= titles
    person = await read(client, "read", path="personas/javier-ortega.md")
    assert person["raw"].startswith("---\n")
    assert person["citations"][0]["id"] == "r1021"
    assert person["citations"][0]["number"] == 1
    assert person["queries"] == [{"kind": "tareas", "filters": {"persona": "Javier Ortega"}}]
    assert any(link["path"] == "personas/marta-ibanez.md" for link in person["outlinks"])
    tasks = await read(client, "tasks", query="Javier Ortega")
    assert any(task["done"] and task["block_id"] == "cache-tarifas" for task in tasks["tasks"])
    assert all(task["who"] == "Javier Ortega" for task in tasks["tasks"])
    project = await read(client, "read", path="proyectos/piloto-hoteles-ribera.md")
    assert len(project["tasks"]) == 5
    assert any(
        dep["prerequisite"].endswith("#cache-tarifas")
        and dep["dependent"].endswith("#demo-aeropuerto")
        for dep in project["dependencies"]
    )
    assert any(dep["relation"] == "after" for dep in project["dependencies"])
    assert overview["okf_version"] == "0.2"
    assert overview["schema"]["path"] == "SCHEMA.md"
    assert overview["recent"][0]["action"] == "aplicar"
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
    await read(client, "read", path="personas/javier-ortega.md")
    assert len(calls) == first
    # Work on a new synthetic note: the supplied fixture stays byte-identical.
    temporary = root / "nota-temporal.md"
    temporary.write_text("# Temporary\nOne.")
    assert (await read(client, "read", path=temporary.name))["body"].endswith("One.")
    temporary.write_text("# Temporary\nTwo, changed.")
    assert (await read(client, "read", path=temporary.name))["body"].endswith("Two, changed.")
    temporary.unlink()
    response = await client.post(
        "/v1/vault/read", json={"operation": "read", "path": temporary.name}
    )
    assert response.status_code == 404


def test_design_validator_has_eight_types_and_only_warns_on_long_fiches():
    from orbit_hub.vault import METHOD_SCHEMA, validate_note

    assert set(METHOD_SCHEMA["types"]) == {
        "persona",
        "proyecto",
        "organizacion",
        "decision",
        "tema",
        "consulta",
        "nota",
        "fuente",
    }
    result = validate_note(
        {"type": "proyecto", "title": "Garden", "estado": "en marcha", "arranque": "2026-10-27"},
        "word " * 401,
    )
    assert result["errors"] == []
    assert result["warnings"] == [{"code": "word_limit", "words": 401, "max_words": 400}]


@pytest.mark.parametrize(
    "fields,code",
    [
        ({"type": {}}, "invalid_type"),
        ({"type": "persona", "rol": 45}, "invalid_rol"),
        ({"type": "proyecto", "estado": "other"}, "invalid_estado"),
        ({"type": "fuente", "duracion_min": -1}, "invalid_duracion_min"),
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
    path = "fuentes/reuniones/2026-10-21-revision-del-diseno-de-reservas.md"
    note = await read(client, "read", path=path)
    assert validate_note(note["frontmatter"], note["body"], path=path)["errors"] == []


async def test_dependencies_resolve_note_paths_and_wikilink_task_owners(design_vault):
    client, root, _ = design_vault
    note = root / "proyectos/synthetic-links.md"
    note.write_text(
        "---\ntype: proyecto\n---\n## Tareas\n- [ ] Plan · [[Javier Ortega|Javier]] · para el 2026-10-30 ^plan\n## Dependencias\nEl **arranque** depende de [Javier](/personas/javier-ortega.md).\nEl **arranque** depende de [outside](https://example.test/note.md#^no).\n"
    )
    try:
        result = await read(client, "read", path="proyectos/synthetic-links.md")
        assert result["tasks"][0]["who"] == "Javier"
        assert result["tasks"][0]["who_path"] == "personas/javier-ortega.md"
        assert result["dependencies"] == [
            {
                "prerequisite": "personas/javier-ortega.md",
                "dependent": "proyectos/synthetic-links.md#milestone-arranque",
                "relation": "requires",
            }
        ]
    finally:
        note.unlink()
