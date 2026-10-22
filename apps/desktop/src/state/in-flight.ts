/**
 * What is in flight: the registry the loading states read.
 *
 * Every wait the interface has is one entry, opened when it begins and closed
 * when the screen shows its result, or when it fails, or past its ceiling
 * (`ceilings.ts`), which turns it into a failure with its numbers through the
 * interceptor. There are three kinds:
 *
 * - an order (`send`): open when it leaves; once its receipt is back, until
 *   the screen has taken the frames up to the receipt's revision. The app
 *   numbers frames with one counter across features and publishes before it
 *   answers, so the frames the order caused carry that revision or an earlier
 *   one, whatever feature they belong to;
 * - a direct call to the app outside `send` (attachments, conversation reads,
 *   an agent's configuration): open until it settles;
 * - a feature's first state: open while the feature says it has none yet.
 *
 * The selectors below are the whole reading API; `use-in-flight.ts` wraps them
 * as hooks.
 */
import type { FeatureFrame, FeatureId, FrameRevision } from "../bridge/generated";
import type { FeatureAvailability } from "../bridge/generated/FeatureAvailability";
import type { ErrorOrigin } from "../app/interceptor";

export type WaitKind = "order" | "call" | "first_state";

export interface Wait {
  op: string;
  kind: WaitKind;
  feature: FeatureId;
  /** The order's or the direct call's name; none for a first state. */
  command?: string;
  /** The view, overlay or panel it was sent from, where its failure is said. */
  origin?: ErrorOrigin;
  /** By the view's clock. */
  startedMs: number;
  ceilingMs: number;
  /** Once an order's receipt is back: the revision the screen must reach. */
  awaiting?: FrameRevision;
}

export interface WaitFilter {
  feature?: FeatureId;
  command?: string;
  origin?: ErrorOrigin;
}

/** The waits that match every field `filter` names; all of them without one. */
export function inFlight(waits: readonly Wait[], filter: WaitFilter = {}): Wait[] {
  return waits.filter(
    (wait) =>
      (filter.feature === undefined || wait.feature === filter.feature) &&
      (filter.command === undefined || wait.command === filter.command) &&
      (filter.origin === undefined || wait.origin === filter.origin),
  );
}

/** Which part of a wait has gone on too long: what its failure names. */
export function stageOf(wait: Wait): "receipt" | "frame" | "call" | "first_state" {
  if (wait.kind === "order") return wait.awaiting ? "frame" : "receipt";
  return wait.kind;
}

/** Whether the screen, having taken frames up to `seen`, shows `target`. */
export function reached(seen: FrameRevision | undefined, target: FrameRevision): boolean {
  if (seen === undefined) return false;
  return (
    seen.epoch > target.epoch || (seen.epoch === target.epoch && seen.counter >= target.counter)
  );
}

/** The later of two revisions. */
export function later(seen: FrameRevision | undefined, next: FrameRevision): FrameRevision {
  return seen !== undefined && reached(seen, next) ? seen : next;
}

/**
 * What a feature says about its own state: on the frame, or for the features
 * that carry it inside their view, there. `undefined` for a feature that
 * does not say (it always has a state once its frame arrived).
 */
export function availabilityOf(frame: FeatureFrame | undefined): FeatureAvailability | undefined {
  if (frame === undefined) return undefined;
  if (frame.availability) return frame.availability;
  switch (frame.feature) {
    case "status":
    case "permissions":
    case "recording":
      return frame.view.availability;
    case "assistant":
      return frame.view.availability;
    case "control":
      return frame.view?.availability;
    case "agenda":
    case "notes":
    case "note":
    case "briefing":
    case "local":
      return undefined;
  }
}

/**
 * Whether a feature has no state yet: its frame has not arrived, or it says
 * its state is unknown. What a skeleton is drawn from; never a «no».
 */
export function hasNoStateYet(
  features: Partial<Record<FeatureId, FeatureFrame>>,
  feature: FeatureId,
): boolean {
  const frame = features[feature];
  return frame === undefined || availabilityOf(frame)?.state === "unknown";
}
