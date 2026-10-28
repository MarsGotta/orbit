"""MCP tools through the real signed hub route, including access revocation."""

from dataclasses import replace

import httpx
from conftest import BRIDGE_NOW_MS, device
from support_orbit_mcp import text_of

from orbit_hub.app import create_app
from orbit_hub.assistant.agent_config import AgentConfigStore
from orbit_mcp.config import Config, Credentials
from orbit_mcp.hub import HubClient
from orbit_mcp.server import build_server


async def test_notes_tools_read_without_a_mac_and_enforce_notes_group(environment, tmp_path):
    settings, store, adapter = environment
    owner = device(store)
    key = bytes(range(32))
    credential = tmp_path / "synthetic-key"
    credential.write_bytes(key)
    credential.chmod(0o600)
    settings = replace(
        settings, meetings_bridge={"key_id": "vault-test", "credential_file": str(credential)}
    )
    app = create_app(
        settings, store=store, assistant=adapter, speech=adapter, now=lambda: BRIDGE_NOW_MS / 1000
    )
    root = tmp_path / "garden"
    root.mkdir()
    (root / "SCHEMA.md").write_text(
        "---\ntype: schema\ntypes: {contact: {role: person}}\n---\nFree prose."
    )
    (root / "Ada.md").write_text(
        "---\ntype: contact\ndescription: A gardener.\n---\n# Ada\nA gardener."
    )
    factory = lambda **kwargs: httpx.AsyncClient(transport=httpx.ASGITransport(app), **kwargs)
    config = Config(
        "https://localhost",
        Credentials("unused", key),
        Credentials("vault-test", key),
        "no-mac",
        {},
        agent_id="default",
    )
    client = HubClient(config, client_factory=factory, clock=lambda: BRIDGE_NOW_MS)
    server = build_server(config, client)
    async with app.router.lifespan_context(app):
        async with factory(
            base_url="https://localhost", headers={"Authorization": f"Bearer {owner['token']}"}
        ) as mac:
            assert (
                await mac.put("/v1/vault/config", json={"kind": "folder", "path": str(root)})
            ).status_code == 200
        names = {tool.name for tool in await server.list_tools()}
        assert {"orbit_notes_search", "orbit_note_read", "orbit_notes_index"} <= names
        for name, args in [
            ("orbit_notes_search", {"query": "gardener"}),
            ("orbit_note_read", {"path": "Ada.md"}),
            ("orbit_notes_index", {}),
        ]:
            result = await server.call_tool(name, args)
            assert not result.is_error, text_of(result)
            assert "Ada" in text_of(result)
            if name == "orbit_note_read":
                assert '"role": "person"' in text_of(result)
                assert '"description": "A gardener."' in text_of(result)
        configs = AgentConfigStore(store)
        configs.set_access("default", {**configs.access("default"), "notes": False})
        # Even an old translator with a cached list is refused by the real hub.
        denied = await server.call_tool("orbit_note_read", {"path": "Ada.md"})
        assert denied.is_error
        assert "agent_access_disabled" in text_of(denied)
        assert "notes" in text_of(denied)
