import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppView } from "../../state/app-view";
import { RecordingSection } from "./RecordingSection";

afterEach(cleanup);

/** An old snapshot cannot restore the retired duration ceiling copy. */
it("never presents retired duration ceilings from an older snapshot", () => {
  const snapshot = {
    meeting: { call_preference: "notify", limit_minutes: 240 },
    settings: { call_minutes: 240 },
  } as unknown as AppView;
  render(<RecordingSection send={vi.fn()} snapshot={snapshot} />);
  const group = screen.queryByRole("region", { name: "Cuánto dura cada cosa" });
  expect(group).toBeNull();
});
