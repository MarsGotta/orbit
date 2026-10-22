"""SKILL.md as the agentskills.io standard defines it, checked by Orbit's own rules.

Each rule is refused on its own, naming the skill and the rule, with its
numbers when it is a length. Skill names and texts are invented.
"""

import pytest

from orbit_hub.assistant.skills import (
    MAX_DESCRIPTION_CHARS,
    MAX_NAME_CHARS,
    MAX_SKILL_BYTES,
    parse,
)
from orbit_hub.protocol import HubError


def skill_md(
    name="resumen-semanal",
    description="Resume la semana en cinco puntos.",
    body="Pasos:\n1. Lee.\n",
):
    return f"---\nname: {name}\ndescription: {description}\n---\n\n{body}"


def refusal(name, content):
    with pytest.raises(HubError) as raised:
        parse(name, content)
    return raised.value


def test_a_valid_skill_keeps_its_file_and_gives_its_body():
    skill = parse("resumen-semanal", skill_md(body="Pasos:\n1. Lee.\n"))
    assert (skill.name, skill.description) == (
        "resumen-semanal",
        "Resume la semana en cinco puntos.",
    )
    assert skill.content == skill_md()
    assert skill.body == "Pasos:\n1. Lee.\n"
    # Keys the standard leaves to each agent are kept as written, not judged.
    other = (
        "---\nname: x1\ndescription: d\nallowed-tools: Read\nmetadata:\n  autor: Ana\n---\ncuerpo"
    )
    assert parse("x1", other).content == other


@pytest.mark.parametrize(
    ("name", "content", "rule", "numbers"),
    [
        ("sin-cabecera", "Sólo instrucciones, sin cabecera.", "frontmatter_missing", {}),
        ("abierta", "---\nname: abierta\ndescription: d\n", "frontmatter_unclosed", {}),
        ("rota", "---\nname: [rota\n---\ncuerpo", "frontmatter_yaml", {}),
        ("lista", "---\n- name\n- description\n---\ncuerpo", "frontmatter_yaml", {}),
        ("anonima", "---\ndescription: d\n---\ncuerpo", "name_missing", {}),
        ("otra", skill_md(name="distinta"), "name_mismatch", {}),
        ("Mayusculas", skill_md(name="Mayusculas"), "name_characters", {}),
        ("con_guion_bajo", skill_md(name="con_guion_bajo"), "name_characters", {}),
        ("acentuada-é", skill_md(name="acentuada-é"), "name_characters", {}),
        ("-borde", skill_md(name="-borde"), "name_hyphen_edge", {}),
        ("borde-", skill_md(name="borde-"), "name_hyphen_edge", {}),
        ("doble--guion", skill_md(name="doble--guion"), "name_consecutive_hyphens", {}),
        (
            "n" * (MAX_NAME_CHARS + 1),
            skill_md(name="n" * (MAX_NAME_CHARS + 1)),
            "name_too_long",
            {"characters": MAX_NAME_CHARS + 1, "max_characters": MAX_NAME_CHARS},
        ),
        ("sin-descripcion", "---\nname: sin-descripcion\n---\ncuerpo", "description_missing", {}),
        (
            "descripcion-vacia",
            skill_md(name="descripcion-vacia", description='"  "'),
            "description_missing",
            {},
        ),
        (
            "descripcion-larga",
            skill_md(name="descripcion-larga", description="d" * (MAX_DESCRIPTION_CHARS + 1)),
            "description_too_long",
            {"characters": MAX_DESCRIPTION_CHARS + 1, "max_characters": MAX_DESCRIPTION_CHARS},
        ),
    ],
)
def test_each_rule_is_refused_by_name_with_its_numbers(name, content, rule, numbers):
    refused = refusal(name, content)
    assert refused.code == "skill_invalid"
    assert refused.numbers == {"name": name, "rule": rule, **numbers}
    assert all(isinstance(value, str | int) for value in refused.numbers.values())
    assert refused.http_status == 422


def test_a_name_that_would_leave_a_folder_breaks_the_name_rules():
    for name in ("..", "../fuera", "a/b", "."):
        assert refusal(name, skill_md(name=name)).numbers["rule"] == "name_characters"


def test_a_skill_over_the_ceiling_is_refused_with_its_bytes():
    content = skill_md(body="é" * (MAX_SKILL_BYTES // 2))
    refused = refusal("resumen-semanal", content)
    assert refused.code == "skill_too_large"
    assert refused.numbers == {
        "name": "resumen-semanal",
        "bytes": len(content.encode("utf-8")),
        "max_bytes": MAX_SKILL_BYTES,
    }
    assert refused.http_status == 413
