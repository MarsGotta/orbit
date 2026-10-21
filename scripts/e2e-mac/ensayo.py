"""The dry run on Linux (SIM-25): the orchestrator and the checks against
example states, never against Orbit.

What is real: the orchestrator (prueba.py), the Mac-side steps and waits
(mac.py) with their predicates, the reading of the trees and the search for
what to click (interfaz.py, arbol.py), the twelve checks, the report, and the
test hub itself — started from this tree on loopback with a Whisper and a
note model that do not exist and are never called, and stopped again.

What is replayed from scripts/e2e-mac/ensayo/<example>/: the system commands
of the Mac (say, SwitchAudioSource, open, ps), Orbit Pruebas's socket (the
frames each wait sees, the reads, the orders), its logs, and CuaDriver's
command line (`status`, `call`, `stop`: the windows, the tree of each screen,
which screen a click leads to, and a refusal when the example says so). An example other than
«bueno» holds only the files that differ from «bueno».

A dry run that passes says the logic of the test holds. It is not the real
test and does not count as one.
"""

from __future__ import annotations

import json
import math
import plistlib
import socket
import tempfile
import time
from datetime import UTC, datetime
from pathlib import Path

import hub_pruebas
import mac as mac_escenario
import prueba
from interfaz import FalloPaso

MUESTRAS = Path(__file__).resolve().parent / "ensayo"
PID = 4242
PID_CUA = 4343
PRINCIPAL = 11
PANEL = 12
# cua-driver walks at most 2 000 nodes of a window (ax/tree.rs) and then says so.
MAX_NODOS = 2000


class Muestra:
    def __init__(self, nombre: str):
        self.directorios = (
            [MUESTRAS / nombre, MUESTRAS / "bueno"] if nombre != "bueno" else [MUESTRAS / "bueno"]
        )
        if not self.directorios[0].is_dir():
            raise FileNotFoundError(f"no existe la muestra {self.directorios[0]}")
        self.mensaje = ""

    def ruta(self, relativa: str) -> Path:
        for directorio in self.directorios:
            if (directorio / relativa).exists():
                return directorio / relativa
        raise FileNotFoundError(relativa)

    def texto(self, relativa: str) -> str:
        return self.ruta(relativa).read_text(encoding="utf-8").replace("{mensaje}", self.mensaje)

    def json(self, relativa: str):
        return json.loads(self.texto(relativa))

    def base(self, relativa: str):
        """«bueno»'s own `relativa`, which the other examples change in part."""
        return json.loads((MUESTRAS / "bueno" / relativa).read_text(encoding="utf-8"))

    def cambios(self, relativa: str) -> dict:
        """The example's own overrides of `relativa`, keyed like it."""
        propio = self.directorios[0] / relativa
        if len(self.directorios) == 1 or not propio.exists():
            return {}
        return json.loads(propio.read_text(encoding="utf-8"))


