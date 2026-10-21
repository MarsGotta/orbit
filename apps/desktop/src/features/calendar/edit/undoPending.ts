import type { CalendarEvent } from "../eventLook";
import { sameTarget, type Target, type UndoEntry } from "./commands";
import type { PendingWrite } from "./pending";

/** An event named only by its occurrence, for an undo that has nothing else. */
function placeholder(title: string, target?: Target): CalendarEvent {
  return {
    end_ms: 0,
    id: `undo:${target?.calendar_item_id ?? title}`,
    start_ms: 0,
    title,
    ...(target ?? {}),
  };
}

/**
 * How an undo is drawn while it is on its way, from the events on
 * screen: a recreated event appears where it was, a created one disappears,
 * a moved or edited one goes back to how it was read.
 */
export function undoPending(
  entry: Exclude<UndoEntry, { kind: "impossible" }>,
  events: readonly CalendarEvent[],
): Omit<PendingWrite, "requestId" | "sentAtMs"> {
  const base = { span: entry.kind === "update" ? entry.span : ("this" as const), undoable: false };
  if (entry.kind === "delete") {
    const shown: CalendarEvent = {
      ...placeholder(entry.title),
      all_day: entry.input.all_day,
      calendar_id: entry.input.calendar_id ?? null,
      end_ms: entry.input.end_ms,
      start_ms: entry.input.start_ms,
    };
    return { ...base, event: shown, op: "create", shown };
  }
  const found = events.find((event) => sameTarget(event, entry.target));
  const event = found ?? placeholder(entry.title, entry.target);
  if (entry.kind === "create") return { ...base, event, op: "delete", shown: null };
  const { inverse } = entry;
  const shown: CalendarEvent | null = found
    ? {
        ...found,
        all_day: inverse.all_day ?? found.all_day,
        end_ms: inverse.end_ms ?? found.end_ms,
        start_ms: inverse.start_ms ?? found.start_ms,
        title: inverse.title ?? found.title,
      }
    : null;
  return { ...base, changes: inverse, event, op: "update", shown };
}
