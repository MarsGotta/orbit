import { useSyncExternalStore } from "react";

const REDUCE = "(prefers-reduced-motion: reduce)";

function query(): MediaQueryList | null {
  try {
    return typeof window.matchMedia === "function" ? window.matchMedia(REDUCE) : null;
  } catch {
    return null;
  }
}

function subscribe(listener: () => void): () => void {
  const media = query();
  media?.addEventListener("change", listener);
  return () => media?.removeEventListener("change", listener);
}

/** «Reducir movimiento» on the Mac, and when it changes. */
export function useReducedMotion(): boolean {
  return useSyncExternalStore(subscribe, () => query()?.matches ?? false);
}
