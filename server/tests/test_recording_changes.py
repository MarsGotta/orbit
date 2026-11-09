"""Which meetings changed while a Mac was away: the hub's change log.

Every change the hub announces as `recording.changed` gets a number of its
own, growing, carried by the notice (`change`); `ready` says the latest. A Mac
that was away asks `GET /v1/recordings/changes?since=N` and reads again only
the meetings named there. The log answers only for what this start of the hub
saw: a number from before a restart (a change saved just before the hub
stopped may have no entry, and a restored copy would give again numbers the
lost one gave), one this hub never gave, or one older than what the log still
holds is unknown (`recordings: null`, «read the list again»), never «nothing».
`[]` is the fact that nothing changed.

The log lives in memory: it writes nothing to the database, so an earlier hub
finds it as it left it.

Changes are announced here through the hub's runtimes, as every route and
worker of the hub announces them, and once through a real route. Every name is
invented.
"""

import json
from contextlib import asynccontextmanager
from dataclasses import dataclass, replace
from pathlib import Path
from uuid import uuid4

import httpx
from conftest import device
from support_client_e2e import tcp_server
from support_waits import wait_for
from test_meeting_views import TEXTS, bearer, meeting
from websockets.asyncio.client import connect

from orbit_hub.app import create_app
from orbit_hub.storage import Store

NOW = 1_800_000_000.0
CHANGES_PATH = "/v1/recordings/changes"
CONSTANTS = Path(__file__).resolve().parents[2] / "contracts" / "constants.json"
# Changes announced before their notices are read: well under the 256 frames
# a connection queues before the hub drops it as a slow client.
BATCH = 128


@dataclass
class Hub:
    app: object
    http: httpx.AsyncClient
    ws_url: str

    def announce(self, device_id: str, recording: str) -> None:
        """A meeting changed, as every route and worker of the hub says it."""
        self.app.state.device_bridge.runtimes.recording_changed(device_id, recording)


@asynccontextmanager
async def running(settings, store, adapter):
    app = create_app(
        replace(settings, allow_insecure_localhost=True, allowed_hosts=("127.0.0.1", "localhost")),
        store=store,
        assistant=adapter,
        speech=adapter,
        now=lambda: NOW,
    )
    async with tcp_server(app) as url, httpx.AsyncClient(base_url=url) as http:
        yield Hub(app, http, url.replace("http://", "ws://") + "/v1/ws")


@asynccontextmanager
async def mac(hub: Hub, paired):
    """A Mac's connection, and the latest number its `ready` said."""
    async with connect(hub.ws_url, additional_headers=bearer(paired)) as ws:
        ready = json.loads(await wait_for(ws.recv(), "ready"))
        assert ready["event"] == "ready", ready
        latest = ready["data"]["change"]
        assert type(latest) is int, ready["data"]
        yield ws, latest


async def notices(ws, recordings: list[str]) -> list[dict]:
    """The `recording.changed` notices of `recordings`, in the order announced."""
    found = []
    while len(found) < len(recordings):
        frame = json.loads(await wait_for(ws.recv(), "the notice of a changed meeting"))
        if frame["event"] != "recording.changed":
            continue
        assert frame["data"]["recording_id"] == recordings[len(found)], frame
        found.append(frame["data"])
    return found


async def announced(hub: Hub, ws, paired, recordings: list[str]) -> list[int]:
    """Announce each change in turn; the numbers their notices carried."""
    numbers = []
    for start in range(0, len(recordings), BATCH):
        batch = recordings[start : start + BATCH]
        for recording in batch:
            hub.announce(paired["device_id"], recording)
        numbers += [notice["change"] for notice in await notices(ws, batch)]
    assert all(type(number) is int for number in numbers), numbers
    return numbers


async def since(hub: Hub, paired, number: int) -> dict:
    answer = await hub.http.get(CHANGES_PATH, params={"since": str(number)}, headers=bearer(paired))
    assert answer.status_code == 200, answer.text
    body = answer.json()
    assert set(body) == {"latest", "recordings"}, body
    return body


def meetings(count: int) -> list[str]:
    return [str(uuid4()) for _ in range(count)]


def dump(store) -> list[str]:
    with store.reading() as db:
        return list(db.iterdump())


async def test_each_change_has_a_growing_number_and_ready_says_the_latest(environment):
    settings, store, adapter = environment
    paired = device(store)
    budget, review = meetings(2)
    async with running(settings, store, adapter) as hub:
        async with mac(hub, paired) as (ws, first):
            numbers = await announced(hub, ws, paired, [budget, review, budget])
        assert first < numbers[0] < numbers[1] < numbers[2]
        async with mac(hub, paired) as (_, latest):
            assert latest == numbers[-1]


async def test_since_names_each_meeting_changed_after_it_once(environment):
    settings, store, adapter = environment
    paired = device(store)
    budget, review, launch = meetings(3)
    async with running(settings, store, adapter) as hub, mac(hub, paired) as (ws, start):
        first, second, third, fourth = await announced(
            hub, ws, paired, [budget, review, budget, launch]
        )
        everything = await since(hub, paired, start)
        assert everything["latest"] == fourth
        assert sorted(everything["recordings"]) == sorted([budget, review, launch])
        assert sorted((await since(hub, paired, first))["recordings"]) == sorted(
            [review, budget, launch]
        )
        assert sorted((await since(hub, paired, second))["recordings"]) == sorted([budget, launch])
        assert (await since(hub, paired, third))["recordings"] == [launch]


