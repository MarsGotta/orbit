"""The Mac side of the real test (SIM-25): what runs on Mars's Mac, over SSH.

The orchestrator on the test server copies this file and the script of the meeting to
`~/.orbit-pruebas/prueba/` on the Mac and calls one subcommand at a time; each
prints one JSON object and exits 0, or prints `{"error": …}` and exits 1.

    preparar --max S          checks, BlackHole as the default input, clean data, opens Orbit Pruebas
    emparejar URL CODIGO NOMBRE
    esperar QUE [ARG] --max S waits on a fact Orbit Pruebas publishes or answers on its socket
    leer detalle ID | leer sesiones
    hablar PARTE              says one part of the script into BlackHole
    nota-de-voz               a voice note to Deimos: voice_start, one phrase into BlackHole, voice_end
    orden call_start|call_stop  starts or stops a call with Deimos
    restaurar --max S         closes Orbit Pruebas and puts the previous input back
    registros                 the report logs of Orbit Pruebas and the call's audio lines of daemon.log
    motor-arrancar --max S    starts CuaDriver, bounded to Orbit Pruebas, under its own identity
    motor-llamar TOOL JSON    one CuaDriver tool call
    motor-parar --max S       stops the CuaDriver this test started, by its pid

The clicks and the readings of the interface go through CuaDriver, the
standalone computer-use driver (cua-driver 0.28.2, /Applications/CuaDriver.app)
with its own macOS permissions, chosen by Mars after Orbit Pruebas's own engine
refused the test's readings in run 5 (SIM-25.md, decision 5). The test starts
CuaDriver bounded by a capability manifest that allows five tools on Orbit
Pruebas alone, targets only Orbit Pruebas's exact pid, and stops it.

It never touches Mars's Orbit, its data or its Keychain: Orbit Pruebas has its
own bundle, data directory and Keychain service (orbit_bundle, feature
`pruebas`). It stops only the processes of the Orbit Pruebas bundle, found by
their exact executable path, never by name.

Runs with the Mac's python3, so it stays within Python 3.9.
"""

from __future__ import annotations

import json
import os
import plistlib
import re
import select
import shutil
import signal
import socket
import subprocess
import sys
import time
import uuid
from pathlib import Path

BUNDLE_ID = "app.orbit.companion.pruebas"
# The last part of orbit_bundle::DATA_DIRECTORY with the feature `pruebas`
# (directories::ProjectDirs on macOS); a guard test compares both.
DIRECTORIO_DE_DATOS = "com.MarsGotta.OrbitPruebas"
GUION = Path(__file__).resolve().with_name("guion.txt")
# Lists a process's windows on every layer (CuaDriver lists layer 0 only).
VENTANAS = Path(__file__).resolve().with_name("ventanas.swift")
BLACKHOLE = "BlackHole 2ch"
# What CuaDriver may do for the test, and nothing more: find Orbit Pruebas's
# windows, read them, click, type and choose a value.
HERRAMIENTAS = ("list_windows", "get_window_state", "click", "type_text", "press_key")
# Where the official installer puts CuaDriver's daemon; a pid is the test's
# only if its executable is in there.
CUA_APP = "/Applications/CuaDriver.app/"
VIGENCIA = "3h"
INACTIVIDAD = "1h"
# How CuaDriver words a refusal on permissions (authorization.rs,
# session_manifest.rs, the macOS permission gate).
RECHAZOS_DE_PERMISO = (
    "Permission denied",
    "outside the capability manifest",
    "capability manifest denies",
    "permission",
)


class Rutas:
    """Where everything lives, from the home directory (a temporary one in the dry run)."""

    def __init__(self, home: Path):
        self.app = home / "Applications" / "Orbit Pruebas.app"
        self.datos = home / "Library" / "Application Support" / DIRECTORIO_DE_DATOS
        self.socket = self.datos / "orbit.sock"
        self.registros = self.datos / "logs"
        self.propio = home / ".orbit-pruebas"
        self.entrada_anterior = self.propio / "entrada-anterior"
        # The helper latches the privacy stop in a file every Orbit on this
        # Mac shares (PrivacyLatch.swift); with it set, Orbit Pruebas starts
        # stopped.
        # CuaDriver's command line, from its official installer, and what the
        # test hands it: the manifest it is bounded by and the pid it started.
        self.cua = home / ".local" / "bin" / "cua-driver"
        self.manifiesto = self.propio / "cua-manifiesto.yaml"
        self.pid_cua = self.propio / "cua-driver.pid"
        self.cerrojo_privacidad = (
            home / "Library" / "Application Support" / "Orbit" / "native-privacy-stop"
        )


