import type { AppView as AppSnapshot } from "../state/app-view";
import type { AppCommand } from "../bridge/generated";
import type { DaemonPort } from "../bridge/port";
import { OnboardingView } from "../features/onboarding/OnboardingView";

export type OnboardingRequest = "full" | "pair" | null;

export interface OnboardingProps {
  /** The assistants' call to the hub: signing in and the primary. */
  agent?: DaemonPort["agent"];
  error: string | null;
  onDone: () => void;
  requested: OnboardingRequest;
  send: (command: AppCommand) => Promise<unknown>;
  snapshot: AppSnapshot;
}

/** The first-run wizard, and the re-pairing shortcut Settings reopens it with. */
export function Onboarding({ agent, error, onDone, requested, send, snapshot }: OnboardingProps) {
  return (
    <OnboardingView
      agent={agent}
      error={error}
      forcePair={requested === "pair"}
      onComplete={onDone}
      send={send}
      snapshot={snapshot}
      startAtPair={requested === "pair"}
    />
  );
}
