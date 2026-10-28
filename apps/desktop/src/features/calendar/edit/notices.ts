import { getCopy } from "../../../i18n";
/**
 * What the calendar says about a write, and what ⌘Z keeps of it.
 * Pure. A refusal and an unknown outcome say what to do, never their cause;
 * a gesture Orbit already knows it cannot make says why before
 * anything is sent.
 */
import type { AppView as AppSnapshot } from "../../../state/app-view";
import type { WriteOutcome } from "../../../bridge/generated/WriteOutcome";

import type { CalendarEvent } from "../eventLook";
import { isSeries, recreateInput, type UndoEntry, inverseChanges, updatedTarget } from "./commands";
import type { PendingWrite } from "./pending";

export interface Notice {
  /** `refused`: not written. `unknown`: may or may not be written, never
   *  sent again. `blocked`: Orbit did not try. `info`: an undo on its way. */
  kind: "refused" | "unknown" | "blocked" | "info";
  text: string;
}

export function failureNotice(record: WriteOutcome | null): Notice {
  if (record === null || record.stage === "outcome_unknown") {
    return { kind: "unknown", text: getCopy().calendar.write.unknown };
  }
  return { kind: "refused", text: getCopy().calendar.write.refused };
}

type Calendars = AppSnapshot["calendar"]["calendars"];
export type BlockedAction = "move" | "edit" | "delete";

/** Why Orbit cannot change `event`, or `null` when it may try. Absent
 *  (`can_edit` unknown) is not «no»: the helper decides and says why. */
export function lockedReason(
  event: CalendarEvent,
  calendars: Calendars,
): "read_only" | "organizer" | "unnamed" | null {
  if (event.ui?.draft === true) return null;
  if (event.calendar_item_id == null || event.occurrence_start_ms == null) return "unnamed";
  if (event.can_edit !== false) return null;
  const calendar = calendars.find((item) => item.id === event.calendar_id);
  return calendar?.can_edit === false ? "read_only" : "organizer";
}

export function blockedNotice(
  event: CalendarEvent,
  action: BlockedAction,
  calendars: Calendars = [],
): Notice {
  const reason = lockedReason(event, calendars) ?? "unnamed";
  return { kind: "blocked", text: getCopy().calendar.write.blocked[reason][action] };
}

/** «Evento borrado», and what undoing it will not bring back. */
export function deletedToast(event: CalendarEvent, span: "this" | "future"): string {
  const guests = event.attendees?.length ?? 0;
  const lost = [
    guests > 0 ? getCopy().calendar.write.withoutGuests(guests) : null,
    (event.alarms_not_editable ?? 0) > 0
      ? getCopy().calendar.write.withoutReminders(event.alarms_not_editable ?? 0)
      : null,
    span === "this" && isSeries(event) ? getCopy().calendar.write.outsideSeries : null,
  ].filter((part): part is string => part !== null);
  if (lost.length === 0) return getCopy().calendar.write.deleted;
  const joined =
    lost.length === 1
      ? lost.join("")
      : `${lost.slice(0, -1).join(", ")} ${getCopy().calendar.write.and} ${lost.at(-1) ?? ""}`;
  return `${getCopy().calendar.write.deleted}. ${getCopy().calendar.write.undoLoses(joined)}`;
}

/** What ⌘Z does about a write that finished, or `null` when it cannot name
 *  what the write left (a create whose occurrence the daemon did not say). */
export function undoEntryOf(write: PendingWrite, timezone: string): UndoEntry | null {
  const title = write.event.title;
  if (write.op === "delete") {
    return { input: recreateInput(write.event, write.span, timezone), kind: "delete", title };
  }
  if (write.op === "create") {
    const saved = write.saved;
    return saved === undefined
      ? null
      : {
          kind: "create",
          target: {
            calendar_item_id: saved.calendar_item_id,
            occurrence_start_ms: saved.occurrence_start_ms,
          },
          title,
        };
  }
  const changes = write.changes ?? {};
  const target = updatedTarget(write.event, changes, write.saved);
  const inverse = inverseChanges(write.event, changes);
  if (target === null || Object.keys(inverse).length === 0) return { kind: "impossible", title };
  return { inverse, kind: "update", span: write.span, target, title };
}
