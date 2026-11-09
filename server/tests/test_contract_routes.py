"""The written contract covers every route the hub serves, and says what the code does.

`contracts/hub-v1.md` is what another hub must do to take this one's place, so
a route it does not name is a route nobody could rebuild without reading
Python. The first test takes the routes from the application `create_app`
builds: whatever mounted them (a decorator, `add_api_route`, the WebSocket,
FastAPI's own `/openapi.json`) they are in its table.

The routes the contract writes as blocks (a heading `` `METHOD /path` `` and
two tables: what goes in and comes out, and what is refused) are then driven
through that same application with invented data, and each block is compared
with what the code read, answered and refused:

- `path` and `query` rows are the route's own parameters, from its table entry;
- `body` rows are the fields of the model the route parses its body with;
- `reply` rows are the top-level fields of every answer the route gave here;
- the refusal rows are, by status and code, the refusals the route can give,
  and each refusal seen here carries only numbers its row names.

A row whose second cell is a link to a heading of the contract (`[a
proposal](#a-proposal)`) takes that section's rows; a shape section names its
fields as `field` rows. Refusals every route shares (credentials, transport,
the disk) are said once for all routes and are not compared here.
"""

import json
import re
from collections import defaultdict
from contextlib import AsyncExitStack
from dataclasses import dataclass, field, replace
from pathlib import Path
from uuid import uuid4

import httpx
from conftest import BRIDGE_NOW_MS, device
from support_distillation import FILES, distillation_client
from test_health_deps import Dependency, closed_port_url, hub, served

from orbit_hub.ai.registry import Registry
from orbit_hub.app import create_app
from orbit_hub.assistant.agent_config import AgentConfigStore
from orbit_hub.distillation.models import Review
from orbit_hub.notes.models import EditRequest
from orbit_hub.private_auth import sign_request
from orbit_hub.vault.models import Configuration, Read
from orbit_hub.vault.routes import BRIDGE_PATH

# Re-exported for pytest: the HTTP fixture with an invented vault and meeting.
__all__ = ["distillation_client"]

CONTRACT = Path(__file__).resolve().parents[2] / "contracts" / "hub-v1.md"
MENTION = re.compile(r"`(GET|PUT|POST|PATCH|DELETE) (/[^`\s?]+)")
HEADING = re.compile(r"^#{2,6} (.+?)\s*$", re.MULTILINE)
ROUTE_HEADING = re.compile(r"^`(GET|PUT|POST|PATCH|DELETE) (/\S+)`$")
LINK = re.compile(r"^\[[^\]]+\]\(#([^)]+)\)$")
NAME = re.compile(r"^`([a-z_][a-z0-9_]*)`$")
KINDS = {"path", "query", "body", "reply", "field"}
SIGNED = "orbit_meetings_bridge_v1"
KEY = bytes(range(32))

