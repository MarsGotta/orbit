import { useRef, useState } from "react";

/** What a control asks for may arrive later, or be refused: a promise says which. */
export type CheckedChange = (checked: boolean) => void | Promise<unknown>;

/**
 * A value that someone else confirms (the hub, macOS). After a press the
 * control knows what was asked and waits until the value it is given reaches
 * it. It never gives up by itself; only a refusal (the promise rejects) ends
 * the wait early, and then the given value stands.
 */
export function useRequested(checked: boolean, onCheckedChange: CheckedChange) {
  const [requested, setRequested] = useState<boolean | null>(null);
  const latest = useRef(0);
  if (requested !== null && requested === checked) setRequested(null);
  const request = (next: boolean) => {
    const current = ++latest.current;
    setRequested(next === checked ? null : next);
    const result = onCheckedChange(next);
    if (result instanceof Promise)
      result.catch(() => {
        if (latest.current === current) setRequested(null);
      });
  };
  return { requested, waiting: requested !== null, request };
}
