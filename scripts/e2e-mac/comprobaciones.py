"""The twelve checks of the real test on the Mac (SIM-25, SIM-27), as pure functions.

Each one reads the evidence the run collected — the accessibility trees of
what was on screen, what the daemon said over its socket, the report logs of
the Mac and the log of the test hub — and says «pasa», «falla» with its cause,
or «sin comprobar» when the run never got to it, naming the step that stopped
it. No check is ever softened to pass: one that fails on the Mac is an Orbit
defect.
"""

from __future__ import annotations

import json
import re
from datetime import UTC, datetime

import arbol

NOMBRES = {
    1: "La transcripción llega en español",
    2: "Los tiempos son relativos al inicio y no pasan de la duración",
    3: "El micrófono local aparece con su nombre",
    4: "No hay entradas repetidas ni vacías",
    5: "La nota se genera con resumen y acciones, y el aviso «Transcribiendo…» desaparece",
    6: "«Abrir nota», desde el aviso y desde la lista de Notas, abre esa nota",
    7: "«Oye Deimos» produce una pregunta y la respuesta aparece en el panel",
    8: "El Asistente responde y abre una conversación anterior con sus mensajes",
    9: "Sin píldora de estado ni textos que SIM-24 quitó",
    10: "Ningún fallo inesperado en los registros",
    11: "Una nota de voz se transcribe y Deimos responde",
    12: "La llamada se abre y escucha",
}
# What the run cannot see, said in every report (SIM-27).
NO_CUBIERTO = (
    (
        "que la llamada abra un turno con la voz sintética: con el procesado de voz, macOS "
        "baja el volumen del resto de las apps, y la voz de `say` por BlackHole llega a una "
        "décima parte, bajo el umbral (medido el 2026-10-17). Que oye una voz real se comprobó "
        "con la de Mars ese día (fallos.md)."
    ),
)

# What a check of the transcript compares against the script. A word is a run
# of five or more letters: short words are the ones speech recognition most
# often drops or splits, and the content words of the script are longer.
PALABRA = re.compile(r"[a-zñ]{5,}")
# Share of the script's words that must come back. The audio is synthetic
# speech, read without noise straight into the recording; a recognizer working
# in Spanish returns nearly all of it, and one that switched language (fallo 7
# of the pausa) returns almost none. The margin is for names and numbers the
# recognizer writes differently («doce mil» as «12.000»).
RECALL_MINIMO = 0.8

UNIDENTIFIED = "Hablante sin identificar"
# Where the tree of the «Transcripción» tab starts: its toolbar's language
# (TranscriptTab.tsx, `es.notes.transcriptLanguage`). Since SIM-52 the note's
# header has a picker labelled «Idioma» above it, so the anchor carries the
# toolbar's own words, and check 1 reads that text, never the picker.
INICIO_TRANSCRIPCION = "Idioma de la transcripción: "
# The note's title is an editable field since SIM-52 (NoteHeader.tsx,
# `es.noteScreen.titleLabel`): its value is the title, its label this.
CAMPO_TITULO = "Título de la reunión"
# The texts SIM-24 and earlier interface revisions removed from the screen. None may appear
# anywhere, ever.
QUITADOS = (
    "Listo · Emparejado",
    "Emparejado con",
    "Privacidad: normal",
    "Privacidad: parada",
    "Parada de privacidad",
    "Autonomía total",
    "Acceso revocado",
    "Sin conexión con Orbit",
    "Datos de hace",
    "Datos no vigentes",
    "Estado del daemon",
    "todavía no se conoce el dato",
    "aún no se ha observado",
    "fragmentos pendientes",
    "Deimos la lee",
    "Deimos puede leer tu agenda",
    "Deimos lee todos tus calendarios",
    "Contexto: calendario",
    "sin contexto adjunto",
    "Reconectando con el hub",
    "Conectando con el hub",
    "El hub está fuera de línea",
    "Reintentar conversaciones",
    "Reunión guardada · transcribiendo",
    "Hora no disponible",
    "Cargando índice de notas",
    "Guardado cifrado en este Mac",
    "trozos por subir",
    "tramos sin transcribir",
    "Transcripción en cola",
    "Esperando confirmación del Mac",
    "Esperando que el Mac cierre el audio",
    "Sin vincular",
    "ID del dispositivo",
)
QUITADOS_PATRONES = (
    re.compile(r"\d+/\d+ permisos"),
    re.compile(r"\d+ ?bytes"),
    re.compile(r"HTTP \d{3}"),
    re.compile(r"\d+ ms\b"),
)
# The status pill's texts (apps/desktop/src/i18n/es/status.ts, `problem`): with
# everything in order none of them shows. A guard test keeps this list in step
# with that file.
PILDORA = (
    "Orbit no responde.",
    "Privacidad detenida",
    "Este Mac no está emparejado",
    "El hub retiró este Mac",
    "Sin conexión. Comprueba tu red.",
    "Falta el permiso de",
    "Faltan permisos",
)
# Report-log classes that mean something went wrong: «puedes arreglarlo» and
# «fallo genérico» (paso-2.md). «working», «done» and «silent» are normal life.
CLASES_FALLO = ("failed", "fixable")
NIVELES_FALLO = ("ERROR", "CRITICAL")


