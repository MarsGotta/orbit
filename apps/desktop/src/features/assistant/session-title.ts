/**
 * A conversation is named by when it began, as the list shows it:
 * «17 de octubre de 2026, 09:53», on the Mac's clock. The hub only sends
 * `created_at_ms`; the words are the interface's.
 */
export function sessionTitle(createdAtMs: number, timeZone?: string): string {
  const parts = new Intl.DateTimeFormat("es-ES", {
    day: "numeric",
    month: "long",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
    timeZone,
  }).formatToParts(createdAtMs);
  const part = (type: Intl.DateTimeFormatPartTypes) =>
    parts.find((piece) => piece.type === type)?.value ?? "";
  return `${part("day")} de ${part("month")} de ${part("year")}, ${part("hour")}:${part("minute")}`;
}
