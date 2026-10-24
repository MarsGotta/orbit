import { expect, it } from "vitest";
import type { Notice } from "../../../bridge/generated/Notice";
import { receive } from "../../../app/interceptor";

function said(code: Notice["code"], data: Notice["data"] = {}) {
  const notice = { code, op: "meeting-end", sequence: 1, data } as Notice;
  return receive([], { kind: "notice", notice }).messages;
}

// A meeting that ends is said once, by its own notice («Reunión guardada»,
// then «Tu nota está lista»: MeetingNoteNotice); the daemon's codes for
// the same end are only logged, or the corner would say it twice.
it("says the end of a recording only once, through the meeting's notice", () => {
  expect(said("recording_finished")).toEqual([]);
  expect(said("call_recording_finished", { cause: "call_ended" })).toEqual([]);
});
