import type { NoteProgressSnapshot } from "../../bridge/generated/NoteProgressSnapshot";
import type { NoteStepCause } from "../../bridge/generated/NoteStepCause";
import { es } from "../../i18n/es";

const causes = es.noteScreen.causes;

/**
 * One step's cause as she can read it (SIM-24): where it stands, or what to
 * do when it failed. A count not observed yet says nothing (`null`); the
 * hub's codes, HTTP statuses and retry times stay in the log.
 */
export function causeText(cause: NoteStepCause): string | null {
  switch (cause.kind) {
    case "recording":
      return causes.recording;
    case "uploading":
      return causes.uploading;
    case "transcribing":
      return causes.transcribing;
    case "transcript_failed":
      return causes.transcriptFailed;
    case "after_recording":
      return causes.afterRecording;
    case "not_requested":
      return causes.notRequested;
    case "waiting_transcript":
      return causes.waitingTranscript;
    case "writing":
      return causes.writing;
    case "retrying":
      return causes.retrying;
    case "hub_unavailable":
      return causes.hubUnavailable;
    case "not_configured":
    case "failed":
      return causes.failed;
  }
}

export type StepKey = keyof NoteProgressSnapshot;
export const STEP_KEYS: StepKey[] = ["upload", "transcript", "summary"];

/** The pill shows while any step is not done. */
export function progressVisible(progress: NoteProgressSnapshot): boolean {
  return STEP_KEYS.some((key) => progress[key].state !== "done");
}

/** Mars is recording this meeting now. */
export function isRecording(progress: NoteProgressSnapshot): boolean {
  return progress.upload.cause?.kind === "recording";
}
