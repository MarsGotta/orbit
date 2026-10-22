import { useCallback, useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { asTextOver, useLocalDraft } from "../../state/local-drafts";
import { useFlushOnQuit } from "../../state/quit-flush";
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
function useSaveOnLeave(flush: () => unknown): void {
  useEffect(() => {
    const hidden = () => {
      if (document.visibilityState === "hidden") flush();
    };
    const leave = () => void flush();
    window.addEventListener("pagehide", leave);
    document.addEventListener("visibilitychange", hidden);
    return () => {
      window.removeEventListener("pagehide", leave);
      document.removeEventListener("visibilitychange", hidden);
      flush();
    };
  }, [flush]);
}

/**
 * What the person typed and the daemon has not taken yet, kept on this Mac
 * as it is typed (`state/local-drafts.ts`), over the text the daemon holds:
 * ⌘Q, an update or a fall within the second before a save loses nothing.
 * It comes back only over that same text, and is let go of once taken.
 */
function useKeptNote(recordingId: string, stored: string) {
  const kept = useLocalDraft(`note.${recordingId}.private`, null, asTextOver);
  // The text the daemon holds: what it published, or the last save it took.
  const base = useRef(stored);
  const [restored] = useState(() =>
    kept.value !== null && kept.value.from === stored ? kept.value.text : null,
  );
  return {
    restored,
    recovered: kept.recovered && restored !== null,
    held: (published: string) => (base.current = published),
    typed: (text: string) => kept.set(text === base.current ? null : { from: base.current, text }),
    taken: (text: string) => {
      base.current = text;
      kept.set((now) => (now?.text === text ? null : now));
    },
  };
}

type KeptNote = ReturnType<typeof useKeptNote>;

/**
 * The saves to the daemon: `flush` sends what waits and hands back the
 * order's promise; a refused save waits to go again, a taken one lets the
 * kept draft go.
 */
function useNoteSaves(
  recordingId: string,
  send: (command: AppCommand) => unknown,
  kept: KeptNote,
  setState: (state: SaveState) => void,
) {
  const timer = useRef<number | null>(null);
  const ceiling = useRef<number | null>(null);
  const pending = useRef<string | null>(kept.restored);
  const inFlight = useRef<string | null>(null);
  const latest = useRef({ send, kept, setState });
  latest.current = { send, kept, setState };

  const flush = useCallback((): unknown => {
    for (const wait of [timer, ceiling]) {
      if (wait.current !== null) window.clearTimeout(wait.current);
      wait.current = null;
    }
    const next = pending.current;
    if (next === null) return undefined;
    pending.current = null;
    inFlight.current = next;
    const command: AppCommand = {
      command: "note_private_save",
      recording_id: recordingId,
      markdown: next,
    };
    return Promise.resolve(latest.current.send(command))
      .then((accepted) => accepted !== false)
      .catch(() => false)
      .then((taken) => {
        // A kept save waits for its echo; a refused one waits to go again.
        if (!taken) {
          if (inFlight.current === next) inFlight.current = null;
          pending.current ??= next;
          latest.current.setState("refused");
          return;
        }
        latest.current.kept.taken(next);
        if (pending.current === null) latest.current.setState("saved");
      });
  }, [recordingId]);

  const schedule = useCallback(() => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(flush, SAVE_AFTER_MS);
    ceiling.current ??= window.setTimeout(flush, SAVE_AT_LEAST_EVERY_MS);
  }, [flush]);

  return { pending, inFlight, flush, schedule };
}

/**
 * The user's draft of their private notes and its saves to the daemon: a
 * second after they stop typing (and at least every five seconds while they
 * type on), and at once when they leave the text, the window or the note,
 * and when Orbit quits. The daemon's echo never replaces what they typed
 * after the save it echoes. A refused save keeps its text waiting: it goes
 * again with the next save, and before the note is left.
 */
export function usePrivateNotesDraft(
  recordingId: string,
  stored: string,
  send: (command: AppCommand) => unknown,
): {
  draft: string;
  state: SaveState;
  recovered: boolean;
  update: (next: string) => void;
  flush: () => unknown;
} {
  const kept = useKeptNote(recordingId, stored);
  const [draft, setDraft] = useState(kept.restored ?? stored);
  const [state, setState] = useState<SaveState>(kept.restored === null ? "saved" : "saving");
  const { pending, inFlight, flush, schedule } = useNoteSaves(recordingId, send, kept, setState);

  const update = (next: string) => {
    setDraft(next);
    setState("saving");
    pending.current = next;
    kept.typed(next);
    schedule();
  };

  // A text brought back from an earlier run goes to the daemon as if just typed.
  useEffect(() => {
    if (pending.current !== null) schedule();
  }, [pending, schedule]);

  useEffect(() => {
    if (inFlight.current !== null) {
      if (stored === inFlight.current) inFlight.current = null;
      return;
    }
    if (pending.current !== null) return;
    kept.held(stored);
    setDraft(stored);
    // Only a newer text from the daemon is news here.
  }, [stored]);

  useSaveOnLeave(flush);
  useFlushOnQuit(flush);

  return { draft, state, recovered: kept.recovered && state !== "saved", update, flush };
}
