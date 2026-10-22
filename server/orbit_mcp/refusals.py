"""What the assistant is told when a tool cannot answer: one sentence per code.

The tables name codes; the Mac's own sentence and the refusal's numbers follow
them, so a refusal says its cause and, when it has them, its sizes, ceilings
and counts.
"""

from __future__ import annotations

import json
import re
import time
from datetime import UTC, datetime
from time import monotonic

from mcp.types import CallToolResult, TextContent

from orbit_hub.captures import CAPTURE_TTL_SECONDS
from orbit_hub.log import report

from . import files, meetings
from .devices import DeviceSelection, UnknownDevice, folded_name, seen_ago
from .hub import MAC_NOTICE_CHARS, HubClientError

# Every message names a control Orbit actually has today: a message that sends
# the user to a switch or a step that no longer exists is worse than a generic one.
ERROR_MESSAGES = {
    "encode_failed": "Orbit no pudo codificar la respuesta de esa lectura.",
    "helper_reply_invalid": "La respuesta del helper no cumple el contrato; se indica el campo y la forma esperada. Su contenido sigue sin conocerse.",
    "read_interrupted": "La lectura se interrumpió por la causa indicada; su contenido sigue sin conocerse.",
    "helper_refused": "El helper rechazó la lectura por la causa indicada.",
    "internal": "Orbit no pudo completar la operación por un fallo interno identificado.",
    "protocol_version": "La versión del protocolo recibida no coincide con la requerida.",
    "hub_reply_invalid": "La respuesta del hub no cumple el contrato; no se usa como una lista vacía.",
    "daemon_reply_invalid": "La respuesta del servicio de Orbit no cumple el contrato; no se usa como una lista vacía.",
    "attachment": "Un adjunto del mensaje no se pudo usar; el mensaje de error dice qué fichero, por qué y con qué números.",
    "calendar_read_limit": "El helper no pudo entregar una lectura de calendario dentro de los límites indicados; nada se recortó.",
    "capture_clock_unavailable": "El helper no pudo comprobar el reloj de la captura. No se entregó ninguna lectura; vuelve a pedirla.",
    "invalid_screenshot_dimensions": "La captura tiene dimensiones no válidas; no se entregó. Vuelve a pedirla si sigue haciendo falta.",
    "invalid_window_dimensions": "La ventana tiene dimensiones no válidas para capturar. Vuelve a pedir las ventanas y elige otra.",
    "recovering": "Orbit está recuperando su estado en ese Mac. Espera a que termine y vuelve a pedir la lectura.",
    "screen_inventory_busy": "El helper sigue enumerando las ventanas para otra petición. Espera a que termine antes de pedirlas otra vez.",
    "screen_inventory_too_large": "El inventario de ventanas supera el tamaño que entrega el helper. No se entregó una lista parcial.",
    "screen_permission_required": "macOS no concede a Orbit Grabación de pantalla en ese Mac. Se concede en Ajustes del Sistema › Privacidad y seguridad › Grabación de pantalla.",
    "screenshot_sdk_failed": "ScreenCaptureKit no pudo obtener la imagen de esa ventana. Vuelve a pedir la captura si sigue haciendo falta.",
    "screenshot_too_large": "El helper no pudo codificar la imagen PNG dentro del tamaño de captura admitido; no se entregó la imagen.",
    "window_identity_changed": "La ventana cambió entre enumerarla y leerla. Vuelve a pedir la ventana o la captura.",
    "invalid_screen_inventory": "El inventario de ventanas no cumple el contrato del helper. Vuelve a pedir las ventanas.",
    "invalid_window_binding": "La identidad de la ventana pedida no es válida. Vuelve a pedir las ventanas y elige una de ellas.",
    "screen_selection_unavailable": "El Mac no puede seleccionar esa ventana ahora. Vuelve a pedir las ventanas.",
    "screen_selection_expired": "La selección de ventana ha caducado. Vuelve a pedir las ventanas antes de capturar.",
    "screenshot_busy": "El helper está terminando otra captura. Espera a que termine antes de pedir otra.",
    "screenshot_expired": "La captura no terminó dentro del plazo del helper. Vuelve a pedirla si sigue haciendo falta.",
    "selected_text_changed": "La selección de texto cambió mientras se leía. Vuelve a pedir el texto seleccionado.",
    "selected_text_classification_incomplete": "El Mac no pudo comprobar si el campo seleccionado es seguro. No se leyó su texto.",
    "selected_text_expired": "La lectura del texto seleccionado caducó. Vuelve a pedirla si sigue haciendo falta.",
    "selected_text_identity_changed": "La ventana o su selección cambió durante la lectura. Vuelve a pedir el texto seleccionado.",
    "selected_text_invalid_range": "El rango del texto seleccionado no es válido. Vuelve a pedir la selección.",
    "selected_text_invalid_text": "El texto que entregó Accesibilidad no es válido; no se usó. Vuelve a pedirlo.",
    "selected_text_limit": "La selección supera el límite de lectura del helper. Pide una selección más corta.",
    "selected_text_secure_field": "La selección está en un campo protegido y Orbit no lo lee. Pide otro campo.",
    "selected_text_empty": "Accesibilidad confirma un rango de selección vacío: no hay texto seleccionado en ese campo.",
    "selected_text_not_exposed": "La aplicación no expone los atributos de Accesibilidad necesarios para leer la selección; no se sabe si hay texto seleccionado.",
    "selected_text_unavailable": "Accesibilidad no pudo entregar la selección de esa ventana; no se sabe si hay texto seleccionado.",
    "selected_text_window_ambiguous": "Accesibilidad no pudo distinguir la ventana pedida de otras. Vuelve a pedir las ventanas y selecciona una inequívoca.",
    "calendar_invalidated": "El calendario cambió durante la lectura. Vuelve a pedir la agenda; su contenido sigue sin conocerse.",
    "calendar_selection_unavailable": "El calendario pedido no está disponible para esta lectura. Su contenido sigue sin conocerse.",
    "invalid_calendar_request": "El helper rechazó los parámetros de la lectura de calendario. Corrígelos antes de volver a pedirla.",
    "expired": "El recurso solicitado caducó en el Mac.",
    "helper_not_running": "El helper de Orbit no está en marcha en ese Mac.",
    "hub_unreachable": "Ese Mac no puede alcanzar el hub de Orbit.",
    "hub_sessions_unpaged": "El hub de Orbit sólo entrega parte de las conversaciones de ese Mac; actualiza el hub para leer las demás.",
    "hub_session_invalid_row": "Una conversación de la lista del hub tiene una fila inválida; actualiza el hub para corregirla.",
    "hub_sessions_cursor_mismatch": "El cursor de conversaciones no coincide con la última fila que entregó el hub; actualiza el hub.",
    "invalid_parameter": "La petición contiene un parámetro inválido; revisa el valor esperado.",
    "legacy": "La causa del Mac usa un código anterior al registro de negativas.",
    "mac_disconnected": "Ese Mac se desconectó del hub de Orbit.",
    "not_found": "El recurso solicitado no existe en ese Mac.",
    "not_paired": "Ese Mac no está emparejado con el hub.",
    "outcome_unknown": "La acción pudo haberse aplicado; consulta su resultado antes de repetirla.",
    "permission_denied": "macOS deniega el permiso indicado para esta petición.",
    "permission_unknown": "El Mac aún no conoce el estado del permiso indicado.",
    "permission_not_granted": "macOS no concede a Orbit el permiso indicado en ese Mac. No dice si se denegó o si aún no se decidió, pero sin él Orbit no puede hacerlo. Se concede en Ajustes del Sistema › Privacidad y seguridad.",
    "revision_changed": "La nota cambió en el Mac mientras se leía o se corregía; vuelve a pedirla desde su revisión actual.",
    "storage_failed": "Falló el almacenamiento durante la operación indicada.",
    "timeout": "La operación agotó su plazo en el Mac; consulta su resultado.",
    "work_in_flight": "Hay otra operación local en vuelo; espera a que termine o cierra la vista. Comprueba el resultado antes de repetir un cambio.",
    "physical_work_finishing": "La operación anterior, ya cerrada, sigue terminando en disco; espera a que termine. Comprueba el resultado antes de repetir un cambio.",
    "work_expired": "El resultado caducó al cambiar la autoridad del Mac; vuelve a solicitar la operación si aún hace falta.",
    "update": "La actualización de Orbit en ese Mac no se completó; la causa dice en qué paso falló y con qué cifras.",
    "too_large": "El recurso excede el tamaño que admite el enlace.",
    "too_many": "La petición contiene más elementos de los permitidos.",
    "upstream_failed": "Falló el servicio del que depende esa operación.",
    "transcription_pending": "La transcripción de esa reunión aún no está lista. Todavía no hay texto que entregar.",
    "transcription_failed": "La transcripción de esa reunión falló. No se reintentará automáticamente; el audio original se conserva. Abre la nota en Orbit y pulsa Reintentar cuando el servicio funcione.",
    "native_command_invalid": "Orbit no pudo construir una orden nativa antes de enviarla al Mac; el helper no rechazó nada.",
    "cancelled": "El Mac canceló la petición antes de completarla; no entregó un resultado.",
    "capture_missing": "La captura enviada por el Mac no estaba disponible cuando el hub preparó la respuesta.",
    "connection_lost": "La conexión con el Mac se perdió durante la acción; el resultado es incierto. No repitas el input sin comprobar su efecto.",
    "device_reply_invalid": "El Mac contestó fuera del contrato de la capacidad pedida. Revisa el campo, el valor esperado y la versión del Mac antes de volver a pedir la lectura; si era una acción, comprueba su efecto y no la repitas.",
    "device_reply_timeout": "El Mac no confirmó la acción dentro del plazo indicado. El resultado es incierto: comprueba su efecto y no la repitas.",
    "hub_response_without_code": "El hub devolvió un error sin código; revisa la versión y el estado del hub.",
    "internal_error": "Orbit encontró un fallo interno al preparar la respuesta.",
    "device_unavailable": "El Mac no está conectado a Orbit. Comprueba que Orbit esté abierto y conectado.",
    # What lets Deimos see and act on a Mac is the macOS permission the user
    # grants, and nothing else. Each code names the one that is missing.
    "accessibility_permission_required": "macOS no concede a Orbit el permiso de Accesibilidad en ese Mac, y sin él no puede usar el ordenador. Se concede en Ajustes del Sistema › Privacidad y seguridad › Accesibilidad.",
    "screen_recording_permission_required": "macOS no concede a Orbit el permiso de Grabación de pantalla en ese Mac, y sin él no puede ver la pantalla. Se concede en Ajustes del Sistema › Privacidad y seguridad › Grabación de pantalla.",
    "computer_permissions_required": "macOS no concede a Orbit Accesibilidad ni Grabación de pantalla en ese Mac, y sin ellas no puede ver ni usar el ordenador. Se conceden en Ajustes del Sistema › Privacidad y seguridad.",
    "engine_unavailable": "El motor de control no está disponible en ese Mac. Comprueba su estado antes de reintentarlo.",
    "no_answer": "El Mac no contestó a tiempo. Comprueba su conexión antes de volver a intentarlo.",
    "catalogue_changed": "El catálogo del motor cambió mientras se leía. Vuelve a pedir la lista de herramientas.",
    "unknown_capability": "El Mac no reconoce esa capacidad. Actualiza Orbit antes de reintentarlo.",
    # These three codes arrived from the Mac's own Refusals but had no entry
    # here, so Deimos read a message without a cause. Each one now says what
    # stands between the request and the Mac.
    "privacy_stopped": "La privacidad está detenida en ese Mac desde sus propios controles: Orbit no ve ni actúa en él hasta que se rearme allí, en el propio Mac. No es un error del Mac ni de la conexión; pregunta de nuevo cuando esté rearmada.",
    "busy": "Ese Mac ya está atendiendo otra petición. No es una negativa: espera a que termine y vuelve a pedirla.",
    "pending_recording": "Ese Mac ya tiene una grabación pendiente. Termínala o espera a que cierre antes de iniciar otra captura o llamada.",
    "hub_unavailable": "El hub de Orbit no está disponible. Comprueba el servicio antes de reintentarlo.",
    "invalid_response": "Orbit devolvió una respuesta no válida. Revisa las versiones del hub y del Mac.",
    "invalid_parameters": "Los parámetros no son válidos. Corrige sus tipos y límites antes de reintentarlo.",
    "operation_not_found": "No se encontró esa operación en el Mac elegido. Revisa el identificador y el dispositivo.",
    "meetings_not_configured": "Falta la clave privada de reuniones en la configuración de Orbit MCP.",
    # Settings › Agents: what each agent may use. Not macOS's permission and
    # not the Mac's refusal; the user changes it in Orbit's settings.
    "agent_access_disabled": "Este agente tiene desactivado en Ajustes › Agentes el acceso al que pertenece esta herramienta, así que no puede usarla. No es un permiso de macOS ni un fallo del Mac.",
    "agent_not_found": "El hub no tiene el agente para el que se arrancó este traductor (ORBIT_MCP_AGENT). Revisa la configuración de ese agente.",
    # orbit_skill: the agent's own skills, which the hub keeps.
    "skill_not_enabled": "Este agente no tiene activada en Orbit ninguna skill con ese nombre. Usa uno de los de tu lista de skills.",
    "skills_not_in_hub": "Orbit no guarda las skills de este agente: son las de su perfil de Hermes, que se cargan con sus propias herramientas.",
    # A recording and its note, read from the hub. Each state of a note
    # that is not there has its own code; none of them means an empty meeting.
    "recording_not_found": "Esa grabación no existe en el hub para ese Mac: ni ha llegado audio suyo ni está cerrada. Revisa el identificador con orbit_meetings.",
    "note_not_requested": "Nadie ha pedido todavía la nota de esa grabación desde el Mac, así que no existe. No significa que la reunión no tenga contenido: su transcripción se lee con orbit_transcript.",
    "note_not_configured": "El hub no tiene configurado el modelo que escribe las notas, así que la de esa grabación no se ha escrito. No es un fallo de la reunión: su transcripción se lee con orbit_transcript.",
    "note_waiting_transcript": "La nota de esa grabación espera a que termine su transcripción; todavía no hay nota que leer. Lo transcrito hasta ahora se lee con orbit_transcript.",
    "note_generating": "El modelo está escribiendo ahora la nota de esa grabación; todavía no hay nota que leer. Vuelve a pedirla en un par de minutos.",
    "note_queued": "La nota de esa grabación falló por una causa pasajera y el hub la volverá a intentar solo; todavía no hay nota que leer.",
    "note_failed": "La nota de esa grabación no se pudo escribir y el hub no lo volverá a intentar solo. Se puede pedir otra vez desde la nota en Orbit cuando la causa esté resuelta.",
    "attachment_not_found": "Ese adjunto no está en el hub para ese Mac, o no es de una conversación de este agente. Usa el id que da el mensaje que lo adjuntó.",
    "after_index_out_of_range": "after_index pasa del final del texto pedido; empieza de nuevo sin after_index o con uno menor.",
    # Orbit no longer keeps a copy of the agenda anywhere, so every code
    # that described one is gone with it. What is left are the conditions that
    # really stand between Deimos and the user's agenda — the macOS permission, the
    # Mac being reachable, its reader — and each one says which it is. None of
    # them may read as «no tiene nada ese día»: that is an ordinary answer.
    "calendar_permission_required": "Orbit no tiene acceso de lectura completo a tus Calendarios en macOS, así que no puede leer la agenda. Esto NO significa que la agenda esté vacía. Concédelo en Ajustes del Sistema › Privacidad y seguridad › Calendarios.",
    # Unknown is its own state. The Mac has not said yet whether it holds
    # the permission — it has just started, or its reader is reconnecting — and
    # saying «no tiene acceso» or pretending it does would both be untrue.
    "calendar_permission_unknown": "Orbit todavía no sabe si tiene permiso para leer los Calendarios de ese Mac: el Mac aún no lo ha dicho, porque acaba de arrancar o su lector se está reconectando. No es una negativa ni significa que la agenda esté vacía. Vuelve a preguntar en unos segundos.",
    "calendar_backend_unavailable": "EventKit no está disponible en ese Mac ahora mismo, así que Orbit no puede leer su agenda. Esto NO significa que la agenda esté vacía. Comprueba el estado de Calendarios en macOS y vuelve a preguntar cuando EventKit esté disponible.",
    "calendar_helper_stopped": "El helper de Orbit está detenido en ese Mac; la agenda sigue sin conocerse. Esto NO significa que esté vacía.",
    "calendar_read_restarted": "El Mac perdió la lectura de la agenda a mitad, porque su lector se reinició, y Orbit la empezó de nuevo sin llegar a terminarla. Vuelve a preguntar en unos segundos.",
    "capture_stopped": "El Mac cortó su captura mientras leía la agenda (por ejemplo, porque se cerró la tapa o la sesión quedó inactiva). No es una parada de privacidad y la lectura se puede reintentar.",
    "calendar_changed": "La agenda cambió varias veces mientras Orbit la leía, así que no hay una lectura coherente que entregar. Vuelve a preguntar en unos segundos.",
    # What EventKit itself could not do. The Mac's own sentence follows each
    # one with its numbers; none of them is an empty agenda.
    "calendar_event_too_large": "Una página de la agenda del Mac pasa del mayor mensaje que su lector puede entregar aun dejando fuera, con su nombre, los eventos que no caben, porque la lista de esos eventos es demasiado larga. Nada se recortó. Esto NO significa que la agenda esté vacía; pregunta por un rango más corto.",
    "calendar_limit_exceeded": "Un evento o calendario tiene un campo más largo de lo que Orbit lee, y la lectura se detuvo en vez de recortarlo. Esto NO significa que la agenda esté vacía.",
    "calendar_read_expired": "EventKit no terminó de leer la agenda en el plazo de una lectura, ni siquiera en el tramo más corto que Orbit le pide. Vuelve a preguntar en unos segundos.",
    "calendar_generation_changed": "La generación de Orbit cambió durante la lectura de la agenda, así que la respuesta anterior no se usó. Esto NO significa que la agenda esté vacía.",
    "calendar_reply_replaced": "La respuesta pertenecía a otra lectura de la agenda y Orbit no la usó. Esto NO significa que la agenda esté vacía.",
    "calendar_clock_unavailable": "Orbit no pudo verificar el reloj de esa lectura de la agenda. Esto NO significa que la agenda esté vacía.",
    "generation_changed": "La generación de Orbit cambió durante la lectura de la fuente; esa respuesta anterior no se usó.",
    "conversation_changed": "La conversación cambió durante la lectura de la fuente; esa respuesta anterior no se usó.",
    "helper_restarted": "El helper se reinició durante la lectura de la fuente; la petición anterior no tiene respuesta válida.",
    "source_reply_replaced": "La respuesta pertenece a otra lectura de la fuente y no se usó.",
    "native_refused": "El helper rechazó la lectura de la fuente; no hay un resultado utilizable.",
    "calendar_read_failed": "EventKit devolvió un error al leer la agenda en el Mac. Esto NO significa que la agenda esté vacía.",
    "calendar_busy": "EventKit seguía terminando una lectura anterior de la agenda en ese Mac y Orbit no la interrumpe. Vuelve a preguntar en unos segundos.",
    "unknown_device": "No se encontró ese Mac en Orbit. Revisa su nombre con orbit_devices.",
    "config_invalid": "La configuración de Orbit MCP cambió y ya no es válida. Corrige mcp.toml.",
    # The assistant's folders: each says its path and folders in
    # `files.refusal_message`; these are the sentences without them.
    "no_folders": "Quien usa Orbit no ha dado al asistente ninguna carpeta de ese Mac.",
    "outside_folders": "Esa ruta está fuera de las carpetas que el asistente puede leer.",
    "folder_unavailable": "Esa carpeta del asistente no se puede abrir ahora.",
    "file_unavailable": "Ese archivo de las carpetas del asistente no se puede leer como se pidió.",
    "file_too_large": "Ese archivo pesa más de lo que Orbit lee de su tipo.",
}

