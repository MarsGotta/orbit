import type { CalendarRecurrence } from "../../bridge/generated/CalendarRecurrence";
import { es } from "../../i18n/es";
import { dayOf, shortDay, timeLabel } from "./date";
import type { Repeat } from "./edit/form";
import { coveredDays } from "./edit/geometry";

/** «Mié 21 oct · 11:00 – 11:45», or «Sáb 24 oct – Dom 25 oct · todo el día». */
export function whenText(
  span: { allDay: boolean; endMs: number; startMs: number },
  timezone: string,
): string {
  if (span.allDay) {
    const { first, last } = coveredDays(span, timezone);
    const days = first === last ? shortDay(first) : `${shortDay(first)} – ${shortDay(last)}`;
    return `${days} · ${es.calendar.allDayLower}`;
  }
  const start = timeLabel(span.startMs, timezone);
  const end = timeLabel(span.endMs, timezone);
  const first = dayOf(span.startMs, timezone);
  const last = dayOf(span.endMs - 1, timezone);
  if (first === last) return `${shortDay(first)} · ${es.calendar.timeRange(start, end)}`;
  return es.calendar.timeRange(`${shortDay(first)} · ${start}`, `${shortDay(last)} · ${end}`);
}

/** «Se repite: cada semana, los miércoles». */
export function repeatText(
  rule: Pick<CalendarRecurrence, "frequency" | "interval" | "days_of_week">,
) {
  const every =
    rule.interval === 1
      ? es.calendar.edit.repeatOptions[rule.frequency].toLowerCase()
      : `${es.calendar.edit.every.toLowerCase()} ${rule.interval} ${es.calendar.edit.units[rule.frequency](rule.interval)}`;
  const days = (rule.days_of_week ?? [])
    .filter((day) => day.week === undefined)
    .map((day) => es.calendar.everyWeekday[day.day]);
  const listed =
    days.length <= 1
      ? days.join("")
      : `${days.slice(0, -1).join(", ")} ${es.calendar.and} ${days.at(-1) ?? ""}`;
  return es.calendar.repeats(listed === "" ? every : `${every}, ${listed}`);
}

export function repeatOfForm(repeat: Repeat): string | null {
  if (repeat.kind === "none") return null;
  if (repeat.kind === "kept") return repeatText(repeat.rule);
  return repeatText({
    days_of_week: repeat.rule?.days_of_week,
    frequency: repeat.frequency,
    interval: repeat.interval,
  });
}
