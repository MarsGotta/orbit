"""The real test on the Mac (SIM-25): the orchestrator, launched from the test server.

    server/.venv/bin/python scripts/e2e-mac/prueba.py preparar
    server/.venv/bin/python scripts/e2e-mac/prueba.py ejecutar
    server/.venv/bin/python scripts/e2e-mac/prueba.py ensayo [--muestra NOMBRE]

`preparar` creates, once, the test's Hermes token under
~/.orbit-pruebas/secretos. `ejecutar` is the real run: it starts Hermes
`deimos-pruebas` and the test hub here, prepares the Mac over SSH
(ORBIT_MAC_HOST, ORBIT_MAC_SSH_KEY, as the release scripts), records a
synthetic meeting in Orbit Pruebas clicking and reading its interface through
CuaDriver — started bounded to Orbit Pruebas and stopped again — sends Deimos
a voice note and opens a call with it over Orbit Pruebas's socket, runs the
twelve checks and cleans up, also when something fails. `ensayo` walks the
same orchestrator and the same checks on Linux, against the example states in
scripts/e2e-mac/ensayo/, with the real test hub on loopback and nothing else
real: it proves the logic of the checks, never Orbit, and it is not the real
test.

Runbook: docs/self-hosting.md.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import time
import uuid
from contextlib import contextmanager
from datetime import UTC, datetime
from pathlib import Path

DIRECTORIO = Path(__file__).resolve().parent
sys.path.insert(0, str(DIRECTORIO))

import comprobaciones
import hub_pruebas
import mac as mac_escenario
from interfaz import FalloPaso, Interfaz, PuenteCua

RAIZ = DIRECTORIO.parents[1]
BASE = Path.home() / ".orbit-pruebas"
NOMBRE_DISPOSITIVO = "Orbit Pruebas"
REMOTO = ".orbit-pruebas/prueba"
# Every ssh of the test leaves with these, as the release scripts' do
# (scripts/release/lib.sh): no password prompt nobody can answer, a sleeping
# Mac found out in seconds, only the given key.
SSH = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "IdentitiesOnly=yes"]
# What an SSH call adds to the wait it carries: connecting (ConnectTimeout
# above) and starting python on the Mac.
MARGEN_SSH_S = 30


def _constante_rust(ruta: str, nombre: str) -> int:
    """A number read from the Rust source, so the bound follows the code."""
    texto = (RAIZ / ruta).read_text(encoding="utf-8")
    hallado = re.search(rf"const {nombre}: [^=]+= (?:Duration::from_secs\()?([0-9_]+)", texto)
    if hallado is None:
        raise RuntimeError(f"{ruta} ya no declara {nombre}")
    return int(hallado.group(1).replace("_", ""))


def plazos() -> dict[str, tuple[float, str]]:
    """Every wait of the run: its bound and why it is that size.

    Each wait ends on the fact it waits for, or earlier on the fact that says
    it will not come; the bound only covers a fact that never arrives. Bounds
    marked «provisional» have no measurement behind them yet: the report of
    every run writes down how long each wait took, and the reviewer replaces
    them with what the measurements say (p99 and worst seen).
    """
    from orbit_hub import grouping, meeting_transcription
    from orbit_hub.processing.config import ProcessingConfiguration

    listo_s = _constante_rust("apps/desktop/src-tauri/src/daemon_thread.rs", "DAEMON_READY_WAIT")
    salida_s = _constante_rust("apps/desktop/src-tauri/src/daemon_thread.rs", "DAEMON_EXIT_WAIT")
    voz_s = _constante_rust("crates/orbit-daemon/src/hub.rs", "VOICE_HUB_HTTP_TIMEOUT")
    turno_s = 180  # Orbit does not bound a Hermes turn: the test's patience
    duracion_s = 180  # the script lasts about two minutes; a third more for pauses
    grupos = -(-duracion_s // grouping.TARGET_SECONDS)
    # SIM-41: the hub asks for the note itself and the Mac reads it again
    # when the hub announces the change (`recording.changed`): no follow
    # interval of the Mac's adds to the wait.
    nota_s = (
        grupos * meeting_transcription.REQUEST_TIMEOUT_SECONDS
        + ProcessingConfiguration.model_fields["timeout_seconds"].default
    )
    return {
        "hermes": (60, "provisional: el arranque de `hermes serve` no está medido"),
        "hub": (35, "veinte veces el peor arranque medido del hub (1,73 s; hub-en-produccion.md)"),
        "daemon": (
            listo_s + 15,
            f"la app espera {listo_s} s a su daemon (DAEMON_READY_WAIT) más lo que tarda macOS en abrirla (provisional)",
        ),
        "emparejado": (30, "provisional: conexión del daemon al hub de pruebas, sin medir"),
        "cuadriver": (30, "provisional: arranque del daemon de CuaDriver, sin medir"),
        "llamada": (
            120,
            "cua-driver call deja de esperar al daemon a los 120 s (daemon.rs, send_request)",
        ),
        "grabando": (30, "provisional: arranque de la captura del micrófono, sin medir"),
        "escucha": (300, "provisional: la primera vez macOS descarga el modelo de voz en español"),
        "respuesta": (
            180,
            "provisional: Orbit no acota un turno de Hermes; ésta es la paciencia de la prueba",
        ),
        "guardada": (
            120,
            "provisional: sellar la grabación y que la lista de Notas la dé por cerrada, sin medir",
        ),
        "nota": (
            nota_s,
            (
                f"lo que el propio sistema tarda en rendirse: {grupos} tramos de {grouping.TARGET_SECONDS} s "
                f"a {meeting_transcription.REQUEST_TIMEOUT_SECONDS} s cada uno y la nota a "
                f"{ProcessingConfiguration.model_fields['timeout_seconds'].default:.0f} s"
            ),
        ),
        "pantalla": (10, "la interfaz recibe los cambios empujados; cubre dos lecturas del motor"),
        "chat": (
            turno_s,
            "provisional: Orbit no acota un turno de Hermes; ésta es la paciencia de la prueba",
        ),
        "nota-de-voz": (
            voz_s + turno_s,
            (
                f"el daemon da {voz_s} s a la transcripción de una nota de voz "
                f"(VOICE_HUB_HTTP_TIMEOUT) y la prueba {turno_s} s al turno de Hermes "
                "(provisional, como «chat»); un fallo de la transcripción la acaba antes"
            ),
        ),
        "llamada-escuchando": (
            30,
            (
                "provisional: abrir el micrófono con el procesado de voz, sin medir; "
                "Orbit no acota esa apertura"
            ),
        ),
        "sesion": (30, "provisional: abrir una conversación guardada en el hub, sin medir"),
        "cerrar": (
            salida_s + 15,
            f"la app da {salida_s} s a su daemon para salir (DAEMON_EXIT_WAIT), más margen",
        ),
        "parar": (
            30,
            "el mismo plazo que la unidad del hub de producción da al parar (TimeoutStopSec=30)",
        ),
    }


# What each reading of the main window asks CuaDriver for (cua-driver's
# `query`: the matching rows and their ancestors). A check reads only what it
# needs; the answer stays small (the path from CuaDriver has no byte bound,
# only its 120 s and its walk of at most 2 000 nodes, whose cut is named).
# Static texts are what a person reads on screen: the times, speakers and
# words of the transcript, the summary, the messages, any pill or notice.
# Measured sizes go to the report; prueba-real-mac.md has the expected ones.
LECTURAS: dict[str, str | None] = {
    "inicio": "Bienvenida a Orbit",
    # «Transcribiendo…» and «Escribiendo la nota» both contain it.
    "nota-lista": "ribiendo",
    # The opened note's title is the field «Título de la reunión» in its
    # first-level heading (SIM-52: editable, it renames the meeting); the list
    # beside it names the note too, as a row, so a row is never taken for it.
    "nota-desde-aviso": "Título de la reunión",
    "nota-desde-lista": "Título de la reunión",
    "nota-resumen": "AXStaticText",
    "nota-acciones": "AXCheckBox",
    # Read whole: since SIM-41 every turn's speaker is a button (it opens the
    # speaker menu, TranscriptSegment.tsx), not a static text, and no one
    # query holds both the times and texts and the speakers. A two-minute
    # meeting is a few dozen turns, far under the 2 000 nodes CuaDriver walks;
    # a cut tree fails by name all the same.
    "nota-transcripcion": None,
    "asistente-respondido": "AXStaticText",
    "asistente-reabierta": "AXStaticText",
    "final": "AXStaticText",
    # The call panel is a card of a handful of rows: read whole too.
    "panel": None,
}


class Parada(Exception):
    """A step the rest of the run needed failed; the run goes to clean up."""


# ── The Mac, over SSH ───────────────────────────────────────────────────────


class MacRemoto:
    """mac.py on the Mac, one subcommand per SSH call."""

    def __init__(self, host: str, clave: str):
        if not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.-]*(@[A-Za-z0-9_][A-Za-z0-9_.-]*)?", host):
            raise ValueError(f"ORBIT_MAC_HOST inválido: «{host}»")
        if not re.fullmatch(r"/[A-Za-z0-9_./-]+", clave) or not Path(clave).is_file():
            raise ValueError(f"ORBIT_MAC_SSH_KEY ausente o inválida: «{clave}»")
        self.host = host
        self.opciones = [*SSH, "-i", clave]

    def copiar(self) -> None:
        subprocess.run(
            [
                "ssh",
                *self.opciones,
                self.host,
                f"mkdir -p -m 700 .orbit-pruebas && mkdir -p -m 700 {REMOTO}",
            ],
            check=True,
            capture_output=True,
            timeout=MARGEN_SSH_S,
        )
        subprocess.run(
            [
                "scp",
                "-q",
                *self.opciones,
                str(DIRECTORIO / "mac.py"),
                str(DIRECTORIO / "guion.txt"),
                str(DIRECTORIO / "ventanas.swift"),
                f"{self.host}:{REMOTO}/",
            ],
            check=True,
            capture_output=True,
            timeout=MARGEN_SSH_S,
        )

    def _llamar(self, plazo_s: float, *argumentos: str) -> dict:
        # A non-interactive SSH session has no /opt/homebrew/bin, where
        # SwitchAudioSource lives (scripts/release/mac-build.sh says the same).
        remota = (
            "PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin "
            f"python3 {REMOTO}/mac.py " + " ".join(shlex.quote(a) for a in argumentos)
        )
        try:
            hecho = subprocess.run(
                ["ssh", *self.opciones, self.host, remota],
                capture_output=True,
                text=True,
                timeout=plazo_s + MARGEN_SSH_S,
                check=False,
            )
        except subprocess.TimeoutExpired:
            raise FalloPaso(
                f"el Mac no contestó a «{argumentos[0]}» en {plazo_s + MARGEN_SSH_S:.0f} s"
            ) from None
        linea = hecho.stdout.strip().splitlines()[-1] if hecho.stdout.strip() else ""
        try:
            salida = json.loads(linea)
        except ValueError:
            raise FalloPaso(
                f"«{argumentos[0]}» en el Mac terminó con {hecho.returncode}: {hecho.stderr.strip()[:300]}"
            ) from None
        if hecho.returncode != 0 or "error" in salida:
            raise FalloPaso(f"«{argumentos[0]}» en el Mac: {salida.get('error')}")
        return salida

    def preparar(self, max_s):
        return self._llamar(max_s, "preparar", "--max", str(max_s))

    def emparejar(self, url, codigo, nombre):
        return self._llamar(30, "emparejar", url, codigo, nombre)

    def esperar(self, que, arg, max_s):
        return self._llamar(max_s, "esperar", que, *([arg] if arg else []), "--max", str(max_s))

    def leer(self, que, ident=None):
        return self._llamar(30, "leer", que, *([ident] if ident else []))

    def hablar(self, parte):
        # Each part is under a minute of speech at say's default rate.
        return self._llamar(240, "hablar", parte)

    def restaurar(self, max_s):
        return self._llamar(max_s, "restaurar", "--max", str(max_s))

    def registros(self):
        return self._llamar(60, "registros")

    def activar(self):
        return self._llamar(30, "activar")

    def flotantes(self):
        # `swift` compiles the script first: a few seconds, far under 60.
        return self._llamar(60, "flotantes")

    def motor_arrancar(self, max_s):
        return self._llamar(max_s, "motor-arrancar", "--max", str(max_s))

    def motor_llamar(self, herramienta, argumentos):
        # cua-driver call gives up on its daemon at 120 s (plazos «llamada»).
        return self._llamar(
            120, "motor-llamar", herramienta, json.dumps(argumentos, ensure_ascii=False)
        )

    def motor_parar(self, max_s):
        return self._llamar(max_s, "motor-parar", "--max", str(max_s))

    def nota_de_voz(self):
        # One short phrase at say's default rate and two beats: seconds.
        return self._llamar(60, "nota-de-voz")

    def orden(self, nombre):
        return self._llamar(30, "orden", nombre)


class MacEnProceso:
    """mac.py's own steps, called in this process (the dry run)."""

    def __init__(self, mac: mac_escenario.Mac):
        self.mac = mac

    def copiar(self) -> None:
        return None

    def __getattr__(self, nombre):
        metodo = getattr(self.mac, nombre)

        def llamar(*argumentos):
            try:
                return metodo(*argumentos)
            except mac_escenario.Falla as causa:
                raise FalloPaso(str(causa)) from None

        return llamar


