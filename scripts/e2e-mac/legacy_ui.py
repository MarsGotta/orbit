"""Historical, removed interface vocabulary: negative regression assertions.

These are deliberately frozen; Orbit must never show them again. They are
not active labels or translations and therefore have no catalogue keys.
"""

import re

REMOVED = (
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
    # Renamed by the design's glossary: the old names must not come back.
    "Privacidad detenida",
    "Rearmar",
    "Briefing de hoy",
    "Pregunta a Orbit",
    "Algo ha fallado",
)
REMOVED_PATTERNS = (
    re.compile(r"\d+/\d+ permisos"),
    re.compile(r"\d+ ?bytes"),
    re.compile(r"HTTP \d{3}"),
    re.compile(r"\d+ ms\b"),
)