# Said once for every route: credentials, transport, origin, the disk, the unforeseen.
SHARED = {
    (401, "unauthorized"),
    (403, "revoked"),
    (403, "tls_required"),
    (403, "origin_forbidden"),
    (500, "internal_error"),
    (507, "storage_full"),
    (507, "quota_exceeded"),
}
BODY = {(400, "invalid_payload"), (413, "too_large")}  # an empty body, or over the route's ceiling
REPLY_SIZE = {(413, "vault_response_too_large")}  # a Notes answer over 4 MiB
# What either connector (a folder on the hub, Fast Note Sync) refuses when it
# opens the vault or lists it, whichever route asked.
CONNECTOR = {
    (400, "vault_path_refused"),
    (400, "vault_folder_missing"),
    (400, "vault_folder_permission"),
    (400, "vault_folder_changed"),
    (400, "vault_scan_limit"),
    (400, "vault_depth_limit"),
    (400, "vault_sync_timeout"),
    (400, "vault_sync_connection"),
    (400, "vault_sync_http"),
    (400, "vault_sync_shape"),
    (400, "vault_sync_missing"),
    (400, "vault_sync_refused"),
    (400, "vault_token_refused"),
    (400, "vault_token_expired"),
    (413, "vault_response_too_large"),
}
NOTE_TEXT = {(400, "vault_encoding"), (413, "vault_note_too_large")}  # one note's text
# Only a configuration not yet saved can be one no connector opens.
OPENING = {(400, "vault_folder_absolute"), (400, "vault_connection_fields")}
READ = (
    {
        (400, "vault_invalid_request"),
        (429, "vault_busy"),
        (409, "vault_not_connected"),
        (404, "vault_note_missing"),
        (400, "vault_not_markdown"),
        (400, "vault_scan_bytes"),
        (400, "vault_frontmatter_invalid"),
    }
    | CONNECTOR
    | NOTE_TEXT
    | REPLY_SIZE
)
PRIVATE = {
    (400, "invalid_request"),
    (503, "unavailable"),
    (503, "consumer_clock"),
    (401, "consumer_clock"),
    (401, "consumer_replay"),
    (429, "consumer_busy"),
    (403, "agent_identity_mismatch"),
    (403, "agent_access_disabled"),
    (404, "agent_not_found"),
}
PROPOSAL = {
    (409, "vault_not_connected"),
    (404, "distillation_not_found"),
    (409, "distillation_vault_changed"),
} | REPLY_SIZE
MEETING = {(404, "recording_not_found"), (409, "recording_id_ambiguous")}
NOTE_ID = {(400, "invalid_note_request"), (409, "recording_id_ambiguous")}

# The routes this contract writes as blocks, the model each parses its body
# with, and the refusals each can give besides the shared ones.
BLOCKS = {
    "GET /v1/vault/config": (None, set()),
    "PUT /v1/vault/config": (
        Configuration,
        BODY | {(400, "vault_invalid_request")} | OPENING | CONNECTOR,
    ),
    "DELETE /v1/vault/config": (None, set()),
    "POST /v1/vault/read": (Read, BODY | READ),
    "POST /internal/v1/vault/read": (Read, PRIVATE | READ),
    "GET /v1/vault/distillations": (
        None,
        {(400, "vault_invalid_request"), (422, "invalid_request"), (409, "vault_not_connected")}
        | REPLY_SIZE,
    ),
    "POST /v1/recordings/{recording_id}/distillations": (None, MEETING | REPLY_SIZE),
    "GET /v1/vault/distillations/{proposal_id}": (None, PROPOSAL),
    "PATCH /v1/vault/distillations/{proposal_id}": (
        Review,
        BODY
        | PROPOSAL
        | {
            (400, "vault_invalid_request"),
            (409, "distillation_not_proposed"),
            (409, "stale_revision"),
            (400, "distillation_edit_invalid"),
            (413, "vault_note_too_large"),
            (413, "distillation_too_large"),
            (404, "recording_not_found"),
        },
    ),
    "POST /v1/vault/distillations/{proposal_id}/prepare": (None, PROPOSAL | MEETING),
    "POST /v1/vault/distillations/{proposal_id}/{action}": (
        None,
        PROPOSAL
        | CONNECTOR
        | NOTE_TEXT
        | {
            (409, "vault_sync_version_missing"),
            (404, "distillation_action_invalid"),
            (409, "distillation_not_proposed"),
            (409, "distillation_undecided"),
            (409, "distillation_not_applied"),
            (409, "distillation_validation_failed"),
            (409, "distillation_content_changed"),
            (409, "distillation_write_failed"),
            (413, "distillation_too_large"),
            (404, "recording_not_found"),
        },
    ),
    "GET /health/deps": (None, set()),
    "GET /v1/recordings/{recording_id}/note": (None, NOTE_ID | {(404, "note_not_requested")}),
    "PUT /v1/recordings/{recording_id}/note/edits": (
        EditRequest,
        NOTE_ID | BODY | {(409, "note_not_ready"), (409, "note_revision_changed")},
    ),
}


def template(path: str) -> str:
    return re.sub(r"\{[^}]*\}", "{}", path)


