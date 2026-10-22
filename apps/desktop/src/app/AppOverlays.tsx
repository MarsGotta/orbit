import { useCallback, type Dispatch, type SetStateAction } from "react";
import type { AppCommand } from "../bridge/generated";
import { MeetingOverlays } from "../features/meetings/MeetingOverlays";
import type { AppContentProps } from "./App";
import { AppNotices } from "./AppNotices";
import { AppOnboarding } from "./AppOnboarding";
import { Palette } from "./Palette";
import type { OnboardingRequest } from "./Onboarding";
import { ViewBoundary } from "./ViewBoundary";
import { messageFor } from "./visible-error";

export function useCommandDispatch(send: AppContentProps["send"]): (command: AppCommand) => void {
  return useCallback(
    (command: AppCommand) => {
      if (command.command === "notes_open") {
        const [recordingId] = command.recording_id.split("?", 1);
        if (recordingId) void send({ ...command, recording_id: recordingId });
        return;
      }
      void send(command);
    },
    [send],
  );
}

export interface OnboardingState {
  done: boolean;
  setDone: Dispatch<SetStateAction<boolean>>;
  requested: OnboardingRequest;
  setRequested: Dispatch<SetStateAction<OnboardingRequest>>;
}

/** The overlays over the window: the new recording, the palette, the first run. */
export function Overlays(props: AppContentProps & { onboarding: OnboardingState }) {
  const recordingMessage = messageFor("recording", props.messages);
  const recordingDispatch = useCommandDispatch(props.sendRecording);
  const { done, setDone, requested, setRequested } = props.onboarding;
  return (
    <>
      <AppNotices {...props} />
      <ViewBoundary onCrash={props.crash}>
        <MeetingOverlays
          error={props.recordingError}
          errorKind={recordingMessage?.class}
          onErrorDismiss={() => recordingMessage && props.dismiss(recordingMessage.op)}
          launcherOpen={props.meetingOpen}
          onLauncherClose={() => props.setMeetingOpen(false)}
          send={recordingDispatch}
          snapshot={props.snapshot}
        />
      </ViewBoundary>
      <Palette {...props.palette} onOpenChange={props.setPaletteOpen} open={props.paletteOpen} />
      <ViewBoundary onCrash={props.crash}>
        <AppOnboarding
          done={done}
          props={{ ...props, send: props.sendOnboarding, error: props.onboardingError }}
          requested={requested}
          setDone={setDone}
          setRequested={setRequested}
        />
      </ViewBoundary>
    </>
  );
}
