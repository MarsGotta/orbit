import type { Dispatch, SetStateAction } from "react";
import type { AppContentProps } from "./App";
import { Onboarding, type OnboardingRequest } from "./Onboarding";

interface Props {
  done: boolean;
  props: Pick<AppContentProps, "error" | "send" | "snapshot">;
  requested: OnboardingRequest;
  setDone: Dispatch<SetStateAction<boolean>>;
  setRequested: Dispatch<SetStateAction<OnboardingRequest>>;
}

export function AppOnboarding({ done, props, requested, setDone, setRequested }: Props) {
  const visible = requested !== null || (!done && !props.snapshot.settings.onboarding_done);
  if (!visible) return null;
  return (
    <Onboarding
      error={props.error}
      onDone={() => {
        setDone(true);
        setRequested(null);
      }}
      requested={requested}
      send={props.send}
      snapshot={props.snapshot}
    />
  );
}
