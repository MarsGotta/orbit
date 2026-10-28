"""Read-only vault acceptance through the authenticated hub boundary."""

import httpx
import pytest
from conftest import device

from orbit_hub.app import create_app


@pytest.fixture
async def vault_client(environment, tmp_path):
    settings, store, adapter = environment
    owner = device(store)
    root = tmp_path / "invented-vault"
    root.mkdir()
    (root / "People").mkdir()
    (root / "People" / "Ada.md").write_text(
        "---\ntype: person\ntitle: Ada\n---\nWorks on [[Garden]].\n", encoding="utf-8"
    )
    (root / "Garden.md").write_text("# Garden\nWith [Ada](People/Ada.md).", encoding="utf-8")
    (root / ".private.md").write_text("hidden")
    (root / "escape.md").symlink_to(tmp_path / "outside.md")
    (tmp_path / "outside.md").write_text("never disclose")
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app),
        base_url="https://localhost",
        headers={"Authorization": f"Bearer {owner['token']}"},
    ) as client:
        yield client, root, store


async def connect(client, root):
    response = await client.put("/v1/vault/config", json={"kind": "folder", "path": str(root)})
    assert response.status_code == 200, response.text
    return response.json()


async def test_folder_reads_links_pagination_and_index(vault_client):
    client, root, _ = vault_client
    assert (await client.get("/v1/vault/config")).json() == {"configured": False}
    await connect(client, root)
    page = (await client.post("/v1/vault/read", json={"operation": "list", "page_size": 1})).json()
    assert len(page["entries"]) == 1
    assert page["next_page"] == 2
    found = (
        await client.post("/v1/vault/read", json={"operation": "search", "query": "works"})
    ).json()
    assert [entry["path"] for entry in found["entries"]] == ["People/Ada.md"]
    note = (
        await client.post("/v1/vault/read", json={"operation": "read", "path": "People/Ada.md"})
    ).json()
    assert note["frontmatter"]["type"] == "person"
    assert note["body"] == "Works on [[Garden]].\n"
    assert note["outlinks"] == [{"target": "Garden", "path": "Garden.md"}]
    backlinks = (
        await client.post(
            "/v1/vault/read", json={"operation": "backlinks", "path": "People/Ada.md"}
        )
    ).json()
    assert [entry["path"] for entry in backlinks["entries"]] == ["Garden.md"]
    index = (await client.post("/v1/vault/read", json={"operation": "index"})).json()
    assert index["synthesized"] is True
    assert "People/Ada.md" in index["body"]


@pytest.mark.parametrize(
    "path", ["../outside.md", "/etc/passwd", "escape.md", ".private.md", "People/../../outside.md"]
)
async def test_refuses_escape_and_hidden(vault_client, path):
    client, root, _ = vault_client
    await connect(client, root)
    response = await client.post("/v1/vault/read", json={"operation": "read", "path": path})
    assert response.status_code in {400, 404}
    assert "never disclose" not in response.text


async def test_failed_validation_preserves_settings_and_names_cause(vault_client):
    client, root, _ = vault_client
    await connect(client, root)
    response = await client.put(
        "/v1/vault/config", json={"kind": "folder", "path": str(root / "missing")}
    )
    assert response.status_code == 400
    assert response.json()["code"] == "vault_folder_missing"
    assert "carpeta" in response.json()["message"]
    assert (await client.get("/v1/vault/config")).json()["path"] == str(root)


async def test_auth_and_size_bounds(vault_client):
    client, root, _ = vault_client
    assert (
        await client.get("/v1/vault/config", headers={"Authorization": "Bearer invented"})
    ).status_code == 401
    await connect(client, root)
    (root / "large.md").write_bytes(b"x" * (256 * 1024 + 1))
    response = await client.post("/v1/vault/read", json={"operation": "read", "path": "large.md"})
    assert response.status_code == 413
    assert response.json()["refusal"]["max_bytes"] == 256 * 1024
    response = await client.post("/v1/vault/read", json={"operation": "list", "page_size": 101})
    assert response.status_code == 400
    assert response.json()["refusal"]["max_page_size"] == 100


def test_method_schema():
    from orbit_hub.vault import validate_note

    fields = {
        "type": "person",
        "title": "Ada",
        "description": "Gardener",
        "tags": [],
        "sources": [],
    }
    assert validate_note(fields, "Short note.") == {"errors": [], "warnings": []}
    assert validate_note(fields, "word " * 401)["warnings"][0]["code"] == "word_limit"
    assert "invalid_status" in validate_note({**fields, "status": []}, "Short")["errors"]
    assert validate_note({**fields, "type": "other"}, "Short")["errors"] == []
    trust = {
        "sources": [{"resource": "/meetings/garden.md", "id": "garden"}],
        "generated": {"by": "orbit/fixture", "at": "2026-10-27T10:00:00Z"},
        "verified": [{"by": "human:ada", "at": "2026-10-27T11:00:00Z"}],
    }
    assert validate_note({**fields, **trust}, "Short")["errors"] == []
    assert "invalid_generated" in validate_note({**fields, "generated": True}, "Short")["errors"]
    assert (
        "invalid_verified"
        in validate_note({**fields, "verified": {"by": "human:ada", "at": "bad"}}, "Short")[
            "errors"
        ]
    )