TYPED_PERMISSION_MESSAGES = {
    ("permission_denied", "calendar"): ERROR_MESSAGES["calendar_permission_required"],
    ("permission_unknown", "calendar"): ERROR_MESSAGES["calendar_permission_unknown"],
    ("permission_denied", "screen_recording"): ERROR_MESSAGES[
        "screen_recording_permission_required"
    ],
    ("permission_denied", "accessibility"): ERROR_MESSAGES["accessibility_permission_required"],
    ("permission_denied", "screen_and_accessibility"): ERROR_MESSAGES[
        "computer_permissions_required"
    ],
    # A `false` from the helper is a known fact — not granted — but macOS does
    # not say whether the user denied it or never decided.
    ("permission_not_granted", "calendar"): ERROR_MESSAGES["calendar_permission_required"],
    ("permission_not_granted", "screen_recording"): ERROR_MESSAGES[
        "screen_recording_permission_required"
    ],
    ("permission_not_granted", "accessibility"): ERROR_MESSAGES[
        "accessibility_permission_required"
    ],
    ("permission_not_granted", "screen_and_accessibility"): ERROR_MESSAGES[
        "computer_permissions_required"
    ],
    (
        "permission_unknown",
        "screen_recording",
    ): "Orbit todavía no sabe si macOS le concede Grabación de pantalla en ese Mac. No es una negativa; vuelve a preguntar cuando el helper informe su estado.",
    (
        "permission_unknown",
        "accessibility",
    ): "Orbit todavía no sabe si macOS le concede Accesibilidad en ese Mac. No es una negativa; vuelve a preguntar cuando el helper informe su estado.",
    (
        "permission_unknown",
        "screen_and_accessibility",
    ): "Orbit todavía no sabe si macOS le concede Grabación de pantalla y Accesibilidad en ese Mac. No es una negativa; vuelve a preguntar cuando el helper informe su estado.",
}