class SistemaEnsayo(mac_escenario.Sistema):
    """say, SwitchAudioSource, open, ps and cua-driver, answered like a Mac
    would. Never signals a process: the pids here are invented."""

    def __init__(self, muestra: Muestra, rutas: mac_escenario.Rutas):
        self.muestra = muestra
        self.rutas = rutas
        self.entrada = "Micrófono del MacBook Pro"
        self.ordenes: list[list[str]] = []
        self.app_abierta = False
        self.senales: list[tuple[int, int]] = []
        self.pausas: list[float] = []
        self.cua = CuaEnsayo(muestra, rutas.registros)

    def correr(self, *orden: str) -> tuple[int, str, str]:
        self.ordenes.append(list(orden))
        if orden[0] == str(self.rutas.cua):
            return self.cua.orden(list(orden[1:]))
        if orden[0] == "SwitchAudioSource":
            if "-a" in orden:
                return 0, f"{self.entrada}\n{mac_escenario.BLACKHOLE}\n", ""
            if "-c" in orden:
                return 0, self.entrada + "\n", ""
            self.entrada = orden[-1]
            return 0, "", ""
        if orden[:3] == ("say", "-v", "?"):
            return 0, self.muestra.texto("voces.txt"), ""
        if orden[:4] == ("open", "-n", "-g", "-a"):
            return self.cua.arrancar(list(orden))
        if orden[0] == "open":
            # Orbit Pruebas «starts»: its logs are the example's.
            self.app_abierta = True
            self.rutas.registros.mkdir(parents=True, exist_ok=True)
            for nombre in ("report-daemon.log", "report-app.log", "daemon.log"):
                (self.rutas.registros / nombre).write_text(
                    self.muestra.texto(f"registros/{nombre}"), encoding="utf-8"
                )
            return 0, "", ""
        if orden[0] == "sw_vers":
            return 0, "26.1\n", ""
        if list(orden) == ["id", "-F"]:
            return 0, "Alina Pérez\n", ""
        if orden[0] == "swift" and orden[1].endswith("ventanas.swift"):
            # WindowServer's listing: the main window at layer 0 and, on
            # screen above it, the floating panel (layer 3, always on top).
            filas = [
                {"window_id": PRINCIPAL, "layer": 0, "is_on_screen": True},
                {"window_id": PANEL, "layer": 3, "is_on_screen": True},
            ]
            return 0, "".join(json.dumps(fila) + "\n" for fila in filas), ""
        if orden[0] == "say":
            return 0, "", ""
        return 1, "", f"orden no prevista en el ensayo: {orden}"

    def existe_orden(self, nombre: str) -> bool:
        return True

    def pausa(self, segundos: float) -> None:
        # Nothing plays here: the beat is noted, never slept.
        self.pausas.append(segundos)

    def hora_local(self, segundos: float) -> time.struct_time:
        # The examples' Mac lives in UTC, so their titles do not depend on
        # the clock of the machine that runs the dry run.
        return time.gmtime(segundos)

    def procesos(self) -> list[tuple[int, str]]:
        filas = []
        if self.app_abierta:
            filas.append((PID, str(self.rutas.app / mac_escenario.MIEMBROS[0])))
        if self.cua.en_marcha:
            filas.append((PID_CUA, mac_escenario.CUA_APP + "Contents/MacOS/CuaDriver"))
        return filas

    def senal(self, pid: int, numero: int) -> None:
        self.senales.append((pid, numero))
        if pid == PID:
            self.app_abierta = False

    def esperar_salida(self, pids: list[int], max_s: float) -> list[int]:
        if PID in pids:
            self.app_abierta = False
        return []


class OrbitEnsayo:
    """The daemon's socket: each subscription gets the next wait of the
    example. A wait that reads (SIM-41: the meeting saved, its note final)
    answers its reads, in order, from that wait's own `lecturas`; the last
    one stays. An example changes a wait with its frames (a list) or with
    `{"frames": …, "lecturas": …}`."""

    def __init__(self, muestra: Muestra):
        self.muestra = muestra
        self.esperas = muestra.base("ipc.json")["esperas"]
        self.siguiente = 0
        self.en_curso: list | None = None

    def responde(self) -> bool:
        return True

    def _espera(self, espera: dict) -> dict:
        cambiada = self.muestra.cambios("ipc.json").get("esperas", {}).get(espera["que"])
        if isinstance(cambiada, list):
            cambiada = {"frames": cambiada}
        return {**espera, **(cambiada or {})}

    def llamar(self, body: dict, plazo_s: float = 30) -> dict:
        if body["command"] == "pair" or body in mac_escenario.ORDENES.values():
            return {"ok": True}
        if body["command"] == "read":
            clase = body["read"]["kind"]
            if self.en_curso and clase == "notes_page":
                valor = self.en_curso.pop(0) if len(self.en_curso) > 1 else self.en_curso[0]
            else:
                lecturas = self.muestra.base("ipc.json")["lecturas"]
                lecturas.update(self.muestra.cambios("ipc.json").get("lecturas", {}))
                valor = lecturas[clase]
            return {"ok": True, "answer": {"state": "present", "value": {"items": valor}}}
        raise mac_escenario.Falla(f"orden no prevista en el ensayo: {body['command']}")

    def suscribir(self, temas: list[str], plazo_s: float, latido_s: float | None = None):
        if self.siguiente >= len(self.esperas):
            raise TimeoutError
        espera = self._espera(self.esperas[self.siguiente])
        self.siguiente += 1
        if sorted(espera["temas"]) != sorted(temas):
            raise mac_escenario.Falla(
                f"el ensayo esperaba «{espera['que']}» con {espera['temas']}, no {temas}"
            )
        if ("lecturas" in espera) != (latido_s is not None):
            raise mac_escenario.Falla(
                f"«{espera['que']}»: el ensayo y la espera no están de acuerdo en si lee"
            )
        self.en_curso = list(espera.get("lecturas") or [])
        try:
            texto = json.dumps(espera["frames"])
            for frame in json.loads(texto.replace("{mensaje}", self.muestra.mensaje)):
                yield frame["topic"], frame["knowledge"]
            # The beats of a reading wait: one per read still to answer.
            for _ in range(len(self.en_curso)):
                yield None, None
        finally:
            self.en_curso = None
        raise TimeoutError


