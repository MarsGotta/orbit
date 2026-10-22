import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import type { Views } from "../scenario";

/** `views` with the meeting changed, and the briefing saying it records. */
export function withMeeting(views: Views, meeting: MeetingSnapshot): Views {
  const recording = meeting.phase !== "idle";
  const next = views.briefing?.next;
  return {
    ...views,
    recording: { ...views.recording, meeting },
    briefing: views.briefing && {
      ...views.briefing,
      next: next ? { ...next, recording: recording ? "recording" : "none" } : null,
      upcoming: views.briefing.upcoming?.map((item, index) =>
        index === 0 ? { ...item, recording: recording ? "recording" : "none" } : item,
      ),
    },
  };
}
