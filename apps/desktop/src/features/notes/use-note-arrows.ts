import { useEffect, useRef } from "react";
import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import { DAY_GROUPS, groupFor } from "./notes-format";

/** The list in the order it is shown: by day group, then as the daemon sent it. */
export function listOrder(list: NoteListItem[]): NoteListItem[] {
  return DAY_GROUPS.flatMap(([group]) => list.filter((note) => groupFor(note) === group));
}

/** The note ↑ or ↓ goes to from `currentId`; the first one when none is open. */
export function adjacentNote(
  list: NoteListItem[],
  currentId: string,
  step: 1 | -1,
): NoteListItem | undefined {
  const ordered = listOrder(list);
  const at = ordered.findIndex((note) => note.recording_id === currentId);
  if (at < 0) return ordered[0];
  return ordered[at + step];
}

function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName) ||
    target.closest("[contenteditable='true'], [role='menu'], [role='dialog']") !== null
  );
}

/** ↑ / ↓ move between notes, never while the user is typing or in a menu. */
export function useNoteArrows(
  list: NoteListItem[],
  currentId: string,
  open: (note: NoteListItem) => void,
) {
  const latest = useRef({ list, currentId, open });
  latest.current = { list, currentId, open };
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return;
      if (event.defaultPrevented || isTyping(event.target)) return;
      const { list: notes, currentId: id, open: go } = latest.current;
      const next = adjacentNote(notes, id, event.key === "ArrowDown" ? 1 : -1);
      if (!next || next.recording_id === id) return;
      event.preventDefault();
      go(next);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
