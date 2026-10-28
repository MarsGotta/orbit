import { formatQuantity } from "../locale";
import type { Numbers } from "./refusalNumbers";

const messages = {
  agent_adapter_missing:
    "El asistente no está disponible en tu hub; publica de nuevo el hub para instalarlo.",
  agent_folder_invalid: "La carpeta del agente no es un directorio.",
  agent_folder_not_removed:
    "No se pudo borrar la carpeta del asistente en el hub; sigue en la lista.",
  agent_mismatch: "Esta conversación pertenece a otro asistente; abre una conversación nueva.",
  agent_not_found: "Este hub no tiene ese agente.",
  agent_not_removable:
    "Este asistente lo da la configuración de tu hub: solo quien lo gestiona puede quitarlo.",
  agent_primary_required:
    "Este asistente es el principal: elige otro como principal para cambiarlo.",
  agent_process_limit: "No hay espacio para otro proceso del asistente.",
  agent_profile_invalid: "Revisa el nombre, la descripción y el color del asistente.",
  agent_start_timeout: "El asistente no terminó de arrancar; no se envió el mensaje.",
  agent_timeout: "El asistente no terminó dentro del plazo.",
  approval_choice_mismatch: "La opción elegida contradice la aprobación; sólo «deny» rechaza.",
  approval_not_allowed: "El agente no ofrece esa opción en esta petición.",
  assistant_admin_invalid: "Hermes contestó algo que el hub no sabe leer.",
  assistant_admin_refused: "Hermes rechazó el cambio.",
  assistant_admin_timeout: "Hermes no contestó dentro del plazo.",
  assistant_admin_too_large: "La respuesta de Hermes supera el límite.",
  assistant_admin_unreachable: "No se pudo conectar con Hermes para el cambio.",
  assistant_connect_timeout: "El proveedor del asistente no aceptó la conexión dentro del plazo.",
  assistant_connection_lost:
    "Se cortó la conexión con el proveedor del asistente antes de terminar la respuesta.",
  assistant_context_exceeded: "El modelo no admite tanto contexto.",
  assistant_http_error: "El proveedor del asistente rechazó la petición.",
  assistant_provider_error: "El proveedor del asistente falló sin decir por qué.",
  assistant_rate_limited: "El proveedor del asistente limitó las peticiones.",
  assistant_timeout: "El proveedor del asistente dejó de contestar.",
  assistant_tool_failed:
    "La herramienta del asistente falló; su resultado es incierto y no se repetirá.",
  assistant_unauthorized: "El proveedor del asistente rechazó la clave.",
  assistant_unreachable: "No se pudo conectar con el proveedor del asistente.",
  attachments_too_large:
    "Los adjuntos superan el límite; adjunta menos o abre una conversación nueva.",
  invalid_turn_transition: "Transición de turno no permitida.",
  note_revision_changed: "La nota cambió desde que empezaste a editarla.",
  quota_exceeded:
    "Cuota del servidor alcanzada; sube storage_quota_bytes o libera espacio. El Mac conserva su copia.",
  session_not_found:
    "La sesión no está conectada con el asistente; se intentará reanudar la conversación al enviar otro mensaje si el turno anterior está cerrado.",
  session_resume_connection_replaced:
    "La conexión con el asistente cambió durante la carga del historial; no se envió el mensaje.",
  session_resume_history_timeout:
    "El asistente no pudo leer el historial guardado; no se envió el mensaje. Puedes enviar uno nuevo para volver a reanudar.",
  session_resume_in_progress:
    "El asistente sigue cargando el historial de esta conversación; espera.",
  session_resume_progress_unavailable:
    "El asistente no confirmó si terminó de cargar el historial; no se envió el mensaje.",
  session_resume_timeout:
    "El asistente no confirmó la carga del historial; no se envió el mensaje.",
  sign_in_not_found: "No hay ningún inicio de sesión de ese asistente.",
  sign_in_unsupported:
    "Este asistente no inicia sesión desde Orbit: lo configura quien gestiona tu hub.",
  sign_out_failed: "La herramienta del proveedor no cerró la sesión.",
  sign_out_timeout: "La herramienta del proveedor no cerró la sesión dentro del plazo.",
  skill_invalid: "La habilidad no cumple el estándar SKILL.md.",
  speech_http_error: "El servicio de voz devolvió un error HTTP.",
  speech_not_audio: "El servicio de voz no devolvió audio.",
  speech_timeout: "El servicio de voz no contestó dentro del plazo.",
  speech_unreachable: "No se pudo conectar con el servicio de voz.",
  turn_outcome_unknown:
    "No se pudo comprobar el turno; puedes interrumpir o abrir una sesión nueva.",
  upstream_rejected: "Hermes rechazó la operación.",
  agent_protocol: "El agente envió una respuesta fuera de contrato.",
} satisfies Record<string, string>;

export function dynamicHubRefusal(code: string, numbers: Numbers): string | undefined {
  if (!Object.hasOwn(messages, code)) return undefined;
  const facts = Object.entries(numbers)
    .filter(([key]) => !["cause", "detail", "error", "message", "reason"].includes(key))
    .map(([key, value]) => `${key}: ${formatQuantity(value)}`)
    .join(", ");
  return `${messages[code as HubDynamicCode]}${facts ? ` (${facts})` : ""}`;
}

export type HubDynamicCode = keyof typeof messages;
