import { pluralRules } from "../../locale";
import { numberFormat } from "../../locale";
import { macPermissions } from "../permissions";
import { systemPrivacy, worded, type Entries, NoticeData } from "./types";

/** Why a cut happened, in the person's words; an unknown code is shown as it is. */
const CUT_CAUSES: Record<string, string> = {
  helper_lost: "Orbit’s audio helper closed",
  daemon_heartbeat_expired: "Orbit’s service stopped responding",
  capture_event_failed: "an audio fragment arrived damaged",
  capture_maintenance_failed: "saving audio failed",
  helper_integrity_failure: "the Mac couldn’t guarantee audio continuity",
  audio_stop_unconfirmed: "the Mac didn’t confirm audio stopped",
  capture_stop_undelivered: "an audio stop command didn’t reach the Mac",
  capture_finalize_timed_out: "the Mac didn’t confirm the recording ended",
  audio_device_changed_voice_note: "the audio device changed during a voice note",
  voice_processing_state_lost: "the Mac lost voice processing",
  session_inactive: "the Mac slept or switched users",
  privacy_stop: "the privacy stop",
  restart: "Orbit restarted",
  cause_not_saved: "a cause that wasn’t saved",
};

function cutCause(data: NoticeData): string {
  const code = typeof data.cause === "string" ? data.cause : undefined;
  if (code === undefined) return "an unknown cause";
  return CUT_CAUSES[code] ?? code;
}

function missing(data: NoticeData): string {
  if (typeof data.elapsed_ms !== "number") return "";
  const seconds = Math.round(data.elapsed_ms / 1_000);
  return seconds < 1
    ? "; less than 1 s of audio is missing"
    : `; ${numberFormat().format(seconds)} s of audio is missing`;
}

/** Recording, the Mac's audio and the helper. */
export const capture = {
  recording_resumed_after_cut: {
    class: "done",
    text: (data) =>
      `Recording was interrupted (${cutCause(data)}) and resumed automatically${missing(data)}.`,
  },
  recording_closed_after_cut: {
    class: "fixable",
    text: (data) => {
      if (data.cause === "capture_finalize_timed_out")
        return "Recording ended without confirmation from the Mac. It was closed with the captured audio, and its note will be written.";
      if (data.fragments === 0) {
        const tries =
          typeof data.attempt === "number" && data.attempt > 0
            ? ` after ${numberFormat().format(data.attempt)} ${pluralRules().select(data.attempt) === "one" ? "attempt" : "attempts"}`
            : "";
        return `Recording was interrupted at startup (${cutCause(data)}) and couldn’t resume${tries}. Nothing was recorded. Start another recording if the meeting is continuing.`;
      }
      return `Recording was interrupted (${cutCause(data)}) and couldn’t resume. It was closed with the captured audio, and its note will be written. Start another recording if the meeting is continuing.`;
    },
  },
  audio_route_recovered: {
    class: "done",
    text: (data) =>
      typeof data.elapsed_ms === "number"
        ? `${data.elapsed_ms < 1_000 ? "Less than 1 s" : `${numberFormat().format(Math.round(data.elapsed_ms / 1_000))} s`} of audio was lost when the audio route changed. Recording continues.`
        : "Audio was lost when the audio route changed; the gap’s duration couldn’t be measured. Recording continues.",
  },
  voice_note_cancelled_by_audio_route: worded(
    "fixable",
    "The audio device changed while you were dictating. The voice note was canceled and saved unsent. The meeting is still recording.",
  ),
  track_stopped: {
    class: "fixable",
    text: (data) =>
      data.origin === "mic_local"
        ? "Your microphone stopped recording and couldn’t recover. Meeting audio is still recording."
        : "The other app’s audio stopped recording and couldn’t recover. Your microphone is still recording.",
  },
  recording_discarded: worded("done", "Meeting discarded"),
  recording_ceiling_near: { class: "silent" },
  recording_ceiling_reached: { class: "silent" },
  recording_silence_resolved: { class: "silent" },
  recording_silence_question: {
    class: "working",
    text: () => "Is the meeting still going? I’ll end and save it in 30 s",
    actions: [
      { label: "Keep recording", command: { command: "meeting_keep" } },
      { label: "End", command: { command: "meeting_finish" } },
    ],
  },
  microphone_permission_missing: worded(
    "fixable",
    "Microphone permission is missing. Allow it in System Settings.",
    systemPrivacy(macPermissions.microphone.pane),
  ),
  screen_permission_missing: worded(
    "fixable",
    "Screen Recording permission is missing. Allow it in System Settings.",
    systemPrivacy(macPermissions.screen_recording.pane),
  ),
  meeting_audio_microphone_only: worded(
    "fixable",
    "Only your microphone is recording: no video call app is open.",
  ),
  meeting_audio_ambiguous: worded(
    "fixable",
    "Only your microphone is recording: several video call apps are open. Choose one under “Audio to record”.",
  ),
  helper_not_running: worded("fixable", "Orbit.app isn’t running. Open it to continue."),
  audio_reserve_exhausted: worded(
    "fixable",
    "Some audio was lost due to insufficient memory. Keep Orbit open until saving finishes.",
  ),
} satisfies Partial<Entries>;
