"""The test hub and the test Hermes of the real test (SIM-25), on the test server.

The hub runs from the tree under test (`server/`), with its own port, data
directory and `orbit.toml` under `~/.orbit-pruebas/`, listening only on
the test server's tailnet address. It uses the real Whisper and the real note model
(`[processing]`, whose key file is named here and read only by the hub), and
talks to Hermes `deimos-pruebas` served on its own loopback port. Both are
started by the test and stopped by it, by the process group it created, also
when the test fails. Production is never touched: other ports, other
directories, another profile.

    python hub_pruebas.py servir --config C --host H --port P

is how the hub is started: the hub's own `serve` refuses plain HTTP off
loopback, and the Mac reaches this one over plain HTTP on the tailnet (only
«Orbit Pruebas» accepts that, `orbit_bundle::plain_http_hub`). The tailnet is
WireGuard: the link is encrypted, so a request that arrives on it counts as
the secure transport the hub demands (`allowed_transport`). That is the only
difference from `orbit-hub serve`.
"""

from __future__ import annotations

import argparse
import ipaddress
import os
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.request
from dataclasses import dataclass
from pathlib import Path

RAIZ = Path(__file__).resolve().parents[2]
SERVIDOR = RAIZ / "server"
PYTHON = SERVIDOR / ".venv" / "bin" / "python"
TAILNET = ipaddress.ip_network("100.64.0.0/10")

# Production uses 8700 (hub) and 8643 (hermes-serve-orbit); the tailnet serve
# of Hermes holds 8642. These are free on the test server and checked before each run.
PUERTO_HUB = 8710
PUERTO_HERMES = 8653
MODELO_WHISPER = "openai/whisper-large-v3"
MODELO_PROCESAMIENTO = "qwen3.8-flash-next"


def required_environment(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"falta {name}; consulta docs/self-hosting.md")
    return value


@dataclass(frozen=True)
class Rutas:
    base: Path

    @property
    def secretos(self) -> Path:
        return self.base / "secretos"

    @property
    def token_hermes(self) -> Path:
        return self.secretos / "hermes-token"

    @property
    def hub(self) -> Path:
        return self.base / "hub"

    @property
    def configuracion(self) -> Path:
        return self.base / "orbit.toml"

    @property
    def ejecuciones(self) -> Path:
        return self.base / "ejecuciones"

    @property
    def procesos(self) -> Path:
        """The processes a run started and has not stopped yet: one
        `pid<TAB>start time<TAB>name` per line, so that a run killed before it
        could clean up is cleaned up by the next one. The start time (from
        /proc) tells the process apart from any later one given the same pid."""
        return self.base / "procesos"


