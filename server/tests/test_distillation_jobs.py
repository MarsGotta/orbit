"""Job triggering, review edits, authentication and durable proposal recovery."""

import json

from conftest import device
from support_distillation import FILES, answer, distillation_client, prepare
from test_distillation import base, decide

from orbit_hub.app import create_app

__all__ = ["distillation_client"]


async def test_ready_note_job_runs_once_without_private_notes(distillation_client):
    client, _, meeting, _, calls, app, _ = distillation_client
    assert await app.state.distillations.run_once()
    assert not await app.state.distillations.run_once()
    listed = (await client.get("/v1/vault/distillations")).json()["entries"]
    assert len(listed) == 1 and listed[0]["meeting_id"] == meeting
    assert listed[0]["status"] == "proposed"
    assert len(calls) == 1 and "PRIVATE NEVER EXPORT" not in json.dumps(calls)


async def test_missing_model_or_vault_does_no_work(distillation_client):
    from orbit_hub.ai.registry import Registry

    client, _, meeting, _, calls, app, _ = distillation_client
    registry = Registry(app.state.store)
    previous = registry.read()
    registry.write({**previous, "functions": {}})
    assert not await app.state.distillations.run_once()
    response = await client.post(f"/v1/recordings/{meeting}/distillations")
    assert response.json()["prepared"] is False
    registry.write(previous)
    await client.delete("/v1/vault/config")
    assert not await app.state.distillations.run_once()
    assert not calls


async def test_edits_and_exclusions_apply_only_included_notes(distillation_client):
    client, root, meeting, _, _, _, _ = distillation_client
    proposal = await prepare(client, meeting)
    decisions = {c["id"]: "incoming" for c in proposal["conflicts"]}
    meeting_text = proposal["changes"][0]["lines"][0]["after"].replace(
        ", [Eli Brook](/People/eli-brook.md)", ""
    )
    reviewed = await client.patch(
        base(proposal),
        json={
            "revision": proposal["revision"],
            "decisions": decisions,
            "changes": {
                "n2": {"include": False},
                "n0": {"lines": {"0": meeting_text}},
                "n1": {"properties": {"kickoff": "2026-11-03"}},
            },
        },
    )
    assert reviewed.status_code == 200, reviewed.text
    assert (await client.patch(base(proposal), json={"revision": 0})).status_code == 409
    applied = await client.post(base(proposal) + "/apply")
    assert applied.status_code == 200, applied.text
    assert not (root / "People/eli-brook.md").exists()
    assert "2026-11-03" in (root / "Projects/cedar-pilot.md").read_text()


async def test_other_paired_mac_can_review_and_anonymous_cannot(distillation_client):
    client, root, meeting, _, _, app, _ = distillation_client
    proposal = await prepare(client, meeting)
    assert (
        await client.get(base(proposal), headers={"Authorization": "Bearer invented-invalid"})
    ).status_code == 401
    other = device(app.state.store, "another-fixture")
    headers = {"Authorization": "Bearer " + other["token"]}
    assert (await client.get(base(proposal), headers=headers)).status_code == 200
    assert (await decide(client, proposal)).status_code == 200
    result = await client.post(base(proposal) + "/apply", headers=headers)
    assert result.status_code == 200
    assert "human:" + other["device_id"] in (root / "Projects/cedar-pilot.md").read_text()


async def test_restart_preserves_proposal_and_undo(distillation_client, environment):
    import httpx

    client, root, meeting, _, _, _, owner = distillation_client
    proposal = await prepare(client, meeting)
    assert (await decide(client, proposal)).status_code == 200
    assert (await client.post(base(proposal) + "/apply")).status_code == 200
    settings, store, adapter = environment
    restarted = create_app(settings, store=store, assistant=adapter, speech=adapter)
    try:
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(restarted),
            base_url="https://localhost",
            headers={"Authorization": "Bearer " + owner["token"]},
        ) as second:
            assert (await second.get(base(proposal))).json()["status"] == "applied"
            undone = await second.post(base(proposal) + "/undo")
            assert undone.status_code == 200 and undone.json()["skipped"] == []
        assert (root / "Projects/cedar-pilot.md").read_text() == FILES["Projects/cedar-pilot.md"]
    finally:
        restarted.state.notes.executor.shutdown(wait=True)


async def test_edited_invalid_type_is_refused_without_mutating_proposal(distillation_client):
    client, _, meeting, _, _, _, _ = distillation_client
    proposal = await prepare(client, meeting)
    response = await client.patch(
        base(proposal), json={"revision": 0, "changes": {"n0": {"properties": {"type": "unknown"}}}}
    )
    assert response.status_code == 400
    assert (await client.get(base(proposal))).json() == proposal


