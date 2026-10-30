"""Distillation through authenticated HTTP, using a recorded model and invented vault."""

import hashlib
import json

import pytest
from support_distillation import FILES, answer, distillation_client, prepare

from orbit_hub.processing import ProcessingError

# Re-export the fixture for pytest without hiding its public HTTP setup.
__all__ = ["distillation_client"]


def base(proposal):
    return "/v1/vault/distillations/" + proposal["id"]


async def decide(client, proposal, choice="incoming"):
    return await client.patch(
        base(proposal),
        json={
            "revision": proposal["revision"],
            "decisions": {c["id"]: choice for c in proposal["conflicts"]},
        },
    )


async def test_prepare_citations_aliases_privacy_and_read_routes(distillation_client):
    client, root, meeting, _, calls, _, _ = distillation_client
    proposal = await prepare(client, meeting)
    assert proposal["status"] == "proposed", proposal["error"]
    assert len(proposal["changes"]) == 3
    assert len(proposal["conflicts"]) == 2  # property and owner's prose
    assert proposal["people"]["Nora"] == "People/nora-vale.md"
    assert proposal["people"]["Owen Reed"] == "People/owen-reed.md"
    assert proposal["people"]["Eli Brook"] is None
    assert calls[0]["model"] == "vault-fixture"
    assert "PRIVATE NEVER EXPORT" not in json.dumps(calls)
    assert "PRIVATE NEVER EXPORT" not in json.dumps(proposal)
    assert (await client.get(base(proposal))).json() == proposal
    listed = (await client.get("/v1/vault/distillations", params={"meeting_id": meeting})).json()
    assert listed["entries"][0]["id"] == proposal["id"]
    assert {p: (root / p).read_bytes().decode() for p in FILES} == FILES


@pytest.mark.parametrize("choice", ["incoming", "current", "both"])
async def test_conflicts_apply_and_undo_with_later_edit(distillation_client, choice):
    client, root, meeting, _, _, _, _ = distillation_client
    proposal = await prepare(client, meeting)
    blocked = await client.post(base(proposal) + "/apply")
    assert blocked.status_code == 409 and blocked.json()["code"] == "distillation_undecided"
    proposal = (await decide(client, proposal, choice)).json()
    applied = await client.post(base(proposal) + "/apply")
    assert applied.status_code == 200, applied.text
    assert applied.json()["status"] == "applied"
    project = (root / "Projects/cedar-pilot.md").read_text()
    assert 'custom: {keep: "Exactly this", nested: [one, two]}' in project
    assert "Keep  this spacing.\tAnd this tab." in project
    assert "human:owen-reed" in project and "orbit-hub/" in project
    assert ("2026-11-02" in project) == (choice != "current")
    assert ("in discussion" in project) == (choice == "both")
    source = root / applied.json()["new_source"]["path"]
    assert source.exists() and "PRIVATE NEVER EXPORT" not in source.read_text()
    assert "sha256:" in source.read_text() and "Owen Reed:" in source.read_text()
    source_view = (
        await client.post(
            "/v1/vault/read", json={"operation": "read", "path": str(source.relative_to(root))}
        )
    ).json()
    assert (
        source_view["frontmatter"]["sha256"]
        == hashlib.sha256(source_view["body"].encode()).hexdigest()
    )
    assert "[Cedar review](/Meetings/cedar-review.md)" in (root / "index.md").read_text()
    assert " · create · " in (root / "log.md").read_text()
    assert " · update · " in (root / "log.md").read_text()
    assert (await client.post(base(proposal) + "/apply")).json()["status"] == "applied"
    (root / "Projects/cedar-pilot.md").write_text(project + "\nOwner edited later.\n")
    undo = await client.post(base(proposal) + "/undo")
    assert undo.status_code == 200, undo.text
    assert undo.json()["skipped"] == [
        {"path": "Projects/cedar-pilot.md", "code": "content_changed"}
    ]
    assert not (root / "Meetings/cedar-review.md").exists()
    assert (
        source.exists() and "Owner edited later." in (root / "Projects/cedar-pilot.md").read_text()
    )
    assert "Undo" in (root / "log.md").read_text()


async def test_hash_preflight_is_all_or_nothing_and_reprepare(distillation_client):
    client, root, meeting, _, _, _, _ = distillation_client
    proposal = (await decide(client, await prepare(client, meeting))).json()
    (root / "Projects/cedar-pilot.md").write_text(FILES["Projects/cedar-pilot.md"] + "\nChanged.\n")
    before = {str(p.relative_to(root)): p.read_bytes() for p in root.rglob("*.md")}
    response = await client.post(base(proposal) + "/apply")
    assert response.status_code == 409, response.text
    saved = (await client.get(base(proposal))).json()
    assert saved["changed_paths"] == ["Projects/cedar-pilot.md"] and saved["can_prepare_again"]
    assert before == {str(p.relative_to(root)): p.read_bytes() for p in root.rglob("*.md")}
    again = await client.post(base(proposal) + "/prepare")
    assert again.status_code == 200 and again.json()["id"] != proposal["id"]


@pytest.mark.parametrize("fault", ["missing", "schema", "quote", "source", "incomplete"])
async def test_invalid_model_never_applies(distillation_client, fault):
    client, root, meeting, responses, _, _, _ = distillation_client
    value = answer()
    if fault == "missing":
        del value["changes"]
    elif fault == "schema":
        value["changes"][0]["properties"][0]["after"] = "undeclared"
    elif fault == "quote":
        value["quotes"][0]["text"] = "Invented speech"
    elif fault == "source":
        value["changes"][0]["path"] = "Sources/overwrite.md"
    else:
        value = ProcessingError(
            "incomplete_completion", completion_tokens=4096, max_completion_tokens=4096
        )
    responses.append(value)
    proposal = await prepare(client, meeting)
    assert proposal["status"] == "failed" and proposal["error"]["code"]
    if fault == "incomplete":
        assert proposal["error"]["max_completion_tokens"] == 4096
    assert (await client.post(base(proposal) + "/apply")).status_code == 409
    assert {p: (root / p).read_bytes().decode() for p in FILES} == FILES


async def test_discard_writes_nothing_and_deleted_meeting_removes_pending(distillation_client):
    client, root, meeting, _, _, _, _ = distillation_client
    proposal = await prepare(client, meeting)
    result = await client.post(base(proposal) + "/discard")
    assert result.json()["status"] == "discarded"
    assert {p: (root / p).read_bytes().decode() for p in FILES} == FILES
    pending = await prepare(client, meeting)
    assert (await client.delete(f"/v1/recordings/{meeting}")).status_code == 200
    assert (await client.get(base(pending))).status_code == 404
    assert (await client.post(f"/v1/recordings/{meeting}/distillations")).status_code == 404
