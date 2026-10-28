import { decodeBriefingCause, hasNumbers } from "../briefing-cause";
import { formatQuantity, counted } from "../locale";
import { lookupCopy } from "../lookup";
import { refusalEntries } from "./refusalEntries";

const states = {
  local_zone_unavailable: "No se pudo leer la zona horaria del Mac.",
  agenda_page_pending: "La página de la agenda aún se está leyendo.",
  agenda_window_missing: "La agenda no indica qué intervalo se ha leído.",
  notes_list_pending: "La lista de reuniones aún se está leyendo.",
  notes_list_failed: "No se pudo leer la lista de reuniones.",
  topic_not_observed: "Este estado aún no se ha leído.",
  topic_stale: "Este estado ha quedado desactualizado.",
  helper_starting: "El ayudante del Mac está arrancando.",
  daemon_starting: "El servicio de Orbit está arrancando.",
  series_not_read: "La serie de la reunión aún se está leyendo.",
  read_series_empty: "La serie de la reunión llegó vacía.",
  read_series_invalid: "La serie de la reunión no tiene un formato válido.",
  read_list_invalid: "Una nota de la lista no tiene un formato válido.",
  read_calendars_invalid: "Un calendario de la lista no tiene un formato válido.",
  read_calendars_cursor: "El cursor del calendario se repitió o pertenece a otra lectura.",
  read_mismatch: "La respuesta pertenece a otra lectura.",
  read_missing: "El servicio de Orbit no entregó la lectura.",
  read_reminder_title: "La reunión no tiene título con el que guardar el recordatorio.",
  read_reminder_empty: "El recordatorio llegó vacío.",
  read_reminder_invalid: "El recordatorio no tiene un formato válido.",
};

/** Wording is selected when displayed, including after a live language change. */
export function briefingCause(value: string): string {
  const { code, facts, refusal } = decodeBriefingCause(value);
  if (code === "topic_stale" && hasNumbers(facts, "since_ms"))
    return `Este estado ha quedado desactualizado desde ${formatQuantity(facts.since_ms)} ms Unix.`;
  if (code === "reading_since" && hasNumbers(facts, "since_ms"))
    return `La lectura está en curso desde ${formatQuantity(facts.since_ms)} ms Unix.`;
  if (
    code === "transcription" &&
    hasNumbers(facts, "completed", "pending", "failed") &&
    typeof facts.queued === "boolean"
  )
    return `Transcripción: ${counted(facts.completed, "completada", "completadas")}, ${counted(facts.pending, "pendiente", "pendientes")}, ${counted(facts.failed, "fallida", "fallidas")}; ${facts.queued === true ? "hay otra en cola" : "no hay ninguna en cola"}.`;
  if (
    code === "agenda_window_excludes_today" &&
    hasNumbers(facts, "window_start_ms", "window_end_ms", "today_start_ms", "today_end_ms")
  )
    return `El intervalo leído (${formatQuantity(facts.window_start_ms)}–${formatQuantity(facts.window_end_ms)} ms Unix) no cubre hoy (${formatQuantity(facts.today_start_ms)}–${formatQuantity(facts.today_end_ms)} ms Unix).`;
  if (code === "read_calendars_count" && hasNumbers(facts, "count", "total"))
    return `Se leyeron ${formatQuantity(facts.count)} de ${formatQuantity(facts.total)} calendarios anunciados.`;
  if (code === "read_calendars_invalid" && hasNumbers(facts, "index"))
    return `El calendario de la posición ${formatQuantity(facts.index)} no tiene un formato válido.`;
  const state = lookupCopy(states, code);
  if (state) return state;
  const entry = lookupCopy(refusalEntries, code);
  if (entry?.text && refusal) return entry.text(refusal) ?? "No se pudo leer este estado de Orbit.";
  return "No se pudo leer este estado de Orbit.";
}
