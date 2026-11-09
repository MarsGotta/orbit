import { useCallback, useEffect, useRef } from "react";
import { useFlushOnQuit } from "../../state/quit-flush";

/** The user's edits reach the daemon one second after they pause, and at
 * least every five seconds while they keep typing. */
export const SAVE_AFTER_MS = 1_000;
export const SAVE_AT_LEAST_EVERY_MS = 5_000;

/**
 * Commits the latest value after a pause, never later than the maximum wait,
 * and once more when the component leaves, or Orbit quits, with a value
 * still pending. `flush` hands back what the commit returns (the order's
 * promise), so the quit can wait for it.
 */
export function useDebouncedCommit<T>(commit: (value: T) => unknown): {
  schedule: (value: T) => void;
  flush: () => unknown;
} {
  const pending = useRef<{ value: T } | null>(null);
  const pause = useRef<number | null>(null);
  const ceiling = useRef<number | null>(null);
  const commitRef = useRef(commit);
  commitRef.current = commit;

  const flush = useCallback((): unknown => {
    if (pause.current !== null) window.clearTimeout(pause.current);
    if (ceiling.current !== null) window.clearTimeout(ceiling.current);
    pause.current = null;
    ceiling.current = null;
    const next = pending.current;
    pending.current = null;
    return next ? commitRef.current(next.value) : undefined;
  }, []);

  const schedule = useCallback(
    (value: T) => {
      pending.current = { value };
      if (pause.current !== null) window.clearTimeout(pause.current);
      pause.current = window.setTimeout(flush, SAVE_AFTER_MS);
      ceiling.current ??= window.setTimeout(flush, SAVE_AT_LEAST_EVERY_MS);
    },
    [flush],
  );

  useEffect(() => () => void flush(), [flush]);
  useFlushOnQuit(flush);
  return { schedule, flush };
}
