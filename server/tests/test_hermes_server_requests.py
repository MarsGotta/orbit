"""The questions Hermes asks the person reach the Mac, and the answers reach Hermes.

Hermes asks over JSON-RPC requests from server to client, and only a client
that declared `client.capabilities {server_requests: true}` on its connection
gets them; any other has every approval withdrawn at once. Every frame of
Hermes here is built from `contracts/hermes-server-requests.json` (generated
from the installed Hermes's models), and every answer of the hub is checked
against it (`ContractPeer.violations`).
"""

import pytest
from support_hermes_hub import HubOnPeer
from support_hermes_requests import (
    CONTRACT,
    METHOD_NOT_FOUND,
    WINDOW_REQUESTS,
    ContractPeer,
    cancel_event,
    methods,
    request_frame,
)

from orbit_hub.protocol import HubError

# Hermes's approvals as its terminal guard words them (tools/approval.py).
CHOICES = ["once", "session", "always", "deny"]
# The ceiling of open requests on one connection, from the spec of this change.
OPEN_REQUESTS_CEILING = 64


@pytest.fixture
async def hub(environment, tmp_path):
    started = []

    async def start(capabilities="answers"):
        peer = ContractPeer(capabilities)
        running = await HubOnPeer(environment, tmp_path, peer).start()
        started.append(running)
        return running, peer

    yield start
    for running in started:
        await running.close()


def approval(live, command="chmod -R 777 /srv/example/shared", choices=CHOICES):
    return request_frame(
        "approval",
        live,
        request_id="0123456789abcdef0123456789abcdef",
        command=command,
        description="world/other-writable permissions",
        choices=choices,
        allow_permanent=True,
        allow_session=True,
    )


async def test_the_hub_declares_it_answers_hermes_requests_before_any_session(hub):
    running, peer = await hub()
    await running.send("Hola")
    sent = methods(peer.requests)
    assert "client.capabilities" in sent, f"the hub never declared it; it sent {sent}"
    declaration = peer.requests[sent.index("client.capabilities")]
    assert declaration["params"] == {"server_requests": True}
    assert sent.index("client.capabilities") < sent.index("session.create")
    assert sent.count("client.capabilities") == 1
    assert peer.violations == []


async def test_an_approval_reaches_the_mac_and_each_answer_reaches_hermes_as_it_reads_it(hub):
    running, peer = await hub()
    session = await running.send("Abre la carpeta compartida a todos")
    live = running.live(session)

    asked = approval(live)
    await peer.push(asked)
    required = (await running.frame("approval.required"))["data"]
    assert required["session_id"] == session
    assert required["description"] == "world/other-writable permissions"
    assert required["choices"] == CHOICES
    assert "deletes" not in required
    name, receipt = await running.approve(session, required["request_id"], "once")
    answer = await peer.request("request.answer")
    assert answer["params"] == {"id": asked["id"], "result": {"choice": "once"}}
    assert name == "interaction.resolved"
    assert receipt["applied"] is True and receipt["resolved"] == 1

    refused = approval(live)
    await peer.push(refused)
    required = (await running.frame("approval.required"))["data"]
    await running.approve(session, required["request_id"], "deny")
    answers = [frame for frame in peer.requests if frame["method"] == "request.answer"]
    assert answers[-1]["params"] == {"id": refused["id"], "result": {"choice": "deny"}}

    peer.answer_status = "expired"
    late = approval(live)
    await peer.push(late)
    required = (await running.frame("approval.required"))["data"]
    name, receipt = await running.approve(session, required["request_id"], "once")
    assert name == "interaction.expired"
    assert receipt["applied"] is False and receipt["resolved"] == 0

    assert "approval.respond" not in methods(peer.requests)
    assert peer.violations == []


async def test_a_command_that_deletes_is_offered_one_at_a_time(hub):
    running, peer = await hub()
    session = await running.send("Borra los dos borradores")
    await peer.push(
        approval(running.live(session), command="rm /srv/example/uno.txt /srv/example/dos.txt")
    )
    required = (await running.frame("approval.required"))["data"]
    assert required["choices"] == ["once", "deny"]
    assert required["deletes"] == {"paths": 2}
    with pytest.raises(HubError) as error:
        await running.approve(session, required["request_id"], "session")
    assert error.value.code == "approval_not_allowed"
    assert "request.answer" not in methods(peer.requests)


