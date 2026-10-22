import { useCallback, useRef, useState } from "react";
import type { AppCommand } from "../../../bridge/generated";
import type { CalendarEventChanges } from "../../../bridge/generated/CalendarEventChanges";
import type { CalendarSpan } from "../../../bridge/generated/CalendarSpan";
import type { WriteOutcome } from "../../../bridge/generated/WriteOutcome";
import { es } from "../../../i18n/es";
import type { CalendarEvent } from "../eventLook";
import {
  createCommand,
  deleteCommand,
  newRequestId,
  targetOf,
  undoCommand,
  type UndoEntry,
  updateCommand,
} from "./commands";
import { type EventForm, inputOf } from "./form";
import { blockedNotice, deletedToast, type Notice, undoEntryOf } from "./notices";
import { type PendingWrite, withPending } from "./pending";
import { usePendingWrites, type WriteSettled } from "./usePendingWrites";
import { undoPending } from "./undoPending";

type Send = (command: AppCommand) => Promise<unknown>;
type Draft = Omit<PendingWrite, "requestId" | "sentAtMs">;
/** ⌘Z reaches this far back in the session: a short stack in the view's
 *  memory; older changes stay as they are. */
const UNDO_DEPTH = 20;
/** How many ended writes the view remembers for the panel that sent them:
 *  only the panel's latest save is ever asked about, so a few are plenty. */
const SETTLED_KEPT = 20;

export interface DeletedToast {
  requestId: string;
  text: string;
}

interface Memory {
  /** Writes whose undo already ran (from the toast) before they finished. */
  consumed: Set<string>;
  /** The undo of each confirmed delete, for its notice. */
  deletions: Map<string, UndoEntry>;
  events: CalendarEvent[];
  stack: { entry: UndoEntry; requestId: string }[];
}

/**
 * Every write the calendar view makes, and what it shows meanwhile.
 * Each write is sent once; its outcome comes from the agenda head; the undo
 * stack lives in this view's memory. Nothing is asked first: the macOS
 * permission is the decision, and a delete is undone, not confirmed.
 */
export function useCalendarWrites(
  calendar: { events: CalendarEvent[]; writes?: WriteOutcome[] },
  send: Send,
  timezone: string,
  writeError: string | null,
) {
  const [notice, setNotice] = useState<Notice | null>(null);
  const [deleted, setDeleted] = useState<DeletedToast | null>(null);
  // How each write of this view ended, for the panel that sent it.
  const [settled, setSettled] = useState<ReadonlyMap<string, WriteSettled>>(new Map());
  const memory = useRef<Memory>({
    consumed: new Set(),
    deletions: new Map(),
    events: [],
    stack: [],
  });
  memory.current.events = calendar.events;
  const [pending, setPending] = usePendingWrites(calendar, timezone, writeError, {
    remember: (write) => {
      remember(write, memory.current, timezone);
      // «Evento borrado · Deshacer» only once the delete is confirmed: a
      // refused or unknown one has nothing to undo.
      const entry = memory.current.deletions.get(write.requestId);
      if (write.op === "delete" && entry !== undefined)
        setDeleted({ requestId: write.requestId, text: deletedToast(write.event, write.span) });
    },
    reread: () => void send({ command: "calendar_refresh" }),
    setNotice,
    settle: (requestId, outcome) =>
      setSettled((current) => {
        const next = new Map(current).set(requestId, outcome);
        const oldest = next.keys().next().value;
        if (next.size > SETTLED_KEPT && oldest !== undefined) next.delete(oldest);
        return next;
      }),
  });
  const write = useCallback(
    (draft: Draft, command: (id: string) => AppCommand) => {
      const requestId = newRequestId();
      // A new event on its way is its own card, no longer the draft.
      const own =
        draft.op === "create" && draft.shown !== null
          ? { ...draft.shown, id: `pending:${requestId}`, ui: {} }
          : null;
      const entry = own ? { ...draft, event: own, shown: own } : draft;
      setPending((current) => [...current, { ...entry, requestId, sentAtMs: Date.now() }]);
      setNotice(null);
      void send(command(requestId));
      return requestId;
    },
    [send],
  );
  const actions = writeActions({ memory: memory.current, setDeleted, setNotice, timezone, write });
  return {
    ...actions,
    deleted,
    dismissDeleted: () => setDeleted(null),
    dismissNotice: () => setNotice(null),
    notice,
    /** How the write `requestId` ended; `undefined` while it is on its way. */
    outcomeOf: (requestId: string) => settled.get(requestId),
    shown: (events: CalendarEvent[]) => withPending(events, pending),
  };
}

