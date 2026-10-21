"""Orbit Pruebas's interface, clicked and read as a person would, through
CuaDriver.

CuaDriver is the standalone computer-use driver (cua-driver 0.28.2, the
version Orbit bundles), installed in /Applications/CuaDriver.app with its own
Accessibility and Screen Recording grants. The test drives it from the test server
over SSH (`mac.py motor-llamar`), bounded by a capability manifest to five
tools on Orbit Pruebas alone. Orbit Pruebas's own engine was the first design
(the test as Deimos through the hub's bridge); after run 5, where it refused
every reading as «operations that target its own authorization process»,
Mars chose CuaDriver (SIM-25.md, decision 5). Only Orbit Pruebas's exact pid
is ever targeted: cua-driver refuses a pid that is its own.

The vocabulary is cua-driver's: `list_windows`, `get_window_state`, `click`,
and `type_text`, an element addressed by the `element_index` of
the latest tree of its window.
"""

from __future__ import annotations

import time
from dataclasses import dataclass
from pathlib import Path

import arbol

BUNDLE_ID = "app.orbit.companion.pruebas"
PANEL_TITLE = "Orbit · llamada"
# How cua-driver says it stopped walking a tree (ax/tree.rs): at most 2 000
# nodes by default (DEFAULT_MAX_ELEMENTS). A tree cut short is never read as
# a whole one.
TRUNCADO = "AX tree truncated at"
# Attempts to choose an option in a native <select> before failing by name;
# each one is read back (real runs 23 and 24 needed a second).
INTENTOS_ELEGIR = 3
# The open menu is not in the tree (run 13): the keys wait this beat for
# it. Not a deadline: the read-back decides whether the choice took.
PAUSA_MENU_S = 0.3


class FalloPaso(Exception):
    """A step could not be done; the text says why, in Spanish, with numbers."""


@dataclass(frozen=True)
class Ventana:
    pid: int
    window_id: int
    titulo: str


