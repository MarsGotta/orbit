"""Claude and Codex on a hub without Linux: refused with their own cause.

Bubblewrap is the only jail, and it is Linux's. Without Linux the hub refuses
to start an official vendor with `assistant_jail_unavailable`, before looking
for bubblewrap or starting any process, and the connection keeps working. A
hub without Linux is simulated with what decides it,
`orbit_hub.host_system.linux`. bubblewrap is made to look missing, so a refusal
that came from looking for it would say `agent_adapter_missing` instead.
"""

import shutil

import pytest
from test_official_assistants import official  # noqa: F401


def error_after(ws, method, **params):
    from test_official_assistants import send

    send(ws, method, **params)
    while (frame := ws.receive_json())["event"] != "error":
        pass
    return frame["data"]


@pytest.mark.parametrize("official", ["claude-code", "codex"], indirect=True)
def test_without_linux_claude_and_codex_refuse_with_their_own_cause(
    official,  # noqa: F811
    request,
    tmp_path,
    monkeypatch,
):
    from orbit_hub import host_system

    service = request.node.callspec.params["official"]
    _, _, _, _, headers, client = official
    monkeypatch.setattr(host_system, "linux", lambda: False)
    which = shutil.which
    monkeypatch.setattr(
        shutil, "which", lambda name, *a, **k: None if name == "bwrap" else which(name, *a, **k)
    )
    with client() as http, http.websocket_connect("wss://localhost/v1/ws", headers=headers) as ws:
        while ws.receive_json()["event"] != "ready":
            pass
        first = error_after(ws, "chat.send", text="Must never reach a vendor")
        # The same connection answers the next message with the same cause.
        second = error_after(ws, "chat.send", text="Nor this one")

    for error in (first, second):
        assert error["code"] == "assistant_jail_unavailable"
        assert error["refusal"]["service"] == service
        assert error["refusal"]["retry"] == {"kind": "never"}
        # Neither publishing again nor installing anything changes it.
        message = error["message"].lower()
        assert "publish" not in message and "install" not in message
    assert not (tmp_path / "record.jsonl").exists(), "no vendor process ran"
