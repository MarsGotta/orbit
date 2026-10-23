"""What the agent does in a turn reaches the Mac as `chat.item`, and its
questions keep the choices the agent offered (hub-v1 «chat.item»)."""

import json
from uuid import uuid4

import pytest
from support_review_regressions import reviewed as reviewed  # noqa: PLC0414
from support_review_regressions import runtime_fixture

from orbit_hub.assistant.activity import MAX_ITEM_LABEL_CHARS, ORBIT_TOOLS, tool_label
from orbit_hub.protocol import ApprovalParams, ClarifyParams, HubError, SendParams


async def hermes(runtime, upstream, kind, payload):
    await runtime.connections["default"].receive(
        {"type": kind, "session_id": upstream, "payload": payload}
    )


def items(queue):
    frames = []
    while not queue.empty():
        frame = queue.get_nowait()
        if frame["event"] == "chat.item":
            frames.append(frame)
    return frames


async def test_a_hermes_tool_is_one_item_from_its_start_to_its_end(reviewed):
    runtime, session, turn, upstream = await runtime_fixture(reviewed)
    queue = runtime.subscribe()
    call = {"tool_id": "call-1", "name": "mcp_orbit_orbit_look", "context": "", "args": {}}
    await hermes(runtime, upstream, "tool.start", call)
    await hermes(runtime, upstream, "tool.complete", {**call, "is_error": False, "result": "{}"})
    started, completed = items(queue)
    assert started["id"] == turn
    assert started["data"] == {
        "session_id": session,
        "item_id": "call-1",
        "kind": "tool",
        "phase": "started",
        "label": "Mirando la pantalla",
        "subject": "la pantalla",
    }
    assert completed["data"] == {**started["data"], "phase": "completed"}
    await runtime.close()


def answers(runtime, session):
    page = runtime.store.chat.session_messages(runtime.device, session, None, 50)
    return [message for message in page["messages"] if message["role"] == "assistant"]


async def test_each_answer_keeps_what_its_turn_did_and_the_next_turn_does_not_take_it(reviewed):
    """«Trabajó…» stays with its answer: the hub keeps each turn's summary with it."""
    runtime, session, _, upstream = await runtime_fixture(reviewed)
    clock = {"now": 1001.0}
    runtime.now = lambda: clock["now"]
    look = {"tool_id": "call-1", "name": "mcp_orbit_orbit_look", "args": {}}
    calendar = {"tool_id": "call-2", "name": "mcp_orbit_orbit_calendar", "args": {}}
    for call in (look, calendar):
        await hermes(runtime, upstream, "tool.start", call)
        await hermes(runtime, upstream, "tool.complete", {**call, "is_error": False})
    # The same step told twice is still one step.
    await hermes(runtime, upstream, "tool.complete", {**look, "is_error": False})
    clock["now"] = 1047.0
    await hermes(runtime, upstream, "message.complete", {"text": "Hecho.", "status": "success"})
    [first] = answers(runtime, session)
    assert first["work"] == {
        "started_at_ms": 1_001_000,
        "ended_at_ms": 1_047_000,
        "tools": 2,
        "subjects": ["la pantalla", "el calendario"],
    }
    # The next turn, without tools: only how long it worked; the first keeps its own.
    await runtime.handle("chat.send", SendParams(text="otra", session_id=session), str(uuid4()))
    clock["now"] = 1050.0
    await hermes(runtime, upstream, "message.complete", {"text": "Vale.", "status": "success"})
    first, second = answers(runtime, session)
    assert first["work"]["tools"] == 2
    assert second["work"] == {
        "started_at_ms": 1_047_000,
        "ended_at_ms": 1_050_000,
        "tools": 0,
        "subjects": [],
    }
    # What the user wrote carries no work.
    page = runtime.store.chat.session_messages(runtime.device, session, None, 50)
    assert all("work" not in m for m in page["messages"] if m["role"] == "user")
    await runtime.close()


async def test_an_answer_stored_before_the_summary_existed_has_none(reviewed):
    runtime, session, _, _ = await runtime_fixture(reviewed)
    runtime.store.chat.append_message(session, "assistant", "Antigua.", now_ms=1)
    [old] = answers(runtime, session)
    assert "work" not in old
    await runtime.close()


async def test_a_failed_hermes_tool_says_its_cause_and_another_tool_keeps_its_name(reviewed):
    runtime, _, _, upstream = await runtime_fixture(reviewed)
    queue = runtime.subscribe()
    call = {"tool_id": "call-2", "name": "web_search", "args": {"query": "tiempo"}}
    await hermes(runtime, upstream, "tool.start", call)
    await hermes(
        runtime,
        upstream,
        "tool.complete",
        {**call, "is_error": True, "result": "Sin conexión\ncon el buscador"},
    )
    started, failed = items(queue)
    assert started["data"]["label"] == "web_search"
    assert started["data"]["subject"] == "web_search"
    assert failed["data"]["phase"] == "failed"
    assert failed["data"]["cause"] == "Sin conexión con el buscador"
    await runtime.close()


