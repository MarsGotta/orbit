"""Pure note generation: what the model is asked and what counts as a note."""

import pytest

from orbit_hub.meetings.turns import Segment, Turn, merge, speaker_key, speaker_name
from orbit_hub.notes.generation import (
    Line,
    answer_schema,
    build_note,
    lengths,
    note_messages,
    part_messages,
    reduce_messages,
    render,
    split,
    transcript_budget,
)
from orbit_hub.notes.models import EventContext, NoteInputs, action_id
from orbit_hub.notes.templates import BUILTIN_TEMPLATES, builtin
from orbit_hub.processing import ProcessingError

GENERAL = builtin("general")


def segment(at_ms, track, text, label="unknown"):
    return Segment(at_ms, track, speaker_key(track, label), text, 0, 0, "es")


def test_the_local_microphone_is_its_owner_and_renamed_voices_use_their_names():
    """The rule `meetings.turns` applies for every reader, the note's included."""
    renames = {"Speaker 2": "Lucía"}
    names = [
        speaker_name(item.speaker_key, item.track, renames, "Alina")
        for item in (
            segment(0, "mic_local", "Empezamos."),
            segment(20_000, "remote_app", "Vale.", "Speaker 2"),
            segment(40_000, "remote_app", "Sí.", "Speaker 3"),
            segment(60_000, "remote_app", "Hola."),
        )
    ]
    assert names == ["Alina", "Lucía", "Otra persona", "Otra persona"]
    assert speaker_name("mic_local", "mic_local", {}, None) == "Micrófono local"
    assert speaker_name("mic_local", "mic_local", {"mic_local": "Alina"}, "Alina") == "Alina"


def test_consecutive_turns_of_one_speaker_are_one_line_in_time_order():
    turns = merge(
        [
            segment(30_000, "mic_local", "segunda"),
            segment(0, "mic_local", "primera"),
            segment(90_000, "mic_local", "lejos"),
        ]
    )
    assert turns == [Turn(0, "mic_local", "primera segunda"), Turn(90_000, "mic_local", "lejos")]


def test_render_gives_clock_speaker_and_text():
    assert render([Line(3_725_000, "Alina", "hola")]) == "[01:02:05] Alina: hola"


def test_split_keeps_order_and_fits_each_part():
    lines = [Line(i * 1000, "Alina", "palabra " * 50) for i in range(40)]
    parts = split(lines, budget_tokens=500)
    assert len(parts) > 1
    assert [line for part in parts for line in part] == lines
    assert all(len(render(part)) / 3 <= 500 + 200 for part in parts)


def test_a_meeting_that_fits_the_window_is_one_part():
    lines = [Line(i * 1000, "Alina", "hola") for i in range(100)]
    budget = transcript_budget(131_072, 8192, NoteInputs())
    assert split(lines, budget) == [lines]


def test_private_notes_shrink_the_transcript_budget():
    plain = transcript_budget(131_072, 8192, NoteInputs())
    noted = transcript_budget(131_072, 8192, NoteInputs(private_notes="x" * 30_000))
    assert noted == plain - 10_000


def test_the_answer_schema_has_one_required_key_per_template_section():
    schema = answer_schema(GENERAL, partial=False)
    sections = schema["properties"]["sections"]
    assert sections["required"] == ["s1", "s2", "s3", "s4"]
    assert sections["additionalProperties"] is False
    assert schema["required"][:2] == ["title", "short_summary"]
    partial = answer_schema(GENERAL, partial=True)
    assert "title" not in partial["properties"]


def test_the_note_scales_with_the_meeting():
    assert lengths(10 * 60_000) == (2, 4)
    assert lengths(60 * 60_000) == (4, 10)
    assert lengths(240 * 60_000) == (6, 15)


def test_the_prompt_carries_template_private_notes_event_and_transcript():
    inputs = NoteInputs(
        owner_name="Alina",
        private_notes="- Ojo con el **presupuesto**",
        event=EventContext(title="Comité", attendees=[{"name": "Lucía"}, {"email": "a@b.es"}]),
    )
    system, user = note_messages(GENERAL, inputs, [Line(0, "Alina", "Hola")], 120 * 60_000)
    assert "Alina" in system["content"] and "español" in system["content"]
    text = user["content"]
    for expected in (
        "Título previsto: Comité",
        "Invitados: Lucía, a@b.es",
        "Duración: 120 minutos",
        "s1 → Resumen",
        "s4 → Próximos pasos",
        "Notas privadas de Alina",
        "Ojo con el **presupuesto**",
        "[00:00:00] Alina: Hola",
        "entre 6 y 15 viñetas",
    ):
        assert expected in text