def routes_of(app) -> list[tuple[frozenset, str, object]]:
    """Every route in the application's table: its methods, path and entry.

    A WebSocket is opened with a GET. HEAD and OPTIONS are what Starlette or a
    signed route adds to answer them, not routes of their own.
    """
    found = []
    for route in app.routes:
        methods = getattr(route, "methods", None) or {"GET"}
        found.append((frozenset(methods - {"HEAD", "OPTIONS"}), route.path, route))
    return found


def mentioned(text: str) -> set[tuple[str, str]]:
    return {(method, template(path)) for method, path in MENTION.findall(text)}


def slug(title: str) -> str:
    return re.sub(r"[^\w\- ]", "", title.strip().lower()).replace(" ", "-")


def sections(text: str) -> dict[str, tuple[str, str]]:
    marks = list(HEADING.finditer(text))
    return {
        slug(mark.group(1)): (
            mark.group(1),
            text[mark.end() : marks[i + 1].start() if i + 1 < len(marks) else len(text)],
        )
        for i, mark in enumerate(marks)
    }


@dataclass
class Block:
    fields: dict = field(default_factory=lambda: defaultdict(set))
    refusals: dict = field(default_factory=dict)
    problems: list = field(default_factory=list)


def read_block(known: dict, anchor: str, trail=()) -> Block:
    """The rows of one section, with the sections its links name."""
    block = Block()
    for line in known[anchor][1].splitlines():
        if not line.startswith("|"):
            continue
        cells = [cell.strip() for cell in re.split(r"(?<!\\)\|", line.strip().strip("|"))]
        if len(cells) < 2:
            continue
        link, name = LINK.match(cells[1]), NAME.match(cells[1])
        if link:
            target = link.group(1)
            if target not in known or target in trail:
                block.problems.append(f"{known[anchor][0]} links to #{target}, no section to read")
                continue
            inner = read_block(known, target, (*trail, anchor))
            for kind, names in inner.fields.items():
                block.fields[cells[0] if kind == "field" else kind] |= names
            for refusal, numbers in inner.refusals.items():
                block.refusals.setdefault(refusal, set()).update(numbers)
            block.problems += inner.problems
        elif name and cells[0] in KINDS:
            block.fields[cells[0]].add(name.group(1))
        elif name and re.fullmatch(r"[1-5]\d\d", cells[0]):
            numbers = set(re.findall(r"`([a-z_][a-z0-9_]*)`", cells[2] if len(cells) > 2 else ""))
            block.refusals.setdefault((int(cells[0]), name.group(1)), set()).update(numbers)
    return block


def route_blocks(text: str) -> dict[str, list[Block]]:
    known = sections(text)
    blocks = defaultdict(list)
    for anchor, (title, _) in known.items():
        if heading := ROUTE_HEADING.match(title):
            blocks[" ".join(heading.groups())].append(read_block(known, anchor))
    return blocks


@dataclass
class Seen:
    """What each route answered and refused while a test drove it."""

    replies: dict = field(default_factory=lambda: defaultdict(set))
    refusals: dict = field(default_factory=lambda: defaultdict(lambda: defaultdict(set)))

    async def call(self, client, key, *, expect=None, content=None, headers=None, **values):
        method, path = key.split(" ", 1)
        target = path.format(**{k: v for k, v in values.items() if k not in {"json", "params"}})
        response = await client.request(
            method,
            target,
            json=values.get("json"),
            params=values.get("params"),
            content=content,
            headers=headers,
        )
        body = response.json()
        if isinstance(body, dict) and "code" in body:
            numbers = set(body.get("refusal", {})) - {"code", "retry"}
            self.refusals[key][(response.status_code, body["code"])] |= numbers
        else:
            self.replies[key] |= set(body)
        if expect is not None:
            got = (response.status_code, body.get("code"))
            assert got == expect, (key, got, response.text[:400])
        return body


