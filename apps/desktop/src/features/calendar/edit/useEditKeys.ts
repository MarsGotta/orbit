import { useEffect, useRef } from "react";
import { isTypingTarget } from "../keyboard";

export interface EditKeys {
  /** ⌫ or Supr with an event selected, outside a text field. */
  onDelete: () => void;
  /** Esc: leaves the selection (a draft without changes disappears). */
  onEscape: () => void;
  /** ⌘↵, also from a field: presses the panel's first button. */
  onSave: () => void;
  /** ⌘Z outside a text field (inside one, it undoes the typing). */
  onUndo: () => void;
  /** Whether an event or a draft is selected. */
  selected: boolean;
}

/** What a key does to the calendar's events (P9-22), or `null`. ⌥ keys
 *  belong to the keyboard (REF-07) and are never taken. */
export function editKeyAction(
  press: { altKey: boolean; ctrlKey: boolean; key: string; metaKey: boolean },
  typing: boolean,
  selected: boolean,
): keyof Omit<EditKeys, "selected"> | null {
  if (press.altKey) return null;
  const command = press.metaKey || press.ctrlKey;
  if (command && press.key === "Enter") return selected ? "onSave" : null;
  if (command && press.key.toLowerCase() === "z") return typing ? null : "onUndo";
  if (command) return null;
  if (press.key === "Escape") return selected ? "onEscape" : null;
  if ((press.key === "Backspace" || press.key === "Delete") && !typing && selected)
    return "onDelete";
  return null;
}

/** The calendar's editing keys while it is on screen (P9-22). */
export function useEditKeys(keys: EditKeys) {
  const latest = useRef(keys);
  latest.current = keys;
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.repeat || event.shiftKey) return;
      // Inside the panel a key belongs to its fields and buttons: ⌫ there
      // never deletes the event the panel is editing.
      const inPanel =
        event.target instanceof HTMLElement && event.target.closest("[data-event-panel]") !== null;
      const typing = inPanel || isTypingTarget(event.target);
      const action = editKeyAction(event, typing, latest.current.selected);
      if (action === null) return;
      event.preventDefault();
      latest.current[action]();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
