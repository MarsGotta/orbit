"""Invented English vault and recorded model output, shared by HTTP acceptance."""

import copy
import json
from pathlib import Path

import httpx
import pytest
from conftest import device
from support_notes import note_answer, write_note
from test_notes import recording

from orbit_hub.ai.registry import Registry
from orbit_hub.app import create_app
from orbit_hub.meeting_final_pass import FinalPassRepository
from orbit_hub.meetings import repository as meetings
from orbit_hub.processing import Completion, ProcessingClient

TEXT = "The Cedar pilot starts on 2026-11-02. Nora will send the plan. Eli joins as designer."
SCHEMA = """---
type: schema
types:
  contact: {role: person, folder: People/}
  initiative: {role: project, folder: Projects/}
  session: {role: meeting, folder: Meetings/}
  evidence: {role: source, folder: Sources/}
fields: {stage: phase, start: kickoff}
---
Keep current facts and link tasks to their owners.
"""
PROJECT = """---
type: initiative
title: Cedar pilot
phase: planning
kickoff: '2026-11-09' # owner's date
custom: {keep: "Exactly this", nested: [one, two]}
---
# Cedar pilot

## Status
Waiting for the plan.

## Owner's words
Keep  this spacing.\tAnd this tab.
"""
FILES = {
    "SCHEMA.md": SCHEMA,
    "People/nora-vale.md": "---\ntype: contact\ntitle: Nora Vale\naliases: [Nora]\n---\nDesigner.\n",
    "People/owen-reed.md": "---\ntype: contact\ntitle: Owen Reed\n---\nCoordinator.\n",
    "Projects/cedar-pilot.md": PROJECT,
    "index.md": '---\nokf_version: "0.2"\n---\n# Garden\n\n## Projects\n* [Cedar pilot](/Projects/cedar-pilot.md) - Pilot\n',
    "log.md": "# Log\n\n## 2026-10-01\n- Older handwritten entry.\n",
}


def answer():
    return json.loads((Path(__file__).parent / "fixtures/distillation/answer.json").read_text())


@pytest.fixture
async def distillation_client(environment, tmp_path, monkeypatch):
    settings, store, adapter = environment
    owner = device(store)
    root = tmp_path / "invented-vault"
    root.mkdir()
    for path, text in FILES.items():
        target = root / path
        target.parent.mkdir(exist_ok=True)
        target.write_bytes(text.encode())
    meeting = recording(store, owner["device_id"], text=TEXT)
    meetings.set_owner_name(store, owner["device_id"], "Owen Reed", 2000)
    final = FinalPassRepository(store)
    final.request(owner["device_id"], meeting)
    job = final.claim()
    final.complete(
        job,
        {
            "words": [
                {
                    "start_ms": 0,
                    "end_ms": 900,
                    "text": TEXT,
                    "track": "mic_local",
                    "voice": "mic_local",
                },
                {
                    "start_ms": 1000,
                    "end_ms": 1400,
                    "text": "I will send the plan.",
                    "track": "remote_app",
                    "voice": "voice_1",
                },
                {
                    "start_ms": 1500,
                    "end_ms": 1900,
                    "text": "I am joining as designer.",
                    "track": "remote_app",
                    "voice": "voice_2",
                },
            ],
            "spans": [
                {"start_ms": 1000, "end_ms": 1400, "voice": "voice_1"},
                {"start_ms": 1500, "end_ms": 1900, "voice": "voice_2"},
            ],
            "audio_ms": 2000,
            "language": "en",
        },
    )
    meetings.rename(
        store,
        owner["device_id"],
        meeting,
        {"voice_1": "Nora", "voice_2": "Eli Brook"},
        {"voice_1", "voice_2", "mic_local"},
        2000,
    )
    generated = note_answer()
    generated.update(
        title="Cedar review",
        short_summary="PRIVATE NEVER EXPORT",
        sections={f"s{i}": "PRIVATE NEVER EXPORT" for i in range(1, 5)},
    )
    worker = await write_note(
        store,
        owner["device_id"],
        meeting,
        private_notes="PRIVATE NEVER EXPORT",
        answers=(generated,),
    )
    await worker.close()
    app = create_app(settings, store=store, assistant=adapter, speech=adapter, now=lambda: 2000)
    responses, calls = [answer()], []

    async def recorded(client, **kwargs):
        calls.append({"model": client.configuration.model, **kwargs})
        value = copy.deepcopy(responses[-1])
        if isinstance(value, Exception):
            raise value
        return Completion(value, "recorded-vault-model", 100, 50, 1)

    monkeypatch.setattr(ProcessingClient, "complete_json", recorded)
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app),
        base_url="https://localhost",
        headers={"Authorization": f"Bearer {owner['token']}"},
    ) as client:
        assert (
            await client.put("/v1/vault/config", json={"kind": "folder", "path": str(root)})
        ).status_code == 200
        Registry(store).write(
            {
                "providers": {
                    "fixture": {
                        "name": "Fixture",
                        "kind": "local",
                        "base_url": "http://127.0.0.1:12345/v1",
                        "models": [{"id": "vault-fixture", "capabilities": ["text"]}],
                    }
                },
                "functions": {"vault": {"provider_id": "fixture", "model": "vault-fixture"}},
            }
        )
        yield client, root, meeting, responses, calls, app, owner
    app.state.notes.executor.shutdown(wait=True)


async def prepare(client, meeting):
    response = await client.post(f"/v1/recordings/{meeting}/distillations")
    assert response.status_code == 200, response.text
    proposal = response.json()
    if proposal.get("status") == "queued":
        await client._transport.app.state.distillations.run_once()
        proposal = (await client.get("/v1/vault/distillations/" + proposal["id"])).json()
    return proposal