def resultado(numero: int, estado: str, detalle: str) -> dict:
    return {"numero": numero, "nombre": NOMBRES[numero], "estado": estado, "detalle": detalle}


class SinComprobar(Exception):
    """The evidence a check needs was never collected."""


def _arbol(ev: dict, etiqueta: str) -> list[arbol.Nodo]:
    texto = ev.get("arboles", {}).get(etiqueta)
    if texto is None:
        raise SinComprobar(f"no se llegó a leer la pantalla «{etiqueta}»")
    return arbol.leer(texto)


def _dato(ev: dict, clave: str):
    valor = ev.get(clave)
    if valor is None:
        raise SinComprobar(f"falta «{clave}»")
    return valor


def _transcripcion(ev: dict) -> tuple[list[str], list[arbol.Segmento]]:
    antes, segmentos = arbol.transcripcion(
        _arbol(ev, "nota-transcripcion"), INICIO_TRANSCRIPCION, "Panel de contexto"
    )
    return antes, segmentos


def c1_espanol(ev: dict) -> tuple[bool, str]:
    antes, segmentos = _transcripcion(ev)
    if not segmentos:
        return False, "la pestaña Transcripción no muestra ninguna entrada"
    # The toolbar's own line: the header's picker says a language too.
    idioma = next((t for t in antes if t.startswith(INICIO_TRANSCRIPCION)), None)
    if idioma is None or idioma.removeprefix(INICIO_TRANSCRIPCION) != "Español":
        return (
            False,
            f"el idioma que muestra la transcripción no es español: «{idioma or 'ninguno'}»",
        )
    guion = " ".join(_dato(ev, "guion"))
    dicho = " ".join(s.texto for s in segmentos)
    vuelven, de, recall = _palabras_que_vuelven(guion, dicho)
    if recall < RECALL_MINIMO:
        # Where the words were lost tells a late start from a lost stretch
        # (2026-10-20: two runs began at 00:25 and one repeated itself).
        oidas = set(PALABRA.findall(arbol.plano(dicho)))
        faltan = [p for p in dict.fromkeys(PALABRA.findall(arbol.plano(guion))) if p not in oidas]
        return False, (
            f"sólo vuelven {vuelven} de {de} palabras del guion "
            f"({recall:.0%}; mínimo {RECALL_MINIMO:.0%}); la primera entrada empieza en "
            f"{segmentos[0].hora}; faltan {len(faltan)}, las primeras: {', '.join(faltan[:10])}"
        )
    return True, f"español; vuelven {vuelven} de {de} palabras ({recall:.0%})"


