import { useCallback } from "react";
import type { AppCommand } from "../bridge/generated";
import type { OrbitState } from "../state/orbit-store";
import type { ViewId } from "../state/ui";

/** Sends a command tagged with the view whose error slot receives its refusal. */
export function useScopedSend(send: OrbitState["send"], view: ViewId) {
  const fromView = useCallback((command: AppCommand) => send(command, view), [send, view]);
  const fromOnboarding = useCallback((command: AppCommand) => send(command, "onboarding"), [send]);
  const fromRecording = useCallback((command: AppCommand) => send(command, "recording"), [send]);
  return { fromView, fromOnboarding, fromRecording };
}