async def test_fast_note_sync_contract_encryption_and_validation(vault_client):
    from starlette.concurrency import run_in_threadpool
    from support_vault_sync import TOKEN, sync_server

    client, _, store = vault_client
    peer = sync_server()
    url, calls = await run_in_threadpool(peer.__enter__)
    try:
        config = {"kind": "fast_note_sync", "url": url, "vault": "Invented Garden", "token": TOKEN}
        response = await client.put("/v1/vault/config", json=config)
        assert response.status_code == 200, response.text
        assert TOKEN not in response.text
        assert response.json()["has_token"] is True
        public = await client.get("/v1/vault/config")
        assert "token" not in public.json()
        with store.read_snapshot() as db:
            assert TOKEN.encode() not in bytes(
                db.execute("SELECT body FROM vault_configuration").fetchone()[0]
            )
        for operation in ["list", "search", "read", "backlinks", "outlinks", "index"]:
            request = {"operation": operation}
            if operation in {"read", "backlinks", "outlinks"}:
                request["path"] = "People/Ada.md"
            if operation == "search":
                request["query"] = "gardener"
            result = await client.post("/v1/vault/read", json=request)
            assert result.status_code == 200, result.text
            assert "private upstream" not in result.text
        assert {path for path, _, _ in calls} >= {
            "/api/notes",
            "/api/folder/notes",
            "/api/note",
            "/api/note/backlinks",
            "/api/note/outlinks",
        }
        assert all(
            token == TOKEN and query["vault"] == config["vault"] for _, query, token in calls
        )
        bad = await client.put("/v1/vault/config", json={**config, "token": "expired-invented"})
        assert bad.status_code == 400
        assert bad.json()["code"] == "vault_token_expired"
        assert "never relay" not in bad.text
        kept = await client.put(
            "/v1/vault/config", json={k: v for k, v in config.items() if k != "token"}
        )
        assert kept.status_code == 200
    finally:
        await run_in_threadpool(peer.__exit__, None, None, None)


async def test_sync_debug_logging_never_records_search_text_or_token(vault_client, caplog):
    import logging

    from starlette.concurrency import run_in_threadpool
    from support_vault_sync import TOKEN, sync_server

    client, _, _ = vault_client
    peer = sync_server()
    url, _ = await run_in_threadpool(peer.__enter__)
    try:
        caplog.set_level(logging.DEBUG, logger="httpx")
        caplog.set_level(logging.DEBUG, logger="httpcore.http11")
        caplog.set_level(logging.DEBUG, logger="httpcore.connection")
        config = {"kind": "fast_note_sync", "url": url, "vault": "Invented Garden", "token": TOKEN}
        assert (await client.put("/v1/vault/config", json=config)).status_code == 200
        assert (
            await client.post(
                "/v1/vault/read",
                json={"operation": "search", "query": "private-invented-search-text"},
            )
        ).status_code == 200
        assert "private-invented-search-text" not in caplog.text
        assert TOKEN not in caplog.text
    finally:
        await run_in_threadpool(peer.__exit__, None, None, None)


@pytest.mark.parametrize(
    "yaml", ["type: !!python/object/apply:os.system []", "a: &a [*a]", "[one, two]", "unclosed: ["]
)
async def test_unsafe_frontmatter_is_redacted(vault_client, yaml):
    client, root, _ = vault_client
    (root / "unsafe.md").write_text(f"---\n{yaml}\n---\nPrivate body")
    await connect(client, root)
    response = await client.post("/v1/vault/read", json={"operation": "read", "path": "unsafe.md"})
    assert response.status_code == 400
    assert response.json()["code"] == "vault_frontmatter_invalid"
    assert "Private body" not in response.text and yaml not in response.text


async def test_configuration_rejects_control_characters_in_folder_paths_in_place(vault_client):
    client, root, _ = vault_client
    await connect(client, root)
    response = await client.put(
        "/v1/vault/config", json={"kind": "folder", "path": str(root) + "/\x00"}
    )
    assert response.status_code == 400, response.text
    assert response.json()["code"] == "vault_path_refused"
    assert (await client.get("/v1/vault/config")).json()["path"] == str(root)
