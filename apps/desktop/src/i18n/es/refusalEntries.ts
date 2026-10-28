import { appOperationText } from "./errors";
import { refusalResource } from "./refusalResources";
import { formatQuantity } from "../locale";
import { interrupted, calendarLimit, daemonReply } from "./readRefusals";
import { numberFormat } from "../locale";
import { busyEntry } from "./busy";
import type { RecoveryStep } from "../../bridge/generated/RecoveryStep";
import { helperRefused, helperReplyInvalid } from "./refusalDetails";
import { attachmentRefusalText } from "./attachments";
import { macPermission } from "./permissions";
import type { Refusal, RefusalCode } from "../../bridge/refusal";
import { openSettings, rearm, systemPrivacy, NoticeAction } from "./notices/types";

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
  internal: { class: "failed", text: appOperationText },
  protocol_version: {
    class: "fixable",
    text: (r) =>
      `La versión de protocolo recibida es ${formatQuantity(r.received)}; se requiere ${formatQuantity(r.required)}.`,
  },
  hub_reply_invalid: {
    class: "failed",
    text: (r) =>
      `La respuesta del hub no cumple el contrato: campo ${String(r.field)}${typeof r.received_bytes === "number" ? `, ${numberFormat().format(r.received_bytes)} bytes recibidos` : ""}.`,
  },
  daemon_reply_invalid: { class: "failed", text: daemonReply },
  calendar_read_limit: { class: "fixable", text: calendarLimit },

  busy: busyEntry,
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
        ? "El audio de esa nota de voz ya no está en este Mac: no se puede reintentar la transcripción. Grábala otra vez."
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
      return `Orbit está recuperando el almacén: paso ${step ?? "desconocido"}, ${formatQuantity(refusal.elapsed_ms)} ms transcurridos. Espera a que termine.`;
    },
  },
  revision_changed: failed,
  storage_failed: failed,
  timeout: failed,
  too_large: {
    class: "failed",
    text: (r) =>
      `El recurso ${refusalResource(r.resource)} ocupa ${formatQuantity(r.bytes)} bytes; el máximo es ${formatQuantity(r.max_bytes)}.`,
  },
  too_many: {
    class: "failed",
    text: (r) =>
      `Hay ${formatQuantity(r.count)} elementos de ${refusalResource(r.resource)}; el máximo es ${formatQuantity(r.max)}.`,
  },
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
