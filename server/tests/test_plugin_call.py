"""A plugin's call on the hub: `POST /v1/plugins/{id}/call`, with Notes first.

`contracts/plugin-v1.md` («Call») defines the route: a paired device sends
`{"name": ..., "input": {...}}` with its credential, and the hub answers
`200 {"value": ...}` or a refusal. Notes (`vault`) runs inside the hub, so its
calls are its routes of today under a name: the name is the `call` tag of the
device's vault call and `input` holds its other fields. What a call answers is
what that route answers: its body as `value`, or its refusal unchanged.

The hub refuses, before a call reaches the plugin:

- an `id` it has no plugin for, whatever its shape: `404 plugin_not_installed`;
- a body over `PLUGIN_CALL_MAX_BYTES`: `413 plugin_call_too_large`, with
  `bytes` and `max_bytes`;
- a body that is not exactly `name` (text) and `input` (object):
  `400 invalid_request`, as the hub's other routes refuse a malformed body;
- a `name` the plugin does not have: `404 plugin_call_unknown`, with no numbers.

The `/v1/vault/*` routes stay as they are; their own tests keep them.
"""

import json
import logging
import re
from pathlib import Path

import httpx
import pytest
from conftest import device
from support_distillation import distillation_client
from test_contract_routes import CONTRACT as HUB_CONTRACT
from test_contract_routes import route_blocks, sections
from test_plugin_registry import build

# Re-exported for pytest: the HTTP fixture with an invented vault and meeting.
__all__ = ["distillation_client"]

ROOT = Path(__file__).resolve().parents[2]
PLUGIN_CONTRACT = ROOT / "contracts" / "plugin-v1.md"
CONSTANTS = ROOT / "contracts" / "constants.json"
ROUTE = "POST /v1/plugins/{id}/call"
RETRY_KINDS = {"never", "after_fix", "check_outcome", "after_ms"}
MISSING_PROPOSAL = "00000000-0000-4000-8000-000000000999"


def call_path(plugin: str) -> str:
    return f"/v1/plugins/{plugin}/call"


async def call(client, name, input, plugin="vault"):
    return await client.post(call_path(plugin), json={"name": name, "input": input})


async def same(client, name, input, method, path, **kwargs):
    """The call answers as Notes' route answers: its body as `value`, or its refusal."""
    old = await client.request(method, path, **kwargs)
    new = await call(client, name, input)
    if old.status_code < 400:
        assert new.status_code == 200, (name, input, new.status_code, new.text[:400])
        assert new.json() == {"value": old.json()}, (name, input)
        assert new.headers.get("cache-control") == "no-store", (name, new.headers)
    else:
        # Each request has its own `op`; the rest of the refusal is the route's.
        assert (new.status_code, without_op(new)) == (old.status_code, without_op(old)), (
            name,
            input,
        )
    return new


def without_op(response) -> dict:
    body = response.json()
    assert isinstance(body.get("op"), str), body
    return {key: value for key, value in body.items() if key != "op"}


def contract_limit(name: str) -> int:
    rows = [
        line
        for line in PLUGIN_CONTRACT.read_text().splitlines()
        if line.startswith(f"| `{name}` |")
    ]
    assert len(rows) == 1, f"plugin-v1.md: {len(rows)} rows for {name} in «Limits»"
    return int(re.search(r"\|\s*(\d+) bytes\s*\|", rows[0]).group(1))


def refusal_of(response, status, code):
    body = response.json()
    assert (response.status_code, body.get("code")) == (status, code), response.text[:400]
    assert isinstance(body.get("message"), str) and body["message"], body
    refusal = body["refusal"]
    assert refusal["code"] == code, body
    assert refusal["retry"]["kind"] in RETRY_KINDS, body
    return refusal


# --- Notes answers through the call as through its routes ----------------------------------


