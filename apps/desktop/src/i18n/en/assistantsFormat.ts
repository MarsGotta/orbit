import { dateTime } from "../locale";
// What the assistants' texts share: the times they say and the vendor of each service.
const clock = dateTime({
  hour: "2-digit",
  minute: "2-digit",
});
export const weekday = dateTime({ weekday: "long", day: "numeric" });
const dayMonth = dateTime({ day: "numeric", month: "short" });

/** «18:40». */
export function hourOf(atMs: number): string {
  return clock.format(new Date(atMs));
}

export const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() &&
  a.getMonth() === b.getMonth() &&
  a.getDate() === b.getDate();

/** «hoy a las 18:40», «mañana a las 9:00» or «el lunes 27 a las 9:00», on the Mac's clock. */
export function whenBack(atMs: number, nowMs: number): string {
  const at = new Date(atMs);
  const now = new Date(nowMs);
  const tomorrow = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
  if (sameDay(at, now)) return `today at ${hourOf(atMs)}`;
  if (sameDay(at, tomorrow)) return `tomorrow at ${hourOf(atMs)}`;
  return `on ${weekday.format(at)} at ${hourOf(atMs)}`;
}

/** The vendor whose account an assistant signs in with. */
const VENDORS: Record<string, string> = { "claude-code": "Claude", codex: "ChatGPT" };

export function vendorOf(service: string | undefined): string | undefined {
  return service === undefined ? undefined : VENDORS[service];
}

/** «de la semana», «de estas 5 horas», as the provider names its window. */
export function usageWindow(window: string | undefined, thisOne = false): string {
  if (window === "seven_day") return thisOne ? "this week" : "the week";
  if (window === "five_hour") return "these 5 hours";
  return "your plan";
}

/** «20 oct», for a date that is not today. */
export function shortDay(atMs: number): string {
  return dayMonth.format(new Date(atMs)).replace(".", "");
}
