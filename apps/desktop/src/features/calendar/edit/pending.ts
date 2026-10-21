/**
 * Writes on their way, and what the calendar shows meanwhile. Pure:
 * the writes the view sent, the outcomes the agenda head carries and the
 * events read in; what stays pending, what finished and how, out.
 *
 * A card moves at once and is marked as saving. A refusal puts it back and
 * says why; an unknown outcome is said as it is and nothing is sent again.
 * A saved write stays drawn where it went until the read the daemon starts
 * after every write shows it there (or `APPLIED_WAIT_MS` passes).
 */
import type { CalendarEventChanges } from "../../../bridge/generated/CalendarEventChanges";
import type { CalendarSpan } from "../../../bridge/generated/CalendarSpan";
import type { WriteOutcome } from "../../../bridge/generated/WriteOutcome";
import type { WriteTarget } from "../../../bridge/generated/WriteTarget";
import type { CalendarEvent } from "../eventLook";
import { sameTarget, type Target, targetOf, updatedTarget } from "./commands";

/**
 * How long a saved write stays drawn by itself when no read shows it: the
 * read after a write takes one EventKit round (milliseconds); ten seconds
 * covers a slow Mac and the agenda's own retries. It protects the screen
 * from a card held for ever when the event left the range on screen. It is
 * not a wait on the write, which has already finished.
 */
export const APPLIED_WAIT_MS = 10_000;

/**
 * How long a write may go unmentioned by the agenda head before its outcome
 * is unknown: the daemon's own write lifetime (`WRITE_LIFETIME_NS`, 5 s in
 * `contracts/constants.json`, compared by `test_constants_copies.py`). A
 * write the daemon accepted appears on the head at once as queued; one the
 * head never mentions was lost on the way — the head went (privacy stop, a
 * withdrawn permission, a daemon restart) or its outcome did not fit — and
 * after the daemon's own lifetime nobody can say whether it was applied. It
 * protects the card from staying «Guardando…» for ever. It is not a retry:
 * the agenda is read again and the write is never sent a second time.
 */
export const WRITE_LIFETIME_MS = 5_000;

export interface PendingWrite {
  appliedAtMs?: number;
  changes?: CalendarEventChanges;
  /** As read before the write (change, delete), or as drawn (create). */
  event: CalendarEvent;
  op: "create" | "update" | "delete";
  requestId: string;
  saved?: WriteTarget;
  /** When it was sent, by the view's clock. */
  sentAtMs: number;
  /** What the card shows meanwhile; none for a delete. */
  shown: CalendarEvent | null;
  span: CalendarSpan;
  /** An undo is not itself undone. */
  undoable: boolean;
}

export interface Finished {
  /** `null`: the head never said, within `WRITE_LIFETIME_MS`. */
  record: WriteOutcome | null;
  write: PendingWrite;
}

function where(write: PendingWrite): Target | null {
  if (write.op === "create") return write.saved ?? null;
  if (write.op === "delete") return targetOf(write.event);
  return updatedTarget(write.event, write.changes ?? {}, write.saved);
}

/** Whether `events` already show the write done. */
export function reflected(write: PendingWrite, events: readonly CalendarEvent[]): boolean {
  if (write.op === "delete") {
    const target = where(write);
    return target !== null && !events.some((event) => sameTarget(event, target));
  }
  const shown = write.shown;
  if (shown === null) return true;
  // The same event at the drawn times: its occurrence date is not compared,
  // since EventKit decides it (a moved single event takes its new start).
  const item = where(write)?.calendar_item_id;
  return events.some(
    (event) =>
      (item === undefined || event.calendar_item_id === item) &&
      event.title === shown.title &&
      event.start_ms === shown.start_ms &&
      event.end_ms === shown.end_ms,
  );
}

export interface Reconciled {
  /** Saved or deleted this round: what ⌘Z can take back. */
  applied: PendingWrite[];
  /** Refused, or its outcome unknown: the notice says which. */
  failed: Finished[];
  pending: PendingWrite[];
}

export function reconcile(
  pending: readonly PendingWrite[],
  writes: readonly WriteOutcome[],
  events: readonly CalendarEvent[],
  nowMs: number,
): Reconciled {
  const result: Reconciled = { applied: [], failed: [], pending: [] };
  for (const write of pending) {
    const record = writes.find((item) => item.request_id === write.requestId) ?? null;
    if (record?.stage === "refused" || record?.stage === "outcome_unknown") {
      result.failed.push({ record, write });
      continue;
    }
    if (record === null && write.appliedAtMs === undefined && unheardFor(write, nowMs) <= 0) {
      result.failed.push({ record: null, write });
      continue;
    }
    let next = write;
    if (
      (record?.stage === "saved" || record?.stage === "deleted") &&
      write.appliedAtMs === undefined
    ) {
      next = { ...write, appliedAtMs: nowMs, ...(record.saved ? { saved: record.saved } : {}) };
      result.applied.push(next);
    }
    const done =
      next.appliedAtMs !== undefined &&
      (reflected(next, events) || nowMs - next.appliedAtMs >= APPLIED_WAIT_MS);
    if (!done) result.pending.push(next);
  }
  return result;
}

/** The events with every write on its way drawn where it is going. */
export function withPending(
  events: readonly CalendarEvent[],
  pending: readonly PendingWrite[],
): CalendarEvent[] {
  const active = pending.filter((write) => !reflected(write, events));
  const hidden = active
    .filter((write) => write.op !== "create")
    .map((write) => targetOf(write.event))
    .filter((target): target is Target => target !== null);
  const shown = active
    .map((write) => write.shown)
    .filter((event): event is CalendarEvent => event !== null)
    .map((event) => ({ ...event, ui: { ...event.ui, saving: true } }));
  return [
    ...events.filter((event) => !hidden.some((target) => sameTarget(event, target))),
    ...shown,
  ];
}

/** How long until `write` counts as unheard (≤ 0: it does). */
function unheardFor(write: PendingWrite, nowMs: number): number {
  return write.sentAtMs + WRITE_LIFETIME_MS - nowMs;
}

/** When `reconcile` next has something to decide by the clock alone: a saved
 *  write no read showed, or a write the head has not mentioned. */
export function nextDeadline(
  pending: readonly PendingWrite[],
  writes: readonly WriteOutcome[],
): number | null {
  const instants = pending.map((write) => {
    if (write.appliedAtMs !== undefined) return write.appliedAtMs + APPLIED_WAIT_MS;
    const heard = writes.some((item) => item.request_id === write.requestId);
    return heard ? Infinity : write.sentAtMs + WRITE_LIFETIME_MS;
  });
  const due = Math.min(...instants);
  return Number.isFinite(due) ? due : null;
}