async def test_hermes_todos_are_a_plan_with_the_state_of_each_step(reviewed):
    runtime, _, _, upstream = await runtime_fixture(reviewed)
    queue = runtime.subscribe()
    todos = [
        {"id": "1", "content": "Mirar el calendario", "status": "in_progress"},
        {"id": "2", "content": "Crear la nota", "status": "pending"},
    ]
    await hermes(runtime, upstream, "todo.updated", {"todos": todos, "revision": 1})
    todos = [{**todos[0], "status": "completed"}, {**todos[1], "status": "in_progress"}]
    # A turn that is not a local viewer's only has the todo tool's result.
    await hermes(
        runtime,
        upstream,
        "tool.complete",
        {"tool_id": "t", "name": "todo", "is_error": False, "result": json.dumps({"todos": todos})},
    )
    todos = [{**todo, "status": "completed"} for todo in todos]
    await hermes(runtime, upstream, "todo.updated", {"todos": todos, "revision": 3})
    frames = [frame["data"] for frame in items(queue)]
    assert all(frame["kind"] == "plan" for frame in frames), "the todo tool is the plan only"
    plans = frames
    assert [plan["phase"] for plan in plans] == ["started", "progress", "completed"]
    assert plans[0]["item_id"] == "plan"
    assert plans[0]["label"] == "Plan · 0 de 2 pasos"
    assert plans[0]["steps"] == [
        {"text": "Mirar el calendario", "status": "in_progress"},
        {"text": "Crear la nota", "status": "pending"},
    ]
    assert plans[1]["label"] == "Plan · 1 de 2 pasos"
    assert plans[2]["label"] == "Plan · 2 de 2 pasos"
    await runtime.close()


async def test_hermes_reasoning_is_its_summary_bounded_and_its_deltas_are_not_shown(reviewed):
    runtime, _, _, upstream = await runtime_fixture(reviewed)
    queue = runtime.subscribe()
    await hermes(runtime, upstream, "reasoning.delta", {"text": "el pensamiento entero"})
    await hermes(runtime, upstream, "thinking.delta", {"text": "(◕‿◕) pensando"})
    await hermes(runtime, upstream, "reasoning.available", {"text": "Comparo dos fechas. " * 60})
    [reasoning] = items(queue)
    assert reasoning["data"]["kind"] == "reasoning"
    assert reasoning["data"]["phase"] == "completed"
    label = reasoning["data"]["label"]
    assert len(label) == MAX_ITEM_LABEL_CHARS and label.endswith("…")
    await runtime.close()


async def test_a_hermes_subagent_is_an_item_and_its_failure_carries_the_cause(reviewed):
    runtime, _, _, upstream = await runtime_fixture(reviewed)
    queue = runtime.subscribe()
    task = {"goal": "Buscar la sala libre", "task_index": 0, "task_count": 1}
    await hermes(runtime, upstream, "subagent.start", {**task, "subagent_id": "sub-1"})
    await hermes(
        runtime,
        upstream,
        "subagent.complete",
        {**task, "subagent_id": "sub-1", "status": "failed", "summary": "Sin acceso"},
    )
    started, failed = (frame["data"] for frame in items(queue))
    assert (started["item_id"], started["kind"], started["phase"]) == (
        "sub-1",
        "subagent",
        "started",
    )
    assert started["label"] == "Buscar la sala libre"
    assert (failed["phase"], failed["cause"]) == ("failed", "Sin acceso")
    await runtime.close()


@pytest.mark.parametrize(
    "name,arguments,label,subject",
    [
        ("orbit_calendar", {"start_ms": 0, "end_ms": 1}, "Leyendo el calendario", "el calendario"),
        ("mcp__orbit__orbit_screenshot", {"application": "Notas"}, "Mirando Notas", "la pantalla"),
        (
            "orbit_perform_action",
            {"action": {"tool": "click", "arguments": {"app": "Notas"}}},
            "Pulsando en Notas",
            "el Mac",
        ),
        (
            "orbit/orbit_perform_action",
            {"action": {"tool": "type_text", "arguments": {"text": "una clave"}}},
            "Escribiendo",
            "el Mac",
        ),
        (
            "orbit_transcript",
            {"recording_id": "r"},
            "Leyendo la transcripción de una reunión",
            "las reuniones",
        ),
        (
            "mcp__orbit__orbit_files_read",
            {"path": "/Users/prueba/Documentos/informe-privado.md"},
            "Leyendo un archivo del Mac",
            "las carpetas",
        ),
        (
            "mcp__orbit__orbit_attachment",
            {"attachment_id": "00000000-0000-4000-8000-000000000003"},
            "Leyendo un adjunto del mensaje",
            "los adjuntos",
        ),
        ("mcp_otro_buscar", {}, "mcp_otro_buscar", "mcp_otro_buscar"),
    ],
)
def test_orbit_tools_have_spanish_labels_and_others_keep_their_name(
    name, arguments, label, subject
):
    assert tool_label(name, arguments) == (label, subject)


