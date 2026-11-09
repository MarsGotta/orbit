"""The Hermes contract the hub's tests read is the installed Hermes's own.

`contracts/hermes-server-requests.json` stands for Hermes where it is not
installed (another machine, a runner): the hub's tests build Hermes's frames
from it. Where Hermes is installed, these tests ask Hermes's own interpreter
(in a child process, writing nothing in its checkout) whether that file is
still what its models declare, and whether what the hub writes back passes
those models. Where it is not installed they are skipped, saying why.
"""

import asyncio

import pytest
from support_hermes_hub import HubOnPeer
from support_hermes_requests import (
    CONTRACT_PATH,
    ContractPeer,
    generate,
    hermes_python,
    request_frame,
    validate_with_hermes,
)

REGENERATE = "server/.venv/bin/python server/tests/support_hermes_requests.py --write"


@pytest.fixture(scope="module")
def hermes():
    python, why = hermes_python()
    if python is None:
        pytest.skip(why)
    return python


def test_the_contract_is_what_the_installed_hermes_declares(hermes):
    stored = CONTRACT_PATH.read_text(encoding="utf-8")
    assert generate(hermes) == stored, (
        "The installed Hermes no longer declares the requests the hub was written for. "
        f"Regenerate the contract with `{REGENERATE}`, read its difference and adapt the hub"
    )


async def test_what_the_hub_writes_back_passes_the_installed_hermes_models(
    hermes, environment, tmp_path
):
    peer = ContractPeer()
    running = await HubOnPeer(environment, tmp_path, peer).start()
    try:
        session = await running.send("Prepara la carpeta")
        live = running.live(session)
        await peer.push(
            request_frame(
                "approval",
                live,
                request_id="fedcba9876543210fedcba9876543210",
                command="chmod -R 777 /srv/example/shared",
                description="world/other-writable permissions",
                choices=["once", "deny"],
            )
        )
        required = (await running.frame("approval.required"))["data"]
        await running.approve(session, required["request_id"], "once")
        await peer.push(
            request_frame("clarify", live, questions=[{"qid": "q", "question": "¿Cuál?"}])
        )
        required = (await running.frame("clarify.required"))["data"]
        await running.clarify(session, required["request_id"], "La primera")
        refused = [request_frame("sudo", live), request_frame("preview.read", live)]
        for frame in refused:
            await peer.push(frame)
        answers = [await peer.response(frame["id"]) for frame in refused]
        declaration = await peer.request("client.capabilities")
        answer = await peer.request("request.answer")
        lock = await peer.request("clarify.lock")
    finally:
        await running.close()

    written = [
        ("client.capabilities.params", declaration["params"]),
        ("request.answer.params", answer["params"]),
        ("server_request.approval.result", answer["params"]["result"]),
        ("clarify.lock.params", lock["params"]),
        *(("response", frame) for frame in answers),
    ]
    assert await asyncio.to_thread(validate_with_hermes, hermes, written) == []
