"""The accessibility tree cua-driver returns, read as data.

`get_window_state` (cua-driver 0.28.2, the version Orbit bundles) renders the
tree of one window as Markdown, one node per line, two spaces of indent per
level (libs/cua-driver/rust/crates/platform-macos/src/ax/tree.rs):

    - [N] AXRole "title" = "value" (description) [id=… help="…" actions=[press]]
    - AXStaticText = "value"

`[N]` is the element index an action takes; only nodes that expose an action
have one. Nothing is escaped, so a quote inside a text is read up to the last
delimiter that still fits the line.

Kept free of I/O: the checks and the dry run read the same trees the Mac gives.
"""

from __future__ import annotations

import re
import unicodedata
from dataclasses import dataclass

_LINE = re.compile(
    r"^(?P<indent>(?:  )*)- (?:\[(?P<index>\d+)\] )?(?P<role>\S+)"
    r'(?: "(?P<title>.*?)")?(?: = "(?P<value>.*?)")?(?: \((?P<description>.*?)\))?'
    r"(?: \[(?P<attributes>(?:[^\[\]]|\[[^\]]*\])*)\])?$"
)
# «00:12», «12:34» or «1:02:03»: how the note screen writes a turn's offset
# (formatTimestamp, apps/desktop/src/features/notes/notes-format.ts).
TIME = re.compile(r"^(?:(\d+):)?(\d{2}):(\d{2})$")


@dataclass(frozen=True)
class Nodo:
    depth: int
    index: int | None
    role: str
    title: str | None
    value: str | None
    description: str | None

    def textos(self) -> list[str]:
        """What a person reads on this node: its title, value and description."""
        return [text for text in (self.title, self.value, self.description) if text]

    def texto(self) -> str:
        return " ".join(self.textos())


# Where a node starts: its indent, «- », the optional index and an AX role.
# A text that holds a line break continues on lines that do not: a row whose
# initial and title are separate lines (real run 15), or Deimos's answer in
# the panel, whose list items start «- **Presupuesto**…» (real run 18).
_NODO = re.compile(r"^(?:  )*- (?:\[\d+\] )?AX")


def _lineas(markdown: str) -> list[str]:
    """One string per node; the lines that continue a text join it with a space."""
    lineas: list[str] = []
    for line in markdown.splitlines():
        if _NODO.match(line) or not lineas:
            lineas.append(line)
        else:
            lineas[-1] += " " + line.strip()
    return lineas


# macOS's menu bar, not Orbit's: a whole-window read brings it, and under the
# Apple menu «Ítems recientes» names Mars's own documents (real run 36). No
# step presses or reads a menu, so its subtree never reaches the evidence.
_BARRA_DE_MENUS = re.compile(r"^(?P<indent>(?:  )*)- (?:\[\d+\] )?AXMenuBar(?:\s|$)")


def sin_barra_de_menus(markdown: str) -> str:
    """The tree without the menu bar and everything under it."""
    salida: list[str] = []
    dentro: int | None = None  # the indent of the menu bar being left out
    for line in markdown.splitlines(keepends=True):
        if dentro is not None:
            if not _NODO.match(line) or len(line) - len(line.lstrip(" ")) > dentro:
                continue
            dentro = None
        barra = _BARRA_DE_MENUS.match(line)
        if barra is not None:
            dentro = len(barra["indent"])
            continue
        salida.append(line)
    return "".join(salida)


def leer(markdown: str) -> list[Nodo]:
    nodos = []
    for line in _lineas(markdown):
        found = _LINE.match(line.rstrip())
        if found is None:
            continue
        index = found["index"]
        nodos.append(
            Nodo(
                depth=len(found["indent"]) // 2,
                index=int(index) if index is not None else None,
                role=found["role"],
                title=found["title"],
                value=found["value"],
                description=found["description"],
            )
        )
    return nodos


def plano(texto: str) -> str:
    """Lower case without accents, for comparisons a person would call equal."""
    descompuesto = unicodedata.normalize("NFD", texto.casefold())
    return "".join(c for c in descompuesto if unicodedata.category(c) != "Mn")


def todo_el_texto(nodos: list[Nodo]) -> str:
    return "\n".join(nodo.texto() for nodo in nodos if nodo.textos())


def buscar(
    nodos: list[Nodo],
    texto: str,
    roles: tuple[str, ...] = (),
    contiene: bool = False,
) -> list[Nodo]:
    """Actionable nodes whose title, value or description is `texto`."""

    def coincide(candidato: str) -> bool:
        return texto in candidato if contiene else candidato == texto

    return [
        nodo
        for nodo in nodos
        if nodo.index is not None
        and (not roles or nodo.role in roles)
        and any(coincide(candidato) for candidato in nodo.textos())
    ]


def segundos(hora: str) -> int | None:
    found = TIME.match(hora)
    if found is None:
        return None
    horas, minutos, segs = found.groups()
    return int(horas or 0) * 3600 + int(minutos) * 60 + int(segs)


@dataclass(frozen=True)
class Segmento:
    hora: str
    hablante: str
    texto: str


def transcripcion(nodos: list[Nodo], inicio: str, fin: str) -> tuple[list[str], list[Segmento]]:
    """The turns of the «Transcripción» tab, in the order they are shown.

    Each turn is an <article> whose header holds the time, the speaker and
    then the text (TranscriptSegment.tsx), so in the tree a time starts a turn,
    the next text is its speaker and what follows up to the next time is what
    was said. Reading starts after the text that begins with `inicio` (the
    toolbar's «Idioma de la transcripción: …») and stops at `fin` («Panel de contexto»,
    the side panel). Returns the texts up to the first turn, the toolbar's
    included, and the turns.
    """
    cabecera: list[str] = []
    segmentos: list[Segmento] = []
    dentro = False
    actual: list[str] | None = None

    def cerrar() -> None:
        if actual is not None:
            hablante = actual[1] if len(actual) > 1 else ""
            segmentos.append(Segmento(actual[0], hablante, " ".join(actual[2:]).strip()))

    for nodo in nodos:
        textos = nodo.textos()
        if not dentro:
            cabecera.extend(textos)
            dentro = any(texto.startswith(inicio) for texto in textos)
            continue
        if fin in textos:
            break
        for texto in textos:
            if TIME.match(texto):
                cerrar()
                actual = [texto]
            elif actual is not None:
                actual.append(texto)
            else:
                cabecera.append(texto)
    cerrar()
    return cabecera, segmentos
