import { getCopy } from "../i18n";

// The countdown of a recording a call started, once the call ended.
// Shared by the floating panel and the recording card of the sidebar.

/** Whole seconds until `endsAtMs`, rounded up and never below zero. */
export function secondsLeft(endsAtMs: number, nowMs: number): number {
  return Math.max(0, Math.ceil((endsAtMs - nowMs) / 1_000));
}

/** The countdown line of a recording the call started, once the call ended. */
export function endingLabel(endsAtMs: number, nowMs: number): string {
  return getCopy().callDetection.ending(secondsLeft(endsAtMs, nowMs));
}
