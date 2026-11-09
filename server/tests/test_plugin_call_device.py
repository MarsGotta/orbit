"""A plugin call through the real daemon to the real hub, as the app sends it.

The device's local channel carries one order, ``{"command": "plugin",
"plugin": <id>, "call": {"name": ..., "input": {...}}}``, and answers it with
one line, ``{"v": 1, "id": ..., "outcome": {...}}``, whose ``outcome`` is one of:

- ``{"outcome": "answered", "value": ...}``: the hub's ``200 {"value": ...}``;
- ``{"outcome": "refused", "status": ..., "body": {...}}``: the hub's refusal,
  its status and its body as the hub wrote them;
- ``{"outcome": "failed", "refusal": {...}}``: the device's own refusal, the
  ``refusal`` object alone (``contracts/plugin-v1.md``, «Refusals»).

The device looks only at being paired, the ``id``'s shape and the two sizes;
it keeps nothing, interprets nothing and repeats nothing. Notes (``vault``) is
the hub's one plugin; the answers the hub cannot give on its own (too large,
out of shape, late) come from fixture plugins in front of the real hub.
"""

import asyncio
import json
import os
import tempfile
import time
from pathlib import Path

import httpx
import pytest
from conftest import device as second_device
from support_client_e2e import BINARY, RustDevice, tcp_server, until

from orbit_hub.app import create_app
from orbit_hub.config import Settings
from orbit_hub.storage import Store, initialize

pytestmark = pytest.mark.skipif(not BINARY, reason="Requires ORBIT_TEST_BINARY")

ROOT = Path(__file__).resolve().parents[2]
PLUGIN_CONTRACT = ROOT / "contracts" / "plugin-v1.md"
CONSTANTS = ROOT / "contracts" / "constants.json"
# The device's wait for the hub: longer than PLUGIN_CALL_BUDGET_MS (7 s), so a
# plugin's own late answer is the one seen, and shorter than the app's 10 s.
DEVICE_WAIT_MS = 8000
APP_WAIT_SECONDS = 10


def contract_limit(name: str) -> int:
    for line in PLUGIN_CONTRACT.read_text().splitlines():
        if line.startswith(f"| `{name}` |"):
            return int(line.split("|")[2].split()[0])
    raise AssertionError(f"{name} is not in the limits of plugin-v1.md")


def plugin_order(plugin, name, input):
    return {"command": "plugin", "plugin": plugin, "call": {"name": name, "input": input}}


async def plugin_call(rust, plugin, name, input):
    """One plugin order over the daemon's socket: its outcome."""
    response = await rust.request(plugin_order(plugin, name, input), ok=False)
    assert set(response) == {"v", "id", "outcome"}, response
    return response["outcome"]


def failed(outcome):
    assert outcome["outcome"] == "failed", outcome
    assert set(outcome) == {"outcome", "refusal"}, outcome
    return outcome["refusal"]


def without_op(body):
    assert isinstance(body.get("op"), str), body
    return {key: value for key, value in body.items() if key != "op"}


class Fixtures:
    """The real hub, with fixture plugins answering in front of it.

    Each fixture plugin answers its call route with what the hub cannot give
    on its own; every other request reaches the hub unchanged. ``seen`` keeps
    the path of every plugin call that arrived, so a test can say one never did.
    """

    def __init__(self, hub, answers):
        self.hub, self.answers, self.seen = hub, answers, []

    async def __call__(self, scope, receive, send):
        if scope["type"] == "http" and scope["path"].startswith("/v1/plugins/"):
            self.seen.append(scope["path"])
            answer = self.answers.get(scope["path"])
            if answer is not None:
                await answer(send)
                return
        await self.hub(scope, receive, send)


def raw(status, body: bytes, delay=0.0):
    async def answer(send):
        if delay:
            await asyncio.sleep(delay)
        await send(
            {
                "type": "http.response.start",
                "status": status,
                "headers": [
                    (b"content-type", b"application/json"),
                    (b"content-length", str(len(body)).encode()),
                ],
            }
        )
        await send({"type": "http.response.body", "body": body})

    return answer


def value_of_size(size: int) -> bytes:
    """A ``200`` body of exactly ``size`` bytes: ``{"value":"xx…x"}``."""
    envelope = len(b'{"value":""}')
    return b'{"value":"' + b"x" * (size - envelope) + b'"}'


