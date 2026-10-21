import { expect, it } from "vitest";
import { findMatches } from "./transcript-search";

it("P9-04 finds every occurrence without minding accents or case", () => {
  const matches = findMatches(["La Firma está bloqueada", "firmar hoy", "nada"], "firma");
  expect(matches).toEqual([
    { segment: 0, start: 3, end: 8 },
    { segment: 1, start: 0, end: 5 },
  ]);
  expect(findMatches(["Está aquí"], "esta")).toEqual([{ segment: 0, start: 0, end: 4 }]);
  expect(findMatches(["algo"], "   ")).toEqual([]);
});
