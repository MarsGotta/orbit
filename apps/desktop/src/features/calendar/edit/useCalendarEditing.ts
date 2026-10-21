import { useRef } from "react";
import type { AppView as AppSnapshot } from "../../../state/app-view";
import type { AppCommand } from "../../../bridge/generated";
import type { CalendarEvent } from "../eventLook";
import { type BlockedAction, blockedNotice, lockedReason } from "./notices";
import type { Span } from "./geometry";
import type { GridEdit } from "./useGridGestures";
import type { LaneEdit } from "./useLaneGestures";
import { useCalendarWrites } from "./useCalendarWrites";
import { drawn, isDirty, saveChoices, useEventEditor } from "./useEventEditor";
import { useEditKeys } from "./useEditKeys";

type Send = (command: AppCommand) => Promise<void>;

interface ViewSelection {
  selectedEvent: CalendarEvent | null;
  setSelectedEvent: (event: CalendarEvent | null) => void;
}

/**
 * Whose error the calendar's banner shows. The store keeps one error per
 * feature, and a write and a read are both the agenda's: a write refused
 * before it was queued is told as that write's refusal, never as «No se pudo
 * leer el calendario» (a state and a different state never share a name).
 */
function useTrackedSend(send: Send, error: string | null) {
  const last = useRef<"write" | "other">("other");
  // The error on screen when the last write left, which is not its own.
  const before = useRef<string | null>(null);
  const shown = useRef(error);
  shown.current = error;
  // The error a write left: it stays the write's until the store clears it,
  // even while a read that follows is still on its way.
  const claimed = useRef<string | null>(null);
  if (error === null) claimed.current = null;
  else if (last.current === "write" && claimed.current === null && error !== before.current)
    claimed.current = error;
  const fromWrite = error !== null && error === claimed.current;
  const tracked: Send = (command) => {
    last.current =
      command.command === "calendar" &&
      (command.request.action === "save" || command.request.action === "delete")
        ? "write"
        : "other";
    if (last.current === "write") before.current = shown.current;
    return send(command);
  };
  return {
    /** A read the view sends by another way (its «Reintentar»). */
    markRead: () => {
      last.current = "other";
    },
    readError: fromWrite ? null : error,
    send: tracked,
    writeError: fromWrite ? error : null,
  };
}

type Writes = ReturnType<typeof useCalendarWrites>;
type Editor = ReturnType<typeof useEventEditor>;
type Calendars = AppSnapshot["calendar"]["calendars"];

/** What the grid and the lane do with a finished gesture. */
function gestureEdits(editor: Editor, writes: Writes, calendars: Calendars) {
  const editable = (event: CalendarEvent) =>
    event.ui?.saving !== true && lockedReason(event, calendars) === null;
  const blocked = (event: CalendarEvent, action: BlockedAction) =>
    writes.warn(blockedNotice(event, action, calendars));
  const change = (event: CalendarEvent, span: Span) => {
    const editing = editor.editing;
    if (event.ui?.draft === true && editing !== null) {
      editor.setForm({ ...editing.form, endMs: span.endMs, startMs: span.startMs });
      return;
    }
    // Only what the gesture changed: stretching sends the end alone.
    const changes = {
      ...(span.startMs !== event.start_ms ? { start_ms: span.startMs } : {}),
      ...(span.endMs !== event.end_ms ? { end_ms: span.endMs } : {}),
    };
    writes.update(event, changes, "this", { ...event, end_ms: span.endMs, start_ms: span.startMs });
  };
  const shared = {
    editable,
    onBlocked: (event: CalendarEvent) => blocked(event, "move"),
    onChange: change,
    onDeselect: () => {
      if (editor.editing === null) return false;
      editor.close();
      return true;
    },
  };
  const grid: GridEdit = { ...shared, onCreate: (span) => editor.startDraft(span, false) };
  const lane: LaneEdit = { ...shared, onCreate: (span) => editor.startDraft(span, true) };
  return { blocked, editable, grid, lane };
}

function useEditorKeys(editor: Editor, writes: Writes, edits: ReturnType<typeof gestureEdits>) {
  useEditKeys({
    onDelete: () => {
      const event = editor.editing?.event;
      if (event == null) return editor.discard();
      if (!edits.editable(event)) return edits.blocked(event, "delete");
      editor.remove("this");
    },
    onEscape: editor.escape,
    onSave: () => {
      // ⌘↵ presses the panel's first button; with nothing to save there is
      // no button, and it does nothing.
      const editing = editor.editing;
      if (editing === null || !isDirty(editing)) return;
      const span = saveChoices(editing)[0];
      if (span !== undefined) editor.save(span);
    },
    onUndo: writes.undoLast,
    selected: editor.editing !== null,
  });
}

/**
 * Creating, moving, stretching, editing and deleting events in the calendar
 * (P9-22): the writes, the selected event's panel, the gestures of the grid
 * and the lane, and the keys.
 */
export function useCalendarEditing(
  calendar: AppSnapshot["calendar"],
  send: Send,
  error: string | null,
  timezone: string,
  view: ViewSelection,
) {
  const tracked = useTrackedSend(send, error);
  const writes = useCalendarWrites(calendar, tracked.send, timezone, tracked.writeError);
  const editor = useEventEditor(
    view.selectedEvent,
    view.setSelectedEvent,
    calendar.calendars,
    writes,
  );
  const edits = gestureEdits(editor, writes, calendar.calendars);
  useEditorKeys(editor, writes, edits);
  return {
    display: (events: CalendarEvent[]) => {
      const shown = writes.shown(events);
      const editing = editor.editing;
      return editing !== null && editing.event === null
        ? [...shown, drawn(editing.form, null)]
        : shown;
    },
    editor,
    grid: edits.grid,
    lane: edits.lane,
    openEvent: (event: CalendarEvent) => {
      if (event.ui?.draft === true || event.ui?.saving === true) return;
      editor.open(event);
    },
    markRead: tracked.markRead,
    readError: tracked.readError,
    send: tracked.send,
    writes,
  };
}