async def test_large_note_warns_without_rejecting(distillation_client):
    client, _, meeting, responses, _, _, _ = distillation_client
    value = answer()
    value["changes"][0]["lines"][0]["after"] += "\n" + "word " * 410
    responses.append(value)
    proposal = await prepare(client, meeting)
    assert proposal["status"] == "proposed"
    warning = proposal["changes"][0]["warnings"][0]
    assert warning["code"] == "word_limit" and warning["words"] > warning["max_words"] == 400


async def test_restart_marks_incomplete_preparation_failed(distillation_client, environment):
    _client, _, meeting, _, _, app, owner = distillation_client
    service = app.state.distillations
    value = service.repository.create(
        owner["device_id"], meeting, service.context(), "2026-10-30T12:00:00+00:00"
    )
    settings, store, adapter = environment
    restarted = create_app(settings, store=store, assistant=adapter, speech=adapter)
    try:
        recovered = restarted.state.distillations.repository.get(value["id"])
        assert recovered["status"] == "failed"
        assert recovered["error"]["code"] == "preparation_interrupted"
    finally:
        restarted.state.notes.executor.shutdown(wait=True)


async def test_input_context_note_changed_blocks_every_write(distillation_client):
    client, root, meeting, _, _, _, _ = distillation_client
    proposal = await prepare(client, meeting)
    assert (await decide(client, proposal)).status_code == 200
    (root / "People/nora-vale.md").write_text(FILES["People/nora-vale.md"] + "\nNew identity.\n")
    response = await client.post(base(proposal) + "/apply")
    assert response.status_code == 409
    assert (await client.get(base(proposal))).json()["changed_paths"] == ["People/nora-vale.md"]
    assert not (root / "Meetings").exists()


async def test_duplicate_person_from_alias_is_failed_proposal(distillation_client):
    client, _, meeting, responses, _, _, _ = distillation_client
    value = answer()
    value["changes"][2]["properties"][1]["after"] = "Nora"
    responses.append(value)
    proposal = await prepare(client, meeting)
    assert proposal["status"] == "failed" and proposal["error"]["code"] == "person_already_exists"


async def test_undo_twice_is_idempotent(distillation_client):
    client, root, meeting, _, _, _, _ = distillation_client
    proposal = await prepare(client, meeting)
    assert (await decide(client, proposal)).status_code == 200
    assert (await client.post(base(proposal) + "/apply")).status_code == 200
    first = await client.post(base(proposal) + "/undo")
    log = (root / "log.md").read_bytes()
    assert (await client.post(base(proposal) + "/undo")).json() == first.json()
    assert (root / "log.md").read_bytes() == log


async def test_source_byte_bound_fails_before_any_vault_write(distillation_client):
    from orbit_hub.vault.models import MAX_NOTE_BYTES

    client, root, meeting, _, _, app, _ = distillation_client
    proposal = await prepare(client, meeting)
    value = app.state.distillations.repository.get(proposal["id"])
    value["source_raw"] += "x" * MAX_NOTE_BYTES
    # A corrupted/incomplete persisted proposal cannot bypass final validation.
    app.state.distillations.repository.save(value)
    assert (await decide(client, proposal)).status_code == 200
    result = await client.post(base(proposal) + "/apply")
    assert result.status_code == 413, result.text
    assert {str(p.relative_to(root)): p.read_text() for p in root.rglob("*.md")} == FILES


async def test_undo_skips_later_index_edit_and_keeps_log_prose(distillation_client):
    client, root, meeting, _, _, _, _ = distillation_client
    proposal = await prepare(client, meeting)
    assert (await decide(client, proposal)).status_code == 200
    assert (await client.post(base(proposal) + "/apply")).status_code == 200
    index = (root / "index.md").read_text() + "\nPersonal index entry.\n"
    (root / "index.md").write_text(index)
    log = (root / "log.md").read_text() + "\nPersonal ledger entry.\n"
    (root / "log.md").write_text(log)
    result = await client.post(base(proposal) + "/undo")
    assert result.status_code == 200
    assert {"path": "index.md", "code": "content_changed"} in result.json()["skipped"]
    assert (root / "index.md").read_text() == index
    assert "Personal ledger entry." in (root / "log.md").read_text()


async def test_vault_uses_its_operator_default_model(distillation_client, environment):
    from support_notes import FakeModel, configuration

    from orbit_hub.ai.registry import Registry

    _, _, meeting, _, _, _original, owner = distillation_client
    settings, store, adapter = environment
    document = Registry(store).read()
    Registry(store).write({**document, "functions": {}})
    from dataclasses import replace

    settings = replace(settings, processing=configuration())
    model = FakeModel()
    model.script("vault_distillation", answer())
    app = create_app(
        settings, store=store, assistant=adapter, speech=adapter, processing_client=model
    )
    try:
        proposal = await app.state.distillations.prepare(owner["device_id"], meeting)
        assert proposal["status"] == "proposed", proposal
        assert model.purposes() == ["vault_distillation"]
    finally:
        app.state.notes.executor.shutdown(wait=True)