def filtrar(markdown: str, consulta: str) -> str:
    """cua-driver's `query`: matching lines and their ancestors."""
    lineas = markdown.splitlines()
    aguja = consulta.lower()
    quedan = set()
    for posicion, linea in enumerate(lineas):
        if aguja in linea.lower():
            quedan.add(posicion)
            sangria = len(linea) - len(linea.lstrip())
            for anterior in range(posicion - 1, -1, -1):
                otra = len(lineas[anterior]) - len(lineas[anterior].lstrip())
                if otra < sangria:
                    quedan.add(anterior)
                    sangria = otra
    return "\n".join(lineas[i] for i in sorted(quedan)) + "\n"


class CuaEnsayo:
    """CuaDriver's command line: `status`, `serve` through `open`, `call` and
    `stop`. A call answers from the tree of the current screen and moves to
    another screen as the example's transiciones.json says; the example's
    cua.json can make it refuse a tool, as the real driver words it."""

    def __init__(self, muestra: Muestra, registros: Path):
        self.muestra = muestra
        self.registros = registros
        self.transiciones = muestra.base("transiciones.json")
        self.transiciones.update(muestra.cambios("transiciones.json"))
        self.rechazos = muestra.cambios("cua.json").get("rechazos", {})
        self.pantalla = "inicio"
        self.en_marcha = False
        self.modo = "bounded"
        self.serve: list[str] | None = None
        self.parado = False

    def arrancar(self, orden: list[str]) -> tuple[int, str, str]:
        self.serve = orden
        self.en_marcha = True
        return 0, "", ""

    def orden(self, argumentos: list[str]) -> tuple[int, str, str]:
        if argumentos == ["status"]:
            if not self.en_marcha:
                return 1, "", "Cua Driver daemon is not running"
            manifiesto = "true" if self.modo == "bounded" else "false"
            estado = (
                "Cua Driver daemon is running\n"
                "  socket: /Users/ensayo/Library/Caches/cua-driver/cua-driver.sock\n"
                f"  pid: {PID_CUA}\n"
                f"  permission mode: {self.modo} (launch)\n"
                f"  capability manifest: configured={manifiesto}, "
                f"approved_at_startup={manifiesto}, valid={manifiesto}\n"
            )
            return 0, estado, ""
        if argumentos[:1] == ["--expected-pid"] and argumentos[2:] == ["stop"]:
            if int(argumentos[1]) == PID_CUA:
                self.en_marcha = False
                self.parado = True
            return 0, "", ""
        if argumentos[:1] == ["call"] and len(argumentos) == 3:
            if not self.en_marcha:
                return 1, "", "Cua Driver daemon is not running"
            herramienta, datos = argumentos[1], json.loads(argumentos[2])
            if herramienta in self.rechazos:
                return 1, "", self.rechazos[herramienta]
            try:
                return 0, json.dumps(self.llamar(herramienta, datos), indent=2) + "\n", ""
            except FalloPaso as causa:
                return 1, "", str(causa)
        return 1, "", f"orden de cua-driver no prevista en el ensayo: {argumentos}"

    def _registrar(self, nombre: str, fila: dict) -> None:
        """A report-log line, stamped now, as the daemon writes it."""
        # Rounded up to the millisecond the log keeps, so the line never
        # seems to come before what wrote it.
        instante = datetime.fromtimestamp(math.ceil(time.time() * 1000) / 1000, UTC)
        fila = dict(fila, ts=instante.isoformat(timespec="milliseconds").replace("+00:00", "Z"))
        with open(self.registros / nombre, "a", encoding="utf-8") as registro:
            registro.write(json.dumps(fila) + "\n")
        # A dry-run step lasts well under a millisecond; this one lasts past
        # the stamped instant, as a real one does.
        time.sleep(0.005)

    def _arbol(self, ventana: int) -> str:
        nombre = "panel" if ventana == PANEL else self.pantalla
        return self.muestra.texto(f"pantallas/{nombre}.md")

    def llamar(self, tool: str, arguments: dict) -> dict:
        if tool not in mac_escenario.HERRAMIENTAS:
            raise FalloPaso(f"tool '{tool}' is outside the capability manifest")
        if arguments.get("pid") != PID:
            raise FalloPaso(
                f"desktop application pid {arguments.get('pid')} is outside the capability manifest"
            )
        if tool == "list_windows":
            return {
                "windows": [
                    {
                        "pid": PID,
                        "window_id": PRINCIPAL,
                        "app_name": "Orbit Pruebas",
                        "title": "Orbit",
                        # As on the real Mac (run 7): on another Space.
                        "is_on_screen": False,
                    },
                    # Never the floating panel: cua-driver lists layer 0 only
                    # (list_windows.rs), as the real Mac showed on 2026-10-17.
                ]
            }
        markdown = self._arbol(arguments["window_id"])
        if tool == "get_window_state":
            lineas = markdown.splitlines()
            aviso = ""
            if len(lineas) > MAX_NODOS:
                # The walk stops at the cap and says so after the tree.
                markdown = "\n".join(lineas[:MAX_NODOS]) + "\n"
                aviso = (
                    f"\n⚠️  AX tree truncated at {MAX_NODOS} nodes (app has a very large "
                    "accessibility tree — Arc, Electron, or similar). Element indices above "
                    "are still valid. Use pixel clicks for elements not visible in this "
                    "partial tree."
                )
            if "query" in arguments:
                markdown = filtrar(markdown, arguments["query"])
            return {"tree_markdown": markdown + aviso, "elements": []}
        if tool == "press_key":
            destino = self.transiciones.get(self.pantalla, {}).get(f"press_key:{arguments['key']}")
            if destino is not None:
                self.pantalla = destino
            return {"ok": True}
        import arbol

        nodo = next(
            (n for n in arbol.leer(markdown) if n.index == arguments.get("element_index")), None
        )
        if nodo is None:
            raise FalloPaso(
                f"{tool}: no hay elemento {arguments.get('element_index')} en «{self.pantalla}»"
            )
        if tool == "type_text":
            self.muestra.mensaje = arguments["text"]
        clave = f"{tool}:{nodo.texto()}"
        destino = self.transiciones.get(self.pantalla, {}).get(clave)
        if isinstance(destino, dict):
            # A transition can also leave a line in a report log, stamped now,
            # as the daemon would while that click happens.
            for nombre, fila in destino.get("registro", {}).items():
                self._registrar(nombre, fila)
            destino = destino["a"]
        if destino is not None:
            self.pantalla = destino
        return {"ok": True}