def _palabras_que_vuelven(guion: str, dicho: str) -> tuple[int, int, float]:
    """How many of the script's words (PALABRA) come back in what was
    transcribed, of how many, and their share."""
    esperadas = set(PALABRA.findall(arbol.plano(guion)))
    oidas = set(PALABRA.findall(arbol.plano(dicho)))
    vuelven = len(esperadas & oidas)
    return vuelven, len(esperadas), vuelven / len(esperadas) if esperadas else 0.0


def c2_tiempos(ev: dict) -> tuple[bool, str]:
    _, segmentos = _transcripcion(ev)
    if not segmentos:
        return False, "la pestaña Transcripción no muestra ninguna entrada"
    duracion_ms = int(_dato(ev, "grabacion")["duration_ms"])
    malos = []
    for numero, segmento in enumerate(segmentos, 1):
        valor = arbol.segundos(segmento.hora)
        if valor is None or valor * 1000 > duracion_ms:
            malos.append(f"entrada {numero}: «{segmento.hora}»")
    if malos:
        return False, (
            f"{len(malos)} de {len(segmentos)} tiempos pasan de la duración "
            f"({duracion_ms // 1000} s) o no se leen: " + ", ".join(malos[:5])
        )
    return True, f"{len(segmentos)} tiempos entre 0 y {duracion_ms // 1000} s"


def c3_nombre(ev: dict) -> tuple[bool, str]:
    _, segmentos = _transcripcion(ev)
    if not segmentos:
        return False, "la pestaña Transcripción no muestra ninguna entrada"
    # The account's name as the daemon takes it (`id -F`, mac.py preparar),
    # never the hub's, which only repeats what the Mac sent it.
    propietario = _dato(ev, "mac").get("propietario") or ""
    # The recording is microphone only: every turn is the local microphone.
    otros = [s.hablante for s in segmentos if s.hablante != propietario]
    if otros:
        sin = sum(1 for nombre in otros if nombre == UNIDENTIFIED)
        return False, (
            f"{len(otros)} de {len(segmentos)} entradas del micrófono local no llevan "
            f"el nombre «{propietario}» ({sin} dicen «{UNIDENTIFIED}»; "
            f"otros: {sorted(set(otros) - {UNIDENTIFIED})})"
        )
    return True, f"las {len(segmentos)} entradas dicen «{propietario}»"


def c4_repetidas(ev: dict) -> tuple[bool, str]:
    _, segmentos = _transcripcion(ev)
    if not segmentos:
        return False, "la pestaña Transcripción no muestra ninguna entrada"
    vacias = [n for n, s in enumerate(segmentos, 1) if not s.texto.strip()]
    vistos: dict[str, int] = {}
    repetidas = []
    for numero, segmento in enumerate(segmentos, 1):
        clave = arbol.plano(segmento.texto).strip()
        if not clave:
            continue
        if clave in vistos:
            repetidas.append(f"{vistos[clave]} y {numero}")
        else:
            vistos[clave] = numero
    if vacias or repetidas:
        return False, (
            f"de {len(segmentos)} entradas, {len(vacias)} vacías ({vacias[:5]}) y "
            f"{len(repetidas)} repetidas ({repetidas[:5]})"
        )
    return True, f"{len(segmentos)} entradas, ninguna vacía ni repetida"


def _nota(ev: dict) -> dict:
    """The note as Mars sees it, from the meeting's document (SIM-41)."""
    nota = _dato(ev, "detalle").get("note") or {}
    if nota.get("state") != "ready" or not nota.get("note"):
        estado = {k: nota.get(k) for k in ("state", "attempts", "error")}
        raise ValueError(f"el daemon no tiene nota escrita: {json.dumps(estado)[:200]}")
    return nota["note"]