class Interfaz:
    def __init__(self, puente, artefactos: Path, pid_app: int):
        self.puente = puente
        self.snapshot_id: str | None = None
        self.artefactos = artefactos
        # The pid of Orbit Pruebas's app, found by its exact executable path:
        # only its windows are ever targeted.
        self.pid_app = pid_app
        self.lecturas: list[dict] = []
        self.arboles: dict[str, str] = {}
        self._lecturas = 0
        self.cortado: str | None = None

    def ventanas(self) -> dict[str, Ventana]:
        """Orbit Pruebas's main window («principal»), among the windows of its
        own pid and of no other process. The floating panel is never here:
        CuaDriver lists layer-0 windows only (see `panel`)."""
        # Not `on_screen_only`: on the real Mac (run 7) the main window was
        # listed with is_on_screen false and on another Space, and the pid
        # filter already keeps the answer to this app's few windows.
        estructura = self.puente.accion("list_windows", {"pid": self.pid_app})
        filas = [w for w in estructura.get("windows", []) if w.get("pid") == self.pid_app]
        encontradas: dict[str, Ventana] = {}
        for fila in filas:
            titulo = fila.get("title") or ""
            if titulo == "Orbit":
                encontradas.setdefault(
                    "principal", Ventana(int(fila["pid"]), int(fila["window_id"]), titulo)
                )
        if "principal" not in encontradas:
            raise FalloPaso(
                f"CuaDriver no ve la ventana de Orbit Pruebas (pid {self.pid_app}): "
                f"{len(filas)} ventanas suyas, ninguna titulada «Orbit»"
            )
        return encontradas

    def panel(self, flotantes, plazo_s: float) -> Ventana:
        """The floating panel («Orbit · llamada»), once it is on screen.

        CuaDriver's `list_windows` lists layer-0 windows only, and the panel
        is always on top: it was never listed, so runs 1 to 17 said «no hay
        ventana» while the panel was on screen (probed on the Mac on
        2026-10-17). `flotantes` asks WindowServer for the app's windows above
        layer 0 that are on screen; the one whose accessibility window is
        titled «Orbit · llamada» is the panel, read by its window id as any
        other window."""
        limite = time.monotonic() + plazo_s
        vistas: list[int] = []
        while True:
            for fila in flotantes().get("ventanas", []):
                if fila.get("pid") != self.pid_app:
                    continue
                ventana = Ventana(self.pid_app, int(fila["window_id"]), PANEL_TITLE)
                estructura = self.puente.accion(
                    "get_window_state",
                    {
                        "pid": ventana.pid,
                        "window_id": ventana.window_id,
                        "include_accessibility_tree": True,
                        "include_screenshot": False,
                    },
                )
                primera = (estructura.get("tree_markdown") or "").lstrip().split("\n", 1)[0]
                if f'AXWindow "{PANEL_TITLE}"' in primera:
                    return ventana
                vistas.append(ventana.window_id)
            if time.monotonic() >= limite:
                raise FalloPaso(
                    f"con la respuesta dada, ninguna ventana flotante de Orbit Pruebas es "
                    f"«{PANEL_TITLE}» en {plazo_s:.0f} s (flotantes vistas: {sorted(set(vistas))})"
                )
            time.sleep(0.5)

    def leer(self, ventana: Ventana, etiqueta: str, consulta: str | None) -> list[arbol.Nodo]:
        """The rows of one window that match `consulta`, and their ancestors.

        Every reading of the main window is filtered (cua-driver's `query`):
        a check reads only what it needs, the answer stays small, and fewer
        nodes leave the window. Only the call panel, a card of a handful of
        rows, is read whole (`consulta=None`). Each answer's size goes to the
        report. CuaDriver walks at most 2 000 nodes of a window; a reading it
        cut short fails by name.
        """
        argumentos = {
            "pid": ventana.pid,
            "window_id": ventana.window_id,
            "include_accessibility_tree": True,
            "include_screenshot": False,
        }
        if consulta is not None:
            argumentos["query"] = consulta
        estructura = self.puente.accion("get_window_state", argumentos)
        bytes_ = getattr(self.puente, "ultimo_recibo_bytes", None)
        self.lecturas.append({"etiqueta": etiqueta, "consulta": consulta, "bytes": bytes_})
        # A click by element_index needs the snapshot it came from
        # (cua-driver 0.28.2 click: «element_index … requires the matching
        # snapshot_id»); each read replaces the window's snapshot.
        self.snapshot_id = estructura.get("snapshot_id")
        if estructura.get("degraded"):
            # CuaDriver returns an empty tree on purpose when it cannot tie the
            # window to its accessibility element (another Space, for one).
            razon = str(estructura.get("degraded_reason") or "")[:200]
            raise FalloPaso(f"CuaDriver no pudo leer «{etiqueta}»: {razon}")
        markdown = estructura.get("tree_markdown")
        if not isinstance(markdown, str):
            raise FalloPaso(f"el motor no devolvió el árbol de «{etiqueta}»: {sorted(estructura)}")
        markdown = arbol.sin_barra_de_menus(markdown)
        self._lecturas += 1
        nombre = f"{self._lecturas:03d}-{etiqueta.replace('/', '-').replace(' ', '-')}.txt"
        (self.artefactos / nombre).write_text(markdown, encoding="utf-8")
        cortado = next((linea for linea in markdown.splitlines() if TRUNCADO in linea), None)
        self.cortado = cortado.strip()[:160] if cortado is not None else None
        if not etiqueta.startswith("buscar "):
            # Evidence read short would be evidence of nothing. A search may
            # still find its target in what was walked (the indices stay valid).
            if self.cortado is not None:
                raise FalloPaso(f"árbol de {etiqueta} truncado: {self.cortado}")
            self.arboles[etiqueta] = markdown
        return arbol.leer(markdown)

    def _uno(
        self,
        ventana: Ventana,
        texto: str,
        roles: tuple[str, ...],
        contiene: bool,
        primero: bool = False,
    ) -> arbol.Nodo:
        # A screen the previous click opened renders a moment later (the
        # route change, then its own read). Wait for the element instead of
        # failing on the first frame: 10 s is far above a render and is the
        # same bound as «la interfaz lo enseña» (plazos «pantalla»).
        limite = time.monotonic() + 10
        while True:
            nodos = self.leer(ventana, f"buscar {texto}", consulta=texto)
            candidatos = arbol.buscar(nodos, texto, roles, contiene)
            if candidatos or self.cortado is not None or time.monotonic() >= limite:
                break
            time.sleep(0.5)
        if not candidatos and self.cortado is None:
            # cua-driver's `query` filters the rendered lines, so a label that
            # holds a line break («R» then «Reunión…», «+» then «Nueva
            # conversación») matches only on its second line and loses its
            # node (real run 16). One whole read of the window finds it.
            nodos = self.leer(ventana, f"buscar {texto} (ventana entera)", consulta=None)
            candidatos = arbol.buscar(nodos, texto, roles, contiene)
        if not candidatos and self.cortado is not None:
            raise FalloPaso(f"«{texto}» no está en el árbol, truncado: {self.cortado}")
        if primero and candidatos:
            # The main view comes before the context panel in the tree.
            return candidatos[0]
        if len(candidatos) != 1:
            vistos = [f"[{n.index}] {n.role} «{n.texto()}»" for n in candidatos[:5]]
            pantalla = ""
            if not candidatos:
                # What the screen says instead, so the failure shows where the
                # app really is (Orbit Pruebas's own, synthetic content).
                textos = [
                    n.texto()
                    for n in self.leer(ventana, f"pantalla sin {texto}", "AXStaticText")
                    if n.texto()
                ]
                pantalla = f"; la pantalla dice {textos[:12]}"
            raise FalloPaso(
                f"«{texto}» ({', '.join(roles) or 'cualquier rol'}): {len(candidatos)} elementos "
                f"pulsables en vez de uno {vistos}{pantalla}"
            )
        return candidatos[0]

    def pulsar(
        self,
        ventana: Ventana,
        texto: str,
        roles: tuple[str, ...] = (),
        contiene: bool = False,
        boton: str | None = None,
        primero: bool = False,
    ) -> None:
        nodo = self._uno(ventana, texto, roles, contiene, primero)
        argumentos = {
            "pid": ventana.pid,
            "window_id": ventana.window_id,
            "element_index": nodo.index,
            "snapshot_id": self.snapshot_id,
        }
        if boton is not None:
            argumentos["button"] = boton
        self.puente.accion("click", argumentos)

    def leer_hasta(
        self, ventana: Ventana, etiqueta: str, consulta: str | None, condicion, plazo_s: float
    ) -> float:
        """Reads the window until `condicion(texto)` holds or the bound passes.

        The interface gets the daemon's changes pushed, but the engine reads
        the screen on its own schedule: waiting on what the screen shows,
        rather than reading once, keeps the result from depending on which of
        the two came first. The last reading is the evidence either way.
        Returns the seconds it took."""
        inicio = time.monotonic()
        while True:
            texto = arbol.todo_el_texto(self.leer(ventana, etiqueta, consulta))
            if condicion(texto) or time.monotonic() - inicio > plazo_s:
                return time.monotonic() - inicio
            time.sleep(1)

    def escribir(self, ventana: Ventana, campo: str, texto: str) -> None:
        nodo = self._uno(ventana, campo, ("AXTextArea", "AXTextField"), False)
        # In the foreground, as a person types: a background insert into web
        # content is never confirmed by the engine (cua-driver type_text).
        self.puente.accion(
            "type_text",
            {
                "pid": ventana.pid,
                "window_id": ventana.window_id,
                "element_index": nodo.index,
                "snapshot_id": self.snapshot_id,
                "text": texto,
                "delivery_mode": "foreground",
            },
        )

    def elegir(self, ventana: Ventana, campo: str, opcion: str, opciones: tuple[str, ...]) -> None:
        """Chooses `opcion` in a native <select>, as a person does with the
        keyboard: open its menu, move to the option, press Return, and read
        the field back.

        Setting the popup's AXValue (set_value) changes what it shows but
        fires no change event in WebKit (real run 12), and the open menu is
        not part of the window's accessibility tree (run 13), so its items
        cannot be pressed. `opciones` is the menu's order, from the app.

        The menu opening cannot be observed, so the keys can reach the popup
        before its menu is up: right after a new build was installed (real
        runs 23 and 24) the field stayed on «Sistema». Each attempt reads the
        field back, and the choice is tried again, `INTENTOS_ELEGIR` times at
        most, before it fails by name."""
        actual = ""
        for _ in range(INTENTOS_ELEGIR):
            nodo = self._uno(ventana, campo, ("AXPopUpButton", "AXComboBox"), False)
            actual = nodo.value or ""
            if actual == opcion:
                return
            if actual not in opciones or opcion not in opciones:
                raise FalloPaso(f"«{campo}» enseña «{actual}», que no está en {list(opciones)}")
            pasos = opciones.index(opcion) - opciones.index(actual)
            self.puente.accion(
                "click",
                {
                    "pid": ventana.pid,
                    "window_id": ventana.window_id,
                    "element_index": nodo.index,
                    "snapshot_id": self.snapshot_id,
                },
            )
            # A beat for the menu to open: it has no element to wait on, and
            # the read-back below is what decides.
            time.sleep(PAUSA_MENU_S)
            teclas = ["down" if pasos > 0 else "up"] * abs(pasos) + ["return"]
            for tecla in teclas:
                # Foreground: an open native menu only takes genuine key events.
                self.puente.accion(
                    "press_key",
                    {
                        "pid": ventana.pid,
                        "window_id": ventana.window_id,
                        "key": tecla,
                        "delivery_mode": "foreground",
                    },
                )
        nodo = self._uno(ventana, campo, ("AXPopUpButton", "AXComboBox"), False)
        if (nodo.value or "") != opcion:
            raise FalloPaso(
                f"«{campo}» sigue en «{nodo.value or actual}» tras {INTENTOS_ELEGIR} intentos "
                f"de elegir «{opcion}» con el teclado"
            )


class PuenteCua:
    """CuaDriver on the Mac, one `cua-driver call` per action (mac.py)."""

    def __init__(self, mac):
        self.mac = mac
        self.ultimo_recibo_bytes: int | None = None

    def accion(self, tool: str, arguments: dict) -> dict:
        hecho = self.mac.motor_llamar(tool, arguments)
        self.ultimo_recibo_bytes = hecho.get("bytes")
        resultado = hecho.get("resultado")
        return resultado if isinstance(resultado, dict) else {}
