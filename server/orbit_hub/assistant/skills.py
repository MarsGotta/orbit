"""Skills in the agentskills.io format (`SKILL.md`), validated by Orbit's own rules.

A skill is one Markdown file: a YAML frontmatter between `---` lines with at
least `name` and `description`, then the instructions. The standard's rules
on those two fields are few, so they are checked here on PyYAML rather than
with `skills-ref`, which says of itself it is not meant for production. Other
frontmatter keys (`license`, `allowed-tools`, `metadata`, Hermes's or Claude
Code's own) are kept as written and not judged: each agent reads its own.

Each refusal names the rule that fired (`skill_invalid {name, rule}`), and the
length rules their numbers.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

import yaml

from ..protocol import HubError

# One SKILL.md. What it protects: the hub's row and the HTTP bodies that carry
# it, the agent's folder, and the model's context when an `openai` agent loads
# the skill (64 KiB is about 16,000 tokens). Why 64 KiB: the standard asks
# for a SKILL.md under 500 lines; the longest of the 58 skills Hermes ships
# measured 35,175 bytes (2026-10-22), so this leaves it room. It does not
# bound a personality, nor how many skills an agent has.
MAX_SKILL_BYTES = 64 * 1024
# Skills of one agent. What it protects: the configuration read the Mac takes
# in one response (128 names and descriptions at the standard's maxima are
# about 140 KiB of ASCII and under 560 KiB even at four bytes a character,
# below the Mac's 1 MiB reader), and, for an `openai` agent, the list of enabled
# skills its instructions carry on every turn. Why 128: Hermes ships 58, and
# twice that leaves room for an operator's own. Not a bound on a skill's size.
MAX_SKILLS = 128
# The standard's own bounds on the two fields it defines.
MAX_NAME_CHARS = 64
MAX_DESCRIPTION_CHARS = 1024
# The standard admits lowercase letters, digits and hyphens. Orbit takes the
# ASCII ones only: the name is a folder in an agent's directory and a segment
# of the hub's URL, and must be the same on every system.
NAME_CHARACTERS = re.compile(r"[a-z0-9-]+")


@dataclass(frozen=True)
class Skill:
    """A validated SKILL.md: its name, its description and the whole file."""

    name: str
    description: str
    content: str

    @property
    def body(self) -> str:
        """The instructions after the frontmatter, as a model reads them."""
        return _split(self.content, self.name)[1].lstrip("\n")


@dataclass(frozen=True)
class SkillEntry:
    """A skill as an agent's library lists it."""

    name: str
    description: str
    enabled: bool

    def public(self) -> dict:
        return {"name": self.name, "description": self.description, "enabled": self.enabled}


def invalid(name: str, rule: str, message: str, **numbers: int) -> HubError:
    return HubError(
        "skill_invalid",
        f"Skill «{name}» does not meet the SKILL.md standard: {message}",
        http_status=422,
        numbers={"name": name, "rule": rule, **numbers},
    )


def check_name(name: str, *, shown: str | None = None) -> None:
    """The standard's rules on a skill's name, in the order a person fixes them."""
    shown = name if shown is None else shown
    if len(name) > MAX_NAME_CHARS:
        raise invalid(
            shown,
            "name_too_long",
            f"the name has {len(name)} characters; maximum {MAX_NAME_CHARS}",
            characters=len(name),
            max_characters=MAX_NAME_CHARS,
        )
    if NAME_CHARACTERS.fullmatch(name) is None:
        raise invalid(
            shown,
            "name_characters",
            "the name allows only lowercase a-z, digits and hyphens",
        )
    if name.startswith("-") or name.endswith("-"):
        raise invalid(shown, "name_hyphen_edge", "the name cannot start or end with a hyphen")
    if "--" in name:
        raise invalid(
            shown, "name_consecutive_hyphens", "the name cannot contain consecutive hyphens"
        )


def size_refusal(name: str, content: str) -> HubError | None:
    size = len(content.encode("utf-8"))
    if size <= MAX_SKILL_BYTES:
        return None
    return HubError(
        "skill_too_large",
        f"Skill «{name}» is {size} bytes; maximum {MAX_SKILL_BYTES}",
        http_status=413,
        numbers={"name": name, "bytes": size, "max_bytes": MAX_SKILL_BYTES},
    )


def parse(name: str, content: str) -> Skill:
    """`content` as the SKILL.md of the skill called `name`, or the rule it breaks."""
    check_name(name)
    if (refusal := size_refusal(name, content)) is not None:
        raise refusal
    frontmatter, _body = _split(content, name)
    try:
        fields = yaml.safe_load(frontmatter)
    except yaml.YAMLError:
        raise invalid(
            name, "frontmatter_yaml", "the header between --- lines is not valid YAML"
        ) from None
    if not isinstance(fields, dict):
        raise invalid(
            name, "frontmatter_yaml", "the header between --- lines is not a YAML mapping"
        )
    declared = fields.get("name")
    if not isinstance(declared, str) or not declared.strip():
        raise invalid(name, "name_missing", "the name field is missing from the header")
    check_name(declared, shown=name)
    if declared != name:
        raise invalid(
            name,
            "name_mismatch",
            f"the header says name: {declared}, and the skill is named {name}",
        )
    description = fields.get("description")
    if not isinstance(description, str) or not description.strip():
        raise invalid(
            name, "description_missing", "the description field is missing from the header"
        )
    if len(description) > MAX_DESCRIPTION_CHARS:
        raise invalid(
            name,
            "description_too_long",
            f"the description has {len(description)} characters; maximum {MAX_DESCRIPTION_CHARS}",
            characters=len(description),
            max_characters=MAX_DESCRIPTION_CHARS,
        )
    return Skill(name, description, content)


def _split(content: str, name: str) -> tuple[str, str]:
    """The frontmatter's text and the body after it."""
    lines = content.removeprefix("\ufeff").split("\n")
    if not lines or lines[0].rstrip("\r ") != "---":
        raise invalid(name, "frontmatter_missing", "the file must start with a --- line")
    for index, line in enumerate(lines[1:], 1):
        if line.rstrip("\r ") == "---":
            return "\n".join(lines[1:index]), "\n".join(lines[index + 1 :])
    raise invalid(name, "frontmatter_unclosed", "the closing --- header line is missing")
