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
        "Una reunión de trabajo cualquiera. Quien la lea no estuvo: necesita saber "
        "de qué se habló, qué quedó decidido y qué hay que hacer.",
        ("Resumen", "Dos a cuatro frases con el propósito de la reunión y su resultado."),
        ("Puntos clave", "Los temas tratados, uno por viñeta, con lo esencial de cada uno."),
        ("Decisiones", "Sólo lo que quedó acordado explícitamente. Si no hubo, dilo."),
        ("Próximos pasos", "Qué sigue, quién lo hace y cuándo, si se dijo."),
    ),
    _builtin(
        "one_on_one",
        "Uno a uno",
        "Una conversación entre dos personas que trabajan juntas de forma habitual.",
        ("Cómo está", "El estado de ánimo, la carga y lo que preocupa, si salió."),
        ("Temas tratados", "Cada tema con lo que se dijo de él."),
        ("Acuerdos", "Lo que las dos personas acordaron."),
        ("Para la próxima", "Lo que queda pendiente de retomar en el siguiente uno a uno."),
    ),
    _builtin(
        "project",
        "Seguimiento de proyecto",
        "Una reunión periódica sobre el avance de un proyecto.",
        ("Estado", "Cómo va el proyecto en una o dos frases."),
        ("Avances", "Lo terminado o desbloqueado desde la última vez."),
        ("Bloqueos y riesgos", "Lo que frena o amenaza el plan, con su causa."),
        ("Decisiones", "Lo acordado en esta reunión."),
        ("Próximos pasos", "Tareas con responsable y plazo cuando se dijeron."),
    ),
    _builtin(
        "client",
        "Cliente",
        "Una conversación con un cliente o posible cliente.",
        ("Contexto", "Quién es, qué busca y en qué punto está la relación."),
        ("Necesidades", "Lo que el cliente pidió o dejó ver que necesita."),
        ("Objeciones y dudas", "Lo que frena al cliente y cómo se respondió."),
        ("Acuerdos", "Lo pactado: precios, plazos, alcance, próximos contactos."),
        ("Seguimiento", "Lo que hay que enviar o hacer después de la llamada."),
    ),
    _builtin(
        "interview",
        "Entrevista",
        "Una entrevista a una persona candidata para un puesto.",
        ("Perfil", "Trayectoria y experiencia relevante que contó."),
        ("Puntos fuertes", "Lo que destacó con ejemplos concretos."),
        ("Dudas", "Lo que no quedó claro o preocupa."),
        ("Valoración", "Impresión general basada sólo en lo dicho."),
        ("Siguiente paso", "Qué se le dijo sobre el proceso."),
    ),
    _builtin(
        "brainstorm",
        "Lluvia de ideas",
        "Una sesión para generar y comparar ideas.",
        ("Ideas", "Cada idea propuesta, con quién la propuso si se sabe."),
        ("Lo que gustó", "Las ideas que el grupo prefirió y por qué."),
        ("Descartado", "Lo que se descartó y la razón."),
        ("Preguntas abiertas", "Lo que falta por averiguar o decidir."),
    ),
)


def builtin(template_id: str) -> Template | None:
    return next((t for t in BUILTIN_TEMPLATES if t.id == template_id), None)