def compare(app, seen: Seen, keys) -> list[str]:
    """Where each written block differs from what the code read, gave and refused."""
    table = {f"{m} {path}": entry for methods, path, entry in routes_of(app) for m in methods}
    blocks, problems = route_blocks(CONTRACT.read_text()), []
    for key in keys:
        model, refusals = BLOCKS[key]
        if len(blocks.get(key, [])) != 1:
            problems.append(f"{key}: {len(blocks.get(key, []))} blocks in the contract, not one")
            continue
        block, entry = blocks[key][0], table[key]
        problems += block.problems
        truth = {
            "path": set(entry.param_convertors),
            "query": {p.name for p in entry.dependant.query_params},
            "body": set(model.model_fields) if model else set(),
            "reply": seen.replies[key],
        }
        if not seen.replies[key]:
            problems.append(f"{key}: this test never got an answer from it")
        for kind, fields in truth.items():
            written = block.fields.get(kind, set())
            if written != fields:
                problems.append(
                    f"{key} {kind}: the contract names {sorted(written - fields)} that the code"
                    f" does not have, and leaves out {sorted(fields - written)}"
                )
        written = set(block.refusals) - SHARED
        if written != refusals:
            problems.append(
                f"{key} refusals: the contract names {sorted(written - refusals)} that the route"
                f" does not give, and leaves out {sorted(refusals - written)}"
            )
        for refusal, numbers in seen.refusals[key].items():
            if refusal not in refusals | SHARED:
                problems.append(f"{key}: refused {refusal}, which the list above does not know")
            elif refusal in block.refusals and not numbers <= block.refusals[refusal]:
                problems.append(
                    f"{key} {refusal}: carries {sorted(numbers - block.refusals[refusal])},"
                    " which its row does not name"
                )
    return problems


def test_every_route_the_hub_serves_is_named_in_the_contract(environment):
    settings, store, adapter = environment
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    named = mentioned(CONTRACT.read_text())
    missing = sorted(
        f"{'/'.join(sorted(methods))} {path}"
        for methods, path, _ in routes_of(app)
        if not any((method, template(path)) in named for method in methods)
    )
    assert not missing, (
        "contracts/hub-v1.md never names these routes as `METHOD /path` (parameter names"
        " aside; a route that takes several methods needs one of them):\n" + "\n".join(missing)
    )


def test_each_written_block_is_a_route_the_hub_serves(environment):
    settings, store, adapter = environment
    app = create_app(settings, store=store, assistant=adapter, speech=adapter)
    table = {f"{m} {path}" for methods, path, _ in routes_of(app) for m in methods}
    blocks = route_blocks(CONTRACT.read_text())
    assert not set(blocks) - table, (
        f"blocks for routes the hub does not serve: {set(blocks) - table}"
    )
    missing = sorted(key for key in BLOCKS if len(blocks.get(key, [])) != 1)
    assert not missing, "routes without exactly one block `METHOD /path`:\n" + "\n".join(missing)


async def read_every_operation(seen, send):
    """Each read operation once, on the invented vault of support_distillation."""
    for operation, extra in (
        ("list", {}),
        ("search", {"query": "Cedar"}),
        ("read", {"path": "Projects/cedar-pilot.md"}),
        ("backlinks", {"path": "Projects/cedar-pilot.md"}),
        ("outlinks", {"path": "index.md"}),
        ("index", {}),
        ("catalog", {}),
        ("tasks", {}),
    ):
        await send(seen, {"operation": operation, **extra}, None)


