import { useEffect, useState, useSyncExternalStore } from "react";

// Three cards fit the corner of the smallest Orbit window. The rest wait,
// mounted and in order, until a place opens: nothing is discarded, and a
// notice's time does not run while it waits.
const VISIBLE = 3;

let queue: number[] = [];
let lastId = 0;
const listeners = new Set<() => void>();

function changed(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/**
 * Whether a notice that asks for a place in the corner (`inCorner`) has one;
 * the first to ask shows first. A notice outside the corner always shows.
 */
export function useCornerPlace(inCorner: boolean): boolean {
  const [id] = useState(() => ++lastId);
  useEffect(() => {
    if (!inCorner) return;
    queue = [...queue, id];
    changed();
    return () => {
      queue = queue.filter((queued) => queued !== id);
      changed();
    };
  }, [id, inCorner]);
  const place = useSyncExternalStore(subscribe, () => queue.indexOf(id));
  return !inCorner || (place >= 0 && place < VISIBLE);
}
