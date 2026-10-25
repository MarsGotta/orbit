"""Each agent's personality, skills and access groups, kept in the hub's database.

Access groups are the hub's for every kind of agent. Personality and skills
are the hub's for `openai` and official vendor agents (`StoredAdmin`); a `hermes`
agent's live in its Hermes profile and are reached through Hermes's panel
(`hermes_panel`). Personality and skill files are encrypted like the rest of
the hub's content; the skill names and the access switches are not content.

Tables are created if missing and never altered: a hub from before this
reads every agent as without personality, without skills and with every
group on.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import TYPE_CHECKING, Protocol

from ..protocol import HubError
from . import access
from .skills import MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS, MAX_SKILLS, Skill, SkillEntry

if TYPE_CHECKING:
    from ..storage import Store

# One personality. What it protects: the model's context, which an `openai`
# agent is sent on every turn beside the conversation (16 KiB is about 4,000
# tokens, an eighth of a 32,000-token window), and what an official vendor agent is
# told at each session. Why 16 KiB: pages of Markdown, far more
# than a character and its manners take; procedures belong in skills, which
# have their own bound. It does not bound skills or the conversation.
MAX_PERSONALITY_BYTES = 16 * 1024

SCHEMA = """
CREATE TABLE IF NOT EXISTS agent_personalities (
    agent_id TEXT PRIMARY KEY, body BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS agent_skills (
    agent_id TEXT NOT NULL, name TEXT NOT NULL, enabled INTEGER NOT NULL, body BLOB NOT NULL,
    PRIMARY KEY(agent_id,name));
CREATE TABLE IF NOT EXISTS agent_access (
    agent_id TEXT PRIMARY KEY, groups TEXT NOT NULL);
"""


@dataclass(frozen=True)
class AgentInstructions:
    """What an `openai` or official vendor agent is told at a turn or a session: its
    personality and its enabled skills."""

    personality: str
    skills: tuple[Skill, ...]


NO_INSTRUCTIONS = AgentInstructions("", ())


def no_instructions() -> AgentInstructions:
    return NO_INSTRUCTIONS


# orbit-mcp's tool that hands an agent one of its enabled skills'
# instructions, for every kind whose skills the hub keeps (`openai`, official vendor).
SKILL_TOOL = "orbit_skill"
SKILLS_INSTRUCTIONS = (
    "Your skills (agentskills.io). Each line is a skill's name and when to use it. "
    f"When a request matches one, call {SKILL_TOOL} with its name to load its "
    "instructions, and follow them:"
)
# The longest `instructions_text` can be: the personality at its bound, the
# preface, and `MAX_SKILLS` lines of `- name: description` at the standard's
# maxima, four bytes a character. This describes the largest instruction
# payload given to an SDK or app-server, independent of conversation history.
# Personality and skill bounds provide the actual admission limits.
MAX_INSTRUCTIONS_TEXT_BYTES = (
    MAX_PERSONALITY_BYTES
    + len("\n\n")
    + len(SKILLS_INSTRUCTIONS.encode("utf-8"))
    + MAX_SKILLS * len("\n- : ")
    + MAX_SKILLS * (MAX_NAME_CHARS + 4 * MAX_DESCRIPTION_CHARS)
)


def instructions_text(current: AgentInstructions) -> str | None:
    """What an agent is told of its personality and skills, or None when it
    has neither: the personality, then each enabled skill by name and
    description, to be loaded with `SKILL_TOOL`. Every kind the hub keeps
    them for is told this same text: an `openai` agent at every turn, Claude
    Code in its session's system prompt, Codex in its developer instructions."""
    parts = []
    if current.personality.strip():
        parts.append(current.personality)
    if current.skills:
        listing = "\n".join(f"- {skill.name}: {skill.description}" for skill in current.skills)
        parts.append(f"{SKILLS_INSTRUCTIONS}\n{listing}")
    return "\n\n".join(parts) or None


def personality_refusal(text: str) -> HubError | None:
    size = len(text.encode("utf-8"))
    if size <= MAX_PERSONALITY_BYTES:
        return None
    return HubError(
        "personality_too_large",
        f"La personalidad ocupa {size} bytes; máximo {MAX_PERSONALITY_BYTES}",
        http_status=413,
        numbers={"bytes": size, "max_bytes": MAX_PERSONALITY_BYTES},
    )


def too_many(count: int) -> HubError:
    return HubError(
        "too_many_skills",
        f"El agente ya tiene {count} skills; máximo {MAX_SKILLS}. Borra alguna antes de crear otra",
        http_status=409,
        numbers={"count": count, "max": MAX_SKILLS},
    )


def not_found(name: str) -> HubError:
    return HubError(
        "skill_not_found",
        f"El agente no tiene ninguna skill llamada «{name}»",
        http_status=404,
        numbers={"name": name},
    )


class AgentConfigStore:
    """The hub's rows for agents' personality, skills and access."""

    def __init__(self, store: Store):
        self.store = store

    # Access, for every kind.

    def access(self, agent_id: str) -> dict[str, bool]:
        with self.store.read_snapshot() as db:
            row = db.execute(
                "SELECT groups FROM agent_access WHERE agent_id=?", (agent_id,)
            ).fetchone()
        groups = access.default_groups()
        if row is not None:
            stored = json.loads(row["groups"])
            # A group added after the row was written reads as its default, on.
            groups.update({group: stored[group] for group in access.GROUPS if group in stored})
        return groups

    def set_access(self, agent_id: str, groups: dict[str, bool]) -> None:
        value = json.dumps({group: groups[group] for group in access.GROUPS})
        with self.store.transaction() as db:
            db.execute(
                "INSERT INTO agent_access(agent_id,groups) VALUES(?,?)"
                " ON CONFLICT(agent_id) DO UPDATE SET groups=excluded.groups",
                (agent_id, value),
            )

    # Personality and skills, for the kinds the hub keeps them for.

    def personality(self, agent_id: str) -> str:
        with self.store.read_snapshot() as db:
            row = db.execute(
                "SELECT body FROM agent_personalities WHERE agent_id=?", (agent_id,)
            ).fetchone()
        if row is None:
            return ""
        return self.store.decrypt(row["body"], _personality_context(agent_id)).decode("utf-8")

    def set_personality(self, agent_id: str, text: str) -> None:
        with self.store.transaction() as db:
            if not text:
                db.execute("DELETE FROM agent_personalities WHERE agent_id=?", (agent_id,))
                return
            data = text.encode("utf-8")
            self.store.ensure_capacity(len(data))
            db.execute(
                "INSERT INTO agent_personalities(agent_id,body) VALUES(?,?)"
                " ON CONFLICT(agent_id) DO UPDATE SET body=excluded.body",
                (agent_id, self.store.encrypt(data, _personality_context(agent_id))),
            )

    def skills(self, agent_id: str) -> list[tuple[Skill, bool]]:
        """Every skill of the agent, by name, and whether it is enabled."""
        with self.store.read_snapshot() as db:
            rows = db.execute(
                "SELECT name,enabled,body FROM agent_skills WHERE agent_id=? ORDER BY name",
                (agent_id,),
            ).fetchall()
        return [(self._skill(agent_id, row), bool(row["enabled"])) for row in rows]

    def skill(self, agent_id: str, name: str) -> tuple[Skill, bool]:
        with self.store.read_snapshot() as db:
            row = db.execute(
                "SELECT name,enabled,body FROM agent_skills WHERE agent_id=? AND name=?",
                (agent_id, name),
            ).fetchone()
        if row is None:
            raise not_found(name)
        return self._skill(agent_id, row), bool(row["enabled"])

    def put_skill(self, agent_id: str, skill: Skill) -> None:
        """Create the skill, enabled, or replace its file keeping whether it is enabled."""
        body = json.dumps({"description": skill.description, "content": skill.content}).encode()
        with self.store.transaction() as db:
            exists = db.execute(
                "SELECT 1 FROM agent_skills WHERE agent_id=? AND name=?", (agent_id, skill.name)
            ).fetchone()
            if exists is None:
                (count,) = db.execute(
                    "SELECT COUNT(*) FROM agent_skills WHERE agent_id=?", (agent_id,)
                ).fetchone()
                if count >= MAX_SKILLS:
                    raise too_many(count)
            self.store.ensure_capacity(len(body))
            db.execute(
                "INSERT INTO agent_skills(agent_id,name,enabled,body) VALUES(?,?,1,?)"
                " ON CONFLICT(agent_id,name) DO UPDATE SET body=excluded.body",
                (
                    agent_id,
                    skill.name,
                    self.store.encrypt(body, _skill_context(agent_id, skill.name)),
                ),
            )

    def set_skill_enabled(self, agent_id: str, name: str, enabled: bool) -> None:
        with self.store.transaction() as db:
            changed = db.execute(
                "UPDATE agent_skills SET enabled=? WHERE agent_id=? AND name=?",
                (int(enabled), agent_id, name),
            ).rowcount
            if not changed:
                raise not_found(name)

    def delete_skill(self, agent_id: str, name: str) -> None:
        with self.store.transaction() as db:
            deleted = db.execute(
                "DELETE FROM agent_skills WHERE agent_id=? AND name=?", (agent_id, name)
            ).rowcount
            if not deleted:
                raise not_found(name)

    def instructions(self, agent_id: str) -> AgentInstructions:
        return AgentInstructions(
            self.personality(agent_id),
            tuple(skill for skill, enabled in self.skills(agent_id) if enabled),
        )

    def _skill(self, agent_id: str, row) -> Skill:
        stored = json.loads(self.store.decrypt(row["body"], _skill_context(agent_id, row["name"])))
        return Skill(row["name"], stored["description"], stored["content"])


def _personality_context(agent_id: str) -> str:
    return f"agent-personality:{agent_id}"


def _skill_context(agent_id: str, name: str) -> str:
    return f"agent-skill:{agent_id}:{name}"


class AgentAdmin(Protocol):
    """Where an agent's personality and skills live, as Orbit reads and changes them.

    Values reaching it are already validated (`skills.parse`, the personality's
    size). Refusals are `HubError`s with their cause and numbers.
    """

    async def personality(self) -> str: ...

    async def set_personality(self, text: str) -> None: ...

    async def skills(self) -> list[SkillEntry]: ...

    async def skill(self, name: str) -> tuple[SkillEntry, str]:
        """The skill's entry and its whole SKILL.md."""

    async def put_skill(self, skill: Skill) -> None:
        """Create the skill, enabled, or replace its SKILL.md."""

    async def set_skill_enabled(self, name: str, enabled: bool) -> None: ...

    async def delete_skill(self, name: str) -> None: ...


class StoredAdmin:
    """An `openai` or official vendor agent's personality and skills, in the hub's database."""

    def __init__(self, configs: AgentConfigStore, agent_id: str):
        self.configs, self.agent_id = configs, agent_id

    async def personality(self) -> str:
        return self.configs.personality(self.agent_id)

    async def set_personality(self, text: str) -> None:
        self.configs.set_personality(self.agent_id, text)

    async def skills(self) -> list[SkillEntry]:
        return [
            SkillEntry(skill.name, skill.description, enabled)
            for skill, enabled in self.configs.skills(self.agent_id)
        ]

    async def skill(self, name: str) -> tuple[SkillEntry, str]:
        skill, enabled = self.configs.skill(self.agent_id, name)
        return SkillEntry(skill.name, skill.description, enabled), skill.content

    async def put_skill(self, skill: Skill) -> None:
        self.configs.put_skill(self.agent_id, skill)

    async def set_skill_enabled(self, name: str, enabled: bool) -> None:
        self.configs.set_skill_enabled(self.agent_id, name, enabled)

    async def delete_skill(self, name: str) -> None:
        self.configs.delete_skill(self.agent_id, name)