async def test_vault_blocks_say_what_the_code_does(distillation_client, tmp_path):
    client, _, _, _, _, app, _ = distillation_client
    seen, config, read = Seen(), "PUT /v1/vault/config", "POST /v1/vault/read"
    await seen.call(client, "GET /v1/vault/config", expect=(200, None))

    async def send(seen, body, expect):
        await seen.call(client, read, json=body, expect=expect)

    await read_every_operation(seen, send)
    for body, expect in (
        ({"operation": "read", "path": "../outside.md"}, (400, "vault_path_refused")),
        ({"operation": "read", "path": "Projects/absent.md"}, (404, "vault_note_missing")),
        ({"operation": "read", "path": "Projects/plan.txt"}, (400, "vault_not_markdown")),
        ({"operation": "rename"}, (400, "vault_invalid_request")),
    ):
        await send(seen, body, expect)
    await seen.call(client, read, content=b"", expect=(400, "invalid_payload"))
    await seen.call(client, read, content=b"{" + b" " * 16384, expect=(413, "too_large"))
    for body, expect in (
        ({"kind": "folder", "path": "relative/vault"}, (400, "vault_folder_absolute")),
        ({"kind": "folder", "path": str(tmp_path / "absent")}, (400, "vault_folder_missing")),
        ({"kind": "folder", "path": "/a/\x01b"}, (400, "vault_path_refused")),
        ({"kind": "fast_note_sync", "url": "ftp://x"}, (400, "vault_connection_fields")),
        ({"kind": "dropbox"}, (400, "vault_invalid_request")),
    ):
        await seen.call(client, config, json=body, expect=expect)
    await seen.call(client, config, content=b"", expect=(400, "invalid_payload"))
    await seen.call(client, config, content=b"{" + b" " * 16384, expect=(413, "too_large"))
    other = tmp_path / "second-vault"
    other.mkdir()
    (other / "Fern.md").write_text("---\ntype: note\ntitle: Fern\n---\nA fern.\n")
    await seen.call(client, config, json={"kind": "folder", "path": str(other)}, expect=(200, None))
    await send(seen, {"operation": "index"}, (200, None))  # no index.md: one is made up
    await seen.call(client, "DELETE /v1/vault/config", expect=(200, None))
    await seen.call(client, "GET /v1/vault/config", expect=(200, None))
    await send(seen, {"operation": "list"}, (409, "vault_not_connected"))
    keys = ["GET /v1/vault/config", config, "DELETE /v1/vault/config", read]
    assert not compare(app, seen, keys), "\n".join(compare(app, seen, keys))


async def test_signed_vault_block_says_what_the_code_does(environment, tmp_path):
    settings, store, adapter = environment
    owner = device(store)
    credential = tmp_path / "synthetic-meetings-key"
    credential.write_bytes(KEY)
    credential.chmod(0o600)
    settings = replace(
        settings, meetings_bridge={"key_id": "vault-test", "credential_file": str(credential)}
    )
    app = create_app(
        settings, store=store, assistant=adapter, speech=adapter, now=lambda: BRIDGE_NOW_MS / 1000
    )
    root = tmp_path / "invented-vault"
    for path, text in FILES.items():
        (root / path).parent.mkdir(parents=True, exist_ok=True)
        (root / path).write_text(text)
    seen, key = Seen(), "POST /internal/v1/vault/read"

    def signed(body: bytes, key_id="vault-test"):
        return {
            "content-type": "application/json",
            **sign_request(
                SIGNED,
                BRIDGE_PATH,
                key_id,
                KEY,
                body,
                issued_at_ms=BRIDGE_NOW_MS,
                nonce=str(uuid4()),
            ),
        }

    async def send(seen, payload, expect):
        body = json.dumps(payload).encode()
        await seen.call(client, key, content=body, headers=signed(body), expect=expect)

    async with (
        app.router.lifespan_context(app),
        httpx.AsyncClient(
            transport=httpx.ASGITransport(app), base_url="https://localhost"
        ) as client,
    ):
        mac = {"Authorization": f"Bearer {owner['token']}"}
        assert (
            await client.put(
                "/v1/vault/config", json={"kind": "folder", "path": str(root)}, headers=mac
            )
        ).status_code == 200
        await read_every_operation(seen, send)
        await send(seen, {"operation": "rename"}, (400, "vault_invalid_request"))
        await send(seen, {"operation": "list", "agent_id": "orion"}, (404, "agent_not_found"))
        body = b'{"operation":"list"}'
        await seen.call(client, key, content=body, headers=signed(body, "another-key"))
        wrong = {**signed(body), "content-type": "text/plain"}
        await seen.call(client, key, content=body, headers=wrong, expect=(400, "invalid_request"))
        large = b'{"operation":"list","query":"' + b"a" * 16384 + b'"}'
        await seen.call(
            client, key, content=large, headers=signed(large), expect=(400, "invalid_request")
        )
        configs = AgentConfigStore(store)
        configs.set_access("default", {**configs.access("default"), "notes": False})
        await send(seen, {"operation": "list"}, (403, "agent_access_disabled"))
        configs.set_access("default", {**configs.access("default"), "notes": True})
        assert (await client.delete("/v1/vault/config", headers=mac)).status_code == 200
        await send(seen, {"operation": "list"}, (409, "vault_not_connected"))
    unconfigured = create_app(
        replace(settings, meetings_bridge=None), store=store, assistant=adapter, speech=adapter
    )
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(unconfigured), base_url="https://localhost"
    ) as client:
        await send(seen, {"operation": "list"}, (503, "unavailable"))
    assert not compare(app, seen, [key]), "\n".join(compare(app, seen, [key]))