CUA_ENGINE_RELATIVE_PATH = "Helpers/Orbit Engine.app/Contents/MacOS/cua-driver"
MIEMBROS = (
    "Contents/MacOS/Orbit",
    "Contents/MacOS/OrbitNative",
    "Contents/" + CUA_ENGINE_RELATIVE_PATH,
)
OFICIAL = re.compile(r"/Orbit\.app(\.anterior-[^/]+)?/Contents/MacOS/Orbit$")
# One request line to the daemon (crates/orbit-ipc, PROTOCOL_VERSION).
PROTOCOLO = 1
# Between two attempts to reach a socket that does not exist yet. Only
# spaces the attempts; the bound is the caller's.
INTERVALO_SOCKET_S = 0.2
# Between two reads of a wait whose fact only a read says (the meeting saved
# and listed, its note final: SIM-41 left the `notes` topic a head of
# revisions). A change of that head reads at once; this only covers a change
# the head does not name, and asks the hub at most once every two seconds,
# nothing next to the waits' bounds (minutes) or the hub's own pace (a
# transcription group every 20-30 s). It spaces reads; the bound is the wait's.
INTERVALO_LECTURA_S = 2.0
# One line of the socket (crates/orbit-ipc, its 1 MiB frame): a line longer
# than this is not the daemon's.
MAX_LINEA_BYTES = 1024 * 1024
# How the daemon names Mars's microphone (orbit_core::notes::owner_name): the
# account's full name, or «Yo» when it has none or it is longer than the
# hub's MAX_PERSON_NAME_CHARS. A guard test compares both copies.
NOMBRE_POR_DEFECTO = "Yo"
MAX_NOMBRE_PERSONA = 128
# SIM-27: what the voice note says. Not a line of guion.txt: that is the
# meeting, and check 1 counts every one of its words in the meeting's
# transcript. Deimos is asked for one word so the answer is short.
NOTA_DE_VOZ = "Deimos, contesta sólo con la palabra recibido."
# The beats around the phrase. Not waits on a fact: no topic says when the
# microphone delivers its first frame after voice_start (voice_active turns
# true when the order is taken, before the helper opens the stream) or when
# the last frame said has reached the daemon before voice_end. These are the
# values of the probe that worked on the Mac on 2026-10-17, unmeasured
# beyond it. A beat too short shows as words missing from the transcript,
# which check 11 names; it never makes the check pass.
PAUSA_ANTES_DE_HABLAR_S = 0.5
PAUSA_DESPUES_DE_HABLAR_S = 0.7
# A call's length when it starts (`call_start`, 1..60 minutes). Two minutes
# cover the wait for «listening» (30 s, prueba.py plazos) plus the SSH margin
# of that wait and of the `call_stop` after it (MARGEN_SSH_S, 30 s each);
# `call_stop` ends it well before, and a call that outlived the step would
# end by itself, not stay open.
DURACION_LLAMADA_MIN = 2
# The orders the test sends besides `pair`, and their bodies
# (orbit_core::protocol::Command).
ORDENES = {
    "voice_start": {"command": "voice_start"},
    "voice_end": {"command": "voice_end"},
    "call_start": {"command": "call_start", "duration_minutes": DURACION_LLAMADA_MIN},
    "call_stop": {"command": "call_stop"},
}
# The daemon.log events the test brings back (call.rs): the call's
# microphone as it opened and what it heard. Only numbers; daemon.log's
# other lines may name paths on the Mac and stay there.
EVENTOS_DE_LLAMADA = ("call_audio_opened", "call_audio_heard")
# The months as the app writes a conversation's title (es-ES, long month;
# session-title.ts): in code, so the Mac's locale cannot change them.
MESES = (
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
)


class Falla(Exception):
    """A fact that ends a wait or a step with a cause, in Spanish."""


# ── The system: commands and processes ─────────────────────────────────────


class Sistema:
    def correr(self, *orden: str) -> tuple[int, str, str]:
        """Runs a command to its end: exit code, stdout and stderr."""
        hecho = subprocess.run(list(orden), capture_output=True, text=True, check=False)
        return hecho.returncode, hecho.stdout, hecho.stderr

    def ejecutar(self, *orden: str) -> str:
        codigo, salida, errores = self.correr(*orden)
        if codigo != 0:
            raise Falla(f"«{orden[0]}» terminó con {codigo}: {errores.strip()[:300]}")
        return salida

    def existe_orden(self, nombre: str) -> bool:
        return shutil.which(nombre) is not None

    def procesos(self) -> list[tuple[int, str]]:
        salida = self.ejecutar("ps", "-axo", "pid=,comm=")
        filas = []
        for linea in salida.splitlines():
            partes = linea.strip().split(None, 1)
            if len(partes) == 2 and partes[0].isdigit():
                filas.append((int(partes[0]), partes[1]))
        return filas

    def senal(self, pid: int, numero: int) -> None:
        try:
            os.kill(pid, numero)
        except ProcessLookupError:
            pass

    def esperar_salida(self, pids: list[int], max_s: float) -> list[int]:
        """Waits on the exit of each process (kqueue NOTE_EXIT); returns the
        ones still alive at the bound."""
        cola = select.kqueue()
        vivos = set()
        for pid in pids:
            evento = select.kevent(
                pid,
                select.KQ_FILTER_PROC,
                select.KQ_EV_ADD | select.KQ_EV_ONESHOT,
                select.KQ_NOTE_EXIT,
            )
            try:
                cola.control([evento], 0, 0)
                vivos.add(pid)
            except ProcessLookupError:
                continue
        limite = time.monotonic() + max_s
        while vivos:
            resto = limite - time.monotonic()
            if resto <= 0:
                break
            for evento in cola.control(None, len(vivos), resto):
                vivos.discard(evento.ident)
        cola.close()
        return sorted(vivos)

    def ahora(self) -> float:
        return time.monotonic()

    def pausa(self, segundos: float) -> None:
        time.sleep(segundos)

    def hora_local(self, segundos: float) -> time.struct_time:
        """The Mac's local time, the clock the app names a conversation by."""
        return time.localtime(segundos)


# ── Orbit Pruebas's socket ──────────────────────────────────────────────────


