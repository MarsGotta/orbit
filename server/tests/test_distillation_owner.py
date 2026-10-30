"""Schema owner attribution through device HTTP, with an invented Cedar vault."""

import pytest
from support_distillation import distillation_client, prepare

__all__ = ["distillation_client"]


@pytest.mark.parametrize("owner", ["cedar-owner", "cedar-équipe", None])
async def test_schema_owner_is_used_for_human_verification(distillation_client, owner):
    client, root, meeting, _, _, _, _ = distillation_client
    if owner:
        schema = root / "SCHEMA.md"
        schema.write_text(schema.read_text().replace("---\n", f"---\nowner: {owner}\n", 1))
    catalog = (await client.post("/v1/vault/read", json={"operation": "catalog"})).json()
    assert catalog["schema"].get("owner") == owner
    proposal = await prepare(client, meeting)
    base = "/v1/vault/distillations/" + proposal["id"]
    review = await client.patch(
        base,
        json={
            "revision": proposal["revision"],
            "decisions": {c["id"]: "incoming" for c in proposal["conflicts"]},
        },
    )
    assert review.status_code == 200
    applied = await client.post(base + "/apply")
    assert applied.status_code == 200, applied.text
    project = (
        await client.post(
            "/v1/vault/read",
            json={
                "operation": "read",
                "path": "Projects/cedar-pilot.md",
            },
        )
    ).json()
    assert project["frontmatter"]["verified"][-1]["by"] == "human:" + (owner or "owen-reed")


@pytest.mark.parametrize(
    "owner", ["[cedar]", "''", "'human:cedar'", "'cedar owner'", "'{cedar: owner}'"]
)
async def test_invalid_owner_prevents_preparation(distillation_client, owner):
    client, root, meeting, _, _, _, _ = distillation_client
    schema = root / "SCHEMA.md"
    schema.write_text(schema.read_text().replace("---\n", f"---\nowner: {owner}\n", 1))
    result = await prepare(client, meeting)
    assert result["status"] == "failed"
    assert result["error"]["code"] == "schema_owner_invalid"