async def test_applied_new_prose_has_source_citations(distillation_client):
    client, root, meeting, _, _, _, _ = distillation_client
    proposal = await prepare(client, meeting)
    assert (await decide(client, proposal)).status_code == 200
    assert (await client.post(base(proposal) + "/apply")).status_code == 200
    note = (root / "Meetings/cedar-review.md").read_text()
    assert "November 2.[^" in note
    parsed = await client.post(
        "/v1/vault/read", json={"operation": "read", "path": "Meetings/cedar-review.md"}
    )
    assert parsed.status_code == 200, parsed.text
    assert parsed.json()["citations"][0]["resource"].startswith("/Sources/")
    assert any(
        s["resource"] == "orbit://meeting/" + meeting
        for s in parsed.json()["frontmatter"]["sources"]
    )
    assert parsed.json()["tasks"][0]["who_path"] == "People/nora-vale.md"


async def test_roundtrip_preserves_crlf_comments_and_unknown_nested_properties(distillation_client):
    client, root, meeting, _, _, _, _ = distillation_client
    target = root / "Projects/cedar-pilot.md"
    original = (
        FILES["Projects/cedar-pilot.md"]
        .replace("custom:", "# Independent operator comment\ncustom:")
        .replace("\n", "\r\n")
    )
    target.write_bytes(original.encode())
    proposal = await prepare(client, meeting)
    assert (await decide(client, proposal)).status_code == 200
    applied = await client.post(base(proposal) + "/apply")
    assert applied.status_code == 200, applied.text
    raw = target.read_bytes().decode()
    assert (
        '# Independent operator comment\r\ncustom: {keep: "Exactly this", nested: [one, two]}\r\n'
        in raw
    )
    assert "## Owner's words\r\nKeep  this spacing.\tAnd this tab.\r\n" in raw


async def test_new_person_can_be_unchecked_without_broken_links(distillation_client):
    client, root, meeting, _, _, _, _ = distillation_client
    proposal = await prepare(client, meeting)
    reviewed = await client.patch(
        base(proposal),
        json={
            "revision": 0,
            "changes": {"n2": {"include": False}},
            "decisions": {c["id"]: "incoming" for c in proposal["conflicts"]},
        },
    )
    assert reviewed.status_code == 200, reviewed.text
    assert (await client.post(base(proposal) + "/apply")).status_code == 200
    assert not (root / "People/eli-brook.md").exists()
    assert "Eli Brook" in (root / "Meetings/cedar-review.md").read_text()
    assert "/People/eli-brook.md" not in (root / "Meetings/cedar-review.md").read_text()


async def test_on_demand_is_durable_queued_work_before_model_runs(distillation_client):
    client, _, meeting, _, calls, app, _ = distillation_client
    queued = await client.post(f"/v1/recordings/{meeting}/distillations")
    assert queued.status_code == 200 and queued.json()["status"] == "queued"
    assert not calls
    assert (await client.get(base(queued.json()))).json()["status"] == "queued"
    assert await app.state.distillations.run_once()
    assert (await client.get(base(queued.json()))).json()["status"] == "proposed"


async def test_note_worker_loop_prepares_when_note_is_ready(distillation_client):
    import asyncio

    client, _, meeting, _, calls, app, _ = distillation_client
    prepared = asyncio.Event()
    app.state.distillations.changed = lambda _device, _recording: prepared.set()
    app.state.notes.start()
    try:
        await asyncio.wait_for(prepared.wait(), 10)
        proposals = (
            await client.get("/v1/vault/distillations", params={"meeting_id": meeting})
        ).json()["entries"]
        assert len(proposals) == 1 and proposals[0]["status"] == "proposed"
        assert len(calls) == 1
    finally:
        await app.state.notes.close()


async def test_generated_metadata_extensions_survive_apply(distillation_client):
    import yaml

    client, root, meeting, _, _, _, _ = distillation_client
    path = root / "Projects/cedar-pilot.md"
    original = FILES["Projects/cedar-pilot.md"].replace(
        "phase: planning",
        'generated: {by: legacy/1, at: "2026-10-01T12:00:00+00:00", custom: {flag: true, count: 7}}\nphase: planning',
    )
    path.write_text(original)
    proposal = await prepare(client, meeting)
    assert (await decide(client, proposal)).status_code == 200
    assert (await client.post(base(proposal) + "/apply")).status_code == 200
    fields = yaml.safe_load(path.read_text().split("---", 2)[1])
    assert fields["generated"]["custom"] == {"flag": True, "count": 7}
