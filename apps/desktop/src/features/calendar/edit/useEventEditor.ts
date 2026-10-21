import { useEffect, useRef, useState } from "react";
import type { AppView as AppSnapshot } from "../../../state/app-view";
import type { CalendarSpan } from "../../../bridge/generated/CalendarSpan";
import { es } from "../../../i18n/es";
import type { CalendarEvent } from "../eventLook";
import { isSeries } from "./commands";
import { changesOf, changesRepeat, type EventForm, formOf, hasChanges } from "./form";
import type { Span } from "./geometry";
import { lockedReason } from "./notices";
import type { useCalendarWrites } from "./useCalendarWrites";

type Writes = ReturnType<typeof useCalendarWrites>;
type Calendars = AppSnapshot["calendar"]["calendars"];

export interface Editing {
  base: EventForm;
  /** `null` for a draft: it does not exist in the calendar until saved. */
  event: CalendarEvent | null;
  form: EventForm;
}

/** A new draft: the default calendar (or macOS's, unnamed), busy, untitled. */
export function draftForm(span: Span, allDay: boolean, calendars: Calendars): EventForm {
  const fallback = calendars.find((calendar) => calendar.is_default === true);
  return {
    alarms: null,
    allDay,
    availability: "busy",
    calendarId: fallback?.id ?? null,
    endMs: span.endMs,
    location: "",
    notes: "",
    repeat: { kind: "none" },
    startMs: span.startMs,
    title: "",
  };
}

/** How an event, or the draft, is drawn with the panel's values. */
export function drawn(form: EventForm, event: CalendarEvent | null): CalendarEvent {
  return {
    ...(event ?? { id: "draft", note: null }),
    all_day: form.allDay,
    calendar_id: form.calendarId,
    end_ms: form.endMs,
    start_ms: form.startMs,
    title: form.title.trim() === "" && event === null ? es.calendar.edit.newEvent : form.title,
    ...(event === null ? { ui: { draft: true } } : {}),
  };
}

/** The span a save without a chosen scope uses: this occurrence, unless the
 *  repetition changed, which reaches this one and the ones after it. */
function defaultSpan(editing: Editing): CalendarSpan {
  return changesRepeat(changesOf(editing.base, editing.form)) ? "future" : "this";
}

/** Saves what the panel holds: a titled draft is created, an event's
 *  changes are written; an untitled draft and an untouched event write
 *  nothing, and an event the user cannot change is never written. */
function commitEdits(
  now: Editing | null,
  writes: Writes,
  calendars: Calendars,
  span?: CalendarSpan,
): void {
  if (now === null) return;
  if (now.event === null) {
    if (now.form.title.trim() !== "") writes.create(now.form, drawn(now.form, null));
    return;
  }
  if (lockedReason(now.event, calendars) !== null) return;
  const changes = changesOf(now.base, now.form);
  if (hasChanges(changes))
    writes.update(now.event, changes, span ?? defaultSpan(now), drawn(now.form, now.event));
}

/**
 * The event selected in the calendar and its panel's edits. A draft
 * is saved when it has a title and the user saves it or leaves it; untitled, it
 * disappears. Edits to an event accumulate and are saved the same way.
 */
export function useEventEditor(
  selected: CalendarEvent | null,
  setSelected: (event: CalendarEvent | null) => void,
  calendars: Calendars,
  writes: Writes,
) {
  const [editing, setEditing] = useState<Editing | null>(null);
  const current = useRef(editing);
  current.current = editing;
  useEffect(() => {
    if (selected !== null && current.current?.event?.id !== selected.id) {
      setEditing({ base: formOf(selected), event: selected, form: formOf(selected) });
    }
  }, [selected]);
  const commit = (span?: CalendarSpan) => commitEdits(current.current, writes, calendars, span);
  const close = () => {
    current.current = null;
    setEditing(null);
    setSelected(null);
  };
  const leave = (span?: CalendarSpan) => {
    commit(span);
    close();
  };
  return {
    close: () => leave(),
    /** Leaves without saving: a draft that is let go of on purpose. */
    discard: close,
    editing,
    escape: () => {
      const now = current.current;
      const untouched = now?.event === null && now.form.title === "";
      if (!untouched) commit();
      close();
    },
    open: (event: CalendarEvent) => {
      commit();
      current.current = null;
      setEditing({ base: formOf(event), event, form: formOf(event) });
      setSelected(event);
    },
    remove: (span: CalendarSpan) => {
      const now = current.current;
      if (now?.event) writes.remove(now.event, span);
      close();
    },
    save: leave,
    setForm: (form: EventForm) => {
      // Read at once by a save that follows in the same keystroke (⌘↵
      // right after a field commits its value).
      if (current.current !== null) current.current = { ...current.current, form };
      setEditing((now) => (now === null ? now : { ...now, form }));
    },
    startDraft: (span: Span, allDay: boolean) => {
      commit();
      const form = draftForm(span, allDay, calendars);
      current.current = { base: form, event: null, form };
      setEditing(current.current);
      setSelected(null);
    },
  };
}

/** Which save buttons the panel offers, first the one ⌘↵ presses. */
export function saveChoices(editing: Editing): CalendarSpan[] {
  if (editing.event === null || !isSeries(editing.event)) return [defaultSpan(editing)];
  return changesRepeat(changesOf(editing.base, editing.form)) ? ["future"] : ["this", "future"];
}

export function isDirty(editing: Editing): boolean {
  if (editing.event === null) return editing.form.title.trim() !== "";
  return hasChanges(changesOf(editing.base, editing.form));
}