def escribir_privado(ruta: Path, contenido: bytes) -> None:
    ruta.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(ruta, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as fichero:
        fichero.write(contenido)
    os.chmod(ruta, 0o600)


def preparar_secretos(rutas: Rutas) -> list[str]:
    """Creates, once, the Hermes token of the test. Never rewrites it: the
    test Hermes may already be using it."""
    creados = []
    if not rutas.token_hermes.exists():
        escribir_privado(rutas.token_hermes, secrets.token_urlsafe(32).encode())
        creados.append(str(rutas.token_hermes))
    return creados


def configuracion(
    rutas: Rutas, host: str, puerto_hermes: int, proveedores: dict | None = None
) -> str:
    """The test hub's orbit.toml. `proveedores` replaces Whisper and the note
    model only in the dry run, which must not call the real ones."""
    p = (
        proveedores
        if proveedores is not None
        else {
            "whisper": required_environment("ORBIT_TEST_WHISPER_URL"),
            "procesamiento": required_environment("ORBIT_TEST_PROCESSING_URL"),
            "clave_procesamiento": required_environment("ORBIT_TEST_PROCESSING_KEY_FILE"),
        }
    )
    profile = (
        required_environment("ORBIT_TEST_HERMES_PROFILE")
        if proveedores is None
        else "fixture-assistant"
    )
    diarization = ""
    if proveedores is None:
        python = required_environment("ORBIT_DIARIZATION_PYTHON")
        bundle = required_environment("ORBIT_DIARIZATION_BUNDLE")
        lock = required_environment("ORBIT_DIARIZATION_GPU_LOCK")
        diarization = (
            f'\n[diarization]\npython = "{python}"\nbundle = "{bundle}"\ngpu_lock = "{lock}"\n'
        )
    return f"""# Hub de pruebas de SIM-25. Lo escribe scripts/e2e-mac en cada ejecución.
data_dir = "{rutas.hub / "datos"}"
key_file = "{rutas.hub / "storage.key"}"
hermes_url = "http://127.0.0.1:{puerto_hermes}"
hermes_credential_file = "{rutas.token_hermes}"
hermes_ws_auth = "token"
profile = "{profile}"
allow_insecure_localhost = true
allowed_hosts = ["{host}"]

[meeting_transcription]
endpoint = "{p["whisper"]}"
model = "{MODELO_WHISPER}"
language = "es"

[processing]
endpoint = "{p["procesamiento"]}"
model = "{MODELO_PROCESAMIENTO}"
credential_file = "{p["clave_procesamiento"]}"
reasoning = "off"

{diarization}
"""


def puerto_libre(host: str, puerto: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as prueba:
        prueba.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            prueba.bind((host, puerto))
        except OSError:
            return False
    return True


def _arranque(pid: int) -> str | None:
    """Field 22 of /proc/<pid>/stat: when the process started, in clock ticks."""
    try:
        estado = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8")
    except OSError:
        return None
    campos = estado.rsplit(")", 1)[-1].split()
    # A zombie has finished; only its parent has not collected it yet.
    return None if campos[0] == "Z" else campos[19]


def parar_restos(rutas: Rutas, plazo_s: float) -> list[str]:
    """Stops what an earlier run started and never stopped: only a pid whose
    start time is the one that run wrote down, and its process group."""
    if not rutas.procesos.exists():
        return []
    parados = []
    for linea in rutas.procesos.read_text(encoding="utf-8").splitlines():
        partes = linea.split("\t")
        if len(partes) != 3 or not partes[0].isdigit():
            continue
        pid, arranque, nombre = int(partes[0]), partes[1], partes[2]
        if _arranque(pid) != arranque:
            continue
        try:
            os.killpg(pid, signal.SIGTERM)
        except ProcessLookupError:
            continue
        limite = time.monotonic() + plazo_s
        while _arranque(pid) == arranque and time.monotonic() < limite:
            time.sleep(0.2)
        if _arranque(pid) == arranque:
            os.killpg(pid, signal.SIGKILL)
        parados.append(f"{nombre} ({pid})")
    rutas.procesos.unlink()
    return parados


class Proceso:
    """A process this test started, in a process group of its own, so that
    stopping it reaches everything it started and nothing else."""

    def __init__(
        self,
        nombre: str,
        orden: list[str],
        registro: Path,
        entorno: dict | None = None,
        apuntes: Path | None = None,
    ):
        self.nombre = nombre
        registro.parent.mkdir(parents=True, exist_ok=True)
        self._salida = open(registro, "ab")  # noqa: SIM115 - closed in parar()
        self.popen = subprocess.Popen(
            orden,
            stdin=subprocess.DEVNULL,
            stdout=self._salida,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            env=entorno,
        )
        self.apuntes = apuntes
        if apuntes is not None:
            with open(apuntes, "a", encoding="utf-8") as fichero:
                fichero.write(f"{self.popen.pid}\t{_arranque(self.popen.pid)}\t{nombre}\n")

    def vivo(self) -> bool:
        return self.popen.poll() is None

    def parar(self, plazo_s: float) -> dict:
        """SIGTERM to its group, the bound to leave, SIGKILL to the group after."""
        forzado = False
        if self.vivo():
            try:
                os.killpg(self.popen.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.popen.wait(plazo_s)
            except subprocess.TimeoutExpired:
                forzado = True
                try:
                    os.killpg(self.popen.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.popen.wait(10)
        self._salida.close()
        if self.apuntes is not None and self.apuntes.exists():
            quedan = [
                linea
                for linea in self.apuntes.read_text(encoding="utf-8").splitlines()
                if not linea.startswith(f"{self.popen.pid}\t")
            ]
            self.apuntes.write_text("".join(f"{linea}\n" for linea in quedan), encoding="utf-8")
        return {
            "proceso": self.nombre,
            "codigo": self.popen.returncode,
            "forzado": forzado,
        }


def esperar_salud(url: str, proceso: Proceso, plazo_s: float) -> float:
    """Until /health answers 200; ends at once if the process dies."""
    inicio = time.monotonic()
    while True:
        if not proceso.vivo():
            raise RuntimeError(
                f"el hub de pruebas terminó al arrancar (código {proceso.popen.returncode})"
            )
        try:
            with urllib.request.urlopen(f"{url}/health", timeout=2) as respuesta:
                if respuesta.status == 200:
                    return time.monotonic() - inicio
        except OSError:
            pass
        if time.monotonic() - inicio > plazo_s:
            raise RuntimeError(f"el hub de pruebas no respondió en /health en {plazo_s:.0f} s")
        time.sleep(0.2)


def esperar_puerto(host: str, puerto: int, proceso: Proceso, plazo_s: float) -> float:
    inicio = time.monotonic()
    while True:
        if not proceso.vivo():
            raise RuntimeError(
                f"{proceso.nombre} terminó al arrancar (código {proceso.popen.returncode})"
            )
        try:
            socket.create_connection((host, puerto), timeout=1).close()
            return time.monotonic() - inicio
        except OSError:
            pass
        if time.monotonic() - inicio > plazo_s:
            raise RuntimeError(f"{proceso.nombre} no abrió {host}:{puerto} en {plazo_s:.0f} s")
        time.sleep(0.2)


def iniciar_almacen(rutas: Rutas) -> None:
    """A fresh store each run: the hub's data of one run is never the next one's."""
    if rutas.hub.exists():
        shutil.rmtree(rutas.hub)
    rutas.hub.mkdir(mode=0o700, parents=True)
    subprocess.run(
        [
            str(PYTHON),
            "-m",
            "orbit_hub.cli",
            "--config",
            str(rutas.configuracion),
            "init",
        ],
        cwd=SERVIDOR,
        check=True,
        capture_output=True,
        timeout=60,
    )


def codigo_de_emparejamiento(rutas: Rutas) -> str:
    hecho = subprocess.run(
        [
            str(PYTHON),
            "-m",
            "orbit_hub.cli",
            "--config",
            str(rutas.configuracion),
            "pair-code",
        ],
        cwd=SERVIDOR,
        check=True,
        capture_output=True,
        text=True,
        timeout=60,
    )
    return hecho.stdout.strip()


def arrancar_hub(rutas: Rutas, host: str, puerto: int, registro: Path) -> Proceso:
    return Proceso(
        "hub de pruebas",
        [
            str(PYTHON),
            str(Path(__file__).resolve()),
            "servir",
            "--config",
            str(rutas.configuracion),
            "--host",
            host,
            "--port",
            str(puerto),
        ],
        registro,
        apuntes=rutas.procesos,
    )


def arrancar_hermes(rutas: Rutas, puerto: int, registro: Path) -> Proceso:
    token_variable = required_environment("ORBIT_TEST_HERMES_TOKEN_ENV")
    hermes = os.environ.get("ORBIT_HUB_HERMES", str(Path.home() / ".local" / "bin" / "hermes"))
    entorno = dict(os.environ)
    entorno[token_variable] = rutas.token_hermes.read_text(encoding="utf-8").strip()
    # The flags of hermes-serve-orbit.service (scripts/tests/hub_harness.py
    # SERVE_ARGS), with the test profile and port.
    return Proceso(
        "Hermes de pruebas",
        [
            hermes,
            "--profile",
            required_environment("ORBIT_TEST_HERMES_PROFILE"),
            "serve",
            "--isolated",
            "--host",
            "127.0.0.1",
            "--port",
            str(puerto),
        ],
        registro,
        entorno,
        apuntes=rutas.procesos,
    )


# ── The server itself ───────────────────────────────────────────────────────


def tailnet_seguro(app):
    """Marks requests that arrive from a tailnet address as the secure
    transport they are, for the hub's own transport check."""

    async def envuelta(scope, receive, send):
        cliente = scope.get("client") or ("",)
        if scope.get("type") in ("http", "websocket"):
            try:
                desde_tailnet = ipaddress.ip_address(cliente[0]) in TAILNET
            except ValueError:
                desde_tailnet = False
            if desde_tailnet:
                scope = dict(
                    scope,
                    scheme={"http": "https", "ws": "wss"}.get(scope["scheme"], scope["scheme"]),
                )
        await app(scope, receive, send)

    return envuelta


def servir(argumentos: list[str]) -> None:
    lector = argparse.ArgumentParser(description="Hub de pruebas de SIM-25")
    lector.add_argument("--config", type=Path, required=True)
    lector.add_argument("--host", required=True)
    lector.add_argument("--port", type=int, required=True)
    opciones = lector.parse_args(argumentos)
    direccion = ipaddress.ip_address(opciones.host)
    if not (direccion in TAILNET or direccion.is_loopback):
        raise SystemExit(
            f"el hub de pruebas sólo escucha en la tailnet o en loopback, no en {direccion}"
        )
    import uvicorn

    from orbit_hub.app import create_app
    from orbit_hub.config import Settings

    settings = Settings.load(opciones.config)
    # The same server options as `orbit-hub serve` (orbit_hub/cli.py).
    uvicorn.run(
        tailnet_seguro(create_app(settings)),
        host=opciones.host,
        port=opciones.port,
        ws_max_size=settings.max_frame_bytes,
        ws_max_queue=16,
        limit_concurrency=64,
        access_log=False,
        proxy_headers=False,
    )


if __name__ == "__main__":
    if sys.argv[1:2] == ["servir"]:
        servir(sys.argv[2:])
    else:
        raise SystemExit("uso: hub_pruebas.py servir --config C --host H --port P")
