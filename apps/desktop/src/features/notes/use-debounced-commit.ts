import { useCallback, useEffect, useRef } from "react";

/** The user's edits reach the daemon one second after they pause, and at
 * least every five seconds while they keep typing. */
export const SAVE_AFTER_MS = 1_000;
export const SAVE_AT_LEAST_EVERY_MS = 5_000;

/**
 * Commits the latest value after a pause, never later than the maximum wait,
 * and once more when the component leaves with a value still pending.
 */
export function useDebouncedCommit<T>(commit: (value: T) => void): {
  schedule: (value: T) => void;
  flush: () => void;
} {
  const pending = useRef<{ value: T } | null>(null);
  const pause = useRef<number | null>(null);
  const ceiling = useRef<number | null>(null);
  const commitRef = useRef(commit);
  commitRef.current = commit;

  const flush = useCallback(() => {
    if (pause.current !== null) window.clearTimeout(pause.current);
    if (ceiling.current !== null) window.clearTimeout(ceiling.current);
    pause.current = null;
    ceiling.current = null;
    const next = pending.current;
    pending.current = null;
    if (next) commitRef.current(next.value);
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

  useEffect(() => flush, [flush]);
  return { schedule, flush };
}