async def test_every_orbit_mcp_tool_has_a_spanish_label(tmp_path):
    from support_orbit_mcp import FakeHub, serving

    async with serving(tmp_path, FakeHub()) as (_client, server):
        served = {tool.name for tool in await server.list_tools()}
    assert served == set(ORBIT_TOOLS)
    for tool in ORBIT_TOOLS:
        label, subject = tool_label(tool, {})
        assert label != tool and subject != tool


async def test_an_approval_offers_hermes_real_choices_and_answers_the_one_chosen(reviewed):
    runtime, session, turn, upstream = await runtime_fixture(reviewed)
    queue = runtime.subscribe()
    await hermes(
        runtime,
        upstream,
        "approval.request",
        {
            "request_id": "aprobar-1",
            # A command that deletes is offered once or no (test_approval_deletes).
            "command": "mkdir -p /tmp/prueba",
            "description": "Crear la carpeta de pruebas",
            "choices": ["once", "deny", "session", "always"],
        },
    )
    required = next(
        frame for frame in iter(queue.get_nowait, None) if frame["event"] == "approval.required"
    )
    assert required["data"]["choices"] == ["once", "deny", "session", "always"]
    event, receipt = await runtime.handle(
        "approval.respond",
        ApprovalParams(
            session_id=session,
            request_id="aprobar-1",
            approved=True,
            turn_id=turn,
            choice="session",
        ),
        "respuesta-1",
    )
    assert (event, receipt["applied"]) == ("interaction.resolved", True)
    connection = runtime.connections["default"]
    assert connection.calls[-1] == (
        "approval.respond",
        {"session_id": upstream, "request_id": "aprobar-1", "choice": "session"},
    )
    await runtime.close()


async def test_a_choice_the_agent_did_not_offer_is_refused_with_what_it_offers(reviewed):
    runtime, session, turn, upstream = await runtime_fixture(reviewed)
    await hermes(
        runtime,
        upstream,
        "approval.request",
        {"request_id": "aprobar-2", "description": "Editar", "choices": ["once", "deny"]},
    )
    with pytest.raises(HubError) as refused:
        await runtime.handle(
            "approval.respond",
            ApprovalParams(
                session_id=session,
                request_id="aprobar-2",
                approved=True,
                turn_id=turn,
                choice="always",
            ),
            "respuesta-2",
        )
    assert refused.value.code == "approval_not_allowed"
    assert "«always»" in refused.value.message and "once, deny" in refused.value.message
    assert all(method != "approval.respond" for method, _ in runtime.connections["default"].calls)
    await runtime.close()


@pytest.mark.parametrize("approved,choice", [(True, "deny"), (False, "once")])
def test_a_choice_that_contradicts_approved_is_refused(approved, choice):
    with pytest.raises(HubError) as refused:
        ApprovalParams.model_validate(
            {
                "session_id": "s",
                "request_id": "r",
                "approved": approved,
                "turn_id": "t",
                "choice": choice,
            }
        )
    assert refused.value.code == "approval_choice_mismatch"


async def test_a_clarification_with_several_questions_answers_each_by_its_id(reviewed):
    runtime, session, _, upstream = await runtime_fixture(reviewed)
    connection = runtime.connections["default"]
    questions = [
        {"qid": "sala", "question": "¿Qué sala?", "choices": ["Norte", "Sur"]},
        {"qid": "hora", "question": "¿A qué hora?"},
    ]
    await hermes(
        runtime,
        upstream,
        "clarify.request",
        {"request_id": "aclarar-1", "question": "Dos preguntas", "questions": questions},
    )
    connection.clarify_result = {"status": "ok", "remaining": ["hora"]}
    event, receipt = await runtime.handle(
        "clarify.respond",
        ClarifyParams(
            session_id=session, request_id="aclarar-1", answer="Norte", question_id="sala"
        ),
        "respuesta-3",
    )
    assert (event, receipt["remaining"]) == ("interaction.resolved", ["hora"])
    assert connection.calls[-1][1]["question_id"] == "sala"
    [pending] = runtime.snapshot()["sessions"][0]["pending"]
    assert [question["qid"] for question in pending["questions"]] == ["hora"]
    await runtime.close()
