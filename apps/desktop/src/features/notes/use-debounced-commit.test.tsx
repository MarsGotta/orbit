import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useDebouncedCommit } from "./use-debounced-commit";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

it("saves one second after a pause and at most every five seconds", () => {
  const commit = vi.fn();
  const { result, unmount } = renderHook(() => useDebouncedCommit<string>(commit));
  for (let index = 0; index < 12; index += 1) {
    act(() => result.current.schedule(`v${index}`));
    act(() => void vi.advanceTimersByTime(500));
  }
  // Typing every 500 ms never pauses a second, yet five seconds commit once.
  expect(commit.mock.calls.map(([value]) => value)).toEqual(["v9"]);
  act(() => void vi.advanceTimersByTime(1_000));
  expect(commit.mock.calls.map(([value]) => value)).toEqual(["v9", "v11"]);
  act(() => result.current.schedule("last"));
  unmount();
  expect(commit.mock.calls.at(-1)?.[0]).toBe("last");
});
