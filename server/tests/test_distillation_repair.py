"""A small model's slips are repaired without invention; what still fails is dropped, not all."""

import pytest
from support_distillation import FILES, answer, distillation_client, prepare

# Re-export the fixture for pytest without hiding its public HTTP setup.
__all__ = ["distillation_client"]


def meeting_text(proposal):
    change = next(c for c in proposal["changes"] if c["path"] == "Meetings/cedar-review.md")
    return change["lines"][0]["after"]


async def test_obsidian_links_become_vault_links_or_plain_text(distillation_client):
    # Recorded from a Spanish meeting: the model wrote Obsidian links.
    client, _, meeting, responses, _, _, _ = distillation_client
    value = answer()
    value["changes"][0]["lines"][0]["after"] = (
        "## Resumen\nEl piloto empieza el 2 de noviembre.\n\n"
        "## Personas\n[[Owen Reed]], [[Nora Vale|Nora]], [[Eli Brook]] y [[Ivy Marsh]].\n\n"
        "## Tareas\n- [ ] [[Nora Vale]] Enviar el plan 📅 2026-11-02 🆔 plan\n"
        "- [ ] Lanzar [[Cedar pilot|el piloto]] ⛔ plan\n"
    )
    responses.append(value)
    proposal = await prepare(client, meeting)
    assert proposal["status"] == "proposed", proposal["error"]
    assert proposal["dropped"] == []
    text = meeting_text(proposal)
    assert "[[" not in text and "]]" not in text
    assert "[Owen Reed](/People/owen-reed.md)" in text
    assert "[Nora](/People/nora-vale.md)" in text
    # A note this proposal creates is a target too; nothing names Ivy Marsh.
    assert "[Eli Brook](/People/eli-brook.md) y Ivy Marsh." in text
    assert "- [ ] [Nora Vale](/People/nora-vale.md) Enviar el plan 📅 2026-11-02" in text
    assert "Lanzar [el piloto](/Projects/cedar-pilot.md) ⛔ plan" in text


async def test_an_invalid_task_date_is_removed_and_the_task_kept(distillation_client):
    # Recorded from an English meeting: dates the vault cannot read.
    client, _, meeting, responses, _, _, _ = distillation_client
    value = answer()
    value["changes"][0]["lines"][0]["after"] = (
        "## Summary\nThe pilot starts soon.\n\n## Tasks\n"
        "- [ ] [Nora Vale](/People/nora-vale.md) Send the plan 📅 next Friday 🆔 plan\n"
        "- [ ] Book the room 📅 2026-11-31\n"
        "- [ ] Launch [Cedar pilot](/Projects/cedar-pilot.md) 📅 2026-11-09 ⛔ plan\n"
    )
    responses.append(value)
    proposal = await prepare(client, meeting)
    assert proposal["status"] == "proposed", proposal["error"]
    assert proposal["dropped"] == []
    assert meeting_text(proposal).endswith(
        "## Tasks\n"
        "- [ ] [Nora Vale](/People/nora-vale.md) Send the plan 🆔 plan\n"
        "- [ ] Book the room\n"
        "- [ ] Launch [Cedar pilot](/Projects/cedar-pilot.md) 📅 2026-11-09 ⛔ plan\n"
    )


def _invalid_person(value):
    value["changes"][2]["properties"][0]["after"] = "undeclared"
    return "People/eli-brook.md", "type_or_role_not_writable", 2


def _broken_project_link(value):
    value["changes"][1]["lines"][0]["after"] = "See [Birch](/Projects/birch.md)."
    return "Projects/cedar-pilot.md", "new_link_target_missing", 0


@pytest.mark.parametrize("fault", [_invalid_person, _broken_project_link])
async def test_an_invalid_note_is_dropped_with_its_cause_and_the_rest_proposed(
    distillation_client, fault
):
    client, root, meeting, responses, _, _, _ = distillation_client
    value = answer()
    path, code, conflicts = fault(value)
    responses.append(value)
    proposal = await prepare(client, meeting)
    assert proposal["status"] == "proposed", proposal["error"]
    assert proposal["dropped"] == [{"path": path, "code": code}]
    assert path not in [c["path"] for c in proposal["changes"]]
    assert len(proposal["changes"]) == 2 and len(proposal["conflicts"]) == conflicts
    if path == "People/eli-brook.md":
        # A link to a note that is no longer created stays as its words.
        assert "[Nora Vale](/People/nora-vale.md), Eli Brook." in meeting_text(proposal)
    decided = await client.patch(
        "/v1/vault/distillations/" + proposal["id"],
        json={
            "revision": proposal["revision"],
            "decisions": {c["id"]: "incoming" for c in proposal["conflicts"]},
        },
    )
    assert decided.status_code == 200, decided.text
    applied = await client.post("/v1/vault/distillations/" + proposal["id"] + "/apply")
    assert applied.status_code == 200 and applied.json()["status"] == "applied", applied.text
    assert (root / "Meetings/cedar-review.md").exists()
    if path in FILES:
        assert (root / path).read_text() == FILES[path]
    else:
        assert not (root / path).exists()


async def test_nothing_valid_left_fails_with_the_cause(distillation_client):
    client, root, meeting, responses, _, _, _ = distillation_client
    value = answer()
    value["changes"][0]["properties"][0]["after"] = "undeclared"
    value["changes"][1]["lines"][0]["before"] = "Text that is not in the note."
    value["changes"][2]["properties"][0]["after"] = "undeclared"
    responses.append(value)
    proposal = await prepare(client, meeting)
    assert proposal["status"] == "failed"
    assert proposal["error"] == {"code": "type_or_role_not_writable"}
    assert proposal["changes"] == [] and proposal["conflicts"] == []
    assert (
        await client.post("/v1/vault/distillations/" + proposal["id"] + "/apply")
    ).status_code == 409
    assert {p: (root / p).read_bytes().decode() for p in FILES} == FILES
