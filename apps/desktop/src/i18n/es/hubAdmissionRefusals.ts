import { formatQuantity } from "../locale";
import type { Numbers } from "./refusalNumbers";

export const hubAdmissionRefusals = {
  agent_add_unsupported: () => "Este hub no puede añadir asistentes",
  agent_name_taken: () => "Ya tienes otro asistente que se llama así",
  agent_protocol: () => "Codex envió un mensaje JSON inválido",
  agent_stopped: () => "Se cerró la conexión mientras arrancaba el agente",
  ai_capability_missing: () => "El modelo no tiene la capacidad que necesita esta función",
  ai_function_invalid: () => "La función de IA no existe",
  ai_key_missing: () => "Introduce la clave de OpenRouter",
  ai_models_invalid: () => "Corrige sólo los modelos que devolvió el proveedor",
  ai_provider_invalid: () => "Identificador de proveedor no válido",
  ai_provider_missing: () => "El proveedor de esta transcripción ya no existe",
  ai_providers_full: (n) =>
    n.count === undefined || n.max_count === undefined
      ? undefined
      : `Hay ${formatQuantity(n.count)} proveedores; el máximo es ${formatQuantity(n.max_count)}`,
  ai_registry_full: (n) =>
    n.bytes === undefined || n.max_bytes === undefined
      ? undefined
      : `La configuración ocupa ${formatQuantity(n.bytes)} bytes; el máximo es ${formatQuantity(n.max_bytes)}`,
  ai_settings_busy: () => "Ya hay una comprobación de IA en curso; espera a que termine",
  ai_settings_invalid: () => "Revisa el nombre, la URL, los modelos y la clave del proveedor",
  assistant_failed: () => "El asistente falló antes de terminar la respuesta; no se repetirá",
  assistant_tools_unavailable: (n) =>
    n.timeout_ms === undefined
      ? undefined
      : `orbit-mcp no arrancó en ${formatQuantity(n.timeout_ms)} ms; el asistente no tiene las herramientas de Orbit`,
  assistant_turn_too_long: (n) =>
    n.max_requests === undefined
      ? undefined
      : `El asistente hizo ${formatQuantity(n.max_requests)} peticiones al modelo en este turno sin terminar; se detuvo`,
  attachment_not_found: () => "Ese adjunto no está en el hub",
  attachment_refused: (n) =>
    n.upstream_code === undefined
      ? undefined
      : `Hermes no aceptó una imagen del mensaje (código ${formatQuantity(n.upstream_code)})`,
  attachment_type_unsupported: (n) =>
    n.mime === undefined || n.name === undefined
      ? undefined
      : `«${formatQuantity(n.name)}» es ${formatQuantity(n.mime)}: ningún agente lo lee. Se pueden adjuntar imágenes PNG, JPEG, GIF o WebP, PDF y ficheros de texto`,
  attachment_unreadable: () =>
    "No se pudieron preparar los adjuntos del mensaje para el agente; no se envió",
  attachments_too_many: (n) =>
    n.files === undefined || n.max_files === undefined
      ? undefined
      : `Este mensaje lleva ${formatQuantity(n.files)} adjuntos; el techo es ${formatQuantity(n.max_files)} por mensaje`,
  capture_duplicate: () => "La captura ya está guardada",
  capture_missing: () => "La captura ya no está en el hub (caduca a los cinco minutos o ya se usó)",
  chunk_conflict: () => "Metadatos de captura distintos para el mismo fragmento",
  clarification_question_invalid: () => "Esa pregunta no está pendiente",
  configuration: () => "Credencial Hermes ausente o no válida",
  diarization_invalid_response: () => "Respuesta del separador no válida",
  diarization_not_configured: () => "Falta la configuración local de separación",
  diarization_output_too_large: () => "Respuesta del separador excesiva",
  diarization_process_failed: () => "Falló el proceso de separación",
  diarization_start_failed: () => "No se pudo iniciar el separador",
  dictionaries_full: (n) =>
    n.dictionaries === undefined || n.max_dictionaries === undefined
      ? undefined
      : `Hay ${formatQuantity(n.dictionaries)} diccionarios; el máximo es ${formatQuantity(n.max_dictionaries)}`,
  dictionaries_too_large: (n) =>
    n.bytes === undefined || n.max_bytes === undefined
      ? undefined
      : `La lista ocupa ${formatQuantity(n.bytes)} bytes; el máximo es ${formatQuantity(n.max_bytes)}`,
  dictionary_forms_full: (n) =>
    n.forms === undefined || n.max_forms === undefined
      ? undefined
      : `Hay ${formatQuantity(n.forms)} formas equivocadas; el máximo es ${formatQuantity(n.max_forms)}`,
  dictionary_full: (n) =>
    n.max_terms === undefined || n.terms === undefined
      ? undefined
      : `Hay ${formatQuantity(n.terms)} términos; el máximo es ${formatQuantity(n.max_terms)}`,
  dictionary_id_ambiguous: () => "Hay dos diccionarios con el mismo identificador",
  dictionary_not_found: () => "El diccionario no existe",
  dictionary_series_full: (n) =>
    n.max_series === undefined || n.series === undefined
      ? undefined
      : `Hay ${formatQuantity(n.series)} series; el máximo es ${formatQuantity(n.max_series)}`,
  dictionary_series_missing: () => "La reunión no tiene serie de calendario",
  dictionary_text_invalid: (n) =>
    n.field === undefined
      ? undefined
      : `El campo ${formatQuantity(n.field)} está vacío o contiene controles`,
  dictionary_text_too_long: (n) =>
    n.characters === undefined || n.field === undefined || n.max_characters === undefined
      ? undefined
      : `El campo ${formatQuantity(n.field)} tiene ${formatQuantity(n.characters)} caracteres; el máximo es ${formatQuantity(n.max_characters)}`,
  dictionary_too_large: (n) =>
    n.bytes === undefined || n.max_bytes === undefined
      ? undefined
      : `El diccionario ocupa ${formatQuantity(n.bytes)} bytes; el máximo es ${formatQuantity(n.max_bytes)}`,
  final_audio_conversion_failed: () => "No se pudo convertir el audio",
  final_audio_empty: () => "La grabación no tiene audio legible",
  final_audio_time_out_of_range: () => "El inicio del fragmento queda fuera de la grabación",
  final_audio_timeout: () => "La conversión de audio agotó su plazo",
  final_window_too_large: () => "La ventana Opus supera el límite de Whisper",
  group_not_found: () => "Grupo no disponible",
  handover_no_question: () =>
    "La conversación no tiene ninguna pregunta sin respuesta que pasar; escribe la pregunta",
  hash_mismatch: () => "El hash no coincide con los bytes recibidos",
  interaction_not_found: () => "Este asistente no pide autorizaciones",
  invalid_attachment: (n) =>
    n.max_characters === undefined
      ? undefined
      : `El nombre del adjunto no vale: 1 a ${formatQuantity(n.max_characters)} caracteres, sin control ni «/»`,
  invalid_audio: () => "Se requiere WAV PCM de 16 bits válido",
  invalid_capture: () => "El resumen de la captura no coincide",
  invalid_chunk: () => "Metadatos de reloj/formato incompletos o distintos del WAV",
  invalid_dictionary_id: () => "Identificador de diccionario no válido",
  invalid_group_audio: () => "Número de fuentes de audio fuera de contrato",
  invalid_manifest: () => "Manifiesto de cierre no válido",
  invalid_note_request: () => "Identificador de grabación no válido",
  invalid_note_template: (n) =>
    n.max === undefined
      ? undefined
      : `Identificador de plantilla no válido: minúsculas, cifras, «-» o «_», hasta ${formatQuantity(n.max)} caracteres`,
  invalid_pairing: () => "Código inválido, utilizado o caducado",
  invalid_parameters: (n) =>
    n.names !== undefined
      ? `Un turno admite un solo nombre (names: ${formatQuantity(n.names)}, max: ${formatQuantity(n.max)})`
      : "La clave del recordatorio no es válida: es el SHA-256 en hexadecimal del título",
  invalid_payload: () => "Carga vacía",
  invalid_recording: () => "ID de grabación no válido",
  invalid_request: () => "Solicitud de emparejado no válida",
  invalid_transcription_setting: () => "Elige Whisper o MAI para transcribir",
  invalid_working_folder: () => "Este asistente no usa una carpeta de trabajo del hub",
  manifest_conflict: () => "La grabación ya tiene un manifiesto de cierre distinto",
  manifest_mismatch: () => "Las pistas del manifiesto no coinciden con los fragmentos durables",
  meeting_audio_unreadable: (n) =>
    n.groups === undefined || n.total === undefined
      ? undefined
      : `El hub no puede leer el audio de ${formatQuantity(n.groups)} de ${formatQuantity(n.total)} grupos`,
  meeting_not_found: () => "La reunión todavía no ha llegado al hub",
  meeting_series_missing: () =>
    "La reunión no tiene serie de calendario: no se guardó ni la plantilla ni el diccionario",
  method_forbidden: () => "Método Hermes fuera de contrato",
} satisfies Record<string, (n: Numbers) => string | undefined>;

export type HubAdmissionCode = keyof typeof hubAdmissionRefusals;