def puerto_libre() -> int:
    with socket.socket() as sonda:
        sonda.bind(("127.0.0.1", 0))
        return sonda.getsockname()[1]


def ejecutar(nombre: str, directorio: Path | None = None) -> tuple[list[dict], Path]:
    muestra = Muestra(nombre)
    raiz = Path(tempfile.mkdtemp(prefix="orbit-ensayo-")) if directorio is None else directorio
    # The Mac's home, with Orbit Pruebas «installed».
    casa = raiz / "mac"
    rutas_mac = mac_escenario.Rutas(casa)
    (rutas_mac.app / "Contents").mkdir(parents=True)
    (rutas_mac.app / "Contents" / "Info.plist").write_bytes(
        plistlib.dumps({"CFBundleIdentifier": mac_escenario.BUNDLE_ID})
    )
    mac = prueba.MacEnProceso(
        mac_escenario.Mac(rutas_mac, SistemaEnsayo(muestra, rutas_mac), OrbitEnsayo(muestra))
    )
    rutas = hub_pruebas.Rutas(raiz / "orbit-pruebas")
    hub_pruebas.preparar_secretos(rutas)
    clave = rutas.secretos / "procesamiento-falso.key"
    hub_pruebas.escribir_privado(clave, b"ensayo")
    # Nothing listens on port 9 (discard), and «.invalid» never resolves: the
    # dry run's hub has no recordings, so neither is ever called.
    proveedores = {
        "whisper": "http://127.0.0.1:9",
        "procesamiento": "https://procesamiento.invalid/v1/chat/completions",
        "clave_procesamiento": str(clave),
    }
    ejecucion = raiz / "ejecucion"

    corrida = prueba.Prueba(
        mac,
        rutas,
        "127.0.0.1",
        puerto_libre(),
        ejecucion,
        hermes=False,
        proveedores=proveedores,
    )
    # A replayed screen never changes: reading it again could not help.
    corrida.plazos["pantalla"] = (0, "en el ensayo las pantallas no cambian solas")
    return corrida.ejecutar(), ejecucion
