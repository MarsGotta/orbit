import { useState } from "react";
import type { MeetingSnapshot } from "../bridge/generated/MeetingSnapshot";

interface Pauses {
  recording: string | null;
  /** When the pause this window saw began, or null while it records. */
  since: number | null;
  /** The pauses this window saw end, in milliseconds. */
  total: number;
}

function pausesAt(meeting: MeetingSnapshot, now: number): Pauses {
  return {
    recording: meeting.recording_id,
    since: meeting.phase === "paused" ? now : null,
    total: 0,
  };
}

/**
 * The time recorded, as the card's clock shows it: it stops in a pause and
 * goes on from there. The daemon publishes only when the recording began, so
 * the pauses are the ones this window saw (it lives as long as Orbit: closing
 * it only hides it); a pause before Orbit opened counts as recorded time, as
 * it counts towards the recording's limit.
 */
export function useRecordedTime(meeting: MeetingSnapshot, now: number): number {
  const [pauses, setPauses] = useState(() => pausesAt(meeting, now));
  let next = pauses;
  if (pauses.recording !== meeting.recording_id) next = pausesAt(meeting, now);
  else if (meeting.phase === "paused" && pauses.since === null) next = { ...pauses, since: now };
  else if (meeting.phase !== "paused" && pauses.since !== null)
    next = { ...pauses, since: null, total: pauses.total + Math.max(0, now - pauses.since) };
  if (next !== pauses) setPauses(next);
  const start = meeting.started_at_ms ?? now;
  return Math.max(0, (next.since ?? now) - start - next.total);
}
