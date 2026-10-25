"""What the hub keeps of each agent beyond its configuration: its profile,
which one is the primary, the agents added from Orbit, and what the hub last
learned of each one's account.

The profile is how Orbit shows and calls an agent: its name (what «Oye
{nombre}» hears), a description, a colour and whether it is the primary, the
one that answers when no agent is named. It is stored over what `orbit.toml`
says and changes nothing of the agent itself: renaming a Hermes agent never
touches Hermes, and an `openai` or official vendor agent is still told its personality,
not its name.

Agents added from Orbit use the official Claude Code or Codex tool. The Mac
chooses only the service, the profile and the access groups; the command that
runs comes from the hub's `[assistant_tools]` table, never from the Mac, and the
id and folder are the hub's own. They live here beside the ones `orbit.toml`
lists; those cannot be removed from Orbit.

Tables are created if missing and never altered: a hub from before this has
no row and reads every agent as `orbit.toml` describes it.
"""

from __future__ import annotations

import unicodedata
from dataclasses import dataclass
from typing import TYPE_CHECKING, Literal

from ..errors import HubError

if TYPE_CHECKING:
    from ..storage import Store

# The four colours of an agent's avatar (the app's `--c1`…`--c4` tokens).
COLORS = ("c1", "c2", "c3", "c4")
# A name set from Orbit. What it protects: the line it shares with the
# service and a notice in «Tus asistentes» (272 px) and the wake phrase «Oye
# {nombre}», which macOS hears as words. Why 24: the design's bound for the
# name field. `orbit.toml` keeps its own, 48, for names an operator writes;
# such a name stays as written until someone renames it in Orbit.
MAX_PROFILE_NAME_CHARS = 24
# One line under the name when choosing whom to talk to; the design's bound.
MAX_DESCRIPTION_CHARS = 90
# How many agents one hub offers, `orbit.toml`'s and the added ones together.
# What it protects: every Mac's status topic, which carries the whole list
# (orbit-core `agents::MAX_AGENTS`, guarded in contracts/constants.json), and
# the size of `ready`. It does not bound conversations or processes.
MAX_AGENTS = 32

Service = Literal["claude-code", "codex"]
# The account states the hub publishes for an agent it can sign in. Absent
# from an agent Orbit cannot sign in (Hermes, `openai`).
AccountState = Literal["signed_in", "signed_out", "expired", "unknown"]

SCHEMA = """
CREATE TABLE IF NOT EXISTS agent_profiles (
    agent_id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL,
    color TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS agent_primary (
    slot INTEGER PRIMARY KEY CHECK(slot=1), agent_id TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS added_agents (
    agent_id TEXT PRIMARY KEY, service TEXT NOT NULL, added_at_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS agent_accounts (
    agent_id TEXT PRIMARY KEY, state TEXT NOT NULL, limited INTEGER NOT NULL,
    limited_until_ms INTEGER, email TEXT, plan TEXT);
"""


@dataclass(frozen=True)
class Profile:
    name: str
    description: str
    color: str


@dataclass(frozen=True)
class Added:
    agent_id: str
    service: Service


@dataclass(frozen=True)
class Account:
    state: AccountState
    # The agent reached its plan's limit; `until_ms` is when it resets, when
    # the provider said.
    limited: bool = False
    until_ms: int | None = None
    # What the agent's official tool says of the account it runs with
    # (`_auth/status_update`): the account's address and the plan's name.
    email: str | None = None
    plan: str | None = None


UNKNOWN_ACCOUNT = Account("unknown")


def _has_control(text: str) -> bool:
    return any(unicodedata.category(character) in {"Cc", "Cf"} for character in text)


def profile_refusal(name: str, description: str, color: str) -> HubError | None:
    """Why a profile cannot be stored, with the field, its count and the ceiling.
    Uniqueness is the registry's (`Agents.set_profile`)."""
    if not name:
        return _invalid("name", "empty", "Ponle un nombre: es como le llamas", 0, 1)
    if len(name) > MAX_PROFILE_NAME_CHARS or _has_control(name):
        cause = "too_long" if len(name) > MAX_PROFILE_NAME_CHARS else "control_characters"
        return _invalid(
            "name",
            cause,
            f"El nombre tiene {len(name)} caracteres; máximo {MAX_PROFILE_NAME_CHARS}, "
            "sin caracteres de control",
            len(name),
            MAX_PROFILE_NAME_CHARS,
        )
    if len(description) > MAX_DESCRIPTION_CHARS or _has_control(description):
        cause = "too_long" if len(description) > MAX_DESCRIPTION_CHARS else "control_characters"
        return _invalid(
            "description",
            cause,
            f"La descripción tiene {len(description)} caracteres; máximo "
            f"{MAX_DESCRIPTION_CHARS}, en una línea",
            len(description),
            MAX_DESCRIPTION_CHARS,
        )
    if color not in COLORS:
        return HubError(
            "agent_profile_invalid",
            f"El color tiene que ser uno de {', '.join(COLORS)}",
            http_status=422,
            numbers={"field": "color", "rule": "unknown_color"},
        )
    return None


