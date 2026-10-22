import type { RecoveryStep } from "../../bridge/generated/RecoveryStep";
import { fields, helperRefused, helperReplyInvalid } from "./refusalDetails";
import { attachmentRefusalText } from "./attachments";
import { macPermission } from "./permissions";
import type { Refusal, RefusalCode } from "../../bridge/refusal";
import { openSettings, rearm, systemPrivacy, type NoticeAction } from "./notices/types";

/**
 * A refused order as the person sees it: something they can fix,
 * work still in progress, or a failure with the generic message and its code.
 * The interceptor reads this as it reads the notice catalogue.
 * The words say what to do; the refusal's numbers stay in the log.
 */
export type RefusalEntry =
  | { class: "failed"; text?: (refusal: Refusal) => string | undefined }
  | {
      class: "fixable" | "working";
      text: (refusal: Refusal) => string;
      action?: (refusal: Refusal) => NoticeAction;
    };

const failed = { class: "failed" } as const;

function privacyPane(refusal: Refusal): NoticeAction {
  const permission =
    typeof refusal.permission === "string" ? macPermission(refusal.permission) : undefined;
  return permission && "pane" in permission ? systemPrivacy(permission.pane) : openSettings;
}

const permission = {
  class: "fixable",
  text: (refusal: Refusal) => {
    const name =
      typeof refusal.permission === "string" ? macPermission(refusal.permission)?.name : undefined;
    return name
      ? `Falta el permiso de ${name}. Concédelo en Ajustes del Sistema.`
      : "Falta un permiso de macOS. Concédelo en Ajustes del Sistema.";
  },
  action: privacyPane,
} as const;

const recoverySteps: Record<RecoveryStep, string> = {
  chunks: "fragmentos",
  upload_jobs: "subidas pendientes",
  snapshot: "instantánea",
  environment: "preparación del servicio",
  operations: "operaciones pendientes",
  call_preference: "preferencia al detectar una llamada",
};

function interrupted(refusal: Refusal): string {
  const cause = fields(refusal.cause);
  let reason: string;
  if (cause.kind === "generation_changed") {
    reason = `La lectura empezó en la generación ${String(cause.started_generation)} y el Mac está en la ${String(cause.current_generation)}.`;
  } else if (cause.kind === "inventory_dispatch") {
    reason = `Solo se enviaron ${String(cause.sent_reads)} de ${String(cause.total_reads)} peticiones del inventario; no hay una lista completa.`;
  } else if (cause.kind === "calendar_changed") {
    reason = `El calendario cambió ${String(cause.changes)} veces; se admiten ${String(cause.max_restarts)} reinicios por lectura.`;
  } else if (cause.kind === "calendar_expired") {
    reason = `La lectura de agenda agotó su plazo de ${String(cause.deadline_ms)} ms.`;
  } else if (cause.kind === "helper_restarted") {
    reason = `El helper se reinició; generación anterior: ${String(cause.previous_generation ?? "sin observar")}; actual: ${String(cause.current_generation ?? "sin observar")}.`;
  } else {
    const reasons: Record<string, string> = {
      cancelled: "La lectura se canceló.",
      authority: "Cambió la autorización de la operación.",
      conversation_changed: "Cambió la conversación durante la lectura.",
      calendar_helper_stopped: "Se desconectó el lector de calendarios.",
      calendar_read_restarted: "Empezó otra lectura de agenda.",
      calendar_generation_changed: "Cambió la generación del lector de calendarios.",
      calendar_reply_replaced: "Otra respuesta sustituyó la lectura de agenda.",
      source_reply_replaced: "Otra respuesta sustituyó la lectura de fuentes.",
      capture_stopped: "La captura se detuvo durante la lectura.",
      computer_revoked: "Se retiró el control del ordenador durante la lectura.",
    };
    reason =
      reasons[String(cause.kind)] ??
      "La lectura se interrumpió por una causa que esta versión no reconoce.";
  }
  return `${reason} Vuelve a pedirla si sigue haciendo falta.`;
}

function calendarLimit(refusal: Refusal): string {
  const cause = fields(refusal.cause);
  if (cause.kind === "page")
    return `Una página de la agenda supera el máximo de ${String(cause.max_bytes)} bytes. Pide un rango más corto.`;
  if (cause.kind === "fields")
    return `La lectura del calendario supera sus límites: título de hasta ${String(cause.max_title_bytes)} bytes, etiqueta de hasta ${String(cause.max_label_bytes)} bytes y zona horaria de hasta ${String(cause.max_timezone_bytes)} bytes. Pide un rango más corto.`;
  return "La lectura del calendario supera un límite que esta versión no reconoce.";
}