def settings_for(root: Path, unit_stt) -> Settings:
    initialize(root / "hub", root / "key")
    credential = root / "credential"
    descriptor = os.open(credential, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        output.write("e2e-synthetic-hermes-token")
    return Settings(
        profile="deimos",
        data_dir=root / "hub",
        key_file=root / "key",
        # Nothing here reaches Hermes; the hub only needs an address for it.
        hermes_url="http://127.0.0.1:9",
        hermes_credential_file=credential,
        meeting_transcription=unit_stt.configuration(),
        allow_insecure_localhost=True,
        allowed_hosts=("127.0.0.1", "localhost"),
    )


async def pair(rust, store, hub):
    await rust.request(
        {
            "command": "pair",
            "hub_url": hub,
            "code": store.devices.create_code(time.time()),
            "device_name": "Synthetic plugin device",
        }
    )
    await until(rust.state, lambda state: state["connection"] == "online")


async def test_a_plugin_call_crosses_the_daemon_to_the_hub_and_back(unit_stt):
    with tempfile.TemporaryDirectory(prefix="plugin-e2e-") as directory:
        root = Path(directory)
        settings = settings_for(root, unit_stt)
        store = Store(root / "hub", root / "key")
        fixtures = Fixtures(create_app(settings, store=store), {})
        rust = RustDevice(Path(BINARY).resolve(), root / "device")
        try:
            async with tcp_server(fixtures) as hub:
                await rust.start()
                # Before pairing there is no hub to ask.
                refusal = failed(await plugin_call(rust, "vault", "config", {}))
                assert refusal == {"code": "not_paired", "retry": {"kind": "after_fix"}}
                assert fixtures.seen == []

                await pair(rust, store, hub)
                token = second_device(store, "Second synthetic device")["token"]
                async with httpx.AsyncClient(
                    base_url=hub, headers={"Authorization": f"Bearer {token}"}
                ) as client:
                    route = await client.get("/v1/vault/config")
                    listing = await client.get("/v1/vault/distillations", params={"page": 0})

                # Answered: the value is the body of Notes' route, as the hub wrote it.
                assert route.status_code == 200, route.text
                outcome = await plugin_call(rust, "vault", "config", {})
                assert outcome == {"outcome": "answered", "value": route.json()}

                # A refusal of Notes passes through with its status and body.
                assert listing.status_code >= 400, listing.text
                outcome = await plugin_call(
                    rust, "vault", "list_distillations", {"meeting_id": None, "page": 0}
                )
                assert outcome["outcome"] == "refused", outcome
                assert set(outcome) == {"outcome", "status", "body"}, outcome
                assert outcome["status"] == listing.status_code
                assert without_op(outcome["body"]) == without_op(listing.json())

                # The hub's own refusals, as the hub wrote them.
                for plugin, name, code in (
                    ("reading_list", "config", "plugin_not_installed"),
                    ("vault", "Config", "plugin_call_unknown"),
                ):
                    outcome = await plugin_call(rust, plugin, name, {})
                    assert outcome["outcome"] == "refused", outcome
                    assert outcome["status"] == 404, outcome
                    body = outcome["body"]
                    assert body["code"] == code and body["refusal"]["code"] == code, body
                    assert isinstance(body["message"], str) and isinstance(body["op"], str), body
                    assert set(body["refusal"]) == {"code", "retry"}, body

                # A malformed id is the device's to refuse: the hub never sees it.
                seen = len(fixtures.seen)
                for plugin in ("Vault", "a" * 33, "reading-list", ""):
                    refusal = failed(await plugin_call(rust, plugin, "config", {}))
                    assert refusal == {
                        "code": "invalid_parameter",
                        "parameter": "identifier",
                        "expected": {"kind": "wire_type", "name": "plugin_id"},
                        "retry": {"kind": "after_fix"},
                    }, plugin
                assert len(fixtures.seen) == seen, fixtures.seen[seen:]

                # One call is one request: nothing is repeated, nothing kept.
                seen = len(fixtures.seen)
                await plugin_call(rust, "vault", "config", {})
                assert fixtures.seen[seen:] == ["/v1/plugins/vault/call"]

                # The privacy stop holds every call on the device.
                await rust.request({"command": "privacy_stop"})
                await until(rust.state, lambda state: state["privacy_stopped"])
                seen = len(fixtures.seen)
                refusal = failed(await plugin_call(rust, "vault", "config", {}))
                assert refusal["code"] == "privacy_stopped", refusal
                assert refusal["retry"] == {"kind": "after_fix"}, refusal
                assert len(fixtures.seen) == seen
        finally:
            await rust.close()


async def test_the_device_names_what_went_wrong_with_the_hubs_answer(unit_stt):
    limit = contract_limit("PLUGIN_RESPONSE_MAX_BYTES")
    answers = {
        "/v1/plugins/at_limit/call": raw(200, value_of_size(limit)),
        "/v1/plugins/over_limit/call": raw(200, value_of_size(limit + 1)),
        "/v1/plugins/no_value/call": raw(200, b'{"result":1}'),
        "/v1/plugins/extra_field/call": raw(200, b'{"value":1,"more":2}'),
        "/v1/plugins/not_json/call": raw(200, b"value"),
        "/v1/plugins/bare_refusal/call": raw(404, b'{"detail":"Not Found"}'),
        "/v1/plugins/odd_code/call": raw(
            400,
            json.dumps(
                {"code": "Odd-Code", "message": "x", "refusal": {"code": "Odd-Code"}}
            ).encode(),
        ),
        "/v1/plugins/redirect/call": raw(302, b"{}"),
        "/v1/plugins/late/call": raw(200, b'{"value":1}', delay=DEVICE_WAIT_MS / 1000 + 3),
    }
    with tempfile.TemporaryDirectory(prefix="plugin-e2e-") as directory:
        root = Path(directory)
        settings = settings_for(root, unit_stt)
        store = Store(root / "hub", root / "key")
        fixtures = Fixtures(create_app(settings, store=store), answers)
        rust = RustDevice(Path(BINARY).resolve(), root / "device")
        try:
            async with tcp_server(fixtures) as hub:
                await rust.start()
                await pair(rust, store, hub)

                # An answer at the limit reaches the caller whole.
                outcome = await plugin_call(rust, "at_limit", "read", {})
                assert outcome["outcome"] == "answered", str(outcome)[:300]
                assert outcome["value"] == "x" * (limit - len(b'{"value":""}'))

                # One byte over is the device's refusal, with its numbers.
                refusal = failed(await plugin_call(rust, "over_limit", "read", {}))
                assert refusal["code"] == "plugin_response_too_large", refusal
                assert refusal["max_bytes"] == limit, refusal
                assert limit < refusal["bytes"] <= limit + 1, refusal
                assert refusal["retry"] == {"kind": "after_fix"}, refusal

                # An answer outside the contract names the part that was wrong.
                for plugin, field, size in (
                    ("no_value", "value", 12),
                    ("extra_field", "value", 20),
                    ("not_json", "value", 5),
                    ("bare_refusal", "refusal", 22),
                    ("odd_code", "refusal", None),
                    ("redirect", "status", 2),
                ):
                    refusal = failed(await plugin_call(rust, plugin, "read", {}))
                    assert refusal["code"] == "hub_reply_invalid", (plugin, refusal)
                    assert refusal["field"] == field, (plugin, refusal)
                    if size is not None:
                        assert refusal["received_bytes"] == size, (plugin, refusal)
                    assert refusal["retry"] == {"kind": "check_outcome"}, refusal

                # The device's own wait, with its numbers, before the app's.
                started = time.monotonic()
                refusal = failed(await plugin_call(rust, "late", "read", {}))
                waited = time.monotonic() - started
                assert refusal["code"] == "timeout", refusal
                assert refusal["operation"] == "plugin_call", refusal
                assert refusal["deadline_ms"] == DEVICE_WAIT_MS, refusal
                assert refusal["waited_ms"] >= DEVICE_WAIT_MS, refusal
                assert refusal["retry"] == {"kind": "check_outcome"}, refusal
                assert waited < APP_WAIT_SECONDS, waited
                assert fixtures.seen.count("/v1/plugins/late/call") == 1

            # The hub is gone: the device says it cannot reach it.
            await until(rust.state, lambda state: state["connection"] != "online")
            refusal = failed(await plugin_call(rust, "vault", "config", {}))
            assert refusal == {
                "code": "hub_unreachable",
                "since_ms": None,
                "retry": {"kind": "check_outcome"},
            }
        finally:
            await rust.close()


def test_the_device_side_of_a_call_is_in_the_contract():
    """Each refusal the device gives a call is a row of plugin-v1.md, and the
    response limit has its copy where the device reads it."""
    contract = PLUGIN_CONTRACT.read_text()
    rows = {
        line.split("|")[1].strip(): line.split("|")[2].strip()
        for line in contract.splitlines()
        if line.startswith("| `")
    }
    for code in (
        "not_paired",
        "privacy_stopped",
        "invalid_parameter",
        "timeout",
        "hub_reply_invalid",
        "upstream_failed",
    ):
        assert f"`{code}`" in rows, code
        assert "device" in rows[f"`{code}`"], (code, rows[f"`{code}`"])
    for code in ("plugin_call_too_large", "plugin_response_too_large", "hub_unreachable"):
        assert "device" in rows[f"`{code}`"], code
    # Nothing about the local channel is left undefined.
    undefined = contract.split("## Not defined yet", 1)[1]
    assert "local channel" not in undefined, undefined
    assert "malformed `id`" not in undefined, undefined

    entries = {entry["name"]: entry for entry in json.loads(CONSTANTS.read_text())}
    for name in ("PLUGIN_CALL_MAX_BYTES", "PLUGIN_RESPONSE_MAX_BYTES"):
        entry = entries[name]
        assert entry["value"] == contract_limit(name), entry
        assert entry["unit"] == "bytes", entry
        rust_copies = [copy for copy in entry["copies"] if copy["file"].endswith(".rs")]
        if entry["source"] != "rust":
            assert rust_copies, f"{name} has no copy in the device's code"
