"""The hub's plugin registry, with Notes as its first entry.

Notes (`vault`) declares itself in `orbit_hub/vault/orbit-plugin.json`, the
manifest `contracts/plugin-v1.md` defines. The hub reads that file when
`create_app` builds it, and:

- `GET /v1/plugins` answers, to a paired device, `{"plugins": [...]}`: each
  plugin's manifest as its file says, written as a block in
  `contracts/hub-v1.md`;
- a manifest outside `plugin-v1.md` stops the hub: `create_app` raises, and the
  error names the file and the cause, never in silence;
- the routes of Notes are mounted from the registry, so `app.py` names no plugin;
- deleting a meeting deletes the Notes data that names it, through the
  registry, so the meetings code names no plugin table.

The manifest is checked against `plugin-v1.md` as the contract is written: its
fields, the shape of an `id`, the reserved ids, the vocabulary of permissions
and events are read from the contract's text, not copied here.
"""

import builtins
import copy
import errno
import io
import json
import os
import re
from pathlib import Path
from uuid import uuid4

import httpx
import pytest
from conftest import device
from support_distillation import distillation_client
from test_contract_routes import CONTRACT as HUB_CONTRACT
from test_contract_routes import SHARED, route_blocks, sections

from orbit_hub.app import create_app

# Re-exported for pytest: the HTTP fixture with an invented vault and meeting.
__all__ = ["distillation_client"]

ROOT = Path(__file__).resolve().parents[2]
PLUGIN_CONTRACT = ROOT / "contracts" / "plugin-v1.md"
HUB = ROOT / "server" / "orbit_hub"
NOTES_MANIFEST = HUB / "vault" / "orbit-plugin.json"
ROUTE = "GET /v1/plugins"

# What Notes does today, measured in the hub, as the contract's vocabulary
# says it: its own tables (`data`); a proposal once a meeting's note is ready
# (`events:note.ready`); the meeting's public transcript and title
# (`read:meetings`); three tools for the assistant (`assistant_tools`). Its
# logic runs inside the hub, with no process to start.
NOTES = {
    "id": "vault",
    "name": {"es": "Notas", "en": "Notes"},
    "orbit_plugin_api": 1,
    "views": {"sidebar": True, "settings": True, "slots": []},
    "permissions": {"data", "events:note.ready", "read:meetings", "assistant_tools"},
    "logic": "none",
}


# --- plugin-v1.md, read as written ---------------------------------------------


def _section(text: str, title: str) -> str:
    match = re.search(rf"^## {re.escape(title)}\n(.*?)(?=^## |\Z)", text, re.MULTILINE | re.DOTALL)
    assert match, f"plugin-v1.md has no section «{title}»"
    return match.group(1)


def _rows(body: str) -> list[list[str]]:
    """The body rows of the first table in `body`, cell by cell."""
    rows, started = [], False
    for line in body.splitlines():
        if not line.startswith("|"):
            if started:
                break
            continue
        started = True
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if set("".join(cells)) <= set("-: "):
            continue
        rows.append(cells)
    return rows[1:]


def _ticked(text: str) -> list[str]:
    return re.findall(r"`([^`]+)`", text)


def _sentence(text: str, pattern: str) -> re.Match:
    match = re.search(pattern, text, re.DOTALL)
    assert match, f"plugin-v1.md no longer says {pattern!r}: read it again"
    return match


def contract_rules() -> dict:
    text = PLUGIN_CONTRACT.read_text()
    manifest = _section(text, "Manifest")
    rows = {_ticked(row[0])[0]: row[1] for row in _rows(manifest)}
    reserved = _ticked(_sentence(manifest, r"An `id` is never[^\n]*\n[^\n]*").group(0))[1:]
    permissions = [_ticked(row[0])[0] for row in _rows(manifest.split("vocabulary of")[1])]
    events = [_ticked(row[0])[0] for row in _rows(_section(text, "Events"))]
    _sentence(manifest, r"`network:<domain>` requires logic: a manifest with `logic` `\"none\"`")
    _sentence(text, r"`slots` is an empty\s+list until it\s+does")
    return {
        "fields": list(rows),
        "id": re.compile(_ticked(rows["id"])[0]),
        "exact": {word for word in reserved if not word.endswith("_")},
        "prefixes": tuple(word for word in reserved if word.endswith("_")),
        "api": int(_sentence(rows["orbit_plugin_api"], r"the integer `(\d+)`").group(1)),
        "name": set(_ticked(rows["name"])),
        "views": set(_ticked(rows["views"])),
        "permissions": permissions,
        "events": events,
        "logic": _ticked(rows["logic"]),
    }


