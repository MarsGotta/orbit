import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import { usePrivateNotesDraft } from "./private-notes-draft";

const RECORDING = "recording-private-notes";

function saves(send: ReturnType<typeof vi.fn>): string[] {
  return send.mock.calls
    .map(([command]) => command as AppCommand)
    .filter((command) => command.command === "note_private_save")
    .map((command) => (command.command === "note_private_save" ? command.markdown : ""));
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

// The box is now the rich editor; these two drive the draft the
// editor reports to, with the same keystrokes and the same expectations.
it("an echo of an earlier save does not replace what Alina typed since", async () => {
  const send = vi.fn();
  const view = renderHook(({ stored }) => usePrivateNotesDraft(RECORDING, stored, send), {
    initialProps: { stored: "a" },
  });
  act(() => view.result.current[1]("ab"));
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_000);
  });
  expect(saves(send)).toEqual(["ab"]);
  act(() => view.result.current[1]("abc"));
  view.rerender({ stored: "ab" });
  expect(view.result.current[0]).toBe("abc");
  view.rerender({ stored: "a" });
  expect(view.result.current[0]).toBe("abc");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_000);
  });
  expect(saves(send)).toEqual(["ab", "abc"]);
});

it("leaving the note sends what was still waiting to be saved", () => {
  const send = vi.fn();
  const view = renderHook(() => usePrivateNotesDraft(RECORDING, "", send));
  act(() => view.result.current[1]("sin perder"));
  view.unmount();
  expect(saves(send)).toEqual(["sin perder"]);
});
