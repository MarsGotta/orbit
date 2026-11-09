"""An older Hermes's single question reaches the Mac, and its answer goes back as it reads it.

Hermes 0.21.4 asks one question, when its model calls `clarify` with a lone
`question`, as `{question, choices?, multi_select?}` without `questions`, and
takes the answer only as that request's result `{"answer": …}`, which
`request.answer` carries; it answers `clarify.lock` on it `expired`. Every
frame of that Hermes here is built from its own contract
(`support_hermes_legacy`), and every answer of the hub is checked against it
(`ContractPeer.violations`).
"""

import pytest
from support_hermes_hub import HubOnPeer
from support_hermes_legacy import LEGACY
from support_hermes_requests import ContractPeer, methods, problems, request_frame

# JSON-RPC's "invalid params": a clarification with no question to show.
INVALID_PARAMS = -32602


@pytest.fixture
async def older(environment, tmp_path):
    peer = ContractPeer(contract=LEGACY)
    running = await HubOnPeer(environment, tmp_path, peer).start()
    yield running, peer
    await running.close()


def lone_question(live, **values):
    return request_frame("clarify", live, contract=LEGACY, **values)


async def test_an_older_hermes_lone_question_is_answered_by_request_answer(older):
    running, peer = older
    session = await running.send("Elige el color de la portada")
    asked = lone_question(running.live(session), question="¿Qué color?", choices=["Rojo", "Azul"])
    # The installed Hermes never sends this shape: its clarification carries `questions`.
    assert problems("server_request.clarify.params", asked["params"]) != []

    await peer.push(asked)
    required = (await running.frame("clarify.required"))["data"]
    assert required["question"] == "¿Qué color?"
    assert required["choices"] == ["Rojo", "Azul"]
    assert "questions" not in required
    name, receipt = await running.clarify(session, required["request_id"], "Azul")
    answer = await peer.request("request.answer")
    assert answer["params"] == {"id": asked["id"], "result": {"answer": "Azul"}}
    assert name == "interaction.resolved"
    assert receipt["status"] == "ok" and receipt["remaining"] == [] and receipt["applied"] is True
    assert "clarify.lock" not in methods(peer.requests)
    assert peer.violations == []


async def test_an_older_hermes_lone_question_answered_too_late_is_expired(older):
    running, peer = older
    session = await running.send("Pon un título")
    peer.answer_status = "expired"
    asked = lone_question(running.live(session), question="¿Qué título?")
    await peer.push(asked)
    required = (await running.frame("clarify.required"))["data"]
    assert required["question"] == "¿Qué título?" and not required.get("choices")
    name, receipt = await running.clarify(session, required["request_id"], "Resumen")
    answer = await peer.request("request.answer")
    assert answer["params"] == {"id": asked["id"], "result": {"answer": "Resumen"}}
    assert name == "interaction.expired" and receipt["applied"] is False
    assert peer.violations == []


async def test_an_older_hermes_clarification_without_a_question_is_refused_at_once(older):
    running, peer = older
    session = await running.send("Hola")
    empty = lone_question(running.live(session))
    await peer.push(empty)
    refusal = await peer.response(empty["id"])
    assert "result" not in refusal
    assert refusal["error"]["code"] == INVALID_PARAMS
    assert running.runtime.pending == {}
    assert peer.violations == []


async def test_an_older_hermes_asks_its_documented_way_as_the_installed_one(older):
    running, peer = older
    session = await running.send("Ordena y comparte los informes")
    live = running.live(session)

    batch = lone_question(
        live,
        questions=[{"qid": "q0", "question": "¿Qué carpeta?", "choices": ["Informes", "Facturas"]}],
    )
    await peer.push(batch)
    required = (await running.frame("clarify.required"))["data"]
    assert required["question"] == "¿Qué carpeta?" and "questions" not in required
    name, receipt = await running.clarify(session, required["request_id"], "Informes")
    lock = await peer.request("clarify.lock")
    assert lock["params"] == {"request_id": batch["id"], "question_id": "q0", "answer": "Informes"}
    assert name == "interaction.resolved" and receipt["applied"] is True

    asked = request_frame(
        "approval",
        live,
        contract=LEGACY,
        request_id="00112233445566778899aabbccddeeff",
        command="chmod 666 /srv/example/compartido.txt",
        description="world/other-writable permissions",
        choices=["once", "session", "always", "deny"],
    )
    await peer.push(asked)
    required = (await running.frame("approval.required"))["data"]
    await running.approve(session, required["request_id"], "once")
    answer = await peer.request("request.answer")
    assert answer["params"] == {"id": asked["id"], "result": {"choice": "once"}}
    assert peer.violations == []