@pytest.mark.parametrize(
    ("language", "named"),
    [("en", "inglés (English)"), ("fr", "francés (Français)"), ("es", "español")],
)
def test_the_language_is_the_last_thing_the_model_reads(language, named):
    """On the real model (2026-10-18): a meeting changed to English got
    its note rewritten in Spanish. The language was one phrase at the top of a
    prompt written in Spanish over a transcript half in Spanish; it now closes
    every request, after the transcript, named in both languages."""
    inputs = NoteInputs(owner_name="Alina", language=language)
    lines = [Line(0, "Alina", "Hola")]
    requests = [
        note_messages(GENERAL, inputs, lines, 60_000),
        part_messages(GENERAL, inputs, lines, 0, 2),
        reduce_messages(GENERAL, inputs, [{"sections": {}}], 60_000),
    ]
    for _system, user in requests:
        last = user["content"].rsplit("\n\n## ", 1)[-1]
        assert last.startswith("Idioma\n"), last
        assert f"Escribe toda la nota en {named}" in last, last


def answer(**changes):
    value = {
        "title": "Comité de octubre",
        "short_summary": "Se aprobó el presupuesto.",
        "sections": {"s1": "Resumen.", "s2": "- uno", "s3": "- aprobado", "s4": "- enviar acta"},
        "action_items": [
            {"owner": "Lucía", "text": "Enviar el acta", "due": "viernes"},
            {"owner": "lucía", "text": " Enviar  el acta", "due": ""},
            {"owner": "", "text": "Revisar cifras", "due": ""},
            {"owner": "Otra persona", "text": "Mirar la latencia", "due": ""},
            {"owner": "Alina", "text": "  ", "due": ""},
        ],
        "decisions": ["Aprobar el presupuesto", " "],
        "open_topics": ["Fecha del lanzamiento"],
    }
    value.update(changes)
    return value


def note(content, inputs=None):
    return build_note(
        content,
        GENERAL,
        inputs or NoteInputs(),
        model="qwen3.8-flash-next",
        generated_at_ms=5,
        segments=12,
        duration_ms=3_600_000,
        parts=1,
    )


def test_a_valid_answer_becomes_a_note_with_the_template_names():
    result = note(answer())
    assert [s.name for s in result.sections] == [
        "Resumen",
        "Puntos clave",
        "Decisiones",
        "Próximos pasos",
    ]
    assert result.sections[2].markdown == "- aprobado"
    assert result.decisions == ("Aprobar el presupuesto",)
    assert (result.template_id, result.template_name, result.parts) == ("general", "General", 1)


def test_tasks_are_deduplicated_get_stable_ids_and_an_unknown_owner_is_none():
    items = note(answer()).action_items
    assert [(i.owner, i.text, i.due) for i in items] == [
        ("Lucía", "Enviar el acta", "viernes"),
        (None, "Revisar cifras", None),
        (None, "Mirar la latencia", None),
    ]
    assert (
        items[0].id == action_id("Lucía", "Enviar el acta") == action_id("lucía", "enviar el  acta")
    )


def test_without_a_title_the_note_takes_the_event_title():
    result = note(answer(title=" "), NoteInputs(event=EventContext(title="Comité")))
    assert result.title == "Comité"


@pytest.mark.parametrize("broken", [{"sections": {"s1": "x"}}, {"action_items": None}])
def test_an_answer_that_is_not_a_note_names_what_failed(broken):
    with pytest.raises(ProcessingError) as caught:
        note(answer(**broken))
    assert caught.value.code == "note_schema_mismatch"
    assert caught.value.details["field"]


def test_every_builtin_template_has_sections_with_instructions():
    assert len({t.id for t in BUILTIN_TEMPLATES}) == len(BUILTIN_TEMPLATES) >= 5
    for template in BUILTIN_TEMPLATES:
        assert template.builtin and template.context
        assert all(section.instructions for section in template.sections)
