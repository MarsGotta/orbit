/**
 * The event panel's fields and what saving them writes. Pure: an
 * event or a draft in, a form out; two forms in, the change between them out.
 * Changes accumulate in the form and nothing is written until the user saves.
 */
import type { CalendarAvailability } from "../../../bridge/generated/CalendarAvailability";
import type { CalendarEventChanges } from "../../../bridge/generated/CalendarEventChanges";
import type { CalendarEventDraft } from "../../../bridge/generated/CalendarEventDraft";
import type { CalendarFrequency } from "../../../bridge/generated/CalendarFrequency";
import type { CalendarRecurrence } from "../../../bridge/generated/CalendarRecurrence";
import type { CalendarEvent } from "../eventLook";
import { allDaySpan, atWall, coveredDays } from "./geometry";

export type Repeat =
  | { kind: "none" }
  | {
      kind: "rule";
      frequency: CalendarFrequency;
      interval: number;
      rule?: CalendarRecurrence;
    }
  /** A rule this panel does not draw (week days, several rules): kept as it is. */
  | { kind: "kept"; rule: CalendarRecurrence };

export interface EventForm {
  /** The reminders Orbit edits, in minutes before the start, as read;
   *  `null` when the Mac did not say (a helper that predates it): an
   *  untouched unknown list is never written. */
  alarms: number[] | null;
  allDay: boolean;
  availability: CalendarAvailability | null;
  calendarId: string | null;
  endMs: number;
  location: string;
  notes: string;
  repeat: Repeat;
  startMs: number;
  title: string;
}

export function repeatOf(rule: CalendarRecurrence | undefined): Repeat {
  if (rule === undefined) return { kind: "none" };
  if ((rule.days_of_week?.length ?? 0) > 0 || (rule.rules_count ?? 1) > 1) {
    return { kind: "kept", rule };
  }
  return { frequency: rule.frequency, interval: rule.interval, kind: "rule", rule };
}

export function formOf(event: CalendarEvent): EventForm {
  return {
    alarms: event.alarms_minutes ?? null,
    allDay: event.all_day === true,
    availability: event.availability ?? null,
    calendarId: event.calendar_id ?? null,
    endMs: event.end_ms,
    location: event.location ?? "",
    notes: event.notes ?? "",
    repeat: repeatOf(event.recurrence),
    startMs: event.start_ms,
    title: event.title,
  };
}

/** The rule a repeat writes: the frequency and interval chosen, keeping
 *  where the original rule ended. */
function ruleOf(
  repeat: Repeat,
): import("../../../bridge/generated/CalendarRecurrenceRule").CalendarRecurrenceRule | null {
  if (repeat.kind === "none") return null;
  const end = repeat.rule?.end;
  if (repeat.kind === "kept") {
    const days = repeat.rule.days_of_week;
    return {
      frequency: repeat.rule.frequency,
      interval: repeat.rule.interval,
      ...(days !== undefined ? { days_of_week: days } : {}),
      ...(end !== undefined ? { end } : {}),
    };
  }
  return {
    frequency: repeat.frequency,
    interval: repeat.interval,
    ...(end !== undefined ? { end } : {}),
  };
}

function sameRepeat(left: Repeat, right: Repeat): boolean {
  return JSON.stringify(ruleOf(left)) === JSON.stringify(ruleOf(right));
}

function sameAlarms(left: number[] | null, right: number[] | null): boolean {
  return JSON.stringify([...(left ?? [])].sort()) === JSON.stringify([...(right ?? [])].sort());
}

/** What saving `after` over `before` writes: only what changed. */
export function changesOf(before: EventForm, after: EventForm): CalendarEventChanges {
  const changes: CalendarEventChanges = {};
  if (after.title !== before.title) changes.title = after.title;
  if (after.startMs !== before.startMs) changes.start_ms = after.startMs;
  if (after.endMs !== before.endMs) changes.end_ms = after.endMs;
  if (after.allDay !== before.allDay) changes.all_day = after.allDay;
  if (after.location !== before.location) changes.location = after.location;
  if (after.notes !== before.notes) changes.notes = after.notes;
  if (after.calendarId !== before.calendarId && after.calendarId !== null)
    changes.calendar_id = after.calendarId;
  if (after.availability !== before.availability && after.availability !== null)
    changes.availability = after.availability;
  if (after.alarms !== null && (before.alarms === null || !sameAlarms(before.alarms, after.alarms)))
    changes.alarms_minutes = [...after.alarms].sort((left, right) => left - right);
  if (!sameRepeat(before.repeat, after.repeat)) {
    const rule = ruleOf(after.repeat);
    if (rule === null) changes.remove_recurrence = true;
    else changes.recurrence = rule;
  }
  return changes;
}

export function hasChanges(changes: CalendarEventChanges): boolean {
  return Object.keys(changes).length > 0;
}

/** A change of the repetition reaches this occurrence and the ones after it,
 *  never this one alone (EventKit; orbit-core's `RecurrenceNeedsFutureSpan`). */
export function changesRepeat(changes: CalendarEventChanges): boolean {
  return changes.recurrence !== undefined || changes.remove_recurrence === true;
}

/** A new event from the form: a timed one in the zone on screen, an all-day
 *  one floating, as Calendar keeps it. */
export function inputOf(form: EventForm, timezone: string): CalendarEventDraft {
  const rule = ruleOf(form.repeat);
  return {
    all_day: form.allDay,
    end_ms: form.endMs,
    start_ms: form.startMs,
    title: form.title.trim(),
    ...(form.allDay ? {} : { time_zone: timezone }),
    ...(form.calendarId !== null ? { calendar_id: form.calendarId } : {}),
    ...(form.location !== "" ? { location: form.location } : {}),
    ...(form.notes !== "" ? { notes: form.notes } : {}),
    ...(form.availability !== null && form.availability !== "not_supported"
      ? { availability: form.availability }
      : {}),
    ...(rule !== null ? { recurrence: rule } : {}),
    ...(form.alarms !== null && form.alarms.length > 0 ? { alarms_minutes: form.alarms } : {}),
  };
}

/** A new start keeps the event's length. */
export function withStart(form: EventForm, startMs: number): EventForm {
  return { ...form, endMs: startMs + (form.endMs - form.startMs), startMs };
}

/** «Todo el día» on: the days it covers. Off: nine o'clock of its first day,
 *  one hour. */
export function withAllDay(form: EventForm, allDay: boolean, timezone: string): EventForm {
  if (allDay === form.allDay) return form;
  const { first, last } = coveredDays(form, timezone);
  if (allDay) return { ...form, allDay, ...allDaySpan(first, last, timezone) };
  const startMs = atWall(first, 9 * 60, timezone);
  return { ...form, allDay, endMs: atWall(first, 10 * 60, timezone), startMs };
}