async def test_distillation_blocks_say_what_the_code_does(distillation_client, tmp_path):
    client, _, meeting, _, _, app, _ = distillation_client
    seen = Seen()
    listing, create = (
        "GET /v1/vault/distillations",
        "POST /v1/recordings/{recording_id}/distillations",
    )
    one, edit = (
        "GET /v1/vault/distillations/{proposal_id}",
        "PATCH /v1/vault/distillations/{proposal_id}",
    )
    again, act = (
        "POST /v1/vault/distillations/{proposal_id}/prepare",
        "POST /v1/vault/distillations/{proposal_id}/{action}",
    )
    queued = await seen.call(client, create, recording_id=meeting, expect=(200, None))
    await app.state.distillations.run_once()
    proposal = await seen.call(client, one, proposal_id=queued["id"], expect=(200, None))
    assert proposal["status"] == "proposed", proposal["error"]
    ids = {"proposal_id": proposal["id"]}
    await seen.call(client, listing, params={"meeting_id": meeting}, expect=(200, None))
    await seen.call(client, listing, params={"page": 0}, expect=(400, "vault_invalid_request"))
    await seen.call(client, listing, params={"page": "first"}, expect=(422, "invalid_request"))
    await seen.call(client, one, proposal_id=str(uuid4()), expect=(404, "distillation_not_found"))
    await seen.call(client, act, **ids, action="apply", expect=(409, "distillation_undecided"))
    await seen.call(
        client, act, **ids, action="publish", expect=(404, "distillation_action_invalid")
    )
    for body, expect in (
        ({"revision": 99}, (409, "stale_revision")),
        ({"revision": "zero"}, (400, "vault_invalid_request")),
        ({"revision": 0, "changes": {"absent": {}}}, (400, "distillation_edit_invalid")),
    ):
        await seen.call(client, edit, **ids, json=body, expect=expect)
    await seen.call(client, edit, **ids, content=b"", expect=(400, "invalid_payload"))
    decisions = {c["id"]: "incoming" for c in proposal["conflicts"]}
    body = {"revision": proposal["revision"], "decisions": decisions}
    await seen.call(client, edit, **ids, json=body, expect=(200, None))
    await seen.call(client, act, **ids, action="apply", expect=(200, None))
    await seen.call(client, edit, **ids, json=body, expect=(409, "distillation_not_proposed"))
    await seen.call(client, act, **ids, action="undo", expect=(200, None))
    fresh = await seen.call(client, again, **ids, expect=(200, None))
    await seen.call(client, act, proposal_id=fresh["id"], action="discard", expect=(200, None))
    await seen.call(
        client,
        act,
        proposal_id=fresh["id"],
        action="undo",
        expect=(409, "distillation_not_applied"),
    )
    await seen.call(client, create, recording_id=str(uuid4()), expect=(404, "recording_not_found"))
    registry = Registry(app.state.store)
    registry.write({**registry.read(), "functions": {}})
    await seen.call(client, create, recording_id=meeting, expect=(200, None))  # no model: not made
    await seen.call(client, again, **ids, expect=(200, None))
    other = tmp_path / "second-vault"
    other.mkdir()
    (other / "Fern.md").write_text("---\ntype: note\ntitle: Fern\n---\nA fern.\n")
    moved = await client.put("/v1/vault/config", json={"kind": "folder", "path": str(other)})
    assert moved.status_code == 200, moved.text
    await seen.call(client, one, **ids, expect=(409, "distillation_vault_changed"))
    assert (await client.delete("/v1/vault/config")).status_code == 200
    await seen.call(client, listing, expect=(409, "vault_not_connected"))
    await seen.call(client, again, **ids, expect=(409, "vault_not_connected"))
    keys = [listing, create, one, edit, again, act]
    assert not compare(app, seen, keys), "\n".join(compare(app, seen, keys))


