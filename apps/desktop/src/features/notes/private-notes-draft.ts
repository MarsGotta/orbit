import { useCallback, useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { SAVE_AFTER_MS, SAVE_AT_LEAST_EVERY_MS } from "./use-debounced-commit";

/**
 * The ceiling of one meeting's private notes, in characters as the daemon
 * counts them (code points): the daemon and the hub refuse a longer text, so
 * the editor stops typing at it and says so before. A copy of orbit-core's,
 * registered in `contracts/constants.json`.
 */
export const MAX_PRIVATE_NOTES_CHARS = 64 * 1024;

/**
 * Where the save stands: nothing waiting and the last one kept on this Mac;
 * one waiting or on its way; or the last one refused, its text still waiting.
 */
export type SaveState = "saved" | "saving" | "refused";

/**
 * Saves what is waiting when the note is left, and when the window is left
 * (hidden, reloaded) while the note is open.
 */
function useSaveOnLeave(flush: () => void): void {
  useEffect(() => {
    const hidden = () => {
      if (document.visibilityState === "hidden") flush();
    };
    window.addEventListener("pagehide", flush);
    document.addEventListener("visibilitychange", hidden);
    return () => {
      window.removeEventListener("pagehide", flush);
      document.removeEventListener("visibilitychange", hidden);
      flush();
    };
  }, [flush]);
}

/**
 * The user's draft of their private notes and its saves to the daemon: a
 * second after they stop typing (and at least every five seconds while they
 * type on), and at once when they leave the text, the window or the note.
 * The daemon's echo never replaces what they typed after the save it echoes.
 * A refused save keeps its text waiting: it goes again with the next save,
 * and before the note is left.
 */
export function usePrivateNotesDraft(
  recordingId: string,
  stored: string,
  send: (command: AppCommand) => unknown,
): { draft: string; state: SaveState; update: (next: string) => void; flush: () => void } {
  const [draft, setDraft] = useState(stored);
  const [state, setState] = useState<SaveState>("saved");
  const timer = useRef<number | null>(null);
  const ceiling = useRef<number | null>(null);
  const pending = useRef<string | null>(null);
  const inFlight = useRef<string | null>(null);
  const sendRef = useRef(send);
  sendRef.current = send;

  const flush = useCallback(() => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    if (ceiling.current !== null) window.clearTimeout(ceiling.current);
    timer.current = null;
    ceiling.current = null;
    const next = pending.current;
    if (next === null) return;
    pending.current = null;
    inFlight.current = next;
    void Promise.resolve(
      sendRef.current({ command: "note_private_save", recording_id: recordingId, markdown: next }),
    )
      .then((accepted) => accepted !== false)
      .catch(() => false)
      .then((kept) => {
        // A kept save waits for its echo (below); a refused one waits to go again.
        if (!kept) {
          if (inFlight.current === next) inFlight.current = null;
          pending.current ??= next;
          setState("refused");
        } else if (pending.current === null) setState("saved");
      });
  }, [recordingId]);

  const update = useCallback(
    (next: string) => {
      setDraft(next);
      setState("saving");
      pending.current = next;
      if (timer.current !== null) window.clearTimeout(timer.current);
      timer.current = window.setTimeout(flush, SAVE_AFTER_MS);
      ceiling.current ??= window.setTimeout(flush, SAVE_AT_LEAST_EVERY_MS);
    },
    [flush],
  );

  useEffect(() => {
    if (inFlight.current !== null) {
      if (stored === inFlight.current) inFlight.current = null;
      return;
    }
    if (pending.current !== null) return;
    setDraft(stored);
  }, [stored]);

  useSaveOnLeave(flush);

  return { draft, state, update, flush };
}