ENGINE_CAUSE_MESSAGES = {
    "not_installed": "El motor de control no está instalado en ese Mac.",
    "not_running": "El motor de control no está en marcha en ese Mac.",
    "unhealthy": "El motor de control dejó de responder en ese Mac; comprueba su estado.",
    "unsupported": "Esta versión de Orbit no admite ese motor o esa función en el Mac; actualiza Orbit.",
    "voice_processing_unsupported": "La llamada necesita VoiceProcessing; este Mac no admite el procesamiento de voz solicitado.",
    "initialization_failed": "El motor de control no logró arrancar en ese Mac; comprueba su estado.",
}


def mac_sentence(detail) -> str:
    """What the Mac itself said about this refusal, if it said anything.

    The tables in this file name codes; only the Mac knows which
    parameter it rejected, which permission is missing or what it was busy
    with. The sentence arrives bounded and on one line from `hub.py`, and is
    bounded again here: a message to a reader is not a place to trust a length.
    """
    if not isinstance(detail, dict):
        return ""
    notice = detail.get("mac_notice")
    if not isinstance(notice, str):
        return ""
    notice = " ".join("".join(c for c in notice if c.isprintable() or c.isspace()).split())
    return re.sub(r"(?:/Users/|/home/|\$HOME)[^\s,;)]*", "[ruta local]", notice[:MAC_NOTICE_CHARS])