async def test_note_blocks_say_what_the_code_does(distillation_client):
    client, _, meeting, _, _, app, _ = distillation_client
    seen, read, edit = (
        Seen(),
        "GET /v1/recordings/{recording_id}/note",
        "PUT /v1/recordings/{recording_id}/note/edits",
    )
    note = await seen.call(client, read, recording_id=meeting, expect=(200, None))
    body = {"base_revision": note["revision"], "title": "Cedar review, edited"}
    await seen.call(client, edit, recording_id=meeting, json=body, expect=(200, None))
    await seen.call(
        client, edit, recording_id=meeting, json=body, expect=(409, "note_revision_changed")
    )
    await seen.call(
        client, edit, recording_id=meeting, json={}, expect=(400, "invalid_note_request")
    )
    await seen.call(
        client, edit, recording_id=meeting, content=b"", expect=(400, "invalid_payload")
    )
    large = b"{" + b" " * 512 * 1024
    await seen.call(client, edit, recording_id=meeting, content=large, expect=(413, "too_large"))
    await seen.call(client, read, recording_id="meeting", expect=(400, "invalid_note_request"))
    await seen.call(client, read, recording_id=str(uuid4()), expect=(404, "note_not_requested"))
    await seen.call(
        client, edit, recording_id=str(uuid4()), json=body, expect=(409, "note_not_ready")
    )
    assert not compare(app, seen, [read, edit]), "\n".join(compare(app, seen, [read, edit]))


async def test_dependency_health_block_says_what_the_code_does(environment):
    seen, key = Seen(), "GET /health/deps"
    async with AsyncExitStack() as stack:
        whisper = await served(stack, Dependency(200))
        hermes = await served(stack, Dependency(404))
        async with hub(environment, whisper=whisper, hermes=hermes) as client:
            await seen.call(client, key, expect=(200, None))
            app = client._transport.app
        async with hub(environment, whisper=closed_port_url(), hermes=hermes) as client:
            await seen.call(client, key)
    assert not compare(app, seen, [key]), "\n".join(compare(app, seen, [key]))


def test_route_blocks_are_read_as_written():
    """The reader of blocks, on a contract made up for it."""
    text = """
#### `GET /v1/example/{item_id}`

| in | field | |
|---|---|---|
| path | `item_id` | string |
| reply | [an item](#an-item) | |

| status | code | numbers |
|---|---|---|
| 404 | `item_missing` | `item_id` |
| — | [Shared refusals](#shared-refusals) | |

#### An item

| in | field | |
|---|---|---|
| field | `id` | string |
| field | `size` | integer \\| null |

#### Shared refusals

| status | code | numbers |
|---|---|---|
| 409 | `item_busy` | `in_flight`, `max` |
"""
    [block] = route_blocks(text)["GET /v1/example/{item_id}"]
    assert dict(block.fields) == {"path": {"item_id"}, "reply": {"id", "size"}}
    assert block.refusals == {
        (404, "item_missing"): {"item_id"},
        (409, "item_busy"): {"in_flight", "max"},
    }
    assert not block.problems
    assert mentioned("`PUT /v1/x/{a}?b=` and `GET /v1/y`") == {
        ("PUT", "/v1/x/{}"),
        ("GET", "/v1/y"),
    }