async def test_each_notes_call_answers_as_its_route_does(distillation_client):
    client, root, meeting, _, _, app, _ = distillation_client
    await same(client, "config", {}, "GET", "/v1/vault/config")
    for operation, extra in (
        ("list", {}),
        ("search", {"query": "Cedar"}),
        ("read", {"path": "Projects/cedar-pilot.md"}),
        ("backlinks", {"path": "Projects/cedar-pilot.md"}),
        ("outlinks", {"path": "index.md"}),
        ("index", {}),
        ("catalog", {}),
        ("tasks", {}),
        ("read", {"path": "../outside.md"}),
    ):
        request = {"operation": operation, **extra}
        await same(client, "read", {"request": request}, "POST", "/v1/vault/read", json=request)

    listing = "/v1/vault/distillations"
    await same(
        client,
        "list_distillations",
        {"meeting_id": None, "page": 1},
        "GET",
        listing,
        params={"page": 1},
    )
    await same(
        client,
        "list_distillations",
        {"meeting_id": None, "page": 0},
        "GET",
        listing,
        params={"page": 0},
    )

    prepared = await call(client, "prepare_distillation", {"meeting_id": meeting})
    assert prepared.status_code == 200, prepared.text[:400]
    proposal = prepared.json()["value"]
    assert set(prepared.json()) == {"value"} and isinstance(proposal.get("id"), str), proposal
    await app.state.distillations.run_once()
    await same(client, "distillation", {"id": proposal["id"]}, "GET", f"{listing}/{proposal['id']}")
    for other in (meeting, MISSING_PROPOSAL):
        await same(
            client,
            "list_distillations",
            {"meeting_id": other, "page": 1},
            "GET",
            listing,
            params={"page": 1, "meeting_id": other},
        )
    await same(
        client,
        "review_distillation",
        {"id": proposal["id"], "review": {"invented": True}},
        "PATCH",
        f"{listing}/{proposal['id']}",
        json={"invented": True},
    )
    await same(
        client, "distillation", {"id": MISSING_PROPOSAL}, "GET", f"{listing}/{MISSING_PROPOSAL}"
    )
    for action in ("apply", "discard", "undo", "prepare"):
        await same(
            client,
            "distillation_action",
            {"id": MISSING_PROPOSAL, "action": action},
            "POST",
            f"{listing}/{MISSING_PROPOSAL}/{action}",
        )

    configuration = {"kind": "folder", "path": str(root)}
    await same(
        client,
        "configure",
        {"configuration": configuration},
        "PUT",
        "/v1/vault/config",
        json=configuration,
    )
    disconnected = await call(client, "disconnect", {})
    assert disconnected.status_code == 200, disconnected.text[:400]
    assert set(disconnected.json()) == {"value"}
    await same(client, "config", {}, "GET", "/v1/vault/config")
    request = {"operation": "catalog"}
    gone = await same(client, "read", {"request": request}, "POST", "/v1/vault/read", json=request)
    assert gone.status_code >= 400, "a disconnected vault reads nothing"


async def test_a_call_keeps_its_input_out_of_the_logs(distillation_client, caplog):
    client, *_ = distillation_client
    caplog.set_level(logging.DEBUG)
    caplog.set_level(logging.DEBUG, logger="orbit_hub")
    query = "zebra-quartz-invented"
    answer = await call(client, "read", {"request": {"operation": "search", "query": query}})
    assert answer.status_code == 200, answer.text[:400]
    assert query not in caplog.text


# --- what the hub refuses before a call reaches the plugin --------------------------------


async def post(app, plugin, token=None, **kwargs):
    headers = {"Authorization": f"Bearer {token}"} if token else {}
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app), base_url="https://localhost", headers=headers
    ) as client:
        return await client.post(call_path(plugin), **kwargs)


async def test_a_call_needs_a_paired_device(environment):
    """As the other `/v1/` routes: a device's credential, refused as theirs is."""
    _, store, _ = environment
    app = build(environment)
    body = {"name": "config", "input": {}}
    anonymous = await post(app, "vault", json=body)
    assert (anonymous.status_code, anonymous.json().get("code")) == (401, "unauthorized")
    paired = device(store)
    store.devices.revoke(paired["device_id"])
    revoked = await post(app, "vault", paired["token"], json=body)
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app),
        base_url="https://localhost",
        headers={"Authorization": f"Bearer {paired['token']}"},
    ) as client:
        agents = await client.get("/v1/agents")
    assert revoked.status_code in {401, 403}, revoked.text
    assert (revoked.status_code, revoked.json().get("code")) == (
        agents.status_code,
        agents.json().get("code"),
    )


@pytest.mark.parametrize(
    "plugin", ["reading_list", "x" * 32, "x" * 33, "Vault", "reading-list", "hub", "plugin_vault"]
)
async def test_a_plugin_the_hub_does_not_have_is_not_installed(environment, plugin):
    _, store, _ = environment
    app, paired = build(environment), device(store)
    answer = await post(app, plugin, paired["token"], json={"name": "config", "input": {}})
    refusal = refusal_of(answer, 404, "plugin_not_installed")
    assert set(refusal) == {"code", "retry"}, "the absent plugin is the whole cause"


