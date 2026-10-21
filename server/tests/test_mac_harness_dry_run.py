"""SIM-25: the dry run of the real test on the Mac, and its parts.

The dry run walks the orchestrator of scripts/e2e-mac and its twelve checks
against the example states in scripts/e2e-mac/ensayo/, with the real test hub
started on loopback (fake Whisper and note model that are never called, no
Hermes) and the Mac and its interface replaced by replays. It proves the logic
of the checks and that each one can fail: every check fails against one
example spoiled on purpose and passes against the good one. It is not the real
test and says nothing about Orbit on a Mac.
"""

import json
import sys
from pathlib import Path

import pytest

RAIZ = Path(__file__).resolve().parents[2]
E2E = RAIZ / "scripts" / "e2e-mac"
sys.path.insert(0, str(E2E))

import ensayo
import interfaz
import mac as mac_escenario

TODAS = list(range(1, 13))


@pytest.fixture(scope="module")
def bueno(tmp_path_factory):
    resultados, directorio = ensayo.ejecutar("bueno", tmp_path_factory.mktemp("bueno"))
    return resultados, directorio


def estados(resultados):
    return {r["numero"]: r["estado"] for r in resultados}


def test_the_good_example_passes_all_twelve_checks(bueno):
    resultados, directorio = bueno
    assert estados(resultados) == {n: "pasa" for n in TODAS}, json.dumps(
        resultados, ensure_ascii=False, indent=1
    )
    informe = (directorio / "informe.txt").read_text(encoding="utf-8")
    for numero in TODAS:
        assert f"{numero:>2}. [pasa]" in informe
    # Every wait of the run is in the report with what it took.
    datos = json.loads((directorio / "informe.json").read_text(encoding="utf-8"))
    nombres = {e["espera"] for e in datos["esperas"]}
    assert {
        "hub",
        "emparejado",
        "grabando",
        "respuesta",
        "nota",
        "chat",
        "nota-de-voz",
        "llamada-escuchando",
    } <= nombres
    assert all("segundos" in e for e in datos["esperas"])
    # SIM-27: what the run does not cover is said in the report, every time.
    assert "No cubierto:" in informe
    assert "voz sintética" in informe
    # Only the call's audio lines of daemon.log come back from the Mac.
    llamada = (directorio / "daemon-llamada.log").read_text(encoding="utf-8")
    eventos = [json.loads(linea)["event"] for linea in llamada.splitlines()]
    assert eventos == ["call_audio_opened", "call_audio_heard"]


def test_the_dry_run_starts_and_stops_the_real_test_hub(bueno):
    _, directorio = bueno
    datos = json.loads((directorio / "informe.json").read_text(encoding="utf-8"))
    parados = datos["parados"]
    assert [p["proceso"] for p in parados] == ["hub de pruebas"]
    assert parados[0]["forzado"] is False
    registro = (directorio / "hub.log").read_text(encoding="utf-8")
    assert "Uvicorn running" in registro


@pytest.mark.parametrize("numero", TODAS)
def test_each_check_fails_against_its_spoiled_example(numero, tmp_path):
    resultados, _ = ensayo.ejecutar(f"malo-{numero}", tmp_path)
    vistos = estados(resultados)
    assert vistos[numero] == "falla", json.dumps(resultados, ensure_ascii=False, indent=1)
    assert {n: e for n, e in vistos.items() if n != numero} == {
        n: "pasa" for n in TODAS if n != numero
    }, json.dumps(resultados, ensure_ascii=False, indent=1)
    detalle = next(r["detalle"] for r in resultados if r["numero"] == numero)
    assert detalle, "un fallo sin causa"


# ── The parts ───────────────────────────────────────────────────────────────


def test_the_menu_bar_never_reaches_the_evidence(tmp_path):
    # Real run 36: a whole-window read brought macOS's menu bar, and under the
    # Apple menu «Ítems recientes» named Mars's own documents.
    leido = (
        '- AXWindow "Orbit Pruebas"\n'
        '  - [3] AXButton "Grabar" [actions=[press]]\n'
        "- [50] AXMenuBar [actions=[cancel]]\n"
        '  - [51] AXMenuBarItem "Apple" [actions=[cancel,press,pick]]\n'
        '    - [89] AXMenuItem "documento-privado.pdf" [id=_recentItemRequested: actions=[press]]\n'
        "      con una segunda línea\n"
        '- AXStaticText = "después"\n'
    )

    class Puente:
        ultimo_recibo_bytes = len(leido)

        def accion(self, tool, arguments):
            return {"tree_markdown": leido, "snapshot_id": "s1"}

    ui = interfaz.Interfaz(Puente(), tmp_path, 7)
    nodos = ui.leer(interfaz.Ventana(7, 1, "Orbit Pruebas"), "inicio", None)
    guardado = (tmp_path / "001-inicio.txt").read_text(encoding="utf-8")
    for texto in (guardado, ui.arboles["inicio"]):
        assert "documento-privado" not in texto and "segunda línea" not in texto
        assert "AXMenuBar" not in texto
    assert [n.role for n in nodos] == ["AXWindow", "AXButton", "AXStaticText"]


def test_a_saved_input_no_longer_connected_does_not_stop_the_next_run(tmp_path):
    # Real run of 2026-10-21: a run that died had saved a pair of headphones
    # as the input to put back, and they were no longer connected.
    muestra = ensayo.Muestra("bueno")
    rutas = mac_escenario.Rutas(tmp_path)
    sistema = ensayo.SistemaEnsayo(muestra, rutas)
    mac = mac_escenario.Mac(rutas, sistema, ensayo.OrbitEnsayo(muestra))
    rutas.propio.mkdir()
    rutas.entrada_anterior.write_text("Auriculares de Alina\n", encoding="utf-8")

    hecho = mac.restaurar(1.0)

    assert "«Auriculares de Alina» ya no está conectada" in hecho["entrada_restaurada"]
    assert not rutas.entrada_anterior.exists()
    assert sistema.entrada == "Micrófono del MacBook Pro"


# ── SIM-30 and SIM-41: the waits that read, the names the Mac computes ──────


# ── SIM-27: the voice note and the call with Deimos ─────────────────────────
