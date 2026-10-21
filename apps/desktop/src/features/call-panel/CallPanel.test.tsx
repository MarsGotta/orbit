import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { CallPanelSnapshot } from "../../bridge/generated/CallPanelSnapshot";
import { es } from "../../i18n/es";
import { TEST_NOW_MS } from "../../test-clock";
import { secondsLeft } from "../../app/call-countdown";
import { CallPanel } from "./CallPanel";

afterEach(() => {
  vi.useRealTimers();
});

function renderPanel(panel: CallPanelSnapshot | null) {
  const sent: AppCommand[] = [];
  const view = render(<CallPanel panel={panel} send={(command) => sent.push(command)} />);
  return { sent, view };
}

describe("P9-11 the floating panel", () => {
  it("offers to record a call, with its event, and each answer names the offer", async () => {
    const { sent } = renderPanel({
      state: "offer",
      call_id: "call-1",
      application: "zoom.us",
      event_title: "Revisión semanal",
    });
    const panel = screen.getByRole("region", { name: es.callDetection.panelLabel });
    expect(panel.textContent).toContain(es.callDetection.offer("zoom.us"));
    expect(panel.textContent).toContain("Revisión semanal");
    await userEvent.click(screen.getByRole("button", { name: es.callDetection.record }));
    await userEvent.click(screen.getByRole("button", { name: es.callDetection.notNow }));
    expect(sent).toEqual([
      { command: "call_record", call_id: "call-1" },
      { command: "call_dismiss", call_id: "call-1" },
    ]);
  });

  it("says a recording started on its own and stops it by hand", async () => {
    const { sent } = renderPanel({
      state: "recording",
      recording_id: "recording-1",
      application: "zoom.us",
    });
    expect(screen.getByText(es.callDetection.recording("zoom.us"))).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: es.callDetection.stop }));
    expect(sent).toEqual([{ command: "meeting_finish" }]);
  });

  it("counts down to the end the daemon published and keeps recording on request", () => {
    vi.useFakeTimers();
    vi.setSystemTime(TEST_NOW_MS);
    const sent: AppCommand[] = [];
    render(
      <CallPanel
        panel={{
          state: "ending",
          recording_id: "recording-1",
          application: "zoom.us",
          ends_at_ms: TEST_NOW_MS + 13_000,
        }}
        send={(command) => sent.push(command)}
      />,
    );
    expect(screen.getByText(es.callDetection.ending(13))).toBeTruthy();
    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByText(es.callDetection.ending(12))).toBeTruthy();
    act(() => vi.advanceTimersByTime(20_000));
    expect(screen.getByText(es.callDetection.ending(0))).toBeTruthy();
    act(() => screen.getByRole("button", { name: es.callDetection.keep }).click());
    expect(sent).toEqual([{ command: "recording_keep", recording_id: "recording-1" }]);
  });

  it("says nothing when there is nothing to say", () => {
    const { view } = renderPanel(null);
    expect(view.container.textContent).toBe("");
  });

  it("rounds the seconds left up and never below zero", () => {
    expect(secondsLeft(10_000, 0)).toBe(10);
    expect(secondsLeft(10_000, 1)).toBe(10);
    expect(secondsLeft(10_000, 999)).toBe(10);
    expect(secondsLeft(10_000, 1_000)).toBe(9);
    expect(secondsLeft(10_000, 10_000)).toBe(0);
    expect(secondsLeft(10_000, 50_000)).toBe(0);
  });
});