async def test_a_clarification_is_answered_one_question_at_a_time(hub):
    running, peer = await hub()
    session = await running.send("Ordena los informes")
    live = running.live(session)

    questions = [
        {
            "qid": "carpeta",
            "question": "¿En qué carpeta?",
            "choices": ["Informes", "Facturas"],
            "multi_select": False,
        },
        {"qid": "nombre", "question": "¿Con qué nombre?", "choices": None, "multi_select": False},
    ]
    batch = request_frame("clarify", live, questions=questions)
    await peer.push(batch)
    required = (await running.frame("clarify.required"))["data"]
    assert [question["qid"] for question in required["questions"]] == ["carpeta", "nombre"]
    name, receipt = await running.clarify(
        session, required["request_id"], "Informes", question_id="carpeta"
    )
    lock = await peer.request("clarify.lock")
    assert lock["params"] == {
        "request_id": batch["id"],
        "question_id": "carpeta",
        "answer": "Informes",
    }
    assert name == "interaction.resolved" and receipt["remaining"] == ["nombre"]
    name, receipt = await running.clarify(
        session, required["request_id"], "Resumen de octubre", question_id="nombre"
    )
    assert name == "interaction.resolved" and receipt["remaining"] == []

    single = request_frame(
        "clarify",
        live,
        questions=[{"qid": "seguir", "question": "¿Sigo?", "choices": ["Sí", "No"]}],
    )
    await peer.push(single)
    required = (await running.frame("clarify.required"))["data"]
    assert required["question"] == "¿Sigo?"
    assert required["choices"] == ["Sí", "No"]
    assert "questions" not in required
    name, receipt = await running.clarify(session, required["request_id"], "Sí")
    locks = [frame for frame in peer.requests if frame["method"] == "clarify.lock"]
    assert locks[-1]["params"] == {
        "request_id": single["id"],
        "question_id": "seguir",
        "answer": "Sí",
    }
    assert name == "interaction.resolved" and receipt["applied"] is True

    assert "clarify.respond" not in methods(peer.requests)
    assert peer.violations == []


async def test_what_orbit_cannot_show_is_refused_at_once_with_its_cause(hub):
    running, peer = await hub()
    session = await running.send("Instala lo que haga falta")
    live = running.live(session)
    unshown = [
        method for method in CONTRACT["server_requests"] if method not in {"approval", "clarify"}
    ]
    assert {"sudo", "secret", *WINDOW_REQUESTS} <= set(unshown)

    pushed = {}
    for method in unshown:
        pushed[method] = request_frame(method, live)
        await peer.push(pushed[method])
    for method, frame in pushed.items():
        answer = await peer.response(frame["id"])
        assert "result" not in answer, f"{method} was answered as if shown: {answer}"
        code = CONTRACT["not_shown_code"] if method in WINDOW_REQUESTS else METHOD_NOT_FOUND
        assert answer["error"]["code"] == code, f"{method}: {answer}"
        assert isinstance(answer["error"]["message"], str) and answer["error"]["message"]

    for method in unshown:
        if method in WINDOW_REQUESTS:
            continue
        await running.frame(
            "chat.status",
            lambda data, method=method: (
                data["session_id"] == session
                and data["kind"] == "warning"
                and method in data["text"]
            ),
        )

    # Hermes's requests are read in order: once the next one reached the Mac,
    # every earlier one had its answer, and only one.
    await peer.push(approval(live))
    await running.frame("approval.required")
    for method, frame in pushed.items():
        assert len(peer.answers_to(frame["id"])) == 1, method
    assert peer.violations == []