async def test_a_call_over_its_limit_is_refused_with_its_size(environment):
    _, store, _ = environment
    app, paired = build(environment), device(store)
    maximum = contract_limit("PLUGIN_CALL_MAX_BYTES")
    assert maximum == 1048576

    def body(size):
        head, tail = b'{"name":"config","input":{"pad":"', b'"}}'
        return head + b"x" * (size - len(head) - len(tail)) + tail

    headers = {"Content-Type": "application/json"}
    over = await post(app, "vault", paired["token"], content=body(maximum + 1), headers=headers)
    refusal = refusal_of(over, 413, "plugin_call_too_large")
    assert refusal["max_bytes"] == maximum
    assert maximum < refusal["bytes"] <= maximum + 1
    assert set(refusal) == {"code", "bytes", "max_bytes", "retry"}

    at = await post(app, "vault", paired["token"], content=body(maximum), headers=headers)
    assert at.status_code != 413 and at.json().get("code") != "plugin_call_too_large", at.text


@pytest.mark.parametrize(
    "content",
    [
        "{}",
        '{"name":"config"}',
        '{"input":{}}',
        '{"name":"config","input":{},"more":1}',
        '{"name":1,"input":{}}',
        '{"name":"config","input":[]}',
        '{"name":"config","input":null}',
        "[]",
        "not json",
    ],
)
async def test_a_body_that_is_not_a_call_is_refused(environment, content):
    _, store, _ = environment
    app, paired = build(environment), device(store)
    answer = await post(
        app,
        "vault",
        paired["token"],
        content=content.encode(),
        headers={"Content-Type": "application/json"},
    )
    refusal_of(answer, 400, "invalid_request")


@pytest.mark.parametrize("name", ["drop_everything", "Config", "", "list"])
async def test_a_name_the_plugin_does_not_have_is_refused(environment, name):
    _, store, _ = environment
    app, paired = build(environment), device(store)
    answer = await post(app, "vault", paired["token"], json={"name": name, "input": {}})
    refusal = refusal_of(answer, 404, "plugin_call_unknown")
    assert set(refusal) == {"code", "retry"}
    assert "drop_everything" not in answer.text, "the refusal does not repeat what was sent"


# --- what the contracts say -----------------------------------------------------------------


def test_the_call_is_written_as_a_block_in_the_hub_contract():
    blocks = route_blocks(HUB_CONTRACT.read_text()).get(ROUTE, [])
    assert len(blocks) == 1, f"{ROUTE}: {len(blocks)} blocks in hub-v1.md, not one"
    [block] = blocks
    assert block.problems == []
    assert dict(block.fields) == {"path": {"id"}, "body": {"name", "input"}, "reply": {"value"}}
    expected = {
        (400, "invalid_request"): set(),
        (404, "plugin_not_installed"): set(),
        (404, "plugin_call_unknown"): set(),
        (413, "plugin_call_too_large"): {"bytes", "max_bytes"},
    }
    for refusal, numbers in expected.items():
        assert block.refusals.get(refusal) == numbers, (refusal, block.refusals.get(refusal))
    [(_, body)] = [
        (title, body)
        for title, body in sections(HUB_CONTRACT.read_text()).values()
        if title == f"`{ROUTE}`"
    ]
    assert "plugin-v1.md" in body, "the block points to the contract that defines a call"


def test_the_plugin_contract_names_the_new_refusal():
    rows = [
        [cell.strip() for cell in line.strip().strip("|").split("|")]
        for line in PLUGIN_CONTRACT.read_text().splitlines()
        if line.startswith("| `plugin_call_unknown` |")
    ]
    assert len(rows) == 1, f"plugin-v1.md: {len(rows)} rows for plugin_call_unknown"
    assert rows[0][1] == "`hub`", rows[0]


def test_the_call_limit_is_in_the_constants_with_the_hub_copy():
    entries = [e for e in json.loads(CONSTANTS.read_text()) if e["name"] == "PLUGIN_CALL_MAX_BYTES"]
    assert len(entries) == 1, f"constants.json: {len(entries)} entries for PLUGIN_CALL_MAX_BYTES"
    [entry] = entries
    assert (entry["value"], entry["unit"]) == (contract_limit("PLUGIN_CALL_MAX_BYTES"), "bytes")
    assert {
        "file": "server/orbit_hub/plugins.py",
        "name": "PLUGIN_CALL_MAX_BYTES",
        "unit": "bytes",
    } in entry["copies"]