def _refusal_sentence(detail) -> str:
    if not isinstance(detail, dict) or not isinstance(detail.get("refusal"), dict):
        return ""
    refusal = detail["refusal"]
    legacy_cause = refusal.get("text") if refusal.get("code") == "legacy" else None
    labels = {
        "resource": "recurso",
        "bytes": "tamaño",
        "chars": "caracteres",
        "max_chars": "máximo de caracteres",
        "start_ms": "inicio en ms",
        "end_ms": "fin en ms",
        "max_bytes": "máximo",
        "count": "elementos",
        "delivered": "conversaciones entregadas",
        "total": "conversaciones totales",
        "position": "posición",
        "field": "campo",
        "rows": "filas de la página",
        "max": "máximo",
        "in_flight": "en vuelo",
        "started_generation": "generación inicial",
        "current_generation": "generación actual",
        "waited_ms": "espera",
        "deadline_ms": "plazo",
        "retry_after_ms": "espera sugerida",
        "since_ms": "desde",
        "last_seen_ms": "última conexión",
        "expired_at_ms": "caducó en",
        "status": "estado HTTP",
        "permission": "permiso",
        "operation": "operación",
        "service": "servicio",
        "cause": "causa",
        "parameter": "parámetro",
        "expected": "valor esperado",
        "received_keys": "claves recibidas",
        "received": "recibida",
        "required": "requerida",
        "received_bytes": "bytes recibidos",
        "helper_code": "causa del helper",
        "issue": "problema de la respuesta",
        "interruption": "motivo de la interrupción",
        "step": "paso de recuperación",
        "elapsed_ms": "ms transcurridos",
        "received_type": "tipo recibido",
        "device_version": "versión del Mac",
        "operation_id": "identificador de operación",
        "retry_in_ms": "nuevo intento en",
        "detail": "detalle",
        "expected_revision": "revisión pedida",
        "actual_revision": "revisión actual",
        "expected_state_sha256": "estado pedido",
        "actual_state_sha256": "estado actual",
        "id": "identificador de grabación",
        "recording_id": "identificador de grabación",
        "title": "título",
        "phase": "fase",
        "sealed_chunks": "fragmentos guardados",
        "gaps": "huecos",
        "expires_at_ms": "caduca en",
        "agent_id": "agente",
        "group": "acceso",
        "agent_ids": "agentes del hub",
    }
    fields = []
    if isinstance(legacy_cause, str):
        fields.append(f"causa del Mac: {mac_sentence({'mac_notice': legacy_cause})}")
    for key, label in labels.items():
        value = refusal.get(key)
        if isinstance(value, (str, int, dict)) and not isinstance(value, bool):
            shown = json.dumps(value, ensure_ascii=False) if isinstance(value, dict) else str(value)
            if key == "detail":
                shown = mac_sentence({"mac_notice": shown})
            fields.append(f"{label}: {shown}")
    return "; ".join(fields) + "." if fields else ""


