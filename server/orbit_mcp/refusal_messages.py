"""The sentences a refusal is said with: one per code, per permission, per
engine cause, and the words for the numbers that follow them."""

# Every message names a control Orbit actually has today: a message that sends
# the user to a switch or a step that no longer exists is worse than a generic one.
from .notes_refusals import MESSAGES as NOTES_MESSAGES

ERROR_MESSAGES = {
    **NOTES_MESSAGES,
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
    "privacy_stopped": "Todo está parado en ese Mac desde sus propios controles: Orbit no ve ni actúa en él hasta que se reactive allí, en el propio Mac. No es un error del Mac ni de la conexión; pregunta de nuevo cuando esté reactivado.",
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
    "recording_not_found": "Esa grabación no existe en el hub: ni ha llegado su audio ni está cerrada. Revisa el identificador con orbit_meetings.",
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
    # A folder request that ended without an answer: nothing to check after
    # it, and no «source» in it.
    "files_timeout": "El Mac no terminó a tiempo de leer en las carpetas del asistente y no entregó nada. Vuelve a pedirlo.",
    "files_helper_restarted": "El helper de Orbit se reinició mientras leía en las carpetas del asistente y no entregó nada. Vuelve a pedirlo.",
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

# The refusal's fields that are said, with the words they are said with, in
# the order they are said.
FIELD_LABELS = {
    "max_page_size": "máximo de entradas por página",
    "max_page": "máxima página",
    "entries": "entradas",
    "max_entries": "máximo de entradas",
    "max_depth": "máximo de niveles",
    "upstream_code": "código de Fast Note Sync",
    "upstream_status": "estado HTTP de Fast Note Sync",
    "timeout_seconds": "plazo en segundos",
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