function daemonReply(refusal: Refusal): string {
  const issue = fields(refusal.issue);
  const reason =
    issue.kind === "count_mismatch"
      ? `se recibieron ${String(issue.delivered)} filas de ${String(issue.total)} declaradas`
      : issue.kind === "cursor_repeated"
        ? "el cursor se repitió"
        : issue.kind === "missing"
          ? "falta la respuesta"
          : "no se pudo interpretar la respuesta";
  return `La respuesta del servicio de Orbit no cumple el contrato: ${String(refusal.field)}; ${reason}.`;
}

export const refusalEntries: Record<RefusalCode, RefusalEntry> = {
  attachment: { class: "fixable", text: attachmentRefusalText },
  encode_failed: {
    class: "failed",
    text: () => "Orbit no pudo codificar la respuesta de esa lectura.",
  },
  helper_reply_invalid: {
    class: "failed",
    text: helperReplyInvalid,
  },
  read_interrupted: { class: "fixable", text: interrupted },
  helper_refused: {
    class: "fixable",
    text: helperRefused,
  },
  internal: failed,
  protocol_version: {
    class: "fixable",
    text: (r) =>
      `La versión de protocolo recibida es ${String(r.received)}; se requiere ${String(r.required)}.`,
  },
  hub_reply_invalid: {
    class: "failed",
    text: (r) =>
      `La respuesta del hub no cumple el contrato: campo ${String(r.field)}${typeof r.received_bytes === "number" ? `, ${r.received_bytes} bytes recibidos` : ""}.`,
  },
  daemon_reply_invalid: { class: "failed", text: daemonReply },
  calendar_read_limit: { class: "fixable", text: calendarLimit },

  busy: failed,
  calendar_backend_unavailable: failed,
  device_reply_invalid: failed,
  device_reply_timeout: failed,
  engine_unavailable: failed,
  expired: failed,
  helper_not_running: {
    class: "fixable",
    text: () => "Orbit.app no está en marcha. Ábrela para continuar.",
  },
  hub_session_invalid_row: failed,
  hub_sessions_cursor_mismatch: failed,
  hub_sessions_unpaged: failed,
  hub_unreachable: failed,
  invalid_parameter: failed,
  legacy: failed,
  mac_disconnected: failed,
  // «Reintentar transcripción» on a note whose audio this Mac no longer has.
  not_found: {
    class: "failed",
    text: (refusal) =>
      refusal.resource === "audio"
        ? "El audio de esa nota de voz ya no está en este Mac: no se puede volver a transcribir. Grábala otra vez."
        : undefined,
  },
  not_paired: {
    class: "fixable",
    text: () => "Este Mac no está emparejado con el hub. Emparéjalo en Ajustes.",
    action: () => openSettings,
  },
  outcome_unknown: failed,
  pending_recording: failed,
  physical_work_finishing: failed,
  permission_denied: permission,
  permission_not_granted: permission,
  permission_unknown: {
    class: "working",
    text: (refusal) => {
      const name =
        typeof refusal.permission === "string"
          ? macPermission(refusal.permission)?.name
          : undefined;
      return `Orbit todavía no sabe si macOS concede ${name ?? "ese permiso"}. Espera a que el Mac lo comunique; no es una negativa.`;
    },
  },
  privacy_stopped: {
    class: "fixable",
    text: () => "Todo está parado. Reactiva Orbit para continuar.",
    action: () => rearm,
  },
  recovering: {
    class: "working",
    text: (refusal) => {
      const step =
        typeof refusal.step === "string" ? recoverySteps[refusal.step as RecoveryStep] : undefined;
      return `Orbit está recuperando el almacén: paso ${step ?? "desconocido"}, ${String(refusal.elapsed_ms)} ms transcurridos. Espera a que termine.`;
    },
  },
  revision_changed: failed,
  storage_failed: failed,
  timeout: failed,
  too_large: failed,
  too_many: failed,
  update: failed,
  upstream_failed: failed,
  work_expired: failed,
  work_in_flight: failed,
  // The assistant's folders: these reach Deimos with their paths; in the app
  // only the missing folder has something to fix.
  no_folders: {
    class: "fixable",
    text: () => "El asistente no tiene carpetas de este Mac. Añádelas en Ajustes.",
    action: () => openSettings,
  },
  outside_folders: failed,
  folder_unavailable: {
    class: "fixable",
    text: (refusal) =>
      `La carpeta ${String(refusal.path)} ya no se puede abrir. Quítala y vuelve a añadirla en Ajustes.`,
    action: () => openSettings,
  },
  file_unavailable: failed,
  file_too_large: failed,
};
