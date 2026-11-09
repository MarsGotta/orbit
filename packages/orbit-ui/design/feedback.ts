import { useCallback, useEffect, useRef, useState } from "react";

/** What a button that acts says: its label, «Guardando…» while it waits, «✓ Guardado» once confirmed. */
export type Feedback = "idle" | "pending" | "done";

// Long enough to read «✓ Guardado» from the corner of the eye, short enough
// that the button is its usual self by the next press (design guide, button).
const CONFIRMED_MS = 1_800;

/**
 * Tracks one action from the press to its confirmation: pending until the
 * work settles, confirmed for 1.8 s when it succeeds, back to its label when
 * it fails (the failure is told elsewhere, by the notice that names it).
 */
export function useFeedback(): { feedback: Feedback; track: (work: Promise<unknown>) => void } {
  const [feedback, setFeedback] = useState<Feedback>("idle");
  const run = useRef(0);
  const timer = useRef<number | undefined>(undefined);
  useEffect(
    () => () => {
      run.current += 1;
      window.clearTimeout(timer.current);
    },
    [],
  );
  const track = useCallback((work: Promise<unknown>) => {
    const current = ++run.current;
    window.clearTimeout(timer.current);
    setFeedback("pending");
    work.then(
      () => {
        if (run.current !== current) return;
        setFeedback("done");
        timer.current = window.setTimeout(() => setFeedback("idle"), CONFIRMED_MS);
      },
      () => {
        if (run.current === current) setFeedback("idle");
      },
    );
  }, []);
  return { feedback, track };
}
