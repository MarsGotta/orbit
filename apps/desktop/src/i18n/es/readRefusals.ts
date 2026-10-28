import { formatQuantity } from "../locale";
import type { Refusal } from "../../bridge/refusal";
import { fields } from "./refusalDetails";

export function interrupted(refusal: Refusal): string {
  const cause = fields(refusal.cause);
  let reason: string;
  if (cause.kind === "generation_changed") {
    reason = `La lectura empezó en la generación ${formatQuantity(cause.started_generation)} y el Mac está en la ${formatQuantity(cause.current_generation)}.`;
  } else if (cause.kind === "inventory_dispatch") {
    reason = `Solo se enviaron ${formatQuantity(cause.sent_reads)} de ${formatQuantity(cause.total_reads)} peticiones del inventario; no hay una lista completa.`;
  } else if (cause.kind === "calendar_changed") {
    reason = `El calendario cambió ${formatQuantity(cause.changes)} veces; se admiten ${formatQuantity(cause.max_restarts)} reinicios por lectura.`;
  } else if (cause.kind === "calendar_expired") {
    reason = `La lectura de agenda agotó su plazo de ${formatQuantity(cause.deadline_ms)} ms.`;
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

export function calendarLimit(refusal: Refusal): string {
  const cause = fields(refusal.cause);
  if (cause.kind === "page")
    return `Una página de la agenda supera el máximo de ${formatQuantity(cause.max_bytes)} bytes. Pide un rango más corto.`;
  if (cause.kind === "fields")
    return `La lectura del calendario supera sus límites: título de hasta ${formatQuantity(cause.max_title_bytes)} bytes, etiqueta de hasta ${formatQuantity(cause.max_label_bytes)} bytes y zona horaria de hasta ${formatQuantity(cause.max_timezone_bytes)} bytes. Pide un rango más corto.`;
  return "La lectura del calendario supera un límite que esta versión no reconoce.";
}

export function daemonReply(refusal: Refusal): string {
  const issue = fields(refusal.issue);
  const reason =
    issue.kind === "count_mismatch"
      ? `se recibieron ${formatQuantity(issue.delivered)} filas de ${formatQuantity(issue.total)} declaradas`
      : issue.kind === "cursor_repeated"
        ? "el cursor se repitió"
        : issue.kind === "missing"
          ? "falta la respuesta"
          : "no se pudo interpretar la respuesta";
  return `La respuesta del servicio de Orbit no cumple el contrato: ${String(refusal.field)}; ${reason}.`;
}