def _titulo(ev: dict) -> str:
    """The meeting's title: the list of Notes and the opened note both name
    the note by it (Mars, 2026-10-17), never by the title the note was
    written with."""
    return _dato(ev, "detalle").get("title") or ""


def c5_nota(ev: dict) -> tuple[bool, str]:
    fase = _dato(ev, "fase_nota")
    if fase.get("phase") != "ready":
        return False, f"la nota no llegó a estar lista: {json.dumps(fase)}"
    try:
        nota = _nota(ev)
    except ValueError as causa:
        return False, str(causa)
    secciones = [s for s in nota.get("sections") or [] if (s.get("markdown") or "").strip()]
    acciones = nota.get("action_items") or []
    if not secciones or not acciones:
        return (
            False,
            f"la nota tiene {len(secciones)} secciones con texto y {len(acciones)} acciones",
        )
    resumen = arbol.todo_el_texto(_arbol(ev, "nota-resumen"))
    if "El resumen aparecerá" in resumen or any(
        _palabras_que_faltan(s["markdown"], resumen) for s in secciones
    ):
        return False, f"la pestaña Resumen no muestra las {len(secciones)} secciones de la nota"
    tareas = arbol.todo_el_texto(_arbol(ev, "nota-acciones"))
    vistas = [a for a in acciones if a["text"] in tareas]
    if not vistas:
        return False, f"la pestaña Acciones no muestra ninguna de las {len(acciones)} acciones"
    despues = ("nota-lista", "nota-resumen", "nota-acciones", "nota-transcripcion")
    quedan = [
        etiqueta
        for etiqueta in despues
        if "transcribiendo" in arbol.plano(ev.get("arboles", {}).get(etiqueta, ""))
    ]
    if quedan:
        return (
            False,
            f"con la nota lista, «Transcribiendo…» sigue en pantalla ({', '.join(quedan)})",
        )
    return True, (
        f"{len(secciones)} secciones y {len(acciones)} acciones ({len(vistas)} vistas); "
        "«Transcribiendo…» ya no está"
    )


def c6_abrir_nota(ev: dict) -> tuple[bool, str]:
    titulo = _titulo(ev)
    if not titulo:
        return False, "la nota no tiene título con el que reconocerla"
    malas = []
    for etiqueta, origen in (("nota-desde-aviso", "el aviso"), ("nota-desde-lista", "la lista")):
        # The note's own title field, not the row that names it in the list.
        campos = [
            n
            for n in _arbol(ev, etiqueta)
            if n.role == "AXTextField" and CAMPO_TITULO in n.textos()
        ]
        if not any(nodo.value == titulo for nodo in campos):
            malas.append(origen)
    if malas:
        return False, f"desde {' y '.join(malas)} no se abre la nota «{titulo}»"
    return True, f"desde el aviso y desde la lista se abre «{titulo}»"


def c7_oye_deimos(ev: dict) -> tuple[bool, str]:
    pregunta = _dato(ev, "pregunta")
    if pregunta.get("origin") != "voice":
        return False, f"la pregunta no vino de la voz: origen «{pregunta.get('origin')}»"
    if pregunta.get("stage") != "answered":
        return False, (
            f"la pregunta terminó «{pregunta.get('stage')}» "
            f"({json.dumps(pregunta.get('cause'), ensure_ascii=False)})"
        )
    if "de que estamos hablando" not in arbol.plano(pregunta.get("question") or ""):
        return False, f"la pregunta oída no es la del guion: «{pregunta.get('question')}»"
    panel = arbol.todo_el_texto(_arbol(ev, "panel"))
    if "Tu pregunta:" not in panel:
        return False, "el panel no muestra la pregunta"
    for fallo in ("Deimos no respondió", "La pregunta no llegó a Deimos", "Deimos está pensando"):
        if fallo in panel:
            return False, f"el panel dice «{fallo}»"
    faltan = _palabras_que_faltan(pregunta.get("respuesta") or "", panel)
    if faltan is None:
        return False, "Deimos contestó sin texto"
    if faltan:
        return False, f"el panel no muestra la respuesta: faltan {faltan}"
    return True, "pregunta oída, respuesta en el panel"


