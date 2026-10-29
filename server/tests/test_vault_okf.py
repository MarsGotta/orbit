"""OKF lifecycle, soft conformance and reserved files through the public API."""

import pytest
from test_vault_format import put, read
from test_vault_format import vault as fixture_vault

vault = fixture_vault


async def test_lifecycle_and_project_stage_are_independent(vault):
    client, root = vault
    put(
        root,
        "SCHEMA.md",
        "---\ntype: schema\nfields: {stage: phase, start: kickoff}\n---\nFree prose",
    )
    for status in ("draft", "stable", "deprecated", ""):
        fields = f"status: {status}\n" if status else ""
        put(
            root,
            f"{status or 'default'}.md",
            f"---\ntype: project\nphase: active\nkickoff: 2026-11-01\n{fields}generated: {{by: orbit/1.0, at: '2026-10-28T12:30:00+02:00'}}\n---\nGarden",
        )
        note = await read(client, "read", path=f"{status or 'default'}.md")
        assert note["properties"]["status"] == (status or "stable")
        assert note["properties"]["stage"] == "active"
        assert note["conformance"] == {"conformant": True, "issues": []}
    catalog = await read(client, "catalog")
    assert catalog["schema"]["fields"] == {"stage": "phase", "start": "kickoff"}
    assert all(n["state"] == "active" for n in catalog["notes"])
    assert all(n["modified_at"] == "2026-10-28T12:30:00+02:00" for n in catalog["notes"])


@pytest.mark.parametrize(
    "fields,location",
    [
        ("status: active", "status"),
        ("description: summary", "description"),
        ("tags: labels", "tags"),
        ("sources: evidence", "sources"),
        ("stage: status", "stage"),
    ],
)
async def test_schema_cannot_rename_or_reuse_okf_fields(vault, fields, location):
    client, root = vault
    put(root, "SCHEMA.md", f"---\ntype: schema\nfields: {{{fields}}}\n---\nText")
    catalog = await read(client, "catalog")
    assert catalog["schema"]["fields"] == {"stage": "stage", "start": "start"}
    assert any(f"fields.{location}" in warning for warning in catalog["schema"]["warnings"])


@pytest.mark.parametrize(
    "path,fields",
    [
        ("plain.md", ""),
        ("People/ada.md", "title: Ada Vale\n"),
        ("SCHEMA.md", "types: {contact: {role: person, folder: People/}}\n"),
        ("README.md", "type: ' '\n"),
        ("AGENTS.md", "type: []\n"),
    ],
)
async def test_missing_type_is_exposed_without_refusing_or_inventing_conformance(
    vault, path, fields
):
    client, root = vault
    if path != "SCHEMA.md":
        put(
            root,
            "SCHEMA.md",
            "---\ntype: schema\ntypes: {contact: {role: person, folder: People/}}\n---\nRules",
        )
    put(root, path, f"---\n{fields}---\nReadable content")
    note = await read(client, "read", path=path)
    assert note["body"] == "Readable content"
    assert note["conformance"]["conformant"] is False
    assert any(
        issue["field"] == "type" and "type" in issue["message"]
        for issue in note["conformance"]["issues"]
    )
    if path.startswith("People/"):
        assert note["type"] == "contact"
        assert note["role"] == "person"
        assert "type" not in note["frontmatter"]


@pytest.mark.parametrize("kind", ["schema", "An arbitrary concept", "Custom Type"])
async def test_schema_is_a_concept_with_any_nonempty_type(vault, kind):
    client, root = vault
    put(root, "SCHEMA.md", f"---\ntype: {kind}\n---\nFree prose")
    note = await read(client, "read", path="SCHEMA.md")
    assert note["frontmatter"]["type"] == kind
    assert note["conformance"] == {"conformant": True, "issues": []}


@pytest.mark.parametrize(
    "metadata,field",
    [
        ("status: active", "status"),
        ("status: []", "status"),
        ("generated: {by: orbit, at: '2026-10-28T12:00:00Z'}", "generated.by"),
        ("generated: {by: orbit/1.0, at: '2026-10-28'}", "generated.at"),
        ("generated: {by: orbit/1.0, at: '2026-10-28T12:00:00'}", "generated.at"),
        ("verified: [{by: 'human:', at: '2026-10-28T12:00:00Z'}]", "verified[0].by"),
        ("verified: {by: 'process:check', at: '2026-10-28'}", "verified[0].at"),
        ("stale_after: 2026-10-28", "stale_after"),
        ("stale_after: 2026-10-28T12:00:00", "stale_after"),
    ],
)
async def test_invalid_metadata_is_soft_and_names_the_field(vault, metadata, field):
    client, root = vault
    raw = f"---\ntype: note\n{metadata}\n---\nAlways readable"
    put(root, "garden.md", raw)
    note = await read(client, "read", path="garden.md")
    assert note["raw"] == raw
    assert note["body"] == "Always readable"
    assert note["conformance"]["conformant"] is False
    issues = [issue for issue in note["conformance"]["issues"] if issue["field"] == field]
    assert issues and field in issues[0]["message"]
    if field == "status":
        assert issues == [
            {
                "field": "status",
                "code": "invalid_status",
                "message": "status: must be draft, stable or deprecated; defaults to stable when absent.",
            }
        ]
    assert (await read(client, "catalog"))["notes"][0]["title"] == "garden"


