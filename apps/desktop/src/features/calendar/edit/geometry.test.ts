import { describe, expect, it } from "vitest";
import { TEST_NOW_MS } from "../../../test-clock";
import { dayOf, startOfDay, timeLabel } from "../date";
import {
  allDaySpan,
  atWall,
  clickSlot,
  coveredDays,
  dragSlot,
  minutesAtY,
  movedSpan,
  resizedEnd,
  shiftedDays,
} from "./geometry";

const MADRID = "Europe/Madrid";
const UTC = "UTC";
/** The suite's pinned day: no clock change in either zone. */
const DAY = dayOf(TEST_NOW_MS, UTC);
const at = (hour: number, minute = 0) => atWall(DAY, hour * 60 + minute, UTC);
const labels = (span: { endMs: number; startMs: number }, zone = UTC) =>
  `${timeLabel(span.startMs, zone)}-${timeLabel(span.endMs, zone)}`;

describe("P9-22 A1 clic en un hueco", () => {
  it("empieza en el cuarto de hora de abajo y dura 30 min", () => {
    expect(labels(clickSlot(DAY, 9 * 60 + 22, UTC, []))).toBe("9:15-9:45");
    expect(labels(clickSlot(DAY, 9 * 60, UTC, []))).toBe("9:00-9:30");
  });

  it("se recorta hasta el siguiente evento del día, nunca bajo 15 min", () => {
    expect(labels(clickSlot(DAY, 9 * 60 + 1, UTC, [at(9, 20)]))).toBe("9:00-9:20");
    expect(labels(clickSlot(DAY, 9 * 60 + 1, UTC, [at(9, 5)]))).toBe("9:00-9:15");
    // An event that already started does not cut the new one.
    expect(labels(clickSlot(DAY, 9 * 60 + 1, UTC, [at(8, 30), at(10)]))).toBe("9:00-9:30");
  });

  it("no pasa de medianoche", () => {
    const late = clickSlot(DAY, 23 * 60 + 50, UTC, []);
    expect(labels(late)).toBe("23:45-0:00");
    expect(late.endMs).toBe(startOfDay(DAY + 1, UTC));
  });

  it("en Madrid pone las 9:00 a las 9:00 del reloj de pared", () => {
    expect(labels(clickSlot(DAY, 9 * 60, MADRID, []), MADRID)).toBe("9:00-9:30");
  });
});

describe("P9-22 A1 arrastrar en la rejilla", () => {
  it("redondea cada extremo al cuarto de hora más cercano", () => {
    expect(labels(dragSlot(DAY, 9 * 60 + 8, 10 * 60 + 52, UTC))).toBe("9:15-10:45");
    expect(labels(dragSlot(DAY, 11 * 60, 10 * 60 + 7, UTC))).toBe("10:00-11:00");
  });

  it("dura 15 min como mínimo y no sale del día", () => {
    expect(labels(dragSlot(DAY, 9 * 60 + 1, 9 * 60 + 4, UTC))).toBe("9:00-9:15");
    expect(labels(dragSlot(DAY, 23 * 60 + 58, 24 * 60, UTC))).toBe("23:45-0:00");
  });

  it("lee los minutos de la posición en píxeles", () => {
    expect(minutesAtY(64 * 9.5, 64)).toBe(9 * 60 + 30);
    expect(minutesAtY(-20, 64)).toBe(0);
    expect(minutesAtY(64 * 30, 64)).toBe(24 * 60);
  });
});

describe("P9-22 A2 mover y estirar", () => {
  const event = { endMs: at(10, 0), startMs: at(9, 5) };

  it("mover deja el inicio en un cuarto de hora y conserva la duración", () => {
    const moved = movedSpan(event, 0, 52, UTC);
    expect(labels(moved)).toBe("10:00-10:55");
    expect(moved.endMs - moved.startMs).toBe(event.endMs - event.startMs);
  });

  it("mover a otro día cambia de columna", () => {
    const moved = movedSpan({ endMs: at(9, 30), startMs: at(9) }, 2, 0, UTC);
    expect(dayOf(moved.startMs, UTC)).toBe(DAY + 2);
    expect(labels(moved)).toBe("9:00-9:30");
  });

  it("estirar el borde inferior va al cuarto más cercano, 15 min como mínimo", () => {
    expect(labels(resizedEnd({ endMs: at(9, 30), startMs: at(9) }, DAY, 11 * 60 + 6, UTC))).toBe(
      "9:00-11:00",
    );
    expect(labels(resizedEnd({ endMs: at(9, 30), startMs: at(9) }, DAY, 8 * 60, UTC))).toBe(
      "9:00-9:15",
    );
  });
});

describe("P9-22 A1 día completo por días", () => {
  it("de la medianoche del primer día al último segundo del último", () => {
    const span = allDaySpan(DAY + 2, DAY, MADRID);
    expect(span.startMs).toBe(startOfDay(DAY, MADRID));
    expect(span.endMs).toBe(startOfDay(DAY + 3, MADRID) - 1_000);
    expect(coveredDays(span, MADRID)).toEqual({ first: DAY, last: DAY + 2 });
  });

  it("una barra se mueve por días y se estira por los lados", () => {
    const bar = allDaySpan(DAY, DAY + 1, UTC);
    expect(coveredDays(shiftedDays(bar, { delta: 3, kind: "move" }, UTC), UTC)).toEqual({
      first: DAY + 3,
      last: DAY + 4,
    });
    expect(coveredDays(shiftedDays(bar, { day: DAY - 2, kind: "start" }, UTC), UTC)).toEqual({
      first: DAY - 2,
      last: DAY + 1,
    });
    expect(coveredDays(shiftedDays(bar, { day: DAY - 5, kind: "end" }, UTC), UTC)).toEqual({
      first: DAY,
      last: DAY,
    });
  });

  it("lee igual un fin a medianoche exclusiva", () => {
    expect(
      coveredDays({ endMs: startOfDay(DAY + 1, UTC), startMs: startOfDay(DAY, UTC) }, UTC),
    ).toEqual({ first: DAY, last: DAY });
  });
});
