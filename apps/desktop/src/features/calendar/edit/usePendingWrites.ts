import { type Dispatch, type SetStateAction, useEffect, useState } from "react";
import type { WriteOutcome } from "../../../bridge/generated/WriteOutcome";
import type { CalendarEvent } from "../eventLook";
import { failureNotice, type Notice } from "./notices";
import { nextDeadline, type PendingWrite, reconcile } from "./pending";

interface Hooks {
  /** A write that finished: what ⌘Z may take back. */
  remember: (write: PendingWrite) => void;
  /** Reads the agenda again (never the write). */
  reread: () => void;
  setNotice: (update: (current: Notice | null) => Notice | null) => void;
}

/**
 * The writes on their way, followed against the agenda head: each
 * outcome that arrives ends its write once; a refusal or an unknown outcome
 * becomes the notice; a saved write no read shows is let go of after
 * `APPLIED_WAIT_MS`. Nothing here sends anything.
 */
export function usePendingWrites(
  calendar: { events: CalendarEvent[]; writes?: WriteOutcome[] },
  timezone: string,
  writeError: string | null,
  hooks: Hooks,
): [PendingWrite[], Dispatch<SetStateAction<PendingWrite[]>>] {
  const [pending, setPending] = useState<PendingWrite[]>([]);
  const [tick, setTick] = useState(0);
  const writes = calendar.writes;
  useEffect(() => {
    if (pending.length === 0) return;
    const now = Date.now();
    const result = reconcile(pending, writes ?? [], calendar.events, now);
    for (const write of result.applied) hooks.remember(write);
    // A write that finished is no longer «on its way»: its line goes.
    if (result.applied.length > 0)
      hooks.setNotice((current) => (current?.kind === "info" ? null : current));
    const failure = result.failed.at(-1);
    if (failure) hooks.setNotice(() => failureNotice(failure.record));
    // The head never said: read the agenda again to see what is there. The
    // write itself is never sent again.
    if (result.failed.some((item) => item.record === null)) hooks.reread();
    if (result.pending.length !== pending.length || result.applied.length > 0)
      setPending(result.pending);
    const due = nextDeadline(result.pending, writes ?? []);
    if (due === null) return;
    const timer = window.setTimeout(() => setTick((value) => value + 1), Math.max(0, due - now));
    return () => window.clearTimeout(timer);
  }, [writes, calendar.events, pending, timezone, tick]);
  useEffect(() => {
    // A write Orbit refused before it was queued has no outcome in the head:
    // the refusal comes back as the command's error, with its numbers.
    if (writeError === null) return;
    hooks.setNotice(() => ({ kind: "refused", text: writeError }));
    setPending((current) => {
      const last = current.at(-1);
      const known = (writes ?? []).some((item) => item.request_id === last?.requestId);
      return last && !known && last.appliedAtMs === undefined ? current.slice(0, -1) : current;
    });
  }, [writeError]);
  return [pending, setPending];
}
