import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppView } from "../../state/app-view";
import { MeetingOverlays } from "./MeetingOverlays";

const snapshot = (phase: "idle" | "recording") =>
  ({
    meeting: { phase, recording_id: "fixture", recent: [{ recording_id: "fixture" }] },
  }) as AppView;

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("the saved meeting says so, and that the note follows, without opening it half-written", () => {
  const props = {
    error: null,
    onErrorDismiss: vi.fn(),
    launcherOpen: false,
    onLauncherClose: vi.fn(),
    send: vi.fn(),
  };
  const view = render(<MeetingOverlays {...props} snapshot={snapshot("recording")} />);
  view.rerender(<MeetingOverlays {...props} snapshot={snapshot("idle")} />);
  const notice = screen.getByRole("status");
  expect(notice.textContent).toContain("Reunión guardada");
  expect(notice.textContent).toContain("Te avisamos cuando la nota esté lista.");
  expect(screen.queryByRole("link")).toBeNull();
  expect(screen.getByRole("button", { name: "Cerrar aviso" })).toBeTruthy();
});

it("the recording failure is shown and closing it discards its message", () => {
  const dismiss = vi.fn();
  render(
    <MeetingOverlays
      error="No hay permiso de micrófono."
      errorKind="fixable"
      onErrorDismiss={dismiss}
      launcherOpen={false}
      onLauncherClose={vi.fn()}
      send={vi.fn()}
      snapshot={snapshot("idle")}
    />,
  );
  expect(screen.getByRole("alert").textContent).toContain("No hay permiso de micrófono.");
  expect(dismiss).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Cerrar aviso" }));
  expect(dismiss).toHaveBeenCalledOnce();
  expect(screen.queryByRole("alert")).toBeNull();
});