def _walk_sentence(detail) -> str:
    """What this translator counted while walking the Mac's pages, if anything.

    Built here, from numbers, never from what the Mac sent: it is how a refusal
    that ended a walk says how many times the walk was tried.
    """
    if not isinstance(detail, dict):
        return ""
    notice = detail.get("walk_notice")
    return notice if isinstance(notice, str) else ""


def error_result(
    error: str | HubClientError, selection: DeviceSelection | None = None
) -> CallToolResult:
    code = error.code if isinstance(error, HubClientError) else error
    if code == "device_unavailable" and selection is not None:
        device = selection.device
        name = device.name if device is not None else selection.requested_name
        newer = [
            candidate
            for candidate in selection.same_name
            if candidate.id != selection.device_id
            and (device is None or candidate.paired_at > device.paired_at)
        ]
        if newer:
            latest = max(newer, key=lambda candidate: (candidate.paired_at, candidate.id))
            message = (
                f"El Mac no está conectado a Orbit. {name} se volvió a emparejar hace "
                f"{seen_ago(latest.paired_at).removeprefix('hace ')}; usa la nueva identidad."
            )
        elif device is not None:
            message = (
                f"El Mac no está conectado a Orbit. {name} fue visto por última vez "
                f"{seen_ago(device.last_seen)}."
            )
        else:
            message = f"El Mac no está conectado a Orbit: {name}."
        return CallToolResult(content=[_text_content(f"{code}: {message}")], isError=True)
    detail = getattr(error, "detail", None)
    spoken_code = code
    if code == "legacy" and isinstance(detail, dict):
        refusal = detail.get("refusal")
        legacy_text = refusal.get("text") if isinstance(refusal, dict) else None
        if isinstance(legacy_text, str):
            match = re.match(r"([a-z0-9_]{1,64})(?:: |$)", legacy_text)
            if match:
                spoken_code = match.group(1)
    message = ERROR_MESSAGES.get(spoken_code)
    refusal = detail.get("refusal") if isinstance(detail, dict) else None
    if isinstance(refusal, dict):
        # Capability models are checked by this consumer before rendering their fields.
        if (
            code == "upstream_failed"
            and refusal.get("service") == "hub"
            and detail.get("capability") == "screenshot"
        ):
            message = "El Mac no pudo entregar la captura a Orbit. Comprueba su conexión antes de volver a pedirla."
        if (
            code == "internal"
            and isinstance(refusal.get("cause"), dict)
            and refusal["cause"].get("kind") == "native_command"
        ):
            message = ERROR_MESSAGES["native_command_invalid"]
        if code == "helper_refused" and isinstance(refusal.get("helper_code"), str):
            message = ERROR_MESSAGES.get(refusal["helper_code"], message)
        if code == "read_interrupted" and isinstance(refusal.get("cause"), dict):
            reason = refusal["cause"].get("kind")
            if isinstance(reason, str):
                message = ERROR_MESSAGES.get(reason, message)
        if code == "recovering":
            steps = {
                "chunks": "fragmentos",
                "upload_jobs": "subidas pendientes",
                "snapshot": "instantánea",
                "environment": "preparación del servicio",
                "operations": "operaciones pendientes",
                "call_preference": "preferencia al detectar una llamada",
            }
            step = steps.get(refusal.get("step"), "desconocido")
            message = f"El almacén se está recuperando: paso {step}; la orden no se aplicó. Espera a que termine la recuperación."
        if code == "skill_not_enabled" and isinstance(refusal.get("name"), str):
            enabled = refusal.get("enabled")
            listed = (
                enabled.replace(",", ", ") if isinstance(enabled, str) and enabled else "ninguna"
            )
            message = (
                f"Este agente no tiene activada en Orbit ninguna skill llamada «{refusal['name']}». "
                f"Las activadas son: {listed}."
            )
        if code == "busy" and refusal.get("operation") == "chat":
            message = "No se puede cambiar de conversación: hay un turno en curso; interrúmpelo o espera a que termine."
        if code == "busy" and refusal.get("operation") == "audio":
            message = "No se puede cambiar de conversación: hay una nota de voz en el búfer; termínala o interrúmpela."
        if code == "busy" and refusal.get("operation") == "voice_transcription":
            message = "No se puede cambiar de conversación: la transcripción de voz sigue en curso; espera o interrúmpela."
        if code == "busy" and refusal.get("operation") == "audio_session":
            message = "Finaliza PTT o la grabación actual antes de cambiar a la ruta de llamada."
        if code == "busy" and refusal.get("operation") == "audio_drain":
            message = "El audio anterior sigue drenándose o esperando almacenamiento durable; espera antes de continuar."
        if code == "too_large" and refusal.get("resource") == "ipc_response":
            message = "La respuesta supera el tamaño admitido. La orden pudo haberse aplicado; consulta el resultado antes de repetirla."
        if code == "invalid_parameter" and isinstance(refusal.get("expected"), dict):
            expected = refusal["expected"]
            kind = expected.get("kind")
            if kind == "open_recording":
                message = "Inicia otra grabación; la anterior ya tiene un cierre durable y no puede reanudarse."
            elif kind == "wire_field":
                field, required = expected.get("field"), expected.get("expected")
                words = {
                    "object": "un objeto",
                    "boolean": "verdadero o falso",
                    "nonnegative_integer": "un entero no negativo",
                    "unix_milliseconds_integer": "un entero en milisegundos desde 1970",
                    "string": "texto",
                }
                message = f"{field} debe ser {words.get(required, required)}."
            elif kind == "selection_filters":
                message = f"Filtro {expected.get('unknown')} desconocido; se admiten {', '.join(expected.get('allowed', []))}."
            elif kind == "desktop_frame":
                message = "Pide antes una captura con full_screen=true: las coordenadas de escritorio necesitan esa imagen."
            elif kind == "desktop_coordinate":
                message = f"La coordenada {expected.get('axis')}={expected.get('received')} queda fuera de la captura de {expected.get('image_width')} × {expected.get('image_height')} píxeles; corrígela antes de repetir."
            elif kind == "selection":
                message = (
                    "No hay ventanas que coincidan con los filtros."
                    if expected.get("reason") == "window_not_found"
                    else "Las ventanas coincidentes no se pueden seleccionar."
                )
                message += f" {expected.get('windows')} ventanas, {expected.get('matching')} coincidencias, {expected.get('selectable')} seleccionables y {expected.get('omitted')} omitidas. Vuelve a listar las ventanas antes de elegir otra."
            elif kind == "calendar_range":
                message = f"Rango de calendario inválido: se exige {expected.get('min_ms')} ≤ start_ms={expected.get('start_ms')} < end_ms={expected.get('end_ms')} ≤ {expected.get('max_ms')}."
            elif kind == "calendar_cursor":
                message = "Para seguir una lectura de agenda hacen falta read_id no vacío, after_ms y after_index; corrige el cursor con los datos recibidos."
            elif kind == "full_screen_filters":
                message = "full_screen=true no admite filtros de aplicación ni de título."
            elif kind == "tool_call":
                message = "La acción del motor no cumple el contrato: revisa el nombre, el objeto de argumentos y sus tamaños antes de repetirla."
            elif kind == "title_length":
                message = f"El título tiene {expected.get('characters')} caracteres; máximo {expected.get('max_characters')}."
        permission = refusal.get("permission")
        cause = refusal.get("cause")
        if code in {"transcription_failed", "transcription_pending"}:
            attempts = refusal.get("attempts")
            provider_status = refusal.get("provider_status")
            timeout_ms = refusal.get("timeout_ms")
            retry_at_ms = refusal.get("retry_at_ms")
            phase = refusal.get("phase")
            if type(provider_status) is int:
                cause_text = f"El servicio devolvió HTTP {provider_status}."
            elif cause == "transcription_connection_timeout" and type(timeout_ms) is int:
                cause_text = f"La conexión con el servicio tardó más de {timeout_ms} ms."
            elif type(timeout_ms) is int:
                cause_text = f"El servicio tardó más de {timeout_ms} ms."
            elif cause == "transcription_protocol_error":
                cause_text = "El servicio cortó la respuesta antes de completarla."
            elif cause == "transcription_queued":
                cause_text = "El grupo de audio está en cola."
            elif cause == "transcription_in_progress":
                cause_text = "El grupo de audio se está transcribiendo."
            elif cause == "transcription_group_not_ready":
                cause_text = "El grupo de audio aún no está preparado."
            else:
                cause_text = (
                    f"Causa: {cause}." if isinstance(cause, str) else "Causa ausente en el hub."
                )
            if type(attempts) is int and phase == "running":
                count_text = f" El intento {attempts} está en curso."
            else:
                count_text = f" {attempts} intentos." if type(attempts) is int else ""
            retry_text = ""
            if type(retry_at_ms) is int and phase == "pending":
                try:
                    retry_time = datetime.fromtimestamp(retry_at_ms // 1000, tz=UTC)
                    retry_text = (
                        f" Próximo intento programado para {retry_time:%Y-%m-%d %H:%M:%S} UTC."
                    )
                except (OverflowError, OSError, ValueError):
                    remaining = max(0, (retry_at_ms - int(time.time() * 1000)) // 1000)
                    retry_text = f" Próximo intento programado dentro de {remaining} segundos."
            message = f"{message} {cause_text}{count_text}{retry_text}"
        if code in {
            "permission_denied",
            "permission_unknown",
            "permission_not_granted",
        } and isinstance(permission, str):
            message = TYPED_PERMISSION_MESSAGES.get((code, permission), message)
        elif code == "engine_unavailable" and isinstance(cause, str):
            message = ENGINE_CAUSE_MESSAGES.get(cause, message)
        elif code == "too_many" and refusal.get("resource") == "capture":
            count, maximum = refusal.get("count"), refusal.get("max")
            if type(count) is int and type(maximum) is int:
                message = (
                    f"El hub ya guarda {maximum} capturas pendientes de este Mac; esta subida "
                    f"sería la número {count}. Espera a que se consuman o caduquen: cada una "
                    f"se guarda como máximo {int(CAPTURE_TTL_SECONDS)} s."
                )
    if message is None:
        message = f"El código {spoken_code} no tiene frase en este traductor."
    if code in files.REFUSALS and isinstance(refusal, dict):
        message = files.refusal_message(code, refusal)
        numbers = ""
    elif code in meetings.NOTE_REFUSALS or code == "after_index_out_of_range":
        # Their numbers are already words: when, why and how many times.
        numbers = meetings.refusal_sentence(code, refusal)
    else:
        numbers = _refusal_sentence(detail)
    if numbers:
        message = f"{message} {numbers}"
    if isinstance(error, UnknownDevice):
        known = []
        seen = set()
        for device in error.devices:
            normalized = folded_name(device.name)
            if normalized in seen:
                continue
            seen.add(normalized)
            marker = " (predeterminado)" if device.id == error.default_device_id else ""
            known.append(f"{device.name}{marker}")
        message = f"{message} Conozco: {', '.join(known) if known else 'ninguno'}."
    mac = mac_sentence(getattr(error, "detail", None))
    if mac:
        message = f"{message} {mac}"
    walk = _walk_sentence(getattr(error, "detail", None))
    if walk:
        message = f"{message} {walk}"
    text = f"{spoken_code}: {message}"
    return CallToolResult(content=[_text_content(text)], isError=True)


def unexpected_result(device: str | None, started: float) -> CallToolResult:
    report(
        event="orbit_mcp_tool_failed",
        code="internal_error",
        device=device,
        elapsed_ms=max(0, int((monotonic() - started) * 1000)),
    )
    return error_result("internal_error")


def _text_content(value: str) -> TextContent:
    return TextContent(type="text", text=value)
