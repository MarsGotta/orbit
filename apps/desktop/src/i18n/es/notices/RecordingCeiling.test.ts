import { expect, it } from "vitest";
import type { Notice } from "../../../bridge/generated/Notice";
import { receive } from "../../../app/interceptor";

function said(code: Notice["code"], data: Notice["data"]) {
  const notice = { code, op: "recording-ceiling", sequence: 1, data } as Notice;
  return receive([], { kind: "notice", notice }).messages;
}

it("warns ten minutes before the ceiling with what is left, recorded and allowed", () => {
  expect(said("recording_ceiling_near", { remaining: 10, minutes: 230, max: 240 })).toEqual([
    {
      op: "recording-ceiling",
      class: "fixable",
      text: "La grabación se parará en 10 min: lleva 3 h 50 min, el máximo es 4 h. Si la reunión sigue, termínala y empieza otra.",
    },
  ]);
});

it("warns ten minutes before a call's ceiling, the recordings' own, as a recording does", () => {
  expect(said("call_ceiling_near", { remaining: 10, minutes: 230, max: 240 })).toEqual([
    {
      op: "recording-ceiling",
      class: "fixable",
      text: "La llamada se colgará en 10 min: lleva 3 h 50 min, el máximo es 4 h. Si quieres seguir, empieza otra.",
    },
  ]);
});

it("says the ceiling it reached in hours", () => {
  expect(said("recording_ceiling_reached", { minutes: 240, fragments: true })).toEqual([
    {
      op: "recording-ceiling",
      class: "fixable",
      text: "La grabación llegó a su máximo de 4 h. Empieza otra si la reunión sigue.",
    },
  ]);
});