interface ActionParts {
  memory: Memory;
  setDeleted: (toast: DeletedToast | null) => void;
  setNotice: (notice: Notice | null) => void;
  timezone: string;
  write: (draft: Draft, command: (id: string) => AppCommand) => string;
}

function undoActions({ memory, setDeleted, setNotice, write }: ActionParts) {
  const runUndo = (entry: UndoEntry) => {
    if (entry.kind === "impossible") {
      setNotice({ kind: "info", text: es.calendar.write.cannotUndo(entry.title) });
      return;
    }
    write(undoPending(entry, memory.events), (id) => undoCommand(entry, id));
    // After the write, which clears the last notice: this one is the undo's.
    setNotice({ kind: "info", text: es.calendar.write.undoing(entry.title) });
  };
  return {
    undoDeleted: (requestId: string) => {
      const entry = memory.deletions.get(requestId);
      if (!entry) return;
      forget(memory, requestId);
      runUndo(entry);
    },
    undoLast: () => {
      const last = memory.stack.pop();
      if (!last) return setNotice({ kind: "info", text: es.calendar.write.nothingToUndo });
      forget(memory, last.requestId);
      setDeleted(null);
      runUndo(last.entry);
    },
  };
}

function writeActions(parts: ActionParts) {
  const { setDeleted, setNotice, timezone, write } = parts;
  return {
    ...undoActions(parts),
    create: (form: EventForm, shown: CalendarEvent): string =>
      write({ event: shown, op: "create", shown, span: "this", undoable: true }, (id) =>
        createCommand(id, inputOf(form, timezone)),
      ),
    remove: (event: CalendarEvent, span: CalendarSpan) => {
      const target = targetOf(event);
      if (target === null) return setNotice(blockedNotice(event, "delete"));
      const draft: Draft = { event, op: "delete", shown: null, span, undoable: true };
      write(draft, (request) => deleteCommand(request, target, span));
      setDeleted(null);
      // On its way: no «Deshacer» yet, there is nothing to undo.
      setNotice({ kind: "info", text: es.calendar.write.deleting(event.title) });
    },
    update: (
      event: CalendarEvent,
      changes: CalendarEventChanges,
      span: CalendarSpan,
      shown: CalendarEvent,
    ): string | null => {
      const target = targetOf(event);
      if (target === null) {
        setNotice(blockedNotice(event, "edit"));
        return null;
      }
      return write({ changes, event, op: "update", shown, span, undoable: true }, (id) =>
        updateCommand(id, target, span, changes),
      );
    },
    warn: (next: Notice) => setNotice(next),
  };
}

/** A write that was applied: only now can ⌘Z (and a delete's notice) undo it. */
function remember(write: PendingWrite, memory: Memory, timezone: string): void {
  if (!write.undoable || memory.consumed.has(write.requestId)) return;
  const entry = undoEntryOf(write, timezone);
  if (entry === null) return;
  if (write.op === "delete") memory.deletions.set(write.requestId, entry);
  memory.stack.push({ entry, requestId: write.requestId });
  if (memory.stack.length > UNDO_DEPTH) memory.stack.shift();
}

function forget(memory: Memory, requestId: string): void {
  memory.consumed.add(requestId);
  memory.deletions.delete(requestId);
  memory.stack = memory.stack.filter((item) => item.requestId !== requestId);
}
