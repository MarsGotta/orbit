import { useEffect, useRef } from "react";

/**
 * What a view on screen still holds and has not handed to the daemon yet: a
 * save waiting for the pause after typing, say. It returns what the handing
 * over returns (the order's promise), so the quit can wait for it.
 */
export type Flush = () => unknown;

// The views on screen now, each by the latest save it gave.
const flushes = new Set<{ current: Flush }>();

/**
 * Joins the quit flush: before Orbit quits or installs an update it calls
 * `flush` and waits for what it returns. Only while the caller is mounted;
 * its latest `flush` is the one called.
 */
export function useFlushOnQuit(flush: Flush): void {
  const latest = useRef(flush);
  latest.current = flush;
  useEffect(() => {
    flushes.add(latest);
    return () => {
      flushes.delete(latest);
    };
  }, []);
}

/**
 * Hands over everything the views hold, and settles once each handing over
 * has. A save that fails holds neither the others nor the quit: it says its
 * own refusal. How long Orbit.app waits for this is its bound
 * (`INTERFACE_FLUSH_BOUND`, `src-tauri/src/interface_flush.rs`), not this one's.
 */
export async function flushBeforeQuit(): Promise<void> {
  await Promise.allSettled(
    [...flushes].map((flush) => {
      try {
        return Promise.resolve(flush.current());
      } catch (error: unknown) {
        return Promise.reject(error);
      }
    }),
  );
}