# ── The run ─────────────────────────────────────────────────────────────────


class Prueba:
    def __init__(
        self,
        mac,
        rutas: hub_pruebas.Rutas,
        host: str,
        puerto: int,
        directorio: Path,
        hermes: bool,
        proveedores: dict | None = None,
    ):
        self.mac = mac
        self.rutas = rutas
        self.host = host
        self.puerto = puerto
        self.directorio = directorio
        self.hermes = hermes
        self.proveedores = proveedores
        self.plazos = plazos()
        self.guion = mac_escenario.partes_del_guion(mac_escenario.GUION.read_text(encoding="utf-8"))
        self.ev: dict = {
            "pasos": [],
            "esperas": [],
            "arboles": {},
            "guion": [dicho for parte in self.guion for _, dicho in parte],
        }
        self.procesos: list[hub_pruebas.Proceso] = []
        # The codes that are failures though they are silent on screen,
        # read from the one place that decides it (NoticeCode::is_failure).
        self.ev["codigos_fallo"] = sorted(
            comprobaciones.codigos_de_fallo(
                (RAIZ / "crates" / "orbit-core" / "src" / "notice.rs").read_text(encoding="utf-8")
            )
        )
        self.ui: Interfaz | None = None

    @contextmanager
    def paso(self, nombre: str, comprobaciones_: tuple[int, ...] = (), critico: bool = True):
        fila = {
            "nombre": nombre,
            "inicio": time.time(),
            "comprobaciones": list(comprobaciones_),
            "ok": True,
        }
        self.ev["pasos"].append(fila)
        try:
            yield
        except (
            FalloPaso,
            comprobaciones.SinComprobar,
            RuntimeError,
            OSError,
            ValueError,
            KeyError,
            subprocess.SubprocessError,
        ) as causa:
            fila["ok"] = False
            fila["causa"] = str(causa) or type(causa).__name__
            if critico:
                raise Parada(nombre) from causa
        finally:
            fila["fin"] = time.time()

    def esperar_mac(self, que: str, arg=None) -> dict:
        max_s, _ = self.plazos[que]
        fila = {"espera": que, "max_s": max_s}
        self.ev["esperas"].append(fila)
        try:
            hecho = self.mac.esperar(que, arg, max_s)
        except FalloPaso as causa:
            fila["fallo"] = str(causa)
            raise
        fila["segundos"] = hecho.get("segundos")
        return hecho["valor"]

    def medir(self, que: str, segundos: float) -> None:
        self.ev["esperas"].append(
            {"espera": que, "max_s": self.plazos[que][0], "segundos": round(segundos, 3)}
        )

    # ── steps ──

    def servicios(self) -> None:
        with self.paso("parar lo que dejó una ejecución anterior"):
            self.ev["restos"] = hub_pruebas.parar_restos(self.rutas, self.plazos["parar"][0])
        # The dry run has no Hermes: nothing there asks it anything.
        if self.hermes:
            with self.paso("Hermes de pruebas", (7, 8), critico=False):
                # Its log stays out of the run's directory: it may carry what
                # the conversations said.
                proceso = hub_pruebas.arrancar_hermes(
                    self.rutas, hub_pruebas.PUERTO_HERMES, self.rutas.base / "hermes.log"
                )
                self.procesos.append(proceso)
                plazo = self.plazos["hermes"][0]
                self.medir(
                    "hermes",
                    hub_pruebas.esperar_puerto(
                        "127.0.0.1", hub_pruebas.PUERTO_HERMES, proceso, plazo
                    ),
                )
        with self.paso("hub de pruebas"):
            self.rutas.configuracion.write_text(
                hub_pruebas.configuracion(
                    self.rutas, self.host, hub_pruebas.PUERTO_HERMES, self.proveedores
                ),
                encoding="utf-8",
            )
            hub_pruebas.iniciar_almacen(self.rutas)
            proceso = hub_pruebas.arrancar_hub(
                self.rutas, self.host, self.puerto, self.directorio / "hub.log"
            )
            self.procesos.append(proceso)
            self.medir(
                "hub", hub_pruebas.esperar_salud(self.url_hub, proceso, self.plazos["hub"][0])
            )

    @property
    def url_hub(self) -> str:
        return f"http://{self.host}:{self.puerto}"

    def mac_lista(self) -> None:
        with self.paso("preparar el Mac"):
            self.mac.copiar()
            self.ev["mac"] = self.mac.preparar(self.plazos["daemon"][0])
        with self.paso("emparejar Orbit Pruebas con el hub de pruebas"):
            codigo = hub_pruebas.codigo_de_emparejamiento(self.rutas)
            self.mac.emparejar(self.url_hub, codigo, NOMBRE_DISPOSITIVO)
            self.esperar_mac("emparejado")

    def interfaz(self) -> None:
        with self.paso("CuaDriver, acotado a Orbit Pruebas"):
            self.ev["cua"] = self.mac.motor_arrancar(self.plazos["cuadriver"][0])
        self.ui = Interfaz(
            PuenteCua(self.mac), self.directorio / "arboles", self.ev["cua"]["pid_app"]
        )
        ui = self.ui
        with self.paso("ventana de Orbit Pruebas"):
            # The window appears after the app's first paint, a little after
            # its daemon answers; 20 s is far above the ~1 s it takes, and the
            # engine offers no window event to wait on instead.
            self.mac.activar()
            limite = time.monotonic() + 20
            while True:
                try:
                    self.principal = ui.ventanas()["principal"]
                    break
                except FalloPaso:
                    if time.monotonic() >= limite:
                        raise
                    time.sleep(0.5)
            inicio = ui.leer(self.principal, "inicio", LECTURAS["inicio"])
            if any("Bienvenida a Orbit" in n.textos() for n in inicio):
                ui.pulsar(self.principal, "Saltar este paso", ("AXButton",))

        with self.paso("empezar a grabar desde la interfaz", (1, 2, 3, 4, 5, 6, 7)):
            ui.pulsar(self.principal, "Notas", ("AXButton",))
            ui.pulsar(self.principal, "Grabar reunión ahora", ("AXButton",))
            # Microphone only: the script plays into BlackHole, the default
            # input, and with system audio too every line would be recorded
            # twice.
            ui.elegir(
                self.principal,
                "Fuente de audio",
                "Solo micrófono",
                # MeetingStartDialog.tsx SourceField: system, then microphone.
                ("Sistema", "Solo micrófono"),
            )
            ui.pulsar(self.principal, "Grabar", ("AXButton",))
            grabacion = self.esperar_mac("grabando")
            self.ev["grabacion"] = dict(grabacion)
            if grabacion.get("audio") != "microphone_only":
                raise FalloPaso(
                    f"la grabación empezó con audio «{grabacion.get('audio')}», no «microphone_only»: "
                    "«Fuente de audio» no cambió a «Solo micrófono»"
                )

        with self.paso("primera mitad del guion"):
            self.mac.hablar("parte1")

        with self.paso("«Oye Deimos» y su respuesta en el panel", (7,), critico=False):
            try:
                self.esperar_mac("escucha")
            finally:
                # Said even if nobody listens: the meeting is the same meeting.
                self.mac.hablar("pregunta")
            self.ev["pregunta"] = self.esperar_mac("respuesta")
            if self.ev["pregunta"].get("stage") == "answered":
                # The panel hides 45 s after the answer (ANSWER_SHOWN_MS).
                panel = ui.panel(self.mac.flotantes, self.plazos["pantalla"][0])
                self.medir(
                    "pantalla",
                    ui.leer_hasta(
                        panel,
                        "panel",
                        LECTURAS["panel"],
                        lambda texto: "Tu pregunta:" in texto,
                        self.plazos["pantalla"][0],
                    ),
                )

        with self.paso("segunda mitad del guion"):
            self.mac.hablar("parte2")

        with self.paso("terminar la grabación desde la interfaz", (1, 2, 3, 4, 5, 6)):
            # «Terminar» is always in the page (MeetingPill.tsx, since run 14
            # found it reachable only under a mouse hover): pressed by
            # accessibility, as the keyboard would.
            ui.pulsar(self.principal, "Terminar", ("AXButton",))

        # The «Reunión guardada» notice goes away by itself after five seconds
        # (SIM-78, Mars's order), long before the note is written. Every round
        # trip to the engine takes seconds (real runs 2 and 3 of 2026-10-20:
        # the link came after the notice had gone), so its link is looked for
        # at once, before waiting on anything else, and the page it opens is
        # read once the note is ready.
        with self.paso("«Abrir nota» desde el aviso", (6,), critico=False):
            ui.pulsar(self.principal, "Abrir nota", ("AXLink", "AXButton"))
            # What the page shows while its note is still being written.
            ui.leer(self.principal, "buscar la nota recién abierta (ventana entera)", None)

        with self.paso("la grabación guardada", (1, 2, 3, 4, 5, 6)):
            guardada = self.esperar_mac("guardada", self.ev["grabacion"]["id"])
            self.ev["grabacion"].update(guardada)

        identificador = self.ev["grabacion"]["id"]
        with self.paso("la nota", (1, 2, 3, 4, 5, 6), critico=False):
            self.ev["fase_nota"] = self.esperar_mac("nota", identificador)
            self.ev["detalle"] = self.mac.leer("detalle", identificador)["items"][0]
            # Until the interface stops saying the note is on its way.
            self.medir(
                "pantalla",
                ui.leer_hasta(
                    self.principal,
                    "nota-lista",
                    LECTURAS["nota-lista"],
                    lambda texto: (
                        not any(t in texto for t in ("Transcribiendo", "Escribiendo la nota"))
                    ),
                    self.plazos["pantalla"][0],
                ),
            )

        with self.paso("la nota que abrió el aviso", (6,), critico=False):
            titulo = comprobaciones._titulo(self.ev)
            self.medir(
                "pantalla",
                ui.leer_hasta(
                    self.principal,
                    "nota-desde-aviso",
                    LECTURAS["nota-desde-aviso"],
                    lambda texto: titulo in texto,
                    self.plazos["pantalla"][0],
                ),
            )
            # The filtered read says only that the title is missing; the whole
            # window says what is there instead.
            ui.leer(self.principal, "buscar la nota del aviso (ventana entera)", None)

        with self.paso("abrir la nota desde la lista de Notas", (6,), critico=False):
            ui.pulsar(self.principal, "Notas", ("AXButton",))
            # The list and the opened note name it by the meeting's title
            # (Mars, 2026-10-17).
            titulo = comprobaciones._titulo(self.ev)
            if not titulo:
                raise FalloPaso("la nota no tiene título con el que buscarla en la lista")
            ui.pulsar(self.principal, titulo, ("AXButton",), contiene=True, primero=True)
            self.medir(
                "pantalla",
                ui.leer_hasta(
                    self.principal,
                    "nota-desde-lista",
                    LECTURAS["nota-desde-lista"],
                    lambda texto: titulo in texto,
                    self.plazos["pantalla"][0],
                ),
            )

        with self.paso("las pestañas de la nota", (1, 2, 3, 4, 5), critico=False):
            for pestana, etiqueta in (
                ("Resumen", "nota-resumen"),
                ("Acciones", "nota-acciones"),
                ("Transcripción", "nota-transcripcion"),
            ):
                ui.pulsar(self.principal, pestana, ("AXRadioButton", "AXTab", "AXButton"))
                ui.leer(self.principal, etiqueta, LECTURAS[etiqueta])

        with self.paso("el Asistente", (8,), critico=False):
            mensaje = (
                f"Prueba de Orbit {uuid.uuid4().hex[:6]}: contesta sólo con la palabra recibido."
            )
            self.ev["chat"] = {"mensaje": mensaje}
            ui.pulsar(self.principal, "Asistente", ("AXButton",))
            ui.escribir(self.principal, "Mensaje", mensaje)
            ui.pulsar(self.principal, "Enviar", ("AXButton",))
            hecho = self.esperar_mac("chat", mensaje)
            self.ev["chat"].update(hecho)
            respondido = self.plazos["pantalla"][0]
            self.medir(
                "pantalla",
                ui.leer_hasta(
                    self.principal,
                    "asistente-respondido",
                    LECTURAS["asistente-respondido"],
                    lambda texto: (
                        mensaje in texto
                        and not comprobaciones._palabras_que_faltan(hecho["respuesta"], texto)
                    ),
                    respondido,
                ),
            )
            # The app names a conversation by its start on the Mac's clock
            # (SIM-30); mac.py writes that same name as `titulo`.
            sesiones = self.mac.leer("sesiones")["items"]
            anterior = next((s for s in sesiones if s.get("id") == hecho["sesion"]), None)
            if anterior is None or not anterior.get("titulo"):
                raise FalloPaso(
                    f"la conversación {hecho['sesion']} no está entre las {len(sesiones)} "
                    "que lista el hub, con su fecha de inicio"
                )
            ui.pulsar(
                self.principal, "Nueva conversación", ("AXButton",), contiene=True, primero=True
            )
            ui.pulsar(
                self.principal, anterior["titulo"], ("AXButton",), contiene=True, primero=True
            )
            self.esperar_mac("sesion", hecho["sesion"])
            self.medir(
                "pantalla",
                ui.leer_hasta(
                    self.principal,
                    "asistente-reabierta",
                    LECTURAS["asistente-reabierta"],
                    lambda texto: mensaje in texto,
                    self.plazos["pantalla"][0],
                ),
            )

        # SIM-27: the voice note and the call, after the meeting has ended:
        # nothing records while they run.
        with self.paso("una nota de voz a Deimos", (11,), critico=False):
            hablada = self.mac.nota_de_voz()
            self.ev["nota_de_voz"] = {
                "frase": mac_escenario.NOTA_DE_VOZ,
                "hablada": hablada,
                **self.esperar_mac("nota-de-voz"),
            }

        with self.paso("una llamada con Deimos", (12,), critico=False):
            self.mac.orden("call_start")
            try:
                self.ev["llamada"] = self.esperar_mac("llamada-escuchando")
            finally:
                # Also when it never listened: no call stays open.
                self.mac.orden("call_stop")

        with self.paso("pantalla final", (9,), critico=False):
            ui.leer(self.principal, "final", LECTURAS["final"])

    def limpiar(self) -> None:
        with self.paso("parar CuaDriver", critico=False):
            # Always, and first: nothing may keep acting on the Mac.
            self.ev.setdefault("cua", {}).update(self.mac.motor_parar(self.plazos["cerrar"][0]))
        with self.paso("cerrar Orbit Pruebas y devolver la entrada de audio", critico=False):
            self.ev["restaurado"] = self.mac.restaurar(self.plazos["cerrar"][0])
        with self.paso("recoger los registros del Mac", (10, 12), critico=False):
            recogidos = self.mac.registros()
            registros = recogidos["registros"]
            self.ev["registros"] = registros
            for nombre, texto in registros.items():
                (self.directorio / nombre).write_text(texto, encoding="utf-8")
            self.ev["audio_llamada"] = recogidos["audio_llamada"]
            (self.directorio / "daemon-llamada.log").write_text(
                recogidos["audio_llamada"], encoding="utf-8"
            )
        with self.paso("parar el hub y Hermes de pruebas", critico=False):
            self.ev["parados"] = [p.parar(self.plazos["parar"][0]) for p in reversed(self.procesos)]
            registro = self.directorio / "hub.log"
            self.ev["hub_log"] = registro.read_text(encoding="utf-8") if registro.exists() else None

    def ejecutar(self) -> list[dict]:
        self.directorio.mkdir(parents=True, exist_ok=True)
        (self.directorio / "arboles").mkdir(exist_ok=True)
        try:
            self.servicios()
            self.mac_lista()
            self.interfaz()
        except Parada:
            pass
        except KeyboardInterrupt as causa:
            instante = time.time()
            self.ev["pasos"].append(
                {
                    "nombre": "interrumpida",
                    "inicio": instante,
                    "fin": instante,
                    "comprobaciones": [],
                    "ok": False,
                    "causa": f"la prueba se interrumpió ({causa or 'Ctrl-C'})",
                }
            )
        finally:
            self.limpiar()
        if self.ui is not None:
            self.ev["arboles"] = self.ui.arboles
            self.ev["lecturas"] = self.ui.lecturas
        resultados = comprobaciones.comprobar(self.ev)
        informe(self.directorio, self.ev, resultados)
        if any(r["estado"] != "pasa" for r in resultados) and self.rutas.hub.exists():
            # The hub's data (the synthetic meeting's audio, its transcript, its
            # final pass) is wiped by the next run. A transcript that lost words
            # in 2 of 6 real runs (2026-10-20) could not be examined afterwards:
            # a failed run keeps what the hub kept, nothing more than invented
            # speech, in this run's own directory.
            shutil.copytree(self.rutas.hub, self.directorio / "hub-datos")
        return resultados