@pytest.mark.parametrize("actor", ["orbit/1.0", "human:ada", "process:nightly"])
async def test_provenance_normalization_preserves_unknown_keys(vault, actor):
    client, root = vault
    raw = f"""---
type: note
custom: {{nested: [alpha, beta]}}
generated: {{by: '{actor}', at: '2026-10-28T12:00:00Z', custom: keep}}
verified: {{by: 'human:ada', at: '2026-10-28T13:30:00+01:00', custom: keep}}
stale_after: 2026-11-01T00:00:00-04:00
sources: [{{id: plan, resource: /plan.md, custom: keep}}]
---
Text
"""
    put(root, "garden.md", raw)
    note = await read(client, "read", path="garden.md")
    assert note["conformance"] == {"conformant": True, "issues": []}
    assert note["properties"]["verified"] == [note["frontmatter"]["verified"]]
    assert note["frontmatter"]["custom"] == {"nested": ["alpha", "beta"]}
    assert note["frontmatter"]["generated"]["custom"] == "keep"
    assert note["frontmatter"]["sources"][0]["custom"] == "keep"
    assert note["raw"] == raw
    assert (await read(client, "catalog"))["notes"][0]["modified_at"] == "2026-10-28T12:00:00Z"


async def test_root_nested_indexes_and_free_prose_logs_are_tolerated(vault):
    client, root = vault
    index = '---\nokf_version: "0.2"\n---\n# People\n* [Ada](/People/ada.md) - A gardener.\n'
    nested = "# People\n* [Ada](/People/ada.md) - A gardener.\n"
    log = "# Update log\n\n## 2026-10-28\n* **Update**: Checked the garden.\n  Kept the sources unchanged.\n\n- 12:30 · apply · [Ada](/People/ada.md) · process:nightly · Checked sources\n\n## 2026-10-27\nAn older prose entry.\n"
    put(root, "index.md", index)
    put(root, "People/index.md", nested)
    put(root, "log.md", log)
    put(root, "People/log.md", "## 2026-10-28\n* Updated the people.\n")
    put(root, "People/ada.md", "---\ntype: person\n---\nPerson")
    for path, raw in (("index.md", index), ("People/index.md", nested), ("log.md", log)):
        note = await read(client, "read", path=path)
        assert note["raw"] == raw
        assert note["conformance"] == {"conformant": True, "issues": []}
    catalog = await read(client, "catalog")
    assert [note["path"] for note in catalog["notes"]] == ["People/ada.md"]
    assert catalog["okf_version"] == "0.2"
    recent = catalog["recent"]
    assert recent[0]["text"] == "* **Update**: Checked the garden.\n  Kept the sources unchanged."
    assert recent[0]["date"] == "2026-10-28"
    assert recent[1]["action"] == "apply"
    assert recent[1]["text"] == ""
    assert recent[2]["text"] == "An older prose entry."
    assert recent[2]["date"] == "2026-10-27"


async def test_synthesized_index_uses_okf_links_and_default_link_style(vault):
    client, root = vault
    put(
        root,
        "People/ada.md",
        "---\ntype: person\ntitle: Ada Vale\ndescription: A gardener.\n---\n[[garden]]",
    )
    put(root, "garden.md", "---\ntype: project\n---\nWith [Ada](/People/ada.md).")
    assert (await client.get("/v1/vault/config")).json()["link_style"] == "markdown"
    index = await read(client, "index")
    assert index["synthesized"] is True
    assert "* [Ada Vale](/People/ada.md) - A gardener." in index["body"]
    assert index["outlinks"]
    assert (await read(client, "read", path="People/ada.md"))["outlinks"][0]["path"] == "garden.md"


@pytest.mark.parametrize(
    "path,raw",
    [
        ("People/index.md", "---\nokf_version: '0.2'\n---\n# People\n"),
        ("log.md", "---\n---\n## 2026-10-28\nA prose entry.\n"),
    ],
)
async def test_reserved_frontmatter_violations_are_soft(vault, path, raw):
    client, root = vault
    put(root, path, raw)
    note = await read(client, "read", path=path)
    assert note["raw"] == raw
    assert note["conformance"]["conformant"] is False
    assert note["conformance"]["issues"][0]["field"] == "frontmatter"


async def test_a_plain_log_entry_linking_a_note_appears_in_its_history(vault):
    client, root = vault
    put(root, "ada.md", "---\ntype: person\n---\nPerson")
    text = "* **Review**: Checked [Ada](/ada.md) against her sources."
    put(root, "log.md", f"## 2026-10-28\n{text}\n")
    note = await read(client, "read", path="ada.md")
    assert note["history"][0]["text"] == text
