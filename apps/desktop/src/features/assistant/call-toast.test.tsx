import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppView } from "../../state/app-view";
import AssistantView from "./AssistantView";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("al terminar la llamada informa sin ofrecer un deshacer vacío", () => {
  vi.useFakeTimers();
  const send = vi.fn();
  const snapshot = {
    connection: { daemon: "ready", hub: "paired" },
    settings: { hub: { paired: true, connection: "online" } },
    chat: { session_id: null, turn: [], pending: null, voice: { state: "idle", level: 0 } },
    call: {
      started_at_ms: Date.now(),
      last_line: null,
      seeing_screen: false,
      voice_unavailable: false,
    },
  } as unknown as AppView;
  render(<AssistantView error={null} send={send} snapshot={snapshot} />);
  fireEvent.click(screen.getByRole("button", { name: "Colgar" }));
  expect(send).toHaveBeenCalledWith({ command: "call_stop" });
  expect(screen.getByText("Llamada terminada")).toBeTruthy();
  expect(screen.queryByRole("button", { name: /Deshacer/ })).toBeNull();
});