@pytest.mark.parametrize("reason", CONTRACT["cancel_reasons"])
async def test_a_request_hermes_withdraws_leaves_with_its_cause_and_is_never_answered(hub, reason):
    running, peer = await hub()
    session = await running.send("Cambia los permisos")
    asked = approval(running.live(session))
    await peer.push(asked)
    required = (await running.frame("approval.required"))["data"]

    await peer.push(cancel_event(asked, reason))
    expired = await running.frame("interaction.expired")
    assert expired["data"] == {"session_id": session, "request_id": required["request_id"]}
    await running.frame(
        "chat.status",
        lambda data: (
            data["session_id"] == session and data["kind"] == "warning" and reason in data["text"]
        ),
    )
    with pytest.raises(HubError) as error:
        await running.approve(session, required["request_id"], "once")
    assert error.value.code == "interaction_not_found"
    assert "request.answer" not in methods(peer.requests)


async def test_a_lost_connection_retires_the_open_card_and_repeats_nothing(hub):
    running, peer = await hub()
    session = await running.send("Cambia los permisos")
    await peer.push(approval(running.live(session)))
    required = (await running.frame("approval.required"))["data"]

    await peer.drop()
    lost = await running.frame("error", lambda data: data.get("session_id") == session)
    assert lost["data"]["code"] == "outcome_unknown"
    assert running.runtime.pending == {}
    with pytest.raises(HubError):
        await running.approve(session, required["request_id"], "once")

    # The cut turn's outcome stays unknown (no new turn in it); another
    # conversation opens a new connection, which repeats nothing of the old one.
    await running.send("Otra cosa")
    sent = methods(peer.requests)
    assert sent.count("client.capabilities") == 2
    assert "request.answer" not in sent and "approval.respond" not in sent


async def test_a_request_for_a_session_orbit_does_not_hold_is_refused_at_once(hub):
    running, peer = await hub()
    await running.send("Hola")
    strays = [
        approval("session-of-nobody"),
        request_frame("clarify", "session-of-nobody", questions=[{"qid": "q", "question": "¿?"}]),
    ]
    for stray in strays:
        await peer.push(stray)
        answer = await peer.response(stray["id"])
        assert "error" in answer and "result" not in answer
    assert running.runtime.pending == {}


async def test_open_requests_are_bounded_on_each_connection(hub):
    running, peer = await hub()
    session = await running.send("Muchas preguntas")
    live = running.live(session)
    held = []
    for _ in range(OPEN_REQUESTS_CEILING):
        held.append(approval(live))
        await peer.push(held[-1])
        await running.frame("approval.required")
    beyond = approval(live)
    await peer.push(beyond)
    refusal = await peer.response(beyond["id"])
    assert str(OPEN_REQUESTS_CEILING) in refusal["error"]["message"]
    assert all(not peer.answers_to(frame["id"]) for frame in held)


async def test_a_hermes_older_than_client_requests_keeps_its_events(hub):
    running, peer = await hub(capabilities="unknown")
    session = await running.send("Cambia los permisos")
    sent = methods(peer.requests)
    assert "client.capabilities" in sent, f"the hub never declared it; it sent {sent}"
    assert sent.index("client.capabilities") < sent.index("session.create")

    await peer.push(
        {
            "jsonrpc": "2.0",
            "method": "event",
            "params": {
                "type": "approval.request",
                "session_id": running.live(session),
                "payload": {
                    "request_id": "old-approval",
                    "command": "chmod -R 777 /srv/example/shared",
                    "description": "world/other-writable permissions",
                    "choices": CHOICES,
                },
            },
        }
    )
    required = (await running.frame("approval.required"))["data"]
    name, _ = await running.approve(session, required["request_id"], "once")
    respond = await peer.request("approval.respond")
    assert respond["params"] == {
        "session_id": running.live(session),
        "request_id": "old-approval",
        "choice": "once",
    }
    assert name == "interaction.resolved"


async def test_hermes_refusing_the_declaration_fails_the_connection_with_its_cause(hub):
    running, peer = await hub(capabilities=5000)
    with pytest.raises(HubError) as error:
        await running.send("Hola")
    assert error.value.code == "hermes_capabilities_refused"
    assert error.value.numbers == {"upstream_code": 5000}
    assert "5000" in error.value.message
    assert "session.create" not in methods(peer.requests)
