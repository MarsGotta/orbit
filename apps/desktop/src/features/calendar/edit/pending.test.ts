import { describe, expect, it } from "vitest";
import { TEST_NOW_MS } from "../../../test-clock";
import type { CalendarEvent } from "../eventLook";
import { changesOf, formOf, inputOf, withStart } from "./form";
import {
  APPLIED_WAIT_MS,
  nextDeadline,
  type PendingWrite,
  QUEUED_WRITE_CEILING_MS,
  reconcile,
  WRITE_LIFETIME_MS,
  withPending,
} from "./pending";
import { inverseChanges } from "./commands";

const HOUR = 3_600_000;
const START = TEST_NOW_MS;
const event: CalendarEvent = {
  calendar_id: "calendar-0",
  calendar_item_id: "item-1",
  end_ms: START + HOUR,
  id: "observation-1",
  occurrence_start_ms: START,
  start_ms: START,
  title: "Taller",
};
const moved: CalendarEvent = { ...event, end_ms: START + 3 * HOUR, start_ms: START + 2 * HOUR };
const write: PendingWrite = {
  changes: { end_ms: moved.end_ms, start_ms: moved.start_ms },
  event,
  op: "update",
  requestId: "move-1",
  sentAtMs: START,
  shown: moved,
  span: "this",
  undoable: true,
};
const record = (stage: "queued" | "saved" | "refused" | "outcome_unknown") => ({
  operation: "update" as const,
  request_id: "move-1",
  stage,
});

describe("reconciling a write with the agenda head", () => {
  it("queued or with no news it stays pending and the card shows in its new place", () => {
    for (const writes of [[], [record("queued")]]) {
      const result = reconcile([write], writes, [event], 0);
      expect(result.pending).toHaveLength(1);
      expect(result.failed).toHaveLength(0);
    }
    const shown = withPending([event], [write]);
    expect(shown).toHaveLength(1);
    expect(shown[0]?.start_ms).toBe(moved.start_ms);
    expect(shown[0]?.ui?.saving).toBe(true);
  });

  it("refused or unknown it ends as a failure, without staying pending", () => {
    for (const stage of ["refused", "outcome_unknown"] as const) {
      const result = reconcile([write], [record(stage)], [event], 0);
      expect(result.pending).toHaveLength(0);
      expect(result.failed[0]?.record?.stage).toBe(stage);
    }
  });

  it("saved it can be undone and stays drawn until the read brings it or the deadline passes", () => {
    const saved = reconcile([write], [record("saved")], [event], 1_000);
    expect(saved.applied).toHaveLength(1);
    expect(saved.pending).toHaveLength(1);
    const read = reconcile(saved.pending, [record("saved")], [moved], 1_100);
    expect(read.pending).toHaveLength(0);
    expect(read.applied).toHaveLength(0);
    const late = reconcile(saved.pending, [record("saved")], [event], 1_000 + APPLIED_WAIT_MS);
    expect(late.pending).toHaveLength(0);
  });
});

describe("a write the agenda head does not mention", () => {
  it("becomes unknown when a daemon write's lifetime runs out, never before", () => {
    const early = reconcile([write], [], [event], START + WRITE_LIFETIME_MS - 1);
    expect(early.pending).toHaveLength(1);
    expect(nextDeadline([write], [])).toBe(START + WRITE_LIFETIME_MS);
    const late = reconcile([write], [], [event], START + WRITE_LIFETIME_MS);
    expect(late.pending).toHaveLength(0);
    expect(late.failed).toEqual([{ record: null, write }]);
  });

  it("when the head names it queued, the view waits as long as the EventKit slot can make it wait", () => {
    const queued = reconcile([write], [record("queued")], [event], START + 60_000);
    expect(queued.pending).toHaveLength(1);
    expect(nextDeadline([write], [record("queued")])).toBe(START + QUEUED_WRITE_CEILING_MS);
  });

  it("a write still queued past that becomes unknown, never sent again", () => {
    const early = reconcile(
      [write],
      [record("queued")],
      [event],
      START + QUEUED_WRITE_CEILING_MS - 1,
    );
    expect(early.pending).toHaveLength(1);
    const late = reconcile([write], [record("queued")], [event], START + QUEUED_WRITE_CEILING_MS);
    expect(late.pending).toHaveLength(0);
    expect(late.failed).toEqual([{ record: null, write }]);
  });
});

describe("the panel accumulates changes", () => {
  it("sends only what changed, and its inverse goes back to what was read", () => {
    const before = formOf({ ...event, location: "Sala 3" });
    const after = { ...withStart(before, START + HOUR), location: "", title: "Taller IA" };
    const changes = changesOf(before, after);
    expect(changes).toEqual({
      end_ms: START + 2 * HOUR,
      location: "",
      start_ms: START + HOUR,
      title: "Taller IA",
    });
    expect(inverseChanges({ ...event, location: "Sala 3" }, changes)).toEqual({
      end_ms: START + HOUR,
      location: "Sala 3",
      start_ms: START,
      title: "Taller",
    });
  });

  it("removing the repetition removes it, and untouched reminders are not written", () => {
    const series = formOf({ ...event, recurrence: { frequency: "weekly", interval: 2 } });
    expect(changesOf(series, { ...series, repeat: { kind: "none" } })).toEqual({
      remove_recurrence: true,
    });
    expect(changesOf(series, series)).toEqual({});
    expect(changesOf(series, { ...series, alarms: [10, 0] })).toEqual({ alarms_minutes: [0, 10] });
  });

  it("a new event carries its zone when it has a time and none when it is all-day", () => {
    const form = { ...formOf(event), availability: "busy" as const };
    expect(inputOf(form, "Europe/Madrid").time_zone).toBe("Europe/Madrid");
    expect(inputOf({ ...form, allDay: true }, "Europe/Madrid").time_zone).toBeUndefined();
  });
});