def _palabras_que_faltan(respuesta: str, pantalla: str) -> list[str] | None:
    """The three longest words of `respuesta` that `pantalla` does not show.

    The screen renders Markdown, so the answer is matched by its words rather
    than as one string."""
    palabras = sorted(set(PALABRA.findall(arbol.plano(respuesta))), key=lambda p: (-len(p), p))
    if not palabras:
        return None
    vista = arbol.plano(pantalla)
    return [p for p in palabras[:3] if p not in vista]


def c8_asistente(ev: dict) -> tuple[bool, str]:
    chat = _dato(ev, "chat")
    mensaje, respuesta = chat.get("mensaje") or "", chat.get("respuesta") or ""
    for etiqueta, momento in (
        ("asistente-respondido", "al responder"),
        ("asistente-reabierta", "al abrir la conversación anterior"),
    ):
        pantalla = arbol.todo_el_texto(_arbol(ev, etiqueta))
        if mensaje not in pantalla:
            return False, f"{momento}, el Asistente no muestra el mensaje enviado"
        faltan = _palabras_que_faltan(respuesta, pantalla)
        if faltan is None:
            return False, "Deimos contestó sin texto"
        if faltan:
            return False, f"{momento}, el Asistente no muestra la respuesta: faltan {faltan}"
    return True, "respuesta recibida; la conversación anterior se abre con sus mensajes"


def c9_sin_ruido(ev: dict) -> tuple[bool, str]:
    arboles = ev.get("arboles", {})
    if "final" not in arboles:
        raise SinComprobar("no se llegó a leer la pantalla «final»")
    vistos = []
    for etiqueta, texto in arboles.items():
        pantalla = arbol.todo_el_texto(arbol.leer(texto))
        for quitado in QUITADOS + PILDORA:
            if quitado in pantalla:
                vistos.append(f"«{quitado}» en {etiqueta}")
        for patron in QUITADOS_PATRONES:
            if (encontrado := patron.search(pantalla)) is not None:
                vistos.append(f"«{encontrado.group(0)}» en {etiqueta}")
    if vistos:
        return False, "; ".join(vistos[:8])
    return True, f"{len(arboles)} pantallas leídas sin píldora ni textos retirados"


def c11_nota_de_voz(ev: dict) -> tuple[bool, str]:
    nota = _dato(ev, "nota_de_voz")
    if nota.get("fallo") is not None:
        return False, (
            "la nota de voz no se transcribió: "
            f"{json.dumps(nota['fallo'], ensure_ascii=False)[:200]}"
        )
    if nota.get("rol") != "user":
        return False, f"el turno de la nota de voz no es del usuario: rol «{nota.get('rol')}»"
    vuelven, de, recall = _palabras_que_vuelven(
        nota.get("frase") or "", nota.get("transcripcion") or ""
    )
    if recall < RECALL_MINIMO:
        return False, (
            f"la transcripción de la nota de voz sólo trae {vuelven} de {de} palabras de la frase "
            f"({recall:.0%}; mínimo {RECALL_MINIMO:.0%}): «{nota.get('transcripcion')}»"
        )
    if not nota.get("completa") or not (nota.get("respuesta") or "").strip():
        return False, (
            f"Deimos no respondió a la nota de voz (respuesta completa: {bool(nota.get('completa'))}, "
            f"{len(nota.get('respuesta') or '')} caracteres)"
        )
    segundos = (nota.get("duracion_ms") or 0) / 1000
    return True, (
        f"nota de voz de {segundos:.1f} s; vuelven {vuelven} de {de} palabras ({recall:.0%}); "
        "Deimos respondió"
    )


