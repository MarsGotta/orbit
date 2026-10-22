const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() &&
  a.getMonth() === b.getMonth() &&
  a.getDate() === b.getDate();

/**
 * When a conversation began, as the redesign's list says it, on the Mac's
 * clock: «Hoy, 09:31», «Ayer, 16:00», «18 oct, 18:02», and the year only
 * when it is not this one: «15 oct 2025, 03:46».
 */
export function sessionWhen(atMs: number, nowMs: number): string {
  const at = new Date(atMs);
  const now = new Date(nowMs);
  const time = new Intl.DateTimeFormat("es-ES", {
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).format(at);
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1);
  if (sameDay(at, now)) return `Hoy, ${time}`;
  if (sameDay(at, yesterday)) return `Ayer, ${time}`;
  const day = new Intl.DateTimeFormat("es-ES", {
    day: "numeric",
    month: "short",
    ...(at.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }),
  }).format(at);
  return `${day}, ${time}`;
}