def _permission_problem(permission, rules) -> str | None:
    if not isinstance(permission, str):
        return f"permission {permission!r} is not text"
    for word in rules["permissions"]:
        head, _, tail = word.partition(":")
        if "<" not in word:
            if permission == word:
                return None
        elif permission.startswith(head + ":"):
            value = permission[len(head) + 1 :]
            if tail == "<event>":
                return None if value in rules["events"] else f"no event {value!r}"
            return None if re.fullmatch(r"[a-z0-9.-]+", value) else f"bad {permission!r}"
    return f"permission {permission!r} is not in the vocabulary"


def contract_problems(manifest, rules) -> list[str]:
    """Where `manifest` leaves `plugin-v1.md`, as read by `contract_rules`."""
    if not isinstance(manifest, dict):
        return ["the manifest is not an object"]
    problems = []
    if set(manifest) != set(rules["fields"]):
        problems.append(f"fields {sorted(manifest)}, not {rules['fields']}")
    plugin_id = manifest.get("id")
    if not isinstance(plugin_id, str) or not rules["id"].fullmatch(plugin_id):
        problems.append(f"id {plugin_id!r} is not {rules['id'].pattern}")
    elif plugin_id in rules["exact"] or plugin_id.startswith(rules["prefixes"]):
        problems.append(f"id {plugin_id!r} is reserved")
    if not isinstance(manifest.get("version"), str):
        problems.append("version is not text")
    name = manifest.get("name")
    if not isinstance(name, dict) or set(name) != rules["name"]:
        problems.append(f"name is not {sorted(rules['name'])}")
    elif not all(isinstance(value, str) for value in name.values()):
        problems.append("name holds something that is not text")
    api = manifest.get("orbit_plugin_api")
    if type(api) is not int or api != rules["api"]:
        problems.append(f"orbit_plugin_api {api!r} is not the integer {rules['api']}")
    views = manifest.get("views")
    if not isinstance(views, dict) or not set(views) <= rules["views"]:
        problems.append(f"views is not an object of {sorted(rules['views'])}")
    else:
        for flag in rules["views"] - {"slots"}:
            if flag in views and type(views[flag]) is not bool:
                problems.append(f"views.{flag} is not a boolean")
        if views.get("slots", []) != []:
            problems.append("views.slots is not empty, and Orbit declares no slots")
    permissions = manifest.get("permissions")
    logic = manifest.get("logic")
    if not isinstance(permissions, list):
        problems.append("permissions is not a list")
    else:
        problems += filter(None, (_permission_problem(p, rules) for p in permissions))
        if logic == "none" and any(str(p).startswith("network:") for p in permissions):
            problems.append("network:<domain> with logic none")
    none, command = rules["logic"][0].strip('"'), rules["logic"][1]
    if logic != none and not (
        isinstance(logic, dict)
        and set(logic) == {command}
        and isinstance(logic[command], list)
        and all(isinstance(part, str) for part in logic[command])
    ):
        problems.append(f"logic {logic!r} is neither {none!r} nor a {command} list")
    return problems


def test_the_contract_examples_pass_this_reading_of_it():
    """The reader above, on the two manifests `plugin-v1.md` gives as examples."""
    rules = contract_rules()
    manifest = _section(PLUGIN_CONTRACT.read_text(), "Manifest")
    examples = [
        json.loads(line)
        for line in re.search(r"```json\n(.*?)```", manifest, re.DOTALL).group(1).splitlines()
    ]
    assert len(examples) == 2
    for example in examples:
        assert contract_problems(example, rules) == [], example
    assert rules["fields"] == [
        "id", "version", "name", "orbit_plugin_api", "views", "permissions", "logic"
    ]  # fmt: skip
    assert rules["exact"] == {"hub", "plugin"} and rules["prefixes"] == ("hub_", "plugin_")
    assert set(rules["events"]) == {"note.ready", "recording.deleted"}


# --- the registry ------------------------------------------------------------------


def notes_on_disk() -> dict:
    assert NOTES_MANIFEST.is_file(), f"Notes declares itself in {NOTES_MANIFEST}"
    return json.loads(NOTES_MANIFEST.read_text())


async def plugins_of(app, token=None, path="/v1/plugins") -> httpx.Response:
    headers = {"Authorization": f"Bearer {token}"} if token else {}
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app), base_url="https://localhost", headers=headers
    ) as client:
        return await client.get(path)


def build(environment):
    settings, store, adapter = environment
    return create_app(settings, store=store, assistant=adapter, speech=adapter)


