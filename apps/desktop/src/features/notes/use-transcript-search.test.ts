import { act, renderHook } from "@testing-library/react";
import { expect, it } from "vitest";
import { useTranscriptSearch } from "./use-transcript-search";

it("distributes matches by turn once and keeps reading order when moving between results", () => {
  const texts = ["Árbol árbol", "no match", "árbol"];
  const { result } = renderHook(() => useTranscriptSearch(texts));
  act(() => result.current.setQuery("arbol"));
  const grouped = result.current.bySegment;
  expect(grouped.get(0)).toEqual([
    { segment: 0, start: 0, end: 5 },
    { segment: 0, start: 6, end: 11 },
  ]);
  expect(grouped.get(1)).toBeUndefined();
  expect(grouped.get(2)).toEqual([{ segment: 2, start: 0, end: 5 }]);
  expect(result.current.total).toBe(3);
  act(() => result.current.move(-1));
  expect(result.current.focused).toEqual({ segment: 2, start: 0, end: 5 });
  expect(result.current.bySegment).toBe(grouped);
  act(() => result.current.setQuery("no"));
  expect(result.current.bySegment.get(0)).toBeUndefined();
  expect(result.current.bySegment.get(1)).toEqual([{ segment: 1, start: 0, end: 2 }]);
});
