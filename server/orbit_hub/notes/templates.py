"""Built-in note templates: a name, a context, and the ordered sections to write.

The wording is Orbit's own. A template tells the processing model what the
meeting is for and which sections the summary has; each section carries the
instruction that shapes it.
"""

from pydantic import BaseModel, ConfigDict, Field

# Bounds of a template, built-in or custom. They protect the request the model
# receives (every section becomes part of the prompt and of the answer schema)
# and the UI that lists them; a meeting summary with more than twelve sections
# stops being a summary.
MAX_SECTIONS = 12
MAX_NAME_CHARS = 80
MAX_CONTEXT_CHARS = 4000
MAX_INSTRUCTIONS_CHARS = 2000


class TemplateSection(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    name: str = Field(min_length=1, max_length=MAX_NAME_CHARS)
    instructions: str = Field(default="", max_length=MAX_INSTRUCTIONS_CHARS)


class Template(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    id: str = Field(min_length=1, max_length=64, pattern=r"^[a-z0-9][a-z0-9_-]*$")
    name: str = Field(min_length=1, max_length=MAX_NAME_CHARS)
    context: str = Field(default="", max_length=MAX_CONTEXT_CHARS)
    sections: tuple[TemplateSection, ...] = Field(min_length=1, max_length=MAX_SECTIONS)
    builtin: bool = False


def _builtin(id: str, name: str, context: str, *sections: tuple[str, str]) -> Template:
    return Template(
        id=id,
        name=name,
        context=context,
        sections=tuple(TemplateSection(name=n, instructions=i) for n, i in sections),
        builtin=True,
    )


DEFAULT_TEMPLATE_ID = "general"

BUILTIN_TEMPLATES: tuple[Template, ...] = (
    _builtin(
        "general",
        "General",
        "An ordinary work meeting. The reader was not there: explain what was discussed, what was decided and what needs to be done.",
        ("Resumen", "Two to four sentences stating the meeting's purpose and outcome."),
        ("Puntos clave", "The topics discussed, one per bullet, with the essentials of each."),
        ("Decisiones", "Only what was explicitly agreed. If nothing was agreed, say so."),
        ("Próximos pasos", "What comes next, who does it and when, if stated."),
    ),
    _builtin(
        "one_on_one",
        "Uno a uno",
        "A conversation between two people who work together regularly.",
        ("Cómo está", "Mood, workload and concerns, if discussed."),
        ("Temas tratados", "Each topic with what was said about it."),
        ("Acuerdos", "What the two people agreed."),
        ("Para la próxima", "What remains to revisit at the next one-on-one."),
    ),
    _builtin(
        "project",
        "Seguimiento de proyecto",
        "A recurring meeting about a project's progress.",
        ("Estado", "How the project is going, in one or two sentences."),
        ("Avances", "What was finished or unblocked since the last meeting."),
        ("Bloqueos y riesgos", "What is blocking or threatening the plan, with its cause."),
        ("Decisiones", "What was agreed at this meeting."),
        ("Próximos pasos", "Tasks with owners and deadlines when stated."),
    ),
    _builtin(
        "client",
        "Cliente",
        "A conversation with a client or prospective client.",
        ("Contexto", "Who they are, what they want and the relationship's current stage."),
        ("Necesidades", "What the client requested or indicated they need."),
        ("Objeciones y dudas", "What is holding the client back and how it was answered."),
        ("Acuerdos", "What was agreed: prices, deadlines, scope and next contacts."),
        ("Seguimiento", "What needs to be sent or done after the call."),
    ),
    _builtin(
        "interview",
        "Entrevista",
        "An interview with a candidate for a position.",
        ("Perfil", "The background and relevant experience they described."),
        ("Puntos fuertes", "Strengths they demonstrated with concrete examples."),
        ("Dudas", "What remained unclear or concerning."),
        ("Valoración", "Overall impression based only on what was said."),
        ("Siguiente paso", "What they were told about the process."),
    ),
    _builtin(
        "brainstorm",
        "Lluvia de ideas",
        "A session to generate and compare ideas.",
        ("Ideas", "Each idea proposed, with who proposed it if known."),
        ("Lo que gustó", "The ideas the group preferred and why."),
        ("Descartado", "What was discarded and why."),
        ("Preguntas abiertas", "What remains to be learned or decided."),
    ),
)


def builtin(template_id: str) -> Template | None:
    return next((t for t in BUILTIN_TEMPLATES if t.id == template_id), None)


_ENGLISH_NAMES = {
    "general": ("General", ("Summary", "Key points", "Decisions", "Next steps")),
    "one_on_one": (
        "One-on-one",
        ("How they are", "Topics discussed", "Agreements", "For next time"),
    ),
    "project": (
        "Project update",
        ("Status", "Progress", "Blockers and risks", "Decisions", "Next steps"),
    ),
    "client": (
        "Client",
        ("Context", "Needs", "Objections and questions", "Agreements", "Follow-up"),
    ),
    "interview": ("Interview", ("Profile", "Strengths", "Questions", "Assessment", "Next step")),
    "brainstorm": ("Brainstorm", ("Ideas", "What resonated", "Discarded", "Open questions")),
}


def for_meeting(template: Template, language: str) -> Template:
    """Built-in headings follow the meeting; custom templates remain user content."""
    if not template.builtin or not language.startswith("en") or template.id not in _ENGLISH_NAMES:
        return template
    name, headings = _ENGLISH_NAMES[template.id]
    return template.model_copy(
        update={
            "name": name,
            "sections": tuple(
                section.model_copy(update={"name": heading})
                for section, heading in zip(template.sections, headings, strict=True)
            ),
        }
    )