class OrbitIpc:
    """One request per connection, one JSON object per line (crates/orbit-ipc)."""

    def __init__(self, ruta: Path):
        self.ruta = ruta

    def _conectar(self, plazo_s: float) -> socket.socket:
        conexion = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        conexion.settimeout(plazo_s)
        conexion.connect(str(self.ruta))
        return conexion

    @staticmethod
    def _peticion(body: dict) -> bytes:
        linea = {"v": PROTOCOLO, "id": f"sim25-{uuid.uuid4().hex[:12]}", "body": body}
        return (json.dumps(linea, ensure_ascii=False) + "\n").encode()

    def responde(self) -> bool:
        try:
            self._conectar(1).close()
            return True
        except OSError:
            return False

    def llamar(self, body: dict, plazo_s: float = 30) -> dict:
        with self._conectar(plazo_s) as conexion:
            conexion.sendall(self._peticion(body))
            linea = conexion.makefile("rb").readline()
        if not linea:
            raise Falla(f"el daemon cerró la conexión sin responder a «{body.get('command')}»")
        return json.loads(linea)

    def suscribir(self, temas: list[str], plazo_s: float, latido_s: float | None = None):
        """Yields (topic, knowledge) for the current frames and every change,
        until the bound; then raises TimeoutError. With `latido_s`, also
        yields (None, None) every `latido_s` seconds, frames or not, so a wait
        can read what no frame says.

        Lines are cut from the socket's bytes here, never by a file object's
        readline under a timeout, whose buffer a timeout may leave
        inconsistent (Python's socket.makefile)."""
        limite = time.monotonic() + plazo_s
        latido = time.monotonic() + latido_s if latido_s is not None else None
        with self._conectar(plazo_s) as conexion:
            conexion.sendall(self._peticion({"command": "subscribe_topics", "topics": temas}))
            conexion.setblocking(False)
            pendiente = b""
            primera = True
            while True:
                ahora = time.monotonic()
                if ahora >= limite:
                    raise TimeoutError
                if latido is not None and ahora >= latido:
                    latido = ahora + latido_s
                    yield None, None
                    continue
                if b"\n" not in pendiente:
                    espera = limite - ahora if latido is None else min(limite, latido) - ahora
                    listos, _, _ = select.select([conexion], [], [], max(espera, 0))
                    if not listos:
                        continue
                    trozo = conexion.recv(65536)
                    if not trozo:
                        raise Falla("el daemon cerró la suscripción")
                    pendiente += trozo
                    if b"\n" not in pendiente:
                        if len(pendiente) > MAX_LINEA_BYTES:
                            raise Falla(
                                f"una línea del daemon pasa de {MAX_LINEA_BYTES} bytes "
                                f"({len(pendiente)} sin terminar)"
                            )
                        continue
                linea, pendiente = pendiente.split(b"\n", 1)
                fila = json.loads(linea)
                if primera:
                    primera = False
                    if not fila.get("ok"):
                        raise Falla(
                            f"el daemon rechazó la suscripción: {json.dumps(fila.get('refusal'))}"
                        )
                    continue
                datos = (fila.get("frame") or fila).get("data") or {}
                if "topic" in datos:
                    yield datos["topic"], datos.get("knowledge") or {}


def _valor(conocido: dict | None):
    if conocido and conocido.get("state") == "present":
        return conocido.get("value")
    return None


# ── What each wait waits for ────────────────────────────────────────────────
#
# A predicate looks at the latest knowledge of each topic and returns the
# result when the fact is there, None to keep waiting, or raises Falla when
# what it sees means the fact will not come.


def emparejado(estado: dict, _arg):
    valor = _valor(estado.get("status")) or {}
    if valor.get("hub") == "revoked":
        raise Falla("el hub de pruebas revocó este Mac")
    return {"hub": "online"} if valor.get("hub") == "online" else None


def grabando(estado: dict, _arg):
    grabacion = (_valor(estado.get("recording")) or {}).get("recording") or {}
    if grabacion.get("phase") == "recording":
        return {k: grabacion.get(k) for k in ("id", "title", "audio", "started_at_ms")}
    return None


# Causes that do not pass while the recording goes on.
_SIN_ESCUCHA = (
    "macos_version",
    "locale_unsupported",
    "unavailable",
    "failed",
    "helper_without_phrase",
    "no_microphone",
    "privacy_stopped",
)


def escucha(estado: dict, _arg):
    voz = (_valor(estado.get("recording")) or {}).get("voice_trigger") or {}
    if voz.get("state") == "listening":
        return {"voice_trigger": "listening"}
    porque = (voz.get("why") or {}) if voz.get("state") == "not_listening" else {}
    if porque.get("cause") in _SIN_ESCUCHA:
        raise Falla(f"«Oye Deimos» no escucha: {json.dumps(porque, ensure_ascii=False)}")
    return None


def _turno(estado: dict) -> dict:
    """The assistant's turn in progress, or the last one (SIM-30): what Mars
    said and Deimos's answer; the conversation itself is the hub's."""
    return (_valor(estado.get("assistant")) or {}).get("turn") or {}


def respuesta(estado: dict, _arg):
    """The voice question once Deimos answered it or it failed. Until then it
    says what it saw: no question was heard, the question's stage, or an
    answer that has not come whole."""
    pregunta = (_valor(estado.get("recording")) or {}).get("question") or {}
    if pregunta.get("origin") != "voice":
        estado["visto"] = (
            "ninguna pregunta de voz: «Oye Deimos» no abrió ninguna"
            if not pregunta
            else f"una pregunta de origen «{pregunta.get('origin')}», no de voz"
        )
        return None
    if pregunta.get("stage") not in ("answered", "failed"):
        estado["visto"] = f"la pregunta de voz está en «{pregunta.get('stage')}»"
        return None
    resultado = {k: pregunta.get(k) for k in ("id", "question", "origin", "stage", "cause")}
    if pregunta["stage"] == "failed":
        resultado["respuesta"] = ""
        return resultado
    # The answer of the turn whose request id is the question's.
    mensaje = _turno(estado).get("answer") or {}
    if mensaje.get("id") != f"{pregunta.get('id')}:assistant" or not mensaje.get("complete"):
        estado["visto"] = (
            "la pregunta está respondida y su respuesta aún no ha llegado entera "
            f"(turno {mensaje.get('id')}, completa: {bool(mensaje.get('complete'))})"
        )
        return None
    resultado["respuesta"] = mensaje.get("text") or ""
    return resultado


# The note states after which nothing more comes without Mars
# (orbit_core::notes::NoteState): written, given up, or no model on the hub.
NOTA_FINAL = ("ready", "failed", "not_configured")


