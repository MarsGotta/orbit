import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../bridge/generated";
import { HUB_CONFIRMATION_MS } from "./ceilings";
import { useHeldChoice } from "./use-held-choice";

const order: AppCommand = { command: "call_preference", preference: "record" };

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});

afterEach(() => {
  vi.useRealTimers();
});

function held(confirmed: string, send: (command: AppCommand) => unknown) {
  return renderHook(({ value }) => useHeldChoice(value, send), {
    initialProps: { value: confirmed },
  });
}

describe("a choice the person made, until it is confirmed", () => {
  it("is shown at once, and gives way to what is confirmed when that changes", async () => {
    const hook = held("notify", async () => true);
    act(() => hook.result.current.choose("record", order));
    expect(hook.result.current).toMatchObject({ shown: "record", pending: true });
    await act(async () => undefined);
    // Accepted, not confirmed yet: still the person's choice.
    expect(hook.result.current).toMatchObject({ shown: "record", pending: true });
    hook.rerender({ value: "record" });
    expect(hook.result.current).toMatchObject({ shown: "record", pending: false });
  });

  it("goes back to what is known when its order is refused", async () => {
    const hook = held("notify", async () => false);
    act(() => hook.result.current.choose("record", order));
    await act(async () => undefined);
    expect(hook.result.current).toMatchObject({ shown: "notify", pending: false });
  });

  it("goes back to what is known when nothing confirms it within its ceiling", async () => {
    const hook = held("notify", async () => true);
    act(() => hook.result.current.choose("record", order));
    await act(async () => vi.advanceTimersByTimeAsync(HUB_CONFIRMATION_MS - 1));
    expect(hook.result.current.shown).toBe("record");
    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(hook.result.current).toMatchObject({ shown: "notify", pending: false });
  });

  it("another confirmed value also ends it: what is known wins", async () => {
    const hook = held("notify", async () => true);
    act(() => hook.result.current.choose("record", order));
    hook.rerender({ value: "nothing" });
    expect(hook.result.current).toMatchObject({ shown: "nothing", pending: false });
  });
});
