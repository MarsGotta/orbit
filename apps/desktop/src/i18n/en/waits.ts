import { numberFormat } from "../locale";
import type { FeatureId } from "../../bridge/generated";

/** What a feature is called when its first state does not arrive. */
const featureNames: Record<FeatureId, string> = {
  status: "Orbit’s status",
  permissions: "this Mac’s permissions",
  agenda: "your calendar",
  notes: "your notes",
  note: "the note",
  recording: "the recording status",
  assistant: "the assistant",
  control: "the assistant’s activity on this Mac",
  briefing: "Today",
  local: "this Mac’s settings",
};

/** Whole seconds, as the screen says a duration («13 s»). */
function seconds(ms: number): number {
  return Math.max(1, Math.round(ms / 1_000));
}

/** A wait that went past its ceiling, said with its numbers; a first read on its way. */
export const waits = {
  readingConversation: "Reading conversation…",
  exceeded: (ceilingMs: number, code: string) =>
    `Couldn’t finish in time: Orbit didn’t respond within ${numberFormat().format(seconds(ceilingMs))} s. Try again. If it happens again, share code ${code} with whoever manages your hub.`,
  firstState: (feature: FeatureId, ceilingMs: number) =>
    `Couldn’t read ${featureNames[feature]} within ${numberFormat().format(seconds(ceilingMs))} s. Close and reopen Orbit.`,
};