def _fila(estado: dict, ident: str) -> dict | None:
    """The meeting `ident` in the latest read of the list of Notes
    (`notes_page`, SIM-41). A list the hub could not give is noted as what
    was seen, and the wait goes on: the hub may come back."""
    lectura = estado.get("lectura") or {}
    if "rechazo" in lectura:
        estado["visto"] = f"la lista de Notas no se pudo leer: {json.dumps(lectura['rechazo'])}"
        return None
    visto = f"la grabación {ident} no está en la lista de Notas"
    for item in lectura.get("items") or []:
        if "hub_refusal" in item:
            visto = f"el hub no dio su parte de la lista: {json.dumps(item['hub_refusal'])}"
        elif item.get("recording_id") == ident:
            return item
    estado["visto"] = visto
    return None


def guardada(estado: dict, ident):
    grabacion = (_valor(estado.get("recording")) or {}).get("recording") or {}
    if grabacion.get("id") == ident and grabacion.get("phase") not in (None, "idle"):
        if grabacion.get("phase") == "interrupted":
            raise Falla("la grabación terminó interrumpida")
        estado["visto"] = f"la grabación sigue en «{grabacion.get('phase')}»"
        return None
    fila = _fila(estado, ident)
    if fila is None:
        return None
    local = fila.get("local") or {}
    if fila.get("capture") != "closed" or local.get("phase") == "recording":
        estado["visto"] = (
            f"la lista de Notas la tiene abierta (capture «{fila.get('capture')}», "
            f"local «{local.get('phase')}»)"
        )
        return None
    return {k: fila.get(k) for k in ("recording_id", "title", "duration_ms", "capture")}


def nota(estado: dict, ident):
    fila = _fila(estado, ident)
    if fila is None:
        return None
    titular = fila.get("note") or {}
    if titular.get("state") in NOTA_FINAL:
        return {
            "phase": titular["state"],
            "short_summary": titular.get("short_summary"),
            "recording_id": fila.get("recording_id"),
            "title": fila.get("title"),
        }
    estado["visto"] = (
        f"transcripción «{fila.get('transcript')}», nota «{titular.get('state', 'ninguna')}»"
    )
    return None


def chat(estado: dict, texto):
    asistente = _valor(estado.get("assistant")) or {}
    turno = asistente.get("turn") or {}
    usuario = turno.get("user") or {}
    if usuario.get("role") != "user" or usuario.get("text") != texto:
        return None
    contestacion = turno.get("answer") or {}
    if asistente.get("turn_in_flight") or not contestacion.get("complete"):
        estado["visto"] = "el turno sigue en curso"
        return None
    return {"sesion": asistente.get("session_id"), "respuesta": contestacion.get("text") or ""}


def sesion(estado: dict, ident):
    actual = (_valor(estado.get("assistant")) or {}).get("session_id")
    estado["visto"] = f"la conversación abierta es {actual}"
    return {"sesion": actual} if actual == ident else None


def nota_de_voz(estado: dict, _arg):
    """The voice note's turn once Deimos answered it, or once a fact says no
    answer will come: its transcription failed, or the turn ended without a
    complete answer. The turn with a voice note is the test's: the turn
    before it is the Assistant's typed message, which has none."""
    asistente = _valor(estado.get("assistant")) or {}
    turno = asistente.get("turn") or {}
    usuario = turno.get("user") or {}
    voz = usuario.get("voice")
    if not isinstance(voz, dict):
        estado["visto"] = "el turno en curso no es una nota de voz"
        return None
    contestacion = turno.get("answer") or {}
    resultado = {
        "rol": usuario.get("role"),
        "transcripcion": voz.get("transcript"),
        "duracion_ms": voz.get("duration_ms"),
        "audio_disponible": voz.get("audio_available", False),
        "fallo": voz.get("failure"),
        "respuesta": contestacion.get("text") or "",
        "completa": bool(contestacion.get("complete")),
    }
    if resultado["fallo"] is not None:
        return resultado
    if resultado["transcripcion"] is None:
        estado["visto"] = "la nota de voz se está transcribiendo"
        return None
    if asistente.get("turn_in_flight"):
        estado["visto"] = "Deimos no ha terminado de responder"
        return None
    return resultado


def llamada_escuchando(estado: dict, _arg):
    """The call once it listens: the daemon says `listening` only after the
    helper confirmed the microphone open with voice processing (CallReady).
    A call seen and then gone ended before it listened."""
    conocida = (_valor(estado.get("assistant")) or {}).get("call") or {}
    if conocida.get("state") != "present":
        estado["visto"] = f"la llamada es «{conocida.get('state')}»"
        return None
    llamada = conocida.get("value")
    if llamada is None:
        if estado.get("fase_llamada") is not None:
            raise Falla(
                "la llamada terminó sin llegar a «listening» "
                f"(última fase «{estado['fase_llamada']}»)"
            )
        estado["visto"] = "no hay llamada"
        return None
    estado["fase_llamada"] = llamada.get("phase")
    if llamada.get("phase") == "listening":
        return {k: llamada.get(k) for k in ("id", "phase", "started_at_ms", "expires_at_ms")}
    estado["visto"] = f"la llamada sigue en «{llamada.get('phase')}»"
    return None


def _lista_de_notas(_arg) -> dict:
    return {"kind": "notes_page", "cursor": None}


# What each wait subscribes to, its predicate, and, for a fact only a read
# says, the read it repeats (on a change of the `notes` head and at least
# every INTERVALO_LECTURA_S).
ESPERAS = {
    "emparejado": (("status",), emparejado, None),
    "grabando": (("recording",), grabando, None),
    "escucha": (("recording",), escucha, None),
    "respuesta": (("recording", "assistant"), respuesta, None),
    "guardada": (("recording", "notes"), guardada, _lista_de_notas),
    "nota": (("notes",), nota, _lista_de_notas),
    "chat": (("assistant",), chat, None),
    "sesion": (("assistant",), sesion, None),
    "nota-de-voz": (("assistant",), nota_de_voz, None),
    "llamada-escuchando": (("assistant",), llamada_escuchando, None),
}


