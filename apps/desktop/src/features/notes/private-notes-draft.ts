import { useCallback, useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { SAVE_AFTER_MS, SAVE_AT_LEAST_EVERY_MS } from "./use-debounced-commit";
/**
 * Mars's draft of her private notes and its saves to the daemon (P9-03).
 * The daemon's echo never replaces what she typed after the save it echoes;
 * a save still waiting goes out when she leaves the note.
 */
export function usePrivateNotesDraft(
  recordingId: string,
  stored: string,
  send: (command: AppCommand) => void,
): [string, (next: string) => void] {
  const [draft, setDraft] = useState(stored);
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
    sendRef.current({ command: "note_private_save", recording_id: recordingId, markdown: next });
  }, [recordingId]);

  const update = useCallback(
    (next: string) => {
      setDraft(next);
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

  useEffect(() => flush, [flush]);

  return [draft, update];
}
