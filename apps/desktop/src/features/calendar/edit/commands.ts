/**
 * The three writes and their inverses. Pure: what to send, built
 * from what was read. Nothing here sends or retries: a write whose outcome is
 * unknown is never built again.
 */
import type { Command } from "../../../bridge/generated/Command";
import type { CalendarEventChanges } from "../../../bridge/generated/CalendarEventChanges";
import type { CalendarEventDraft } from "../../../bridge/generated/CalendarEventDraft";
import type { CalendarSpan } from "../../../bridge/generated/CalendarSpan";
import type { WriteTarget } from "../../../bridge/generated/WriteTarget";
import type { CalendarEvent } from "../eventLook";
import { changesOf, formOf, inputOf } from "./form";

/** What names one occurrence for a change or a delete. */
export interface Target {
  calendar_item_id: string;
  occurrence_start_ms: number;
}

export function targetOf(event: CalendarEvent): Target | null {
  const item = event.calendar_item_id;
  const occurrence = event.occurrence_start_ms;
  if (item === undefined || item === "" || occurrence === undefined) return null;
  return { calendar_item_id: item, occurrence_start_ms: occurrence };
}

export function sameTarget(event: CalendarEvent, target: Target): boolean {
  return (
    event.calendar_item_id === target.calendar_item_id &&
    event.occurrence_start_ms === target.occurrence_start_ms
  );
}

export function isSeries(event: CalendarEvent): boolean {
  return event.recurrence !== undefined;
}

/** A fresh identifier per write: the daemon refuses one used twice. */
export function newRequestId(): string {
  const random = globalThis.crypto?.randomUUID?.();
  return random ?? `write-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

/** Only the two fields that name the occurrence travel. */
function named(target: Target): Target {
  return {
    calendar_item_id: target.calendar_item_id,
    occurrence_start_ms: target.occurrence_start_ms,
  };
}

export function createCommand(requestId: string, event: CalendarEventDraft): Command {
  return {
    command: "calendar",
    request: { action: "save", request_id: requestId, save: { create: event } },
  };
}

export function updateCommand(
  requestId: string,
  target: Target,
  span: CalendarSpan,
  changes: CalendarEventChanges,
): Command {
  return {
    command: "calendar",
    request: {
      action: "save",
      request_id: requestId,
      save: { change: { span, changes, ...named(target) } },
    },
  };
}

export function deleteCommand(requestId: string, target: Target, span: CalendarSpan): Command {
  return {
    command: "calendar",
    request: { action: "delete", request_id: requestId, delete: { span, ...named(target) } },
  };
}

/**
 * The event again, from what was read: title, times, place, notes, link,
 * repetition and calendar. One occurrence deleted alone comes back as one
 * event. Guests cannot be written and alarms are not read, so neither comes
 * back (the delete's notice says so when it had guests).
 */
export function recreateInput(
  event: CalendarEvent,
  span: CalendarSpan,
  timezone: string,
): CalendarEventDraft {
  const form = formOf(event);
  const single = span === "this" && isSeries(event);
  const input = inputOf({ ...form, repeat: single ? { kind: "none" } : form.repeat }, timezone);
  // The link (a video call's, often) comes back too.
  return event.url != null && event.url !== "" ? { ...input, url: event.url } : input;
}

/** Where an updated occurrence is now when the daemon did not say: the same
 *  event; a single event's occurrence is its new start, a series keeps the
 *  original date of the occurrence. */
export function updatedTarget(
  event: CalendarEvent,
  changes: CalendarEventChanges,
  saved: WriteTarget | undefined,
): Target | null {
  if (saved !== undefined) return named(saved);
  const target = targetOf(event);
  if (target === null) return null;
  if (isSeries(event)) return target;
  return { ...target, occurrence_start_ms: changes.start_ms ?? target.occurrence_start_ms };
}

/** One change of the session that ⌘Z can take back, and what it names. */
export type UndoEntry =
  | { kind: "create"; target: Target; title: string }
  | {
      kind: "update";
      target: Target;
      span: CalendarSpan;
      inverse: CalendarEventChanges;
      title: string;
    }
  | { kind: "delete"; input: CalendarEventDraft; title: string }
  /** A change with nothing to put back (reminders the Mac did not report):
   *  ⌘Z says so instead of skipping to an older change. */
  | { kind: "impossible"; title: string };

/** What puts the fields a change touched back as they were read. Reminders
 *  the Mac did not report have no inverse and are left out. */
export function inverseChanges(
  event: CalendarEvent,
  changes: CalendarEventChanges,
): CalendarEventChanges {
  const before = formOf(event);
  const after = { ...before };
  if (changes.title !== undefined) after.title = changes.title;
  if (changes.start_ms !== undefined) after.startMs = changes.start_ms;
  if (changes.end_ms !== undefined) after.endMs = changes.end_ms;
  if (changes.all_day !== undefined) after.allDay = changes.all_day;
  if (changes.location !== undefined) after.location = changes.location;
  if (changes.notes !== undefined) after.notes = changes.notes;
  if (changes.calendar_id !== undefined) after.calendarId = changes.calendar_id;
  if (changes.availability !== undefined) after.availability = changes.availability;
  if (changes.alarms_minutes !== undefined) after.alarms = changes.alarms_minutes;
  if (changes.remove_recurrence === true) after.repeat = { kind: "none" };
  if (changes.recurrence !== undefined) after.repeat = { kind: "kept", rule: changes.recurrence };
  return changesOf(after, before);
}

export function undoCommand(
  entry: Exclude<UndoEntry, { kind: "impossible" }>,
  requestId: string,
): Command {
  switch (entry.kind) {
    case "create":
      return deleteCommand(requestId, entry.target, "this");
    case "update":
      return updateCommand(requestId, entry.target, entry.span, entry.inverse);
    case "delete":
      return createCommand(requestId, entry.input);
  }
}