async def test_the_hub_lists_notes_as_its_manifest_says(environment):
    _, store, _ = environment
    app, paired = build(environment), device(store)
    answer = await plugins_of(app, paired["token"])
    assert answer.status_code == 200, answer.text
    assert answer.json() == {"plugins": [notes_on_disk()]}
    [notes] = answer.json()["plugins"]
    assert contract_problems(notes, contract_rules()) == []
    measured = {**notes, "permissions": set(notes["permissions"])}
    assert {key: measured[key] for key in NOTES} == NOTES
    assert isinstance(notes["version"], str) and notes["version"]
    assert len(notes["permissions"]) == len(set(notes["permissions"]))


async def test_the_list_needs_a_paired_device(environment):
    """As the other `/v1/` routes: a device's credential, refused as theirs is."""
    _, store, _ = environment
    app = build(environment)
    anonymous = await plugins_of(app)
    assert (anonymous.status_code, anonymous.json().get("code")) == (401, "unauthorized")
    paired = device(store)
    store.devices.revoke(paired["device_id"])
    revoked = await plugins_of(app, paired["token"])
    agents = await plugins_of(app, paired["token"], "/v1/agents")
    assert revoked.status_code in {401, 403}, revoked.text
    assert (revoked.status_code, revoked.json().get("code")) == (
        agents.status_code,
        agents.json().get("code"),
    )


def test_the_list_is_written_as_a_block_in_the_hub_contract(environment):
    blocks = route_blocks(HUB_CONTRACT.read_text()).get(ROUTE, [])
    assert len(blocks) == 1, f"{ROUTE}: {len(blocks)} blocks in hub-v1.md, not one"
    [block] = blocks
    assert block.problems == []
    assert dict(block.fields) == {"reply": {"plugins"}}
    assert set(block.refusals) - SHARED == set(), "the list refuses nothing of its own"
    [(_, body)] = [
        (title, body)
        for title, body in sections(HUB_CONTRACT.read_text()).values()
        if title == f"`{ROUTE}`"
    ]
    assert "plugin-v1.md" in body, "the block says each entry is a manifest of plugin-v1.md"


# --- a manifest outside the contract stops the hub ------------------------------------


@pytest.fixture
def manifest_reads(monkeypatch, tmp_path):
    """Make every way of opening the Notes manifest open `text` instead.

    It starts as the file on disk; `None` makes it a file that is not there.
    Any other file opens as usual.
    """
    target = NOTES_MANIFEST.resolve()
    real_open, real_path_open = builtins.open, Path.open
    swapped = {"text": target.read_text() if target.is_file() else None}

    def swap(file):
        try:
            same = Path(os.fspath(file)).resolve() == target
        except TypeError:
            return file
        if not same:
            return file
        if swapped["text"] is None:
            raise FileNotFoundError(errno.ENOENT, "No such file or directory", str(file))
        replacement = tmp_path / "orbit-plugin.json"
        replacement.write_text(swapped["text"])
        return replacement

    def fake_open(file, *args, **kwargs):
        return real_open(swap(file), *args, **kwargs)

    def fake_path_open(self, *args, **kwargs):
        return real_path_open(Path(swap(self)), *args, **kwargs)

    monkeypatch.setattr(builtins, "open", fake_open)
    monkeypatch.setattr(io, "open", fake_open)
    monkeypatch.setattr(Path, "open", fake_path_open)

    def use(text):
        swapped["text"] = text

    return use


def broken_manifests(base: dict, rules: dict):
    """(what it breaks, the manifest, a word the hub's error has to name)."""
    for field in rules["fields"]:
        yield f"no {field}", {k: v for k, v in base.items() if k != field}, field
    yield "a field more", {**base, "author": "Ada Example"}, "author"
    for bad in ("Vault", "notes-and-tasks", "x" * 33, ""):
        yield f"id {bad!r}", {**base, "id": bad}, "id"
    for word in sorted(rules["exact"]) + [p + "notes" for p in rules["prefixes"]]:
        yield f"reserved id {word}", {**base, "id": word}, word
    yield "a number for version", {**base, "version": 1}, "version"
    yield "name without en", {**base, "name": {"es": "Notas"}}, "name"
    yield "name in three languages", {**base, "name": {**base["name"], "fr": "Notes"}}, "name"
    for api in (2, "1", True):
        yield f"orbit_plugin_api {api!r}", {**base, "orbit_plugin_api": api}, "orbit_plugin_api"
    views = base["views"]
    yield "a view Orbit lacks", {**base, "views": {**views, "toolbar": True}}, "toolbar"
    yield "sidebar as text", {**base, "views": {**views, "sidebar": "yes"}}, "sidebar"
    yield "a slot", {**base, "views": {**views, "slots": ["meeting_header"]}}, "slots"
    yield "permissions as text", {**base, "permissions": "data"}, "permissions"
    for permission in ("audio", "keychain", "events:meeting.started", "read:everything"):
        yield (
            f"permission {permission}",
            {**base, "permissions": [*base["permissions"], permission]},
            permission,
        )
    network = "network:example.test"
    yield (
        "network with logic none",
        {**base, "permissions": [*base["permissions"], network], "logic": "none"},
        network,
    )
    yield "logic hub", {**base, "logic": "hub"}, "logic"
    yield "a command as text", {**base, "logic": {"command": "./bin/notes"}}, "logic"
    yield (
        "a logic field more",
        {**base, "logic": {"command": ["./bin/notes"], "env": {}}},
        "logic",
    )


