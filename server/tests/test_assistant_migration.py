"""Conversations of a hub without `[[agents]]` after the move to `[[agents]]`.

A hub without `[[agents]]` is one implicit agent whose id is `default`, and
every conversation it opens stores that id. The self-hosting guide's example
then lists `id = "deimos"` first and says to keep the old profile's agent
first so they resume where they were kept. Doing exactly that, those
conversations belong to the default agent: in the turns, in what the Mac reads
of them, and when a Mac that remembers `default` still names it.
The Hermes is the synthetic gateway of `test_agents`; names are invented.
"""

import pytest
from conftest import device
from test_agents import SyntheticHermes, answered, hub, send, socket, until


@pytest.mark.parametrize("named", [{}, {"agent_id": "default"}], ids=["unnamed", "remembered"])
def test_conversations_of_the_implicit_agent_continue_after_listing_it_first(
    environment, tmp_path, named
):
    settings, store, adapter = environment
    paired = device(store)
    headers = {"Authorization": "Bearer " + paired["token"]}
    with SyntheticHermes(tmp_path) as hermes:
        implicit = hermes.settings(settings)
        with hub(implicit, store, adapter) as http, socket(http, paired) as ws:
            session = answered(ws, send(ws, "Hola"))["session_id"]
        # The old profile's agent first, as the guide says, with the same profile.
        listed = hermes.settings(
            settings,
            agents=(
                {"id": "deimos", "name": "Deimos", "kind": "hermes", "profile": settings.profile},
                {"id": "lyra", "name": "Lyra", "kind": "hermes", "profile": "lyra-profile"},
            ),
        )
        with hub(listed, store, adapter) as http:
            [row] = http.get("/v1/sessions", headers=headers).json()["sessions"]
            page = http.get(f"/v1/sessions/{session}/messages", headers=headers).json()
            with socket(http, paired) as ws:
                again = until(
                    ws, send(ws, "Seguimos", session_id=session, **named), "chat.completed", "error"
                )
    assert (row["agent_id"], page["agent_id"]) == ("deimos", "deimos")
    assert again["event"] == "chat.completed", again["data"]
