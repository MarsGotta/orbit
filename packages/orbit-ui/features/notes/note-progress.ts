import { getCopy } from "../../i18n";
import type { NoteProgressSnapshot } from "../../bridge/generated/NoteProgressSnapshot";
import type { NoteStepSnapshot } from "../../bridge/generated/NoteStepSnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import type { AppCommand } from "../../bridge/generated";

export type StepKey = keyof NoteProgressSnapshot;
export const STEP_KEYS: StepKey[] = ["upload", "transcript", "summary"];

/** The user is recording this meeting now. */
export function isRecording(progress: NoteProgressSnapshot): boolean {
  return progress.upload.cause?.kind === "recording";
}

/** «Audio guardado», «Transcribiendo…», «Escribir el resumen»: each step as it stands. */
export function stepLabel(key: StepKey, step: NoteStepSnapshot): string {
  return getCopy().noteScreen.steps[key][step.state];
}

/** The template a note is written with again: its own, its series', or General. */
export function templateOf(note: OpenNoteSnapshot): string {
  if (note.summary) return note.summary.template_id;
  const assigned = note.series?.assigned;
  return (assigned?.state === "known" ? assigned.template_id : null) ?? "general";
}

export interface NoteFailure {
  title: string;
  body: string;
  retry: AppCommand;
}

/**
 * Why a meeting has no note, when something failed: the transcript the hub
 * gave up on, or the note it could not write. Each says what to do and
 * «Reintentar» asks for it again. Codes and counts stay in the log.
 */
export function noteFailure(note: OpenNoteSnapshot): NoteFailure | null {
  const { transcript, summary } = note.progress;
  if (transcript.state === "failed" || note.transcript.failed_groups > 0) {
    return {
      title: getCopy().noteScreen.transcriptFailed,
      body: getCopy().noteScreen.transcriptFailedBody,
      retry: { command: "transcript_retry", recording_id: note.recording_id },
    };
  }
  if (summary.state === "failed") {
    return {
      title: getCopy().noteScreen.noteFailed,
      body:
        summary.cause?.kind === "not_configured"
          ? getCopy().noteScreen.notConfigured
          : getCopy().noteScreen.noteFailedBody,
      retry: {
        command: "note_regenerate",
        recording_id: note.recording_id,
        template_id: templateOf(note),
      },
    };
  }
  return null;
}

/** The note was never asked for: nothing is on its way. */
export function notRequested(note: OpenNoteSnapshot): boolean {
  return note.summary === null && note.progress.summary.cause?.kind === "not_requested";
}

/** «Orbit está preparando tu nota»: after the recording, while a step is left. */
export function isPreparing(note: OpenNoteSnapshot): boolean {
  if (isRecording(note.progress) || noteFailure(note) || notRequested(note)) return false;
  return STEP_KEYS.some((key) => note.progress[key].state !== "done");
}

/** A step waits for the hub to come back. */
export function waitsForHub(progress: NoteProgressSnapshot): boolean {
  return STEP_KEYS.some((key) => progress[key].cause?.kind === "hub_unavailable");
}