def _audio_de_llamada(ev: dict) -> list[dict]:
    """The call's lines of daemon.log (mac.py registros), parsed."""
    filas = []
    for linea in (ev.get("audio_llamada") or "").splitlines():
        try:
            fila = json.loads(linea)
        except ValueError:
            continue
        if isinstance(fila, dict):
            filas.append(fila)
    return filas


def c12_llamada(ev: dict) -> tuple[bool, str]:
    _dato(ev, "llamada")
    if "registros" not in ev or "audio_llamada" not in ev:
        raise SinComprobar("no se recogieron los registros del Mac")
    # The call's own failure; check 10 looks at every other one.
    arranques = [
        f for f in lineas_de_fallo(ev) if (f.get("datos") or {}).get("step") == "audio_start"
    ]
    if arranques:
        return False, "falló audio_start: " + "; ".join(
            f"código {(f.get('datos') or {}).get('code')} "
            f"({(f.get('datos') or {}).get('cause')}, {(f.get('datos') or {}).get('status')}), "
            f"op {f['op']}"
            for f in arranques[:3]
        )
    filas = _audio_de_llamada(ev)
    abiertas = [f.get("fields") or {} for f in filas if f.get("event") == "call_audio_opened"]
    oidas = [f.get("fields") or {} for f in filas if f.get("event") == "call_audio_heard"]
    if not abiertas:
        return False, "daemon.log no tiene ningún call_audio_opened: el micrófono no se abrió"
    if not oidas:
        return False, "daemon.log no tiene ningún call_audio_heard: la llamada no dijo qué oyó"
    oido = oidas[-1]
    tramas = str(oido.get("frames"))
    if not tramas.isdigit() or int(tramas) == 0:
        return False, f"la llamada llegó a escuchar y oyó {tramas} tramas"
    abierta = abiertas[-1]
    return True, (
        f"escucha con el micrófono abierto ({abierta.get('input_channels')} canales a "
        f"{abierta.get('sample_rate')} Hz) y oyó {tramas} tramas (RMS máximo "
        f"{oido.get('loudest_rms')}, umbral {oido.get('speech_rms_threshold')})"
    )


def codigos_de_fallo(notice_rs: str) -> set[str]:
    """The codes `NoticeCode::is_failure` names besides the `failed` class:
    failures that are silent on screen (crates/orbit-core/src/notice.rs). Read
    from the source so the test follows the one place that decides it."""
    inicio = notice_rs.index("pub const fn is_failure(self) -> bool {")
    cuerpo = notice_rs[inicio : notice_rs.index("\n    }\n", inicio)]
    nombres = re.findall(r"Self::([A-Z]\w+)", cuerpo)
    return {re.sub(r"(?<!^)(?=[A-Z])", "_", nombre).lower() for nombre in nombres}


def lineas_de_fallo(ev: dict) -> list[dict]:
    """Every failure the logs recorded, with when, where, code and op."""
    silenciosos = set(ev.get("codigos_fallo") or ())
    fallos = []
    for nombre, texto in sorted(ev.get("registros", {}).items()):
        for linea in texto.splitlines():
            try:
                fila = json.loads(linea)
            except ValueError:
                fallos.append(
                    {"registro": nombre, "codigo": "linea_ilegible", "op": None, "ts": None}
                )
                continue
            if fila.get("class") in CLASES_FALLO or fila.get("code") in silenciosos:
                fallos.append(
                    {
                        "registro": nombre,
                        "codigo": fila.get("code"),
                        "op": fila.get("op"),
                        "clase": fila.get("class"),
                        "datos": fila.get("data"),
                        "ts": _instante(fila.get("ts"), utc=True),
                    }
                )
    for linea in (ev.get("hub_log") or "").splitlines():
        try:
            fila = json.loads(linea)
        except ValueError:
            # uvicorn's own lines are plain text; only the hub's are JSON.
            if "Traceback" in linea or linea.startswith("ERROR"):
                fallos.append({"registro": "hub", "codigo": "sin_formato", "op": None, "ts": None})
            continue
        if fila.get("level") in NIVELES_FALLO:
            fallos.append(
                {
                    "registro": "hub",
                    "codigo": fila.get("code") or fila.get("event"),
                    "op": fila.get("op"),
                    "evento": fila.get("event"),
                    "ts": _instante(fila.get("ts"), utc=False),
                }
            )
    return fallos