async def test_nothing_changed_is_an_empty_list_never_unknown(environment):
    settings, store, adapter = environment
    paired = device(store)
    async with running(settings, store, adapter) as hub, mac(hub, paired) as (ws, start):
        assert await since(hub, paired, start) == {"latest": start, "recordings": []}
        [number] = await announced(hub, ws, paired, meetings(1))
        assert await since(hub, paired, number) == {"latest": number, "recordings": []}


async def test_after_a_restart_every_earlier_number_is_unknown_also_once_new_ones_pass_it(
    environment,
):
    settings, store, adapter = environment
    paired = device(store)
    async with running(settings, store, adapter) as hub, mac(hub, paired) as (ws, start):
        [before] = await announced(hub, ws, paired, meetings(1))
    store.close()
    reopened = Store(settings.data_dir, settings.key_file)
    try:
        async with running(settings, reopened, adapter) as hub, mac(hub, paired) as (ws, again):
            for old in (start, before):
                assert (await since(hub, paired, old))["recordings"] is None, old
            later = meetings(3)
            numbers = await announced(hub, ws, paired, later)
            assert before not in numbers
            for old in (start, before):
                assert (await since(hub, paired, old))["recordings"] is None, old
            assert sorted((await since(hub, paired, again))["recordings"]) == sorted(later)
    finally:
        reopened.close()


async def test_a_number_this_hub_never_gave_is_unknown(environment):
    """Above the latest (another hub's, or one made up) or below this start's."""
    settings, store, adapter = environment
    paired = device(store)
    async with running(settings, store, adapter) as hub, mac(hub, paired) as (ws, start):
        [number] = await announced(hub, ws, paired, meetings(1))
        unknown = [number + 1, number + 10**6] + ([start - 1] if start > 0 else [])
        for other in unknown:
            assert await since(hub, paired, other) == {"latest": number, "recordings": None}, other


async def test_more_changed_meetings_than_the_ceiling_is_unknown_and_the_longest_answer_fits(
    environment,
):
    """The answer is bounded so that it always fits in what the Mac reads of
    one hub answer; past the ceiling the Mac reads the list, as before."""
    from orbit_hub.runtime.recording_changes import MAX_CHANGED_RECORDINGS

    body_bound = next(
        entry["value"]
        for entry in json.loads(CONSTANTS.read_text(encoding="utf-8"))
        if entry["name"] == "MAX_HUB_HTTP_BODY_BYTES"
    )
    settings, store, adapter = environment
    paired = device(store)
    many = meetings(MAX_CHANGED_RECORDINGS + 1)
    async with running(settings, store, adapter) as hub, mac(hub, paired) as (ws, start):
        numbers = await announced(hub, ws, paired, many[:-1])
        longest = await hub.http.get(
            CHANGES_PATH, params={"since": str(start)}, headers=bearer(paired)
        )
        assert longest.status_code == 200, longest.text
        assert sorted(longest.json()["recordings"]) == sorted(many[:-1])
        assert len(longest.content) <= body_bound
        numbers += await announced(hub, ws, paired, many[-1:])
        assert (await since(hub, paired, start))["recordings"] is None
        assert sorted((await since(hub, paired, numbers[0]))["recordings"]) == sorted(many[1:])


async def test_noting_changes_writes_nothing_to_the_database(environment):
    """An earlier hub then finds the database as it left it, and going back to
    one keeps what was written since (`backup.activation_plan` keeps a
    database whose schema did not change). Passes today on purpose."""
    settings, store, adapter = environment
    paired = device(store)
    async with (
        running(settings, store, adapter) as hub,
        connect(hub.ws_url, additional_headers=bearer(paired)) as ws,
    ):
        assert json.loads(await wait_for(ws.recv(), "ready"))["event"] == "ready"
        before = dump(store)
        changed = meetings(3)
        for recording in changed:
            hub.announce(paired["device_id"], recording)
        await notices(ws, changed)
        await hub.http.get(CHANGES_PATH, params={"since": "1"}, headers=bearer(paired))
        assert dump(store) == before


async def test_since_needs_a_device_token_and_a_whole_number(environment):
    settings, store, adapter = environment
    paired = device(store)
    async with running(settings, store, adapter) as hub:
        anonymous = await hub.http.get(CHANGES_PATH, params={"since": "0"})
        assert anonymous.status_code == 401, anonymous.text
        for wrong in (None, "", "-1", "uno", "1.5"):
            params = {} if wrong is None else {"since": wrong}
            answer = await hub.http.get(CHANGES_PATH, params=params, headers=bearer(paired))
            assert answer.status_code == 400, (wrong, answer.text)
            assert answer.json()["code"] == "invalid_parameters", answer.text
            assert answer.json()["refusal"]["field"] == "since", answer.text


async def test_a_meeting_renamed_through_its_route_is_in_the_log(environment):
    settings, store, adapter = environment
    paired = device(store)
    recording = meeting(store, paired["device_id"], texts=TEXTS)
    async with running(settings, store, adapter) as hub, mac(hub, paired) as (ws, start):
        answer = await hub.http.put(
            f"/v1/recordings/{recording}/title",
            json={"title": "Revisión del presupuesto"},
            headers=bearer(paired),
        )
        assert answer.status_code == 200, answer.text
        [notice] = await notices(ws, [recording])
        assert type(notice["change"]) is int, notice
        body = await since(hub, paired, start)
        assert body["recordings"] == [recording]
        assert body["latest"] >= notice["change"]
