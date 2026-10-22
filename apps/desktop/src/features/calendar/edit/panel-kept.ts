import { useEffect, useRef, useState } from "react";
import { useLocalDraft } from "../../../state/local-drafts";
import type { CalendarEvent } from "../eventLook";
import { changesOf, type EventForm, hasChanges } from "./form";
import type { CalendarSpan } from "../../../bridge/generated/CalendarSpan";
import type { WriteSettled } from "./usePendingWrites";
import type { Current, Editing } from "./useEventEditor";

/** What the panel keeps on this Mac until the calendar has it: a draft with
 *  anything written, or an event's unsaved changes. */
type KeptPanel = Pick<Editing, "base" | "event" | "form">;

function keptPanel(editing: Editing | null): KeptPanel | null {
  if (editing === null || editing.saving?.state === "pending") return null;
  const { base, event, form } = editing;
  const written =
    event === null
      ? form.title.trim() !== "" || form.notes.trim() !== "" || form.location.trim() !== ""
      : hasChanges(changesOf(base, form));
  return written ? { base, event, form } : null;
}

function isForm(value: unknown): value is EventForm {
  if (typeof value !== "object" || value === null) return false;
  const form = value as Record<string, unknown>;
  return (
    typeof form.title === "string" &&
    typeof form.notes === "string" &&
    typeof form.location === "string" &&
    typeof form.startMs === "number" &&
    typeof form.endMs === "number" &&
    typeof form.allDay === "boolean" &&
    typeof form.repeat === "object" &&
    form.repeat !== null
  );
}

/** A panel an earlier run left, if this version can read it. */
function acceptPanel(raw: unknown): KeptPanel | null {
  if (typeof raw !== "object" || raw === null) return null;
  const { base, event, form } = raw as Record<string, unknown>;
  if (!isForm(base) || !isForm(form)) return null;
  const known =
    event === null ||
    (typeof event === "object" && typeof (event as { id?: unknown }).id === "string");
  return known ? { base, event: event as CalendarEvent | null, form } : null;
}

/**
 * The panel as it stood, kept on this Mac on every change and brought back
 * when the view opens again: leaving the calendar, a quit or a fall do not
 * take what was written. A panel closed, saved or let go of is let go here too.
 */
export function usePanelKept() {
  const kept = useLocalDraft<KeptPanel | null>("calendar.panel", null, acceptPanel);
  const [editing, setEditing] = useState<Editing | null>(() =>
    kept.value === null ? null : { ...kept.value, ...(kept.recovered ? { recovered: true } : {}) },
  );
  const keep = kept.set;
  useEffect(() => {
    keep(keptPanel(editing));
  }, [editing, keep]);
  // Read at once by a save that follows in the same keystroke.
  const current = useRef(editing);
  current.current = editing;
  const replace = (next: Editing | null) => {
    current.current = next;
    setEditing(next);
  };
  return { current, editing, replace, setEditing };
}

/** A new event on its way to the calendar, given back to the panel if refused. */
interface Created {
  requestId: string;
  form: EventForm;
}

/**
 * Saves what the panel holds, remembering the last new event sent: refused
 * or of unknown outcome, it is the panel's again, notes and place included.
 */
export function useCreated(
  current: Current,
  replace: (next: Editing | null) => void,
  write: (span?: CalendarSpan) => string | null,
) {
  const [created, setCreated] = useState<Created | null>(null);
  const commit = (span?: CalendarSpan) => {
    const now = current.current;
    const requestId = write(span);
    if (requestId !== null && now !== null && now.event === null)
      setCreated({ requestId, form: now.form });
    return requestId;
  };
  const settleCreated = (requestId: string, outcome: WriteSettled) => {
    if (created?.requestId !== requestId) return;
    setCreated(null);
    if (outcome.state !== "failed") return;
    commit();
    replace({ base: created.form, event: null, form: created.form });
  };
  return { commit, created, settleCreated };
}
