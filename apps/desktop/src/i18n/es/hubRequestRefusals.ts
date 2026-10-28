import { formatQuantity } from "../locale";
import type { Numbers } from "./refusalNumbers";

export const hubRequestRefusals = {
  not_found: () => "Dispositivo desconocido",
  note_not_ready: () => "La nota todavía no está escrita",
  note_not_requested: () => "Nadie ha pedido todavía la nota de esta grabación",
  note_template_builtin: (n) =>
    n.template_id === undefined
      ? undefined
      : `La plantilla «${formatQuantity(n.template_id)}» es de Orbit y no se cambia ni se borra: guarda los cambios como una plantilla propia con forked_from`,
  note_template_links_full: (n) =>
    n.links === undefined || n.max_links === undefined
      ? undefined
      : `Ya hay ${formatQuantity(n.links)} series con plantilla asignada y el máximo es ${formatQuantity(n.max_links)}: quita una antes de asignar otra`,
  note_template_not_found: (n) =>
    n.template_id === undefined
      ? undefined
      : `No existe la plantilla «${formatQuantity(n.template_id)}»`,
  note_templates_full: (n) =>
    n.max_templates === undefined || n.templates === undefined
      ? undefined
      : `Ya hay ${formatQuantity(n.templates)} plantillas propias y el máximo es ${formatQuantity(n.max_templates)}: borra una antes de crear otra`,
  note_templates_too_large: (n) =>
    n.bytes === undefined || n.max_bytes === undefined
      ? undefined
      : `Con esta plantilla la lista ocuparía ${formatQuantity(n.bytes)} bytes y el máximo es ${formatQuantity(n.max_bytes)}: acorta sus instrucciones o borra otra`,
  operation_conflict: () => "El ID ya pertenece a otra operación",
  outcome_unknown: () => "Control ya recibido; no se repetirá automáticamente",
  permission_choice_unavailable: () => "Esta petición ofrece una vez o denegar",
  personality_too_large: (n) =>
    n.bytes === undefined || n.max_bytes === undefined
      ? undefined
      : `La personalidad ocupa ${formatQuantity(n.bytes)} bytes; máximo ${formatQuantity(n.max_bytes)}`,
  rate_limited: () => "Cola de transcripción llena; conserva el audio",
  recording_deleted: () => "La grabación se borró; no admite nada más",
  recording_finalized: () => "La captura está cerrada; no admite nuevos fragmentos",
  recording_id_ambiguous: () => "Hay dos grabaciones con el mismo identificador en distintos Mac",
  recording_not_found: () => "Grabación no disponible para este dispositivo",
  recording_still_open: () => "La grabación todavía está abierta",
  revoked: () => "Dispositivo revocado",
  safety_capacity: () => "Demasiados controles activos; la revocación local sigue disponible",
  safety_receipt_invalid: () => "Recibo de control fuera de contrato",
  session_busy: () => "Espera o interrumpe el turno en curso",
  session_disconnected: () =>
    "La sesión no está conectada con el asistente; se intentará reanudar la conversación al enviar otro mensaje si el turno anterior está cerrado",
  session_message_too_large: () => "Un mensaje de la conversación no cabe en una página",
  session_resume_busy: () => "Hermes sigue cerrando el turno anterior (código 4009); espera",
  session_resume_closed: () => "Se cerró la sesión de Orbit; no se envió el mensaje",
  session_resume_connection_lost: () =>
    "Se perdió Hermes durante la reanudación; no se envió el mensaje",
  session_resume_history_unreadable: () =>
    "Hermes no pudo leer el historial guardado (código 5000); no se envió el mensaje",
  session_resume_not_found: () =>
    "Hermes no conserva la conversación guardada (código 4007); el historial local sigue disponible",
  session_resume_race: () =>
    "Hermes cerró la sesión durante la reanudación (código 4007); vuelve a intentarlo",
  session_resume_storage_unavailable: () =>
    "Hermes no puede abrir su almacén de sesiones (código 5000)",
  session_resume_too_large: () =>
    "Hermes rechazó reanudar por tamaño (código 4130); no entregó cuenta y límite",
  session_resume_unsupported: () =>
    "Esta versión de Hermes no permite reanudar sesiones guardadas (código -32601)",
  session_runtime_not_found: () => "La sesión del agente ya no está activa",
  shutting_down: () => "Servidor deteniéndose",
  sign_in_code_invalid: () =>
    "El código no es el que da la página de Claude: cópialo entero, con su «#»",
  sign_in_not_waiting_for_code: () => "Este inicio de sesión no espera ningún código",
  skill_delete_unsupported: () =>
    "El panel de Hermes no puede borrar skills: desactívala aquí o bórrala en el servidor de Hermes",
  skill_not_found: (n) =>
    n.name === undefined
      ? undefined
      : `El agente no tiene ninguna skill llamada «${formatQuantity(n.name)}»`,
  skill_too_large: (n) =>
    n.bytes === undefined || n.max_bytes === undefined || n.name === undefined
      ? undefined
      : `La skill «${formatQuantity(n.name)}» ocupa ${formatQuantity(n.bytes)} bytes; máximo ${formatQuantity(n.max_bytes)}`,
  speech_empty: () => "El servicio de voz entregó un clip vacío",
  speech_not_configured: () => "No hay escucha configurada para este asistente",
  speech_too_large: () => "El clip de voz supera el límite de 4 MiB",
  speech_unavailable: () => "Hermes no entregó el clip de voz configurado",
  stale_group_attempt: () => "Resultado de un intento de transcripción obsoleto",
  stale_revision: (n) =>
    n.base_revision === undefined || n.revision === undefined
      ? undefined
      : `Las notas privadas cambiaron: revisión ${formatQuantity(n.revision)}, el cambio partía de ${formatQuantity(n.base_revision)}`,
  stale_turn: () => "El control no pertenece al turno actual",
  storage_full: () =>
    "El disco del servidor está lleno y no se guardó nada; libera espacio en el servidor",
  too_large: () => "Mensaje demasiado grande",
  too_many: () => "Demasiadas capturas pendientes",
  too_many_agents: (n) =>
    n.count === undefined || n.max === undefined
      ? undefined
      : `Este hub ya tiene ${formatQuantity(n.count)} asistentes; máximo ${formatQuantity(n.max)}`,
  too_many_skills: (n) =>
    n.count === undefined || n.max === undefined
      ? undefined
      : `El agente ya tiene ${formatQuantity(n.count)} skills; máximo ${formatQuantity(n.max)}. Borra alguna antes de crear otra`,
  transcription_audio_empty: () => "El audio de transcripción está vacío",
  transcription_connection_error: () => "No se pudo conectar con el servicio de transcripción",
  transcription_connection_timeout: (n) =>
    n.timeout_ms === undefined
      ? undefined
      : `La conexión con el servicio de transcripción tardó más de ${formatQuantity(n.timeout_ms)} ms`,
  transcription_failed: () => "Audio de reunión fuera de límites",
  transcription_http_error: (n) =>
    n.provider_status === undefined
      ? undefined
      : `El servicio de transcripción devolvió HTTP ${formatQuantity(n.provider_status)}`,
  transcription_invalid_response: () =>
    "El servicio de transcripción devolvió una respuesta inválida",
  transcription_key_missing: () => "Configura la clave de OpenRouter en Ajustes",
  transcription_protocol_error: () =>
    "El servicio de transcripción cerró la respuesta sin completarla",
  transcription_provider_error: () =>
    "El servicio de transcripción falló antes de entregar una respuesta válida",
  transcription_provider_refused: (n) =>
    n.duration_ms === undefined || n.provider_status === undefined
      ? undefined
      : `El servicio de transcripción no aceptó ${formatQuantity(n.duration_ms)} ms de audio (HTTP ${formatQuantity(n.provider_status)})`,
  transcription_timeout: (n) =>
    n.timeout_ms === undefined
      ? undefined
      : `El servicio de transcripción tardó más de ${formatQuantity(n.timeout_ms)} ms`,
  transcription_unavailable: () => "Worker de transcripción detenido; conserva el audio",
  transcription_word_out_of_range: () => "Tiempo de palabra fuera de su ventana",
  transcription_words_missing: () => "Whisper no devolvió tiempos por palabra",
  turn_id_required: () => "El control requiere la identidad del turno original",
  turn_no_longer_exists: () => "El turno ya no existe en la transcripción",
  unauthorized: () => "Credencial de dispositivo requerida",
  unknown_speaker: () => "El hablante ya no existe en la transcripción",
  unsupported_capture_type: () => "Se requiere una imagen PNG",
  upstream_protocol: () => "Hermes omitió la identidad durable",
  upstream_unavailable: () => "Hermes no está conectado",
} satisfies Record<string, (n: Numbers) => string | undefined>;

export type HubRequestCode = keyof typeof hubRequestRefusals;
