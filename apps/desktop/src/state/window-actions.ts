import { useEffect } from "react";
import { create } from "zustand";

/** Existing contextual handlers. Only the last deleted template is retained, in memory. */
export const useWindowActions = create<{
  findTranscript: (() => void) | null;
  undoTemplate: (() => void) | null;
}>(() => ({ findTranscript: null, undoTemplate: null }));

export function textHasFocus(): boolean {
  return (
    document.activeElement?.closest(
      "input, textarea, [contenteditable]:not([contenteditable='false']), [role='textbox']",
    ) != null
  );
}

export function undoTemplateKey(event: KeyboardEvent): void {
  const undo = useWindowActions.getState().undoTemplate;
  if (
    !event.defaultPrevented &&
    !event.shiftKey &&
    (event.metaKey || event.ctrlKey) &&
    event.key.toLowerCase() === "z" &&
    !textHasFocus() &&
    undo !== null
  ) {
    event.preventDefault();
    undo();
  }
}

export function useTemplateUndoKeyboard() {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => undoTemplateKey(event);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