def test_a_manifest_outside_the_contract_stops_the_hub_with_its_cause(environment, manifest_reads):
    base, rules = notes_on_disk(), contract_rules()
    assert contract_problems(base, rules) == []
    silent = []
    cases = list(broken_manifests(base, rules))
    cases += [("not JSON", "{", None), ("not an object", "[]", None), ("no file", None, None)]
    for what, manifest, word in cases:
        if isinstance(manifest, dict):
            assert contract_problems(manifest, rules), f"{what}: the contract allows it"
        manifest_reads(json.dumps(manifest) if isinstance(manifest, dict) else manifest)
        try:
            build(environment)
        except Exception as error:  # noqa: BLE001 - any refusal to start, with its cause
            cause = str(error)
            if "orbit-plugin.json" not in cause or (word and word not in cause):
                silent.append(f"{what}: started no hub, but said {cause!r}")
        else:
            silent.append(f"{what}: the hub started")
    assert not silent, f"{len(silent)} of {len(cases)}:\n" + "\n".join(silent)


async def test_the_list_is_read_from_the_file(environment, manifest_reads):
    """A valid manifest other than today's reaches the list as written."""
    _, store, _ = environment
    other = copy.deepcopy(notes_on_disk())
    other.update(version="0.0.1-invented", name={"es": "Cuaderno", "en": "Notebook"})
    other["views"] = {"sidebar": False}
    manifest_reads(json.dumps(other))
    answer = await plugins_of(build(environment), device(store)["token"])
    assert answer.status_code == 200, answer.text
    assert answer.json() == {"plugins": [other]}


# --- what the shared hub no longer names --------------------------------------------


def _python(path: Path) -> list[Path]:
    return [path] if path.is_file() else sorted(path.rglob("*.py"))


def test_the_shared_hub_names_no_plugin():
    """`app.py` and the meetings code know no plugin: the registry does."""
    named = [
        f"{file.relative_to(HUB)}:{number}: {line.strip()}"
        for place in (HUB / "app.py", HUB / "meetings")
        for file in _python(place)
        for number, line in enumerate(file.read_text().splitlines(), 1)
        if re.search(r"vault|distill", line, re.IGNORECASE)
    ]
    assert not named, "the shared hub names Notes:\n" + "\n".join(named)


def test_notes_tables_are_named_only_by_notes():
    outside = [
        str(file.relative_to(HUB))
        for file in sorted(HUB.rglob("*.py"))
        if "vault_distillations" in file.read_text()
        and file.relative_to(HUB).parts[0] not in {"vault", "distillation"}
    ]
    assert not outside, f"files outside Notes that name its table: {outside}"


async def test_deleting_a_meeting_deletes_its_notes_data(distillation_client):
    client, _, meeting, _, _, app, owner = distillation_client
    queued = await client.post(f"/v1/recordings/{meeting}/distillations")
    assert queued.status_code == 200, queued.text
    proposal = queued.json()["id"]
    other = str(uuid4())
    store = app.state.store
    with store.transaction() as db:
        db.execute(
            "INSERT INTO vault_distillations VALUES(?,?,?,?,?,?)",
            (str(uuid4()), owner["device_id"], other, "invented", "queued", b"x"),
        )

    def rows(recording):
        with store.read_snapshot() as db:
            return db.execute(
                "SELECT count(*) FROM vault_distillations WHERE device_id=? AND recording=?",
                (owner["device_id"], recording),
            ).fetchone()[0]

    assert rows(meeting) == 1 and rows(other) == 1
    deleted = await client.delete(f"/v1/recordings/{meeting}")
    assert deleted.status_code == 200, deleted.text
    assert rows(meeting) == 0, "the meeting's proposal outlived the meeting"
    assert rows(other) == 1, "another meeting's proposal went with it"
    gone = await client.get(f"/v1/vault/distillations/{proposal}")
    assert (gone.status_code, gone.json()["code"]) == (404, "distillation_not_found")
