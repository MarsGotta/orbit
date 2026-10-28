import type { Refusal } from "../../bridge/refusal";
import { lookupCopy } from "../lookup";

const appOperations = {
  link_invalid: "No se abre el enlace: no es una dirección web válida.",
  link_scheme: "No se abre el enlace: sólo se abren enlaces http y https.",
  link_credentials:
    "No se abre el enlace: lleva usuario o contraseña antes del sitio, que puede ocultar adónde lleva.",
  link_no_host: "No se abre el enlace: no dice a qué sitio lleva.",
  note_unavailable: "La nota aún no está disponible. Actualízala e inténtalo de nuevo.",
  speaker_undo_unavailable: "No hay un cambio de nombre de hablante que deshacer.",
  reminder_without_title: "La reunión no tiene título con el que guardar el recordatorio.",
  desktop_command: "Esta acción necesita la aplicación de Orbit.",
  client_command: "La aplicación no admite esta orden.",
  internal_command: "La interfaz no puede enviar esa orden interna.",
};

export type AppOperationCode = keyof typeof appOperations;

export const errors = {
  errors: {
    title: "No se pudo completar",
  },
} as const;

/** App-core and desktop errors carry codes; diagnostic text stays out of copy. */
export function appOperationText(refusal: Refusal): string | undefined {
  if (typeof refusal.cause !== "object" || refusal.cause === null) return undefined;
  const cause = refusal.cause as Record<string, unknown>;
  if (cause.kind !== "app_operation" || typeof cause.code !== "string") return undefined;
  if (cause.code === "meeting_event_not_found") {
    const facts: unknown = typeof cause.error === "string" ? safeFacts(cause.error) : undefined;
    const count =
      typeof facts === "object" && facts !== null && "count" in facts ? facts.count : undefined;
    return typeof count === "number"
      ? `No se encuentra el evento entre los ${count} eventos leídos; actualiza el calendario.`
      : "No se encuentra el evento; actualiza el calendario.";
  }
  return lookupCopy(appOperations, cause.code);
}

function safeFacts(value: string): unknown {
  try {
    return JSON.parse(value) as unknown;
  } catch {
    return undefined;
  }
}
