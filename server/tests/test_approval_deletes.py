"""A command that deletes is approved one at a time: the hub offers only
`once` and `deny` for it and refuses a wider answer (hub-v1 «Choices and
questions»)."""

import pytest
from support_review_regressions import reviewed as reviewed  # noqa: PLC0414
from support_review_regressions import runtime_fixture

from orbit_hub.assistant.deletion import Deletion, command_deletion
from orbit_hub.protocol import ApprovalParams, HubError


@pytest.mark.parametrize(
    "command,paths",
    [
        ("rm tarifas-semana-41.csv tarifas-semana-42.csv", 2),
        ("rm -f informe.txt", 1),
        ("unlink borrador.md", 1),
        ("rm -- -raro.txt", 1),
        ("/bin/rm viejo.log", 1),
        ("sudo rm /var/tmp/prueba.txt", 1),
        ("LANG=C rm prueba.txt", 1),
        ("rm -r carpeta-vieja", None),
        ("rm -rf build", None),
        ("rm --recursive build", None),
        ("rm *.csv", None),
        ("rm semana-$N.csv", None),
        ("rmdir vacia", None),
        ("git rm notas.md", None),
        ("git clean -fd", None),
        ("find . -name '*.tmp' -delete", None),
        ("find . -name '*.tmp' -exec rm {} +", None),
        ("cd Piloto && rm a.csv", None),
        ("ls | xargs rm", None),
        ("bash -c 'rm a.csv'", None),
        ("bash -lc 'rm a.csv'", None),
        ("shred -u secreto.txt", None),
        ("trash viejo.pdf", None),
        ("echo hola\nrm a.csv", None),
        ("rm 'unbalanced", None),
    ],
)
def test_a_command_that_deletes_is_seen_with_the_files_it_names(command, paths):
    assert command_deletion(command) == Deletion(paths=paths)


@pytest.mark.parametrize(
    "command",
    [
        "ls -la",
        "python3 exportar_tarifas.py --semana 43",
        "echo rm -rf /",
        "grep -r rm .",
        "rmate notas.md",
        "cat rm.txt",
        "git status",
        "git commit -m 'rm the old file'",
        "find . -name '*.tmp'",
        "bash -lc 'ls -la'",
        "",
        "   ",
    ],
)
def test_a_command_that_does_not_delete_is_not_one(command):
    assert command_deletion(command) is None


async def hermes(runtime, upstream, payload):
    await runtime.connections["default"].receive(
        {"type": "approval.request", "session_id": upstream, "payload": payload}
    )


def required(queue):
    while not queue.empty():
        frame = queue.get_nowait()
        if frame["event"] == "approval.required":
            return frame
    raise AssertionError("no approval.required")


async def test_a_deleting_command_is_offered_only_once_or_no(reviewed):
    runtime, session, turn, upstream = await runtime_fixture(reviewed)
    queue = runtime.subscribe()
    await hermes(
        runtime,
        upstream,
        {
            "request_id": "borrar-1",
            "command": "rm tarifas-semana-41.csv tarifas-semana-42.csv",
            "description": "Borrar dos archivos",
            "choices": ["once", "deny", "session", "always"],
        },
    )
    data = required(queue)["data"]
    assert data["choices"] == ["once", "deny"]
    assert data["deletes"] == {"paths": 2}
    snapshot = runtime.snapshot()
    [pending] = [item for entry in snapshot["sessions"] for item in entry["pending"]]
    assert pending["choices"] == ["once", "deny"]
    assert pending["deletes"] == {"paths": 2}
    for choice in ("session", "always"):
        with pytest.raises(HubError) as refused:
            await runtime.handle(
                "approval.respond",
                ApprovalParams(
                    session_id=session,
                    request_id="borrar-1",
                    approved=True,
                    turn_id=turn,
                    choice=choice,
                ),
                f"respuesta-{choice}",
            )
        assert refused.value.code == "approval_not_allowed"
        assert f"«{choice}»" in refused.value.message and "once, deny" in refused.value.message
    assert all(method != "approval.respond" for method, _ in runtime.connections["default"].calls)
    event, receipt = await runtime.handle(
        "approval.respond",
        ApprovalParams(
            session_id=session, request_id="borrar-1", approved=True, turn_id=turn, choice="once"
        ),
        "respuesta-once",
    )
    assert (event, receipt["applied"]) == ("interaction.resolved", True)
    await runtime.close()


async def test_a_deleting_command_without_named_choices_stays_once_or_no(reviewed):
    runtime, _, _, upstream = await runtime_fixture(reviewed)
    queue = runtime.subscribe()
    await hermes(runtime, upstream, {"request_id": "borrar-2", "command": "rm -rf build"})
    data = required(queue)["data"]
    assert data["choices"] == ["once", "deny"]
    assert data["deletes"] == {}
    await runtime.close()


async def test_a_command_that_does_not_delete_keeps_every_choice(reviewed):
    runtime, _, _, upstream = await runtime_fixture(reviewed)
    queue = runtime.subscribe()
    await hermes(
        runtime,
        upstream,
        {
            "request_id": "listar-1",
            "command": "ls -la",
            "description": "Listar la carpeta",
            "choices": ["once", "deny", "session", "always"],
        },
    )
    data = required(queue)["data"]
    assert data["choices"] == ["once", "deny", "session", "always"]
    assert "deletes" not in data
    await runtime.close()