def titulo_de_sesion(hora: time.struct_time) -> str:
    """A conversation's title as the app writes it from its start
    (session-title.ts, SIM-30): «17 de octubre de 2026, 09:53»."""
    return (
        f"{hora.tm_mday} de {MESES[hora.tm_mon - 1]} de {hora.tm_year}, "
        f"{hora.tm_hour:02d}:{hora.tm_min:02d}"
    )


def nombre_del_propietario(completo: str | None) -> str:
    """How the daemon names Mars's microphone from the account's full name
    (orbit_core::notes::owner_name, fed by `id -F`)."""
    nombre = (completo or "").split(",")[0].strip()
    if (
        not nombre
        or len(nombre) > MAX_NOMBRE_PERSONA
        or any(ord(c) < 32 or 127 <= ord(c) < 160 for c in nombre)
    ):
        return NOMBRE_POR_DEFECTO
    return nombre


# ── The script of the meeting ───────────────────────────────────────────────


def partes_del_guion(texto: str) -> list[list[tuple[str, str]]]:
    """The three parts, each a list of (voice, line); «---» separates them."""
    partes: list[list[tuple[str, str]]] = [[]]
    for linea in texto.splitlines():
        linea = linea.strip()
        if not linea or linea.startswith("#"):
            continue
        if linea == "---":
            partes.append([])
            continue
        voz, separador, dicho = linea.partition(":")
        if not separador or voz.strip() not in ("A", "B") or not dicho.strip():
            raise Falla(f"línea del guion sin «A:» ni «B:»: {linea[:60]}")
        partes[-1].append((voz.strip(), dicho.strip()))
    if len(partes) != 3 or not all(partes):
        raise Falla(f"el guion debe tener tres partes separadas por «---» y tiene {len(partes)}")
    return partes


PARTES = {"parte1": 0, "pregunta": 1, "parte2": 2}


def voces_en_espanol(listado: str) -> list[str]:
    """Spanish voices from `say -v '?'`, Spain first: «Mónica  es_ES  # Hola…»."""
    encontradas = []
    for linea in listado.splitlines():
        hallada = re.match(r"^(?P<nombre>.+?)\s+(?P<idioma>[a-z]{2}_[A-Z]{2})\s+#", linea)
        if hallada and hallada["idioma"].startswith("es_"):
            encontradas.append((hallada["idioma"] != "es_ES", hallada["nombre"].strip()))
    return [nombre for _, nombre in sorted(encontradas, key=lambda par: par[0])]


# ── The steps ───────────────────────────────────────────────────────────────