def _instante(texto: object, utc: bool) -> float | None:
    """Seconds since the epoch. The Mac writes UTC with a «Z»; the hub writes
    the local time of the machine that runs it, which is this one."""
    if not isinstance(texto, str):
        return None
    try:
        momento = datetime.fromisoformat(texto)
    except ValueError:
        return None
    if momento.tzinfo is None:
        momento = momento.replace(tzinfo=UTC) if utc else momento.astimezone()
    return momento.timestamp()


def c10_sin_fallos(ev: dict) -> tuple[bool, str]:
    if "registros" not in ev:
        raise SinComprobar("no se recogieron los registros del Mac")
    fallos = lineas_de_fallo(ev)
    if fallos:
        return False, "; ".join(_describir(f, ev) for f in fallos[:10])
    nombres = ", ".join(sorted(ev["registros"])) + (
        ", hub" if ev.get("hub_log") is not None else ""
    )
    return True, f"ningún fallo en {nombres}"


def _describir(fallo: dict, ev: dict) -> str:
    paso = paso_de(ev, fallo.get("ts"))
    donde = f" durante «{paso}»" if paso else ""
    return f"{fallo['registro']}: código {fallo['codigo']}, op {fallo['op']}{donde}"


def paso_de(ev: dict, instante: float | None) -> str | None:
    if instante is None:
        return None
    for paso in ev.get("pasos", []):
        if paso["inicio"] <= instante <= paso.get("fin", float("inf")):
            return paso["nombre"]
    return None


CHECKS = {
    1: c1_espanol,
    2: c2_tiempos,
    3: c3_nombre,
    4: c4_repetidas,
    5: c5_nota,
    6: c6_abrir_nota,
    7: c7_oye_deimos,
    8: c8_asistente,
    9: c9_sin_ruido,
    10: c10_sin_fallos,
    11: c11_nota_de_voz,
    12: c12_llamada,
}


def comprobar(ev: dict) -> list[dict]:
    """The twelve results. A step that failed fails the check it serves; the
    checks it left without evidence say which step stopped them."""
    pasos_fallidos = [p for p in ev.get("pasos", []) if not p.get("ok", True)]
    fallos = lineas_de_fallo(ev)
    resultados = []
    for numero, check in CHECKS.items():
        propio = next((p for p in pasos_fallidos if numero in p.get("comprobaciones", ())), None)
        if propio is not None:
            res = resultado(numero, "falla", f"paso «{propio['nombre']}»: {propio['causa']}")
        else:
            try:
                ok, detalle = check(ev)
                res = resultado(numero, "pasa" if ok else "falla", detalle)
            except SinComprobar as causa:
                parada = pasos_fallidos[0] if pasos_fallidos else None
                motivo = (
                    f"la prueba se paró en «{parada['nombre']}»: {parada['causa']}"
                    if parada
                    else str(causa)
                )
                res = resultado(numero, "sin comprobar", motivo)
        if res["estado"] != "pasa" and numero != 10:
            # What the logs said while this check's steps ran.
            ventanas = [p for p in ev.get("pasos", []) if numero in p.get("comprobaciones", ())]
            res["registro"] = [
                {k: f[k] for k in ("registro", "codigo", "op")}
                for f in fallos
                if f.get("ts") is not None
                and any(p["inicio"] <= f["ts"] <= p.get("fin", float("inf")) for p in ventanas)
            ]
        resultados.append(res)
    return resultados
