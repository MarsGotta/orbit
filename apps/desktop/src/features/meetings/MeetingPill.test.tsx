import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { MeetingPill } from "./MeetingPill";

// SIM-25 run 14: «Terminar» only existed while the mouse hovered the pill, so
// neither the keyboard nor accessibility could reach it. The controls are
// always in the page; the stylesheet shows them on hover and on focus.

function recording(): AppSnapshot {
  return {
    meeting: {
      phase: "recording",
      started_at_ms: Date.now() - 62_000,
      title: "Reunión",
      recent: [],
      tracks: [],
    },
  } as unknown as AppSnapshot;
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("MeetingPill", () => {
  it("finishes the recording from the keyboard", async () => {
    const sent: AppCommand[] = [];
    render(
      <MeetingPill
        send={async (command) => {
          sent.push(command);
        }}
        snapshot={recording()}
      />,
    );
    const user = userEvent.setup();
    screen.getByRole("button", { name: "Terminar" }).focus();
    await user.keyboard("{Enter}");
    expect(sent).toContainEqual({ command: "meeting_finish" });
  });
});

it("actualiza el tiempo grabado cada segundo y desaparece al quedar inactivo", () => {
  vi.useFakeTimers();
  const snapshot = recording();
  const { rerender } = render(<MeetingPill send={() => undefined} snapshot={snapshot} />);
  expect(screen.getByRole("status").textContent).toContain("01:02");
  act(() => vi.advanceTimersByTime(2_000));
  expect(screen.getByRole("status").textContent).toContain("01:04");
  rerender(
    <MeetingPill
      send={() => undefined}
      snapshot={{ ...snapshot, meeting: { ...snapshot.meeting, phase: "idle" } }}
    />,
  );
  expect(screen.queryByRole("status")).toBeNull();
});
