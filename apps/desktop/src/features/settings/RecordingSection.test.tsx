import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppView } from "../../state/app-view";
import { RecordingSection } from "./RecordingSection";

afterEach(cleanup);

/** «Cuánto dura cada cosa» says each length as orbit-core gives it. */
it("says how long a recording and a call, however it began, last at most", () => {
  const snapshot = {
    meeting: { call_preference: "notify", limit_minutes: 240 },
    settings: { call_minutes: 240 },
  } as unknown as AppView;
  render(<RecordingSection send={vi.fn()} snapshot={snapshot} />);
  const group = screen.getByRole("region", { name: "Cuánto dura cada cosa" });
  expect(group.textContent).toContain("Una grabación, la empiece quien la empiecehasta 4 h");
  expect(group.textContent).toContain("Una llamada, desde la app o con ⌃⌥Dhasta 4 h");
  expect(group.textContent).not.toContain("15 min");
});