class Mac:
    def __init__(self, rutas: Rutas, sistema: Sistema | None = None, orbit: OrbitIpc | None = None):
        self.rutas = rutas
        self.sistema = sistema or Sistema()
        self.orbit = orbit or OrbitIpc(rutas.socket)

    def _nuestros(self) -> list[int]:
        rutas = {str(self.rutas.app / miembro) for miembro in MIEMBROS}
        return [pid for pid, orden in self.sistema.procesos() if orden in rutas]

    def _cerrar_nuestros(self, max_s: float) -> dict:
        pids = self._nuestros()
        app = [
            pid
            for pid, orden in self.sistema.procesos()
            if orden == str(self.rutas.app / MIEMBROS[0])
        ]
        for pid in app:
            # The app only: its daemon runs inside it (SIM-51) and hears the
            # SIGTERM, closes in order and takes the helper and the engine
            # with it; the app follows.
            self.sistema.senal(pid, signal.SIGTERM)
        quedan = self.sistema.esperar_salida(pids, max_s)
        for pid in quedan:
            self.sistema.senal(pid, signal.SIGKILL)
        return {"cerrados": len(pids), "forzados": len(quedan)}

    def _restaurar_entrada(self) -> str | None:
        if not self.rutas.entrada_anterior.exists():
            return None
        anterior = self.rutas.entrada_anterior.read_text(encoding="utf-8").strip()
        # Real run of 2026-10-21: the input saved was a pair of headphones no
        # longer connected; SwitchAudioSource refused it and every later run
        # stopped while preparing. Such an input cannot be put back.
        entradas = self.sistema.ejecutar("SwitchAudioSource", "-a", "-t", "input").splitlines()
        if anterior not in [e.strip() for e in entradas]:
            self.rutas.entrada_anterior.unlink()
            return f"«{anterior}» ya no está conectada; se queda la entrada actual"
        self.sistema.ejecutar("SwitchAudioSource", "-t", "input", "-s", anterior)
        self.rutas.entrada_anterior.unlink()
        return anterior

    def preparar(self, max_s: float) -> dict:
        rutas = self.rutas
        info = rutas.app / "Contents" / "Info.plist"
        if not info.exists():
            raise Falla(f"Orbit Pruebas no está instalada en {rutas.app}")
        identificador = plistlib.loads(info.read_bytes()).get("CFBundleIdentifier")
        if identificador != BUNDLE_ID:
            raise Falla(f"{rutas.app} es «{identificador}», no {BUNDLE_ID}")
        oficiales = [orden for _, orden in self.sistema.procesos() if OFICIAL.search(orden)]
        if oficiales:
            raise Falla(
                "Orbit está abierta; ciérrala antes de la prueba (comparten los atajos globales "
                f"del helper): {oficiales[0]}"
            )
        if rutas.cerrojo_privacidad.exists():
            raise Falla(
                f"la parada de privacidad está echada ({rutas.cerrojo_privacidad}); rearma Orbit primero"
            )
        for orden in ("SwitchAudioSource", "say", "open"):
            if not self.sistema.existe_orden(orden):
                raise Falla(f"falta «{orden}» (brew install switchaudio-osx)")
        entradas = self.sistema.ejecutar("SwitchAudioSource", "-a", "-t", "input").splitlines()
        if BLACKHOLE not in [e.strip() for e in entradas]:
            raise Falla(f"no hay entrada «{BLACKHOLE}» (brew install blackhole-2ch): {entradas}")
        voces = voces_en_espanol(self.sistema.ejecutar("say", "-v", "?"))
        if not voces:
            raise Falla("macOS no tiene ninguna voz en español para «say»")
        partes_del_guion(GUION.read_text(encoding="utf-8"))
        # What a run that died left running: its CuaDriver, its Orbit Pruebas.
        self.motor_parar(max_s)
        previa = self._cerrar_nuestros(max_s)
        # A run that could not restore the input left the original here.
        self._restaurar_entrada()
        rutas.propio.mkdir(mode=0o700, exist_ok=True)
        actual = self.sistema.ejecutar("SwitchAudioSource", "-c", "-t", "input").strip()
        rutas.entrada_anterior.write_text(actual + "\n", encoding="utf-8")
        self.sistema.ejecutar("SwitchAudioSource", "-t", "input", "-s", BLACKHOLE)
        if rutas.datos.exists():
            shutil.rmtree(rutas.datos)
        self.sistema.ejecutar("open", str(rutas.app))
        inicio = self.sistema.ahora()
        while not self.orbit.responde():
            if self.sistema.ahora() - inicio > max_s:
                raise Falla(f"el daemon de Orbit Pruebas no abrió su socket en {max_s:.0f} s")
            time.sleep(INTERVALO_SOCKET_S)
        macos = self.sistema.ejecutar("sw_vers", "-productVersion").strip()
        # The name the microphone's turns must carry, taken where the daemon
        # takes it, never from the hub, which only repeats what the Mac sent.
        codigo, completo, _ = self.sistema.correr("id", "-F")
        return {
            "voces": voces[:2],
            "entrada_anterior": actual,
            "macos": macos,
            "cerradas_antes": previa,
            "propietario": nombre_del_propietario(completo if codigo == 0 else None),
        }

    def emparejar(self, url: str, codigo: str, nombre: str) -> dict:
        respuesta_ = self.orbit.llamar(
            {"command": "pair", "hub_url": url, "code": codigo, "device_name": nombre}
        )
        if not respuesta_.get("ok"):
            raise Falla(
                f"no se emparejó: {json.dumps(respuesta_.get('refusal'), ensure_ascii=False)}"
            )
        return {"emparejado": True}

    def esperar(self, que: str, arg, max_s: float) -> dict:
        """Waits on a fact: on the topic frames, and for a fact only a read
        says, on that read, repeated when the `notes` head moves and at least
        every INTERVALO_LECTURA_S, up to the bound. A wait that times out
        says what it saw last."""
        temas, predicado, lectura = ESPERAS[que]
        inicio = self.sistema.ahora()
        estado: dict = {}
        latido = INTERVALO_LECTURA_S if lectura is not None else None
        try:
            for tema, conocido in self.orbit.suscribir(list(temas), max_s, latido):
                if tema is not None:
                    estado[tema] = conocido
                if lectura is not None and (
                    tema is None or tema == "notes" or "lectura" not in estado
                ):
                    estado["lectura"] = self._leer(lectura(arg))
                hallado = predicado(estado, arg)
                if hallado is not None:
                    return {"valor": hallado, "segundos": round(self.sistema.ahora() - inicio, 3)}
        except TimeoutError:
            pass
        visto = f"; lo último: {estado['visto']}" if estado.get("visto") else ""
        raise Falla(f"«{que}» no llegó en {max_s:.0f} s{visto}")

    def _leer(self, lectura: dict) -> dict:
        """One read: its items, or the daemon's answer when it has none."""
        respuesta_ = self.orbit.llamar({"command": "read", "read": lectura})
        respuestas = respuesta_.get("answer") or {}
        if respuestas.get("state") != "present":
            return {"rechazo": respuestas or respuesta_.get("refusal")}
        return {"items": respuestas["value"].get("items") or []}

    def leer(self, que: str, ident: str | None = None) -> dict:
        if que == "detalle":
            lectura = {"kind": "note_detail", "id": ident, "cursor": None}
        elif que == "sesiones":
            lectura = {"kind": "sessions", "cursor": None}
        else:
            raise Falla(f"lectura desconocida «{que}»")
        leido = self._leer(lectura)
        if "rechazo" in leido:
            raise Falla(
                f"el daemon no tiene «{que}»: "
                f"{json.dumps(leido['rechazo'], ensure_ascii=False)[:300]}"
            )
        if que == "sesiones":
            # The app names a conversation by its start on this Mac's clock
            # (SIM-30); the test finds it in the list by that same name.
            for fila in leido["items"]:
                inicio = fila.get("created_at_ms")
                if isinstance(inicio, int):
                    fila["titulo"] = titulo_de_sesion(self.sistema.hora_local(inicio / 1000))
        return leido

    def hablar(self, parte: str) -> dict:
        lineas = partes_del_guion(GUION.read_text(encoding="utf-8"))[PARTES[parte]]
        voces = voces_en_espanol(self.sistema.ejecutar("say", "-v", "?"))
        if not voces:
            raise Falla("macOS no tiene ninguna voz en español para «say»")
        asignadas = {"A": voces[0], "B": voces[1] if len(voces) > 1 else voces[0]}
        inicio = self.sistema.ahora()
        for voz, dicho in lineas:
            self.sistema.ejecutar("say", "-v", asignadas[voz], "-a", BLACKHOLE, dicho)
        return {"lineas": len(lineas), "segundos": round(self.sistema.ahora() - inicio, 3)}

    def orden(self, nombre: str) -> dict:
        """One of the test's orders to the daemon; a refusal is said with its
        cause."""
        if nombre not in ORDENES:
            raise Falla(f"orden desconocida «{nombre}»")
        respuesta_ = self.orbit.llamar(ORDENES[nombre])
        if not respuesta_.get("ok"):
            causa = respuesta_.get("refusal") or respuesta_.get("error")
            raise Falla(
                f"el daemon rechazó «{nombre}»: {json.dumps(causa, ensure_ascii=False)[:300]}"
            )
        return {"orden": nombre}

    def nota_de_voz(self) -> dict:
        """A voice note, as Mars records one: start it, say the phrase into
        BlackHole, end it. Ended even when the phrase could not be said."""
        voces = voces_en_espanol(self.sistema.ejecutar("say", "-v", "?"))
        if not voces:
            raise Falla("macOS no tiene ninguna voz en español para «say»")
        self.orden("voice_start")
        inicio = self.sistema.ahora()
        try:
            self.sistema.pausa(PAUSA_ANTES_DE_HABLAR_S)
            self.sistema.ejecutar("say", "-v", voces[0], "-a", BLACKHOLE, NOTA_DE_VOZ)
            self.sistema.pausa(PAUSA_DESPUES_DE_HABLAR_S)
        finally:
            self.orden("voice_end")
        return {
            "frase": NOTA_DE_VOZ,
            "voz": voces[0],
            "segundos": round(self.sistema.ahora() - inicio, 3),
        }

    def restaurar(self, max_s: float) -> dict:
        cua = self.motor_parar(max_s)
        cerrados = self._cerrar_nuestros(max_s)
        return {
            "cua_driver": cua,
            "orbit_pruebas": cerrados,
            "entrada_restaurada": self._restaurar_entrada(),
        }

    # ── CuaDriver ──

    def _pid_app(self) -> int:
        app = str(self.rutas.app / MIEMBROS[0])
        pids = [pid for pid, orden in self.sistema.procesos() if orden == app]
        if len(pids) != 1:
            raise Falla(f"hay {len(pids)} procesos de {app}, no uno")
        return pids[0]

    def _estado_cua(self) -> str | None:
        """`cua-driver status`, or None when no CuaDriver daemon is running."""
        codigo, salida, _ = self.sistema.correr(str(self.rutas.cua), "status")
        return salida if codigo == 0 else None

    def manifiesto(self, pid_app: int) -> str:
        """The capability manifest CuaDriver is started with: the five tools
        the test uses, on Orbit Pruebas only — its bundle identifier, its
        executable and the pid it runs with now — and nothing else: no other
        app, no display, no browser, no files, no processes to end
        (session_manifest.rs, cua-driver-rs-v0.28.2)."""
        ejecutable = json.dumps(str(self.rutas.app / MIEMBROS[0]), ensure_ascii=False)
        herramientas = ", ".join(HERRAMIENTAS)
        return (
            "# CuaDriver para la prueba real de SIM-25: sólo Orbit Pruebas, sólo estas herramientas.\n"
            "version: 3\n"
            "# The longest run is under three hours (the note waits up to 52 min).\n"
            f"expires_after: {VIGENCIA}\n"
            "# The longest silence between two calls is the wait for the note.\n"
            f"idle_timeout: {INACTIVIDAD}\n"
            "resources:\n"
            "  apps:\n"
            f"    - bundle_id: {BUNDLE_ID}\n"
            "      windows: all\n"
            "      terminate: deny\n"
            f"    - executable: {ejecutable}\n"
            "      windows: all\n"
            "      terminate: deny\n"
            "  desktop:\n"
            f"    applications: [{pid_app}]\n"
            "allow:\n"
            f"  tools: [{herramientas}]\n"
        )

    def activar(self) -> dict:
        """Brings Orbit Pruebas to the front and to the current Space.

        On the real Mac (run 8) its window opened on another Space, where
        CuaDriver resolves no accessibility window and returns an empty tree
        on purpose. `open` on a running app activates it, which moves the
        view to its window."""
        self.sistema.ejecutar("open", str(self.rutas.app))
        return {"activada": True}

    def flotantes(self) -> dict:
        """Orbit Pruebas's windows on screen above the normal layer: the
        floating panel. CuaDriver's `list_windows` lists layer-0 windows only
        (cua-driver 0.28.2, list_windows.rs), so the panel, always on top,
        never appears there; WindowServer lists it (ventanas.swift)."""
        pid_app = self._pid_app()
        filas = [
            json.loads(linea)
            for linea in self.sistema.ejecutar("swift", str(VENTANAS), str(pid_app)).splitlines()
            if linea.strip()
        ]
        return {
            "ventanas": [
                {"pid": pid_app, **fila}
                for fila in filas
                if fila.get("layer", 0) > 0 and fila.get("is_on_screen")
            ]
        }

    def motor_arrancar(self, max_s: float) -> dict:
        """Starts CuaDriver's daemon under its own macOS identity (LaunchServices,
        so its own Accessibility and Screen Recording grants apply), bounded by
        the manifest, and checks that it took it."""
        if self._estado_cua() is not None:
            raise Falla(
                "ya hay un CuaDriver en marcha; la prueba no lo toca: ciérralo con "
                "«cua-driver stop» y vuelve a lanzarla"
            )
        pid_app = self._pid_app()
        self.rutas.propio.mkdir(mode=0o700, exist_ok=True)
        descriptor = os.open(self.rutas.manifiesto, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as fichero:
            fichero.write(self.manifiesto(pid_app))
        self.sistema.ejecutar(
            "open", "-n", "-g", "-a", "CuaDriver", "--args", "serve",
            "--permission-mode", "bounded",
            "--capability-manifest", str(self.rutas.manifiesto),
            "--approve-capability-manifest",
        )  # fmt: skip
        inicio = self.sistema.ahora()
        while True:
            estado = self._estado_cua()
            pid = re.search(r"^\s*pid: (\d+)$", estado or "", re.MULTILINE)
            if pid is not None:
                break
            if self.sistema.ahora() - inicio > max_s:
                raise Falla(f"CuaDriver no respondió en {max_s:.0f} s tras arrancarlo")
            time.sleep(INTERVALO_SOCKET_S)
        self.rutas.pid_cua.write_text(pid.group(1) + "\n", encoding="utf-8")
        modo = re.search(r"permission mode: (\w+)", estado or "")
        manifiesto = "capability manifest: configured=true, approved_at_startup=true, valid=true"
        if modo is None or modo.group(1) != "bounded" or manifiesto not in (estado or ""):
            raise Falla(
                "CuaDriver no arrancó en modo acotado con su manifiesto: "
                + " | ".join(linea.strip() for linea in (estado or "").splitlines()[1:])
            )
        return {"pid": int(pid.group(1)), "pid_app": pid_app}

    def motor_llamar(self, herramienta: str, argumentos: dict) -> dict:
        """One tool call. A refusal is said by its name: the driver refusing its
        own authorization process, or a permission the manifest or macOS does
        not give, never a bare failure."""
        codigo, salida, errores = self.sistema.correr(
            str(self.rutas.cua), "call", herramienta, json.dumps(argumentos, ensure_ascii=False)
        )
        if codigo != 0:
            causa = errores.strip()[:500]
            if "own authorization process" in causa:
                raise Falla(
                    f"CuaDriver se niega a actuar sobre su propio proceso de autorización "
                    f"(pid {argumentos.get('pid')}) en {herramienta}: {causa}"
                )
            if any(marca in causa for marca in RECHAZOS_DE_PERMISO):
                raise Falla(f"CuaDriver rechazó {herramienta} por permisos: {causa}")
            raise Falla(f"CuaDriver falló en {herramienta} (código {codigo}): {causa}")
        try:
            resultado = json.loads(salida)
        except ValueError:
            resultado = {"texto": salida.strip()}
        return {"resultado": resultado, "bytes": len(salida.encode())}

    def motor_parar(self, max_s: float) -> dict:
        """Stops the CuaDriver this test started: by the pid it wrote down,
        and only if that pid is still CuaDriver's."""
        if not self.rutas.pid_cua.exists():
            return {"parado": True, "pid": None}
        pid = int(self.rutas.pid_cua.read_text(encoding="utf-8").strip())
        nuestro = any(
            p == pid and orden.startswith(CUA_APP) for p, orden in self.sistema.procesos()
        )
        if nuestro:
            self.sistema.correr(str(self.rutas.cua), "--expected-pid", str(pid), "stop")
            if self.sistema.esperar_salida([pid], max_s):
                self.sistema.senal(pid, signal.SIGTERM)
        self.rutas.pid_cua.unlink()
        return {"parado": True, "pid": pid}

    def registros(self) -> dict:
        textos = {}
        for nombre in (
            "report-daemon.log.1",
            "report-daemon.log",
            "report-app.log.1",
            "report-app.log",
        ):
            ruta = self.rutas.registros / nombre
            if ruta.exists():
                textos[nombre] = ruta.read_text(encoding="utf-8", errors="replace")
        # The data directory is cleared before every run (preparar), so
        # these lines are this run's.
        llamada = []
        for nombre in ("daemon.log.1", "daemon.log"):
            ruta = self.rutas.registros / nombre
            if not ruta.exists():
                continue
            for linea in ruta.read_text(encoding="utf-8", errors="replace").splitlines():
                try:
                    fila = json.loads(linea)
                except ValueError:
                    continue
                if isinstance(fila, dict) and fila.get("event") in EVENTOS_DE_LLAMADA:
                    llamada.append(json.dumps(fila, ensure_ascii=False) + "\n")
        return {"registros": textos, "audio_llamada": "".join(llamada)}


def _plazo(argumentos: list[str]) -> tuple[list[str], float]:
    if "--max" not in argumentos:
        raise Falla("falta --max")
    posicion = argumentos.index("--max")
    return argumentos[:posicion] + argumentos[posicion + 2 :], float(argumentos[posicion + 1])


def main(argv: list[str], mac: Mac | None = None) -> int:
    mac = mac or Mac(Rutas(Path.home()))
    try:
        if not argv:
            raise Falla("falta la orden")
        orden, resto = argv[0], argv[1:]
        if orden == "preparar":
            _, max_s = _plazo(resto)
            salida = mac.preparar(max_s)
        elif orden == "emparejar" and len(resto) == 3:
            salida = mac.emparejar(*resto)
        elif orden == "esperar":
            resto, max_s = _plazo(resto)
            salida = mac.esperar(resto[0], resto[1] if len(resto) > 1 else None, max_s)
        elif orden == "leer" and resto:
            salida = mac.leer(resto[0], resto[1] if len(resto) > 1 else None)
        elif orden == "hablar" and len(resto) == 1 and resto[0] in PARTES:
            salida = mac.hablar(resto[0])
        elif orden == "nota-de-voz" and not resto:
            salida = mac.nota_de_voz()
        elif orden == "orden" and len(resto) == 1 and resto[0] in ("call_start", "call_stop"):
            salida = mac.orden(resto[0])
        elif orden == "restaurar":
            _, max_s = _plazo(resto)
            salida = mac.restaurar(max_s)
        elif orden == "registros":
            salida = mac.registros()
        elif orden == "activar":
            salida = mac.activar()
        elif orden == "flotantes":
            salida = mac.flotantes()
        elif orden == "motor-arrancar":
            _, max_s = _plazo(resto)
            salida = mac.motor_arrancar(max_s)
        elif orden == "motor-llamar" and len(resto) == 2:
            salida = mac.motor_llamar(resto[0], json.loads(resto[1]))
        elif orden == "motor-parar":
            _, max_s = _plazo(resto)
            salida = mac.motor_parar(max_s)
        else:
            raise Falla(f"orden desconocida: {' '.join(argv)[:80]}")
    except (Falla, OSError, ValueError, KeyError) as causa:
        print(json.dumps({"error": str(causa) or type(causa).__name__}, ensure_ascii=False))
        return 1
    print(json.dumps(salida, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
