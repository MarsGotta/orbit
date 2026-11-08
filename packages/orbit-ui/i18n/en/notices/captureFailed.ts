import { pluralRules } from "../../locale";
import { numberFormat } from "../../locale";
import type { Entries, NoticeData } from "./types";

/**
 * Why a capture was cut, by the code the daemon gives it: a stop cause's
 * name or the helper's own code (`privacy_stop.rs`). One this version does not
 * know is still named, by its code.
 */
const cutCauses: Record<string, string> = {
  helper_lost: "Orbit’s audio helper stopped responding",
  helper_disconnected: "Orbit’s audio helper stopped responding",
  daemon_heartbeat_expired: "Orbit took too long to respond to the audio helper",
  helper_capture_stop: "the audio helper stopped capture",
  helper_integrity_failure: "audio from the helper arrived damaged",
  capture_event_failed: "audio continuity was lost",
  session_inactive: "the Mac locked or switched users",
  audio_device_changed: "the audio device changed",
  voice_processing_state_lost: "macOS stopped processing microphone audio",
  system_audio_stream_stopped: "macOS stopped the other app’s audio",
};

function cause(data: NoticeData): string {
  const code = data.cause;
  if (typeof code !== "string" || code === "") return "audio capture failed";
  return cutCauses[code] ?? `audio capture failed (code ${code})`;
}

function fragments(count: unknown): string | null {
  if (typeof count !== "number" || count <= 0) return null;
  return pluralRules().select(count) === "one"
    ? "1 fragment"
    : `${numberFormat().format(count)} fragments`;
}

/** What was kept of a recording, and what to do while the meeting goes on. */
function kept(count: unknown): string {
  const saved = fragments(count);
  const what = saved === null ? "" : ` (${saved})`;
  return `Audio captured so far is saved on this Mac${what}. Start another recording if the meeting is continuing.`;
}

function cutText(data: NoticeData): string {
  const why = cause(data);
  if (data.recording === true)
    return `Recording was interrupted because ${why}. ${kept(data.fragments)}`;
  if (data.call === true) return `The call was interrupted because ${why}.`;
  if (data.voice_note === true) return `The voice note was interrupted because ${why}.`;
  return `Audio capture was interrupted because ${why}.`;
}

function count(value: unknown): number {
  return typeof value === "number" && value > 0 ? value : 0;
}

function lostFilesText(data: NoticeData): string {
  const damaged = count(data.invalid);
  const missing = count(data.missing);
  const unfinished = count(data.incomplete);
  const lost = damaged + missing + unfinished;
  const parts = [
    damaged > 0 ? `${numberFormat().format(damaged)} damaged` : null,
    missing > 0 ? `${numberFormat().format(missing)} missing` : null,
    unfinished > 0 ? `${numberFormat().format(unfinished)} partially written` : null,
  ].filter((part) => part !== null);
  const files =
    pluralRules().select(lost) === "one"
      ? "1 audio file"
      : `${numberFormat().format(lost)} audio files`;
  const which = parts.length > 0 ? ` (${parts.join(", ")})` : "";
  return `At startup, Orbit couldn’t recover ${files} from this Mac${which}; it recovered ${numberFormat().format(count(data.recovered))}.`;
}

/** Why a seal failed, by the cause the daemon gives it (`seal_cause.rs`). */
const sealCauses: Record<string, string> = {
  disk_full: "this Mac’s disk is full",
  not_writable: "macOS won’t let Orbit write to its folder",
  disk_error: "the disk returned a write error",
  store_busy: "Orbit’s store was busy",
  store_error: "Orbit’s store returned an error",
  integrity: "Orbit’s encrypted store couldn’t be verified",
  destination_invalid: "the audio destination on this Mac isn’t a regular folder",
  reserve_exhausted: "the audio memory reserve filled up",
  writer_unavailable: "Orbit’s store couldn’t accept more work",
  writer_panicked: "Orbit’s storage writer failed to save it and restarted",
};

/** What to do about a cause the person can act on. */
const sealActions: Record<string, string> = {
  disk_full: " Free up disk space.",
  not_writable: " Check permissions on Orbit’s data folder.",
};

function sealText(data: NoticeData): string {
  const code = data.cause;
  if (typeof code !== "string" || code === "")
    return "Some recording audio couldn’t be written to this Mac. If the disk is full, free up space. Orbit will retry while it stays open.";
  const why = sealCauses[code] ?? `the store failed (code ${code})`;
  const owed = fragments(data.fragments);
  const waiting = owed === null ? "" : ` ${owed} of audio is waiting in Orbit’s memory.`;
  return `Some recording audio couldn’t be written to this Mac because ${why}.${waiting} Orbit will retry while it stays open. Don’t close it.${sealActions[code] ?? ""}`;
}

function closeUnconfirmedText(data: NoticeData): string {
  const kept =
    data.fragments === true
      ? "Audio captured so far is saved on this Mac. Orbit closes the recording with it, and the hub writes the note."
      : "No audio from this recording was saved.";
  return `The audio helper didn’t confirm the end of recording in time. ${kept} No action is needed.`;
}

function startRefusedText(data: NoticeData): string {
  const code = typeof data.code === "string" && data.code !== "" ? ` (code ${data.code})` : "";
  return `The audio helper couldn’t open the microphone for the call${code}, so none of it was recorded. Call again; if it happens again, check Orbit’s microphone access in System Settings.`;
}

function closedText(data: NoticeData): string {
  if (data.audio_unsaved !== true)
    return "Orbit closed with work pending. Anything already saved on this Mac will resume when it opens.";
  const why = data.reserve_exhausted === true ? " because the memory reserve ran out" : "";
  return `Orbit closed before writing some audio from the last recording to this Mac${why}. That audio was lost.`;
}

/**
 * What could not be recorded or saved, in words of its own: what happened,
 * its cause when the notice carries one, and what to do. Never the generic
 * failure with only a code to share.
 */
export const captureFailed: Entries<
  | "capture_cut"
  | "recording_seal_failed"
  | "recording_close_unconfirmed"
  | "audio_start_refused"
  | "recording_interrupted_by_restart"
  | "store_recovery_lost_files"
  | "closed_with_pending_work"
> = {
  capture_cut: { class: "failed", text: cutText },
  recording_seal_failed: { class: "failed", text: sealText },
  recording_close_unconfirmed: { class: "failed", text: closeUnconfirmedText },
  audio_start_refused: { class: "failed", text: startRefusedText },
  recording_interrupted_by_restart: {
    class: "failed",
    text: (data) => `Orbit closed during a recording. ${kept(data.fragments)}`,
  },
  store_recovery_lost_files: { class: "failed", text: lostFilesText },
  closed_with_pending_work: { class: "failed", text: closedText },
};