def informe(directorio: Path, ev: dict, resultados: list[dict]) -> None:
    """informe.json for machines, informe.txt for people. No content but the
    synthetic meeting's, which is invented."""
    datos = {
        "resultados": resultados,
        "pasos": ev["pasos"],
        "esperas": ev["esperas"],
        "mac": ev.get("mac"),
        "grabacion": ev.get("grabacion"),
        "parados": ev.get("parados"),
        "restaurado": ev.get("restaurado"),
        "lecturas": ev.get("lecturas"),
        "cua": ev.get("cua"),
    }
    (directorio / "informe.json").write_text(
        json.dumps(datos, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    lineas = [f"Prueba real en el Mac · {directorio.name}", ""]
    for r in resultados:
        lineas.append(f"{r['numero']:>2}. [{r['estado']}] {r['nombre']}")
        lineas.append(f"    {r['detalle']}")
        for fila in r.get("registro", []):
            lineas.append(
                f"    registro {fila['registro']}: código {fila['codigo']}, op {fila['op']}"
            )
    lineas += ["", "No cubierto:"]
    lineas += [f"  {texto}" for texto in comprobaciones.NO_CUBIERTO]
    lineas += ["", "Pasos:"]
    for p in ev["pasos"]:
        estado = "bien" if p.get("ok", True) else f"FALLA: {p.get('causa')}"
        lineas.append(
            f"  {p['nombre']}: {estado} ({p.get('fin', p['inicio']) - p['inicio']:.1f} s)"
        )
    lineas += ["", "Lecturas del motor (bytes de la respuesta de CuaDriver):"]
    for lectura in ev.get("lecturas") or []:
        lineas.append(
            f"  {lectura['etiqueta']} [{lectura['consulta'] or 'ventana entera'}]: "
            f"{lectura['bytes'] if lectura['bytes'] is not None else '—'} bytes"
        )
    lineas += ["", "Esperas (lo que tardó cada una y su plazo):"]
    for e in ev["esperas"]:
        lineas.append(
            f"  {e['espera']}: {e.get('segundos', '—')} s de {e['max_s']:.0f} s {e.get('fallo', '')}"
        )
    (directorio / "informe.txt").write_text("\n".join(lineas) + "\n", encoding="utf-8")


# ── Entry points ────────────────────────────────────────────────────────────


def directorio_de_ejecucion(base: Path) -> Path:
    return base / "ejecuciones" / datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")


def ip_tailnet() -> str:
    hecho = subprocess.run(
        ["tailscale", "ip", "-4"], capture_output=True, text=True, timeout=15, check=True
    )
    direccion = hecho.stdout.split()[0]
    if hub_pruebas.ipaddress.ip_address(direccion) not in hub_pruebas.TAILNET:
        raise RuntimeError(f"`tailscale ip -4` dio {direccion}, fuera de la tailnet")
    return direccion


def ejecutar() -> int:
    rutas = hub_pruebas.Rutas(BASE)
    hub_pruebas.configuracion(rutas, "127.0.0.1", hub_pruebas.PUERTO_HERMES)
    hub_pruebas.required_environment("ORBIT_TEST_HERMES_TOKEN_ENV")
    for secreto in (rutas.token_hermes,):
        if not secreto.exists() or secreto.stat().st_mode & 0o077:
            raise SystemExit(f"falta {secreto} con modo 0600: ejecuta antes «prueba.py preparar»")
    try:
        mac = MacRemoto(
            os.environ.get("ORBIT_MAC_HOST", ""), os.environ.get("ORBIT_MAC_SSH_KEY", "")
        )
        host = ip_tailnet()
    except (ValueError, RuntimeError, OSError, subprocess.SubprocessError) as causa:
        raise SystemExit(f"no se puede empezar: {causa}") from None
    for direccion, puerto in (
        (host, hub_pruebas.PUERTO_HUB),
        ("127.0.0.1", hub_pruebas.PUERTO_HERMES),
    ):
        if not hub_pruebas.puerto_libre(direccion, puerto):
            raise SystemExit(
                f"el puerto {direccion}:{puerto} está ocupado; la prueba no toca lo que lo usa"
            )
    directorio = directorio_de_ejecucion(BASE)
    prueba = Prueba(mac, rutas, host, hub_pruebas.PUERTO_HUB, directorio, hermes=True)

    # Closing the terminal or a plain `kill` still cleans up: the run goes to
    # its `finally` as with Ctrl-C.
    def interrumpir(numero, _marco):
        raise KeyboardInterrupt(f"señal {numero}")

    signal.signal(signal.SIGTERM, interrumpir)
    signal.signal(signal.SIGHUP, interrumpir)
    resultados = prueba.ejecutar()
    print((directorio / "informe.txt").read_text(encoding="utf-8"))
    return 0 if all(r["estado"] == "pasa" for r in resultados) else 1


def main(argv: list[str]) -> int:
    lector = argparse.ArgumentParser(description="La prueba real de Orbit en el Mac (SIM-25)")
    ordenes = lector.add_subparsers(dest="orden", required=True)
    ordenes.add_parser("preparar", help="crea una vez las claves de la prueba en ~/.orbit-pruebas")
    ordenes.add_parser("ejecutar", help="la prueba real, contra el Mac")
    ensayo = ordenes.add_parser(
        "ensayo", help="ensayo en Linux contra estados de ejemplo; no es la prueba"
    )
    ensayo.add_argument("--muestra", default="bueno")
    ensayo.add_argument("--directorio", type=Path)
    opciones = lector.parse_args(argv)
    if opciones.orden == "preparar":
        creados = hub_pruebas.preparar_secretos(hub_pruebas.Rutas(BASE))
        print("Creados: " + (", ".join(creados) if creados else "nada; ya existían"))
        return 0
    if opciones.orden == "ejecutar":
        return ejecutar()
    import ensayo as ensayo_

    resultados, directorio = ensayo_.ejecutar(opciones.muestra, opciones.directorio)
    print((directorio / "informe.txt").read_text(encoding="utf-8"))
    return 0 if all(r["estado"] == "pasa" for r in resultados) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