def _invalid(field: str, rule: str, message: str, characters: int, ceiling: int) -> HubError:
    return HubError(
        "agent_profile_invalid",
        message,
        http_status=422,
        numbers={
            "field": field,
            "rule": rule,
            "characters": characters,
            "max_characters": ceiling,
        },
    )


class Roster:
    """The hub's rows for profiles, the primary, added agents and accounts."""

    def __init__(self, store: Store):
        self.store = store

    def profiles(self) -> dict[str, Profile]:
        with self.store.read_snapshot() as db:
            rows = db.execute(
                "SELECT agent_id,name,description,color FROM agent_profiles"
            ).fetchall()
        return {row["agent_id"]: Profile(row["name"], row["description"], row["color"]) for row in rows}  # fmt: skip

    def primary(self) -> str | None:
        with self.store.read_snapshot() as db:
            row = db.execute("SELECT agent_id FROM agent_primary WHERE slot=1").fetchone()
        return None if row is None else row["agent_id"]

    def save_profile(self, agent_id: str, profile: Profile, primary: bool) -> None:
        with self.store.transaction() as db:
            db.execute(
                "INSERT INTO agent_profiles(agent_id,name,description,color) VALUES(?,?,?,?)"
                " ON CONFLICT(agent_id) DO UPDATE SET name=excluded.name,"
                " description=excluded.description, color=excluded.color",
                (agent_id, profile.name, profile.description, profile.color),
            )
            if primary:
                db.execute(
                    "INSERT INTO agent_primary(slot,agent_id) VALUES(1,?)"
                    " ON CONFLICT(slot) DO UPDATE SET agent_id=excluded.agent_id",
                    (agent_id,),
                )

    def added(self) -> list[Added]:
        """The agents added from Orbit, the oldest first."""
        with self.store.read_snapshot() as db:
            rows = db.execute(
                "SELECT agent_id,service FROM added_agents ORDER BY added_at_ms, agent_id"
            ).fetchall()
        return [Added(row["agent_id"], row["service"]) for row in rows]

    def add(self, added: Added, profile: Profile, now_ms: int) -> None:
        with self.store.transaction() as db:
            db.execute(
                "INSERT INTO added_agents(agent_id,service,added_at_ms) VALUES(?,?,?)",
                (added.agent_id, added.service, now_ms),
            )
            db.execute(
                "INSERT INTO agent_profiles(agent_id,name,description,color) VALUES(?,?,?,?)",
                (added.agent_id, profile.name, profile.description, profile.color),
            )
            db.execute(
                "INSERT INTO agent_accounts(agent_id,state,limited,limited_until_ms)"
                " VALUES(?,'signed_out',0,NULL)",
                (added.agent_id,),
            )

    def remove(self, agent_id: str) -> None:
        """Forget an added agent and everything the hub kept for it but its
        conversations, which stay with its id."""
        with self.store.transaction() as db:
            for table in (
                "added_agents",
                "agent_profiles",
                "agent_accounts",
                "agent_access",
                "agent_personalities",
                "agent_skills",
            ):
                db.execute(f"DELETE FROM {table} WHERE agent_id=?", (agent_id,))
            db.execute("DELETE FROM agent_primary WHERE agent_id=?", (agent_id,))

    def accounts(self) -> dict[str, Account]:
        with self.store.read_snapshot() as db:
            rows = db.execute(
                "SELECT agent_id,state,limited,limited_until_ms,email,plan FROM agent_accounts"
            ).fetchall()
        return {
            row["agent_id"]: Account(
                row["state"],
                bool(row["limited"]),
                row["limited_until_ms"],
                row["email"],
                row["plan"],
            )
            for row in rows
        }

    def save_account(self, agent_id: str, account: Account) -> None:
        with self.store.transaction() as db:
            db.execute(
                "INSERT INTO agent_accounts(agent_id,state,limited,limited_until_ms,email,plan)"
                " VALUES(?,?,?,?,?,?) ON CONFLICT(agent_id) DO UPDATE SET state=excluded.state,"
                " limited=excluded.limited, limited_until_ms=excluded.limited_until_ms,"
                " email=excluded.email, plan=excluded.plan",
                (
                    agent_id,
                    account.state,
                    int(account.limited),
                    account.until_ms,
                    account.email,
                    account.plan,
                ),
            )
