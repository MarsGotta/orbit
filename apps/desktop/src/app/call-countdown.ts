import { es } from "../i18n/es";

// P9-11: the countdown of a recording a call started, once the call ended.
// Shared by the floating panel and the main window's pill.

/** Whole seconds until `endsAtMs`, rounded up and never below zero. */
export function secondsLeft(endsAtMs: number, nowMs: number): number {
  return Math.max(0, Math.ceil((endsAtMs - nowMs) / 1_000));
}

/** The countdown line of a recording the call started, once the call ended. */
export function endingLabel(endsAtMs: number, nowMs: number): string {
  return es.callDetection.ending(secondsLeft(endsAtMs, nowMs));
}
