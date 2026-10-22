import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import type { RecentMeetingSnapshot } from "../../bridge/generated/RecentMeetingSnapshot";
import type { Views } from "../scenario";
import { DAILY_ID } from "./notes-list";
import { at, MINUTE } from "./time";

const recent: RecentMeetingSnapshot[] = [
  {
    recording_id: DAILY_ID,
    title: "Daily del equipo de producto",
    started_at_ms: at(0, 9, 0),
    duration_ms: 14 * MINUTE,
    calendar_event_id: "daily-0",
    transcription: { pending: 0, running: 0, completed: 6, failed: 0 },
    summary: "ready",
  },
];

export const idleMeeting: MeetingSnapshot = {
  phase: "idle",
  recording_id: null,
  title: null,
  started_at_ms: null,
  ends_at_ms: null,
  limit_minutes: 60,
  audio: "system",
  tracks: [],
  upload: { pending_chunks: 0, offline: false },
  call_panel: null,
  call_preference: "notify",
  ask: {
    windows: [{ minutes: 2 }, { minutes: 5 }, { minutes: 10 }],
    default_minutes: 5,
    max_question_bytes: 1024,
  },
  voice_trigger: { state: "listening" },
  recent,
  applications: [
    { name: "Google Meet", pid: 4121 },
    { name: "zoom.us", pid: 3877 },
  ],
  applications_state: "ready",
};

/** A meeting being recorded: the design review, 18 minutes in. */
export const recordingMeeting: MeetingSnapshot = {
  ...idleMeeting,
  phase: "recording",
  recording_id: "rec-review-21",
  title: "Revisión del diseño de reservas",
  started_at_ms: at(0, 10, 22),
  ends_at_ms: at(0, 11, 45),
  audio: "application",
  tracks: [
    { name: "mic_local", level: "ok", chunks_uploaded: 18, gaps: 0 },
    { name: "remote_app", level: "ok", chunks_uploaded: 18, gaps: 0 },
  ],
};

export const recording: Views["recording"] = {
  availability: { state: "current" },
  call_availability: { state: "current" },
  meeting: idleMeeting,
  call: null,
  freshness: { state: "current" },
};
