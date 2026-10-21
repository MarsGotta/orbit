"""REF-04 A7: the conversation list does not break by growing.

The hub keeps every conversation a Mac ever opened and each hub restart opens
new ones, so the list only grows. It used to be served whole, and the daemon
refuses a list longer than 128 entries whole, with no numbers. Here the route
serves the most recent ones under a ceiling and says how many there are, and
nothing is deleted to get there.
"""

from uuid import uuid4

from conftest import device
from support_app import client

# What the daemon reads (`crates/orbit-daemon/src/hub.rs`): pages of at most
# 128 entries, each with a non-empty id of at most 121 bytes and its times
# (SIM-30).
PUBLISHED_DAEMON_MAX_SESSIONS = 128
CONVERSATIONS = 500
FIRST_CREATED = 1_700_000_000.0


def _open_conversations(store, device_id: str, count: int, created_step: float = 1) -> list[str]:
    """The fixture models stored rows while paying one FULL fsync, not one per row."""
    opened = [str(uuid4()) for _ in range(count)]
    with store.transaction() as db:
        store.ensure_capacity(1024 * count)
        db.executemany(
            "INSERT INTO sessions VALUES (?,?,?,?,?,?,?)",
            (
                (
                    session_id,
                    device_id,
                    f"runtime-{index}",
                    f"stored-{index}",
                    "epoch",
                    "idle",
                    FIRST_CREATED + index * created_step,
                )
                for index, session_id in enumerate(opened)
            ),
        )
    return opened


def _session_rows(store, device_id: str) -> int:
    with store._lock:
        row = store._db.execute(
            "SELECT COUNT(*) AS n FROM sessions WHERE device_id=?", (device_id,)
        ).fetchone()
    return row["n"]


def test_a7_the_most_recent_conversations_come_with_the_total(environment):
    _, store, _ = environment
    paired = device(store)
    opened = _open_conversations(store, paired["device_id"], CONVERSATIONS)

    with client(environment) as http:
        response = http.get("/v1/sessions", headers={"Authorization": "Bearer " + paired["token"]})

    assert response.status_code == 200
    body = response.json()
    assert body["total"] == CONVERSATIONS
    listed = [session["id"] for session in body["sessions"]]
    assert listed == list(reversed(opened))[:PUBLISHED_DAEMON_MAX_SESSIONS], (
        "the page is the most recent conversations, newest first"
    )


def test_ref211_five_hundred_conversations_continue_by_cursor(environment):
    _, store, _ = environment
    paired = device(store)
    opened = _open_conversations(store, paired["device_id"], CONVERSATIONS)
    seen = []
    visited = set()
    before = ""
    with client(environment) as http:
        while True:
            response = http.get(
                "/v1/sessions",
                params={"before": before},
                headers={"Authorization": "Bearer " + paired["token"]},
            )
            assert response.status_code == 200
            body = response.json()
            assert body["total"] == CONVERSATIONS
            assert 0 < len(body["sessions"]) <= PUBLISHED_DAEMON_MAX_SESSIONS
            seen.extend(session["id"] for session in body["sessions"])
            before = body.get("next")
            if before is None:
                break
            assert before not in visited, "the cursor repeated instead of advancing"
            visited.add(before)
    assert seen == list(reversed(opened))


def test_ref211_cursor_respects_device_and_equal_creation_times(environment):
    _, store, _ = environment
    owner = device(store, "owner")
    other = device(store, "other")
    foreign = _open_conversations(store, other["device_id"], 1)[0]
    with client(environment) as http:
        refused = http.get(
            "/v1/sessions",
            params={"before": foreign},
            headers={"Authorization": "Bearer " + owner["token"]},
        )
    assert refused.status_code == 404
    assert refused.json()["code"] == "session_not_found"

    tied = _open_conversations(store, owner["device_id"], PUBLISHED_DAEMON_MAX_SESSIONS + 1, 0)
    with client(environment) as http:
        first = http.get(
            "/v1/sessions",
            params={"before": ""},
            headers={"Authorization": "Bearer " + owner["token"]},
        ).json()
        second = http.get(
            "/v1/sessions",
            params={"before": first["next"]},
            headers={"Authorization": "Bearer " + owner["token"]},
        ).json()
    assert [row["id"] for row in first["sessions"] + second["sessions"]] == sorted(tied)
    assert first["total"] == second["total"] == len(tied)
    assert second["next"] is None


def test_a7_the_page_is_one_a_published_daemon_accepts(environment):
    _, store, _ = environment
    paired = device(store)
    _open_conversations(store, paired["device_id"], CONVERSATIONS)

    with client(environment) as http:
        body = http.get(
            "/v1/sessions", headers={"Authorization": "Bearer " + paired["token"]}
        ).json()

    sessions = body["sessions"]
    assert len(sessions) <= PUBLISHED_DAEMON_MAX_SESSIONS
    for session in sessions:
        assert set(session) == {"id", "created_at_ms", "last_at_ms", "message_count"}
        assert 0 < len(session["id"].encode()) <= 121
        assert session["last_at_ms"] >= session["created_at_ms"] >= 0
        assert session["message_count"] == 0


def test_a7_listing_deletes_nothing(environment):
    _, store, _ = environment
    paired = device(store)
    _open_conversations(store, paired["device_id"], CONVERSATIONS)

    with client(environment) as http:
        http.get("/v1/sessions", headers={"Authorization": "Bearer " + paired["token"]})
        http.get("/v1/sessions", headers={"Authorization": "Bearer " + paired["token"]})

    assert _session_rows(store, paired["device_id"]) == CONVERSATIONS


def test_a7_a_short_list_is_whole_and_counts_only_this_mac(environment):
    _, store, _ = environment
    paired = device(store)
    other = device(store, "otro")
    opened = _open_conversations(store, paired["device_id"], 3)
    _open_conversations(store, other["device_id"], 5)

    with client(environment) as http:
        body = http.get(
            "/v1/sessions", headers={"Authorization": "Bearer " + paired["token"]}
        ).json()

    assert body["total"] == 3
    assert [session["id"] for session in body["sessions"]] == list(reversed(opened))
