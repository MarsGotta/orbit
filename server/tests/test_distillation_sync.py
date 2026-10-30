"""Versioned REST writes and failure recovery through the app's public routes."""

import pytest
from support_distillation import FILES, distillation_client, prepare
from support_distillation_sync import sync_peer
from test_distillation import base, decide

__all__ = ["distillation_client"]


async def connect(client, url):
    result = await client.put(
        "/v1/vault/config",
        json={
            "kind": "fast_note_sync",
            "url": url,
            "vault": "Invented Garden",
            "token": "invented-token",
        },
    )
    assert result.status_code == 200, result.text


async def test_sync_apply_versions_sources_and_undo(distillation_client):
    client, _, meeting, _, _, _, _ = distillation_client
    with sync_peer(FILES) as (url, state):
        await connect(client, url)
        proposal = await prepare(client, meeting)
        assert proposal["status"] == "proposed", proposal["error"]
        reviewed = await decide(client, proposal)
        assert reviewed.status_code == 200, reviewed.text
        applied = await client.post(base(proposal) + "/apply")
        assert applied.status_code == 200, applied.text
        note = state["notes"]["Projects/cedar-pilot.md"]
        assert note["version"] == 2 and "2026-11-02" in note["content"]
        request = next(w for w in state["writes"] if w["path"] == "Projects/cedar-pilot.md")
        assert (
            request["baseHash"] == state["histories"]["Projects/cedar-pilot.md"][0]["contentHash"]
        )
        assert all(w["createOnly"] == (w["path"] not in FILES) for w in state["writes"])
        source = applied.json()["new_source"]["path"]
        undone = await client.post(base(proposal) + "/undo")
        assert undone.status_code == 200, undone.text
        assert undone.json()["skipped"] == []
        assert (
            state["notes"]["Projects/cedar-pilot.md"]["content"] == FILES["Projects/cedar-pilot.md"]
        )
        assert state["notes"]["Projects/cedar-pilot.md"]["version"] == 3
        assert source in state["notes"] and "Meetings/cedar-review.md" not in state["notes"]


@pytest.mark.parametrize("failure", ["conflict", "http", "receipt"])
async def test_sync_preflight_or_compensation_never_claims_success(distillation_client, failure):
    client, _, meeting, _, _, _, _ = distillation_client
    with sync_peer(FILES) as (url, state):
        await connect(client, url)
        proposal = await prepare(client, meeting)
        assert proposal["status"] == "proposed", proposal["error"]
        assert (await decide(client, proposal)).status_code == 200
        if failure == "conflict":
            state["notes"]["Projects/cedar-pilot.md"]["version"] += 1
        elif failure == "http":
            state["fail_path"] = "Projects/cedar-pilot.md"
        else:
            state["lose_receipt"] = "Projects/cedar-pilot.md"
        result = await client.post(base(proposal) + "/apply")
        assert result.status_code == 409, result.text
        saved = (await client.get(base(proposal))).json()
        assert saved["status"] in {"stale", "failed", "recovery_required"}
        if failure == "conflict":
            assert not state["writes"]
        elif failure == "http":
            assert {p: n["content"] for p, n in state["notes"].items()} == FILES
        else:
            writes = [w for w in state["writes"] if w["path"] == "Projects/cedar-pilot.md"]
            assert sum(w["content"] != FILES["Projects/cedar-pilot.md"] for w in writes) == 1
            assert saved["status"] == "recovery_required"


async def test_preparation_has_a_separate_background_io_budget(distillation_client, monkeypatch):
    from orbit_hub.vault import fast_note_sync

    client, _, meeting, _, _, _, _ = distillation_client
    with sync_peer(FILES) as (url, _state):
        await connect(client, url)
        ticks = iter(range(10000))
        # Deterministic connector accounting, no sleeping or timing assertion.
        # A scan consumes more than an app read's seven seconds of this clock.
        monkeypatch.setattr(fast_note_sync, "monotonic", lambda: next(ticks))
        proposal = await prepare(client, meeting)
        assert proposal["status"] == "proposed", proposal["error"]
