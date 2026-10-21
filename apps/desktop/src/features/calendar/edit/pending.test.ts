import { describe, expect, it } from "vitest";
import { TEST_NOW_MS } from "../../../test-clock";
import type { CalendarEvent } from "../eventLook";
import { changesOf, formOf, inputOf, withStart } from "./form";
import {
  APPLIED_WAIT_MS,
  nextDeadline,
  type PendingWrite,
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

describe("P9-22 A3 conciliar una escritura con la cabecera", () => {
  it("en cola o sin noticia sigue pendiente y la tarjeta se ve en su sitio nuevo", () => {
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

  it("rechazada o desconocida termina como fallo, sin quedarse pendiente", () => {
    for (const stage of ["refused", "outcome_unknown"] as const) {
      const result = reconcile([write], [record(stage)], [event], 0);
      expect(result.pending).toHaveLength(0);
      expect(result.failed[0]?.record?.stage).toBe(stage);
    }
  });

  it("guardada se puede deshacer y sigue dibujada hasta que la lectura la trae o pasa el plazo", () => {
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

describe("P9-22 revisión 2: una escritura que la cabecera no menciona", () => {
  it("pasa a desconocida al vencer la vida de una escritura del daemon, nunca antes", () => {
    const early = reconcile([write], [], [event], START + WRITE_LIFETIME_MS - 1);
    expect(early.pending).toHaveLength(1);
    expect(nextDeadline([write], [])).toBe(START + WRITE_LIFETIME_MS);
    const late = reconcile([write], [], [event], START + WRITE_LIFETIME_MS);
    expect(late.pending).toHaveLength(0);
    expect(late.failed).toEqual([{ record: null, write }]);
  });

  it("si la cabecera la nombra, el plazo es del daemon y la vista espera", () => {
    const queued = reconcile([write], [record("queued")], [event], START + 60_000);
    expect(queued.pending).toHaveLength(1);
    expect(nextDeadline([write], [record("queued")])).toBeNull();
  });
});

describe("P9-22 el panel acumula cambios", () => {
  it("sólo manda lo que cambió y su inverso vuelve a lo leído", () => {
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

  it("quitar la repetición la quita y los recordatorios sin tocar no se escriben", () => {
    const series = formOf({ ...event, recurrence: { frequency: "weekly", interval: 2 } });
    expect(changesOf(series, { ...series, repeat: { kind: "none" } })).toEqual({
      remove_recurrence: true,
    });
    expect(changesOf(series, series)).toEqual({});
    expect(changesOf(series, { ...series, alarms: [10, 0] })).toEqual({ alarms_minutes: [0, 10] });
  });

  it("un evento nuevo lleva su zona si tiene hora y ninguna si es de día completo", () => {
    const form = { ...formOf(event), availability: "busy" as const };
    expect(inputOf(form, "Europe/Madrid").time_zone).toBe("Europe/Madrid");
    expect(inputOf({ ...form, allDay: true }, "Europe/Madrid").time_zone).toBeUndefined();
  });
});
