import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { CallSnapshot } from "../../bridge/generated/CallSnapshot";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import { AssistantCopy } from "../../i18n/AssistantCopy";
import type { AppView } from "../../state/app-view";
import AssistantView from "./AssistantView";
import { callLimits } from "./CallView";

// The call with the agent: a calm view with the circle whose bars move only
// while the agent speaks, the stopwatch, the last question and answer, the
// limits in view, and a small floating card once minimised.

const NOW = 1_800_000_000_000;

beforeEach(() => {
  vi.useFakeTimers({ now: NOW, toFake: ["Date", "setInterval", "clearInterval"] });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function call(change: Partial<CallSnapshot> = {}): CallSnapshot {
  return {
    started_at_ms: NOW - 252_000,
    seeing_screen: true,
    last_line: "Ribera Aeropuerto va 16 puntos por debajo.",
    speaking: true,
    expires_at_ms: NOW - 252_000 + 60 * 60_000,
    origin: "app",
    idle_hangup_ms: 60_000,
    ...change,
  };
}

const question: ChatMessage = {
  id: "t1",
  role: "user",
  text: "¿Qué hotel va peor comparado con septiembre?",
  at_ms: NOW - 20_000,
  streaming: false,
  voice: null,
  receipts: [],
};

function snapshot(current: CallSnapshot | null): AppView {
  return {
    connection: { daemon: "ready", hub: "paired" },
    status_availability: { state: "current" },
    call_availability: { state: "current" },
    settings: { hub: { paired: true, connection: "online" }, agents: [{ id: "a", name: "Atlas" }] },
    chat: {
      session_id: null,
      turn: [question],
      pending: null,
      voice: { level: 0, state: "idle" },
      availability: { state: "current" },
      agent_id: "a",
    },
    call: current,
  } as unknown as AppView;
}

function show(current: CallSnapshot | null) {
  const sent: AppCommand[] = [];
  const view = (value: CallSnapshot | null) => (
    <AssistantCopy name="Atlas">
      <AssistantView
        error={null}
        send={(command) => sent.push(command)}
        snapshot={snapshot(value)}
      />
    </AssistantCopy>
  );
  const rendered = render(view(current));
  return { sent, rerender: (value: CallSnapshot | null) => rendered.rerender(view(value)) };
}

describe("the limits of a call", () => {
  it("says the minutes left, the silence that hangs up and the shortcut", () => {
    expect(callLimits(call(), NOW)).toBe(
      "Quedan 55 min · se cuelga sola tras 1 minuto sin que hables · ⌃⌥D cuelga desde cualquier app",
    );
    expect(callLimits(call({ idle_hangup_ms: 90_000 }), NOW)).toContain(
      "tras 1 min 30 s sin que hables",
    );
  });
});

describe("the call view", () => {
  it("shows who speaks, the time, the last question and answer and the limits", () => {
    show(call());
    expect(screen.getByRole("heading", { name: "Llamada con Atlas" })).toBeTruthy();
    expect(screen.getByText("Atlas está hablando")).toBeTruthy();
    expect(screen.getByRole("timer").textContent).toBe("04:12");
    expect(screen.getByText("Tú: ¿Qué hotel va peor comparado con septiembre?")).toBeTruthy();
    expect(screen.getByText("Ribera Aeropuerto va 16 puntos por debajo.")).toBeTruthy();
    expect(screen.getByText("Puede ver tu pantalla")).toBeTruthy();
    expect(screen.getByText(/^Quedan 55 min/)).toBeTruthy();
  });

  it("the bars rest while the user speaks", () => {
    show(call({ speaking: false }));
    expect(screen.getByText("Te escucha")).toBeTruthy();
    expect(screen.getByTestId("call-bars").getAttribute("data-speaking")).toBe("false");
  });

  it("hangs up from «Colgar» and says the call ended", () => {
    const { sent } = show(call());
    fireEvent.click(screen.getByRole("button", { name: "Colgar" }));
    expect(sent).toEqual([{ command: "call_stop" }]);
    expect(screen.getByText("Llamada terminada")).toBeTruthy();
    // Nothing to undo: the notice offers no empty «Deshacer».
    expect(screen.queryByRole("button", { name: /Deshacer/ })).toBeNull();
  });

  it("«Silenciar»: the assistant stops hearing her, and the view says it once confirmed", () => {
    const { sent, rerender } = show(call({ speaking: false }));
    fireEvent.click(screen.getByRole("button", { name: "Silenciar" }));
    expect(sent).toEqual([{ command: "microphone_mute", muted: true }]);
    expect(screen.getByText("Te escucha")).toBeTruthy();
    rerender(call({ speaking: false, microphone_muted: true }));
    expect(screen.getByText("Micrófono silenciado")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Reactivar micrófono" }));
    expect(sent).toEqual([
      { command: "microphone_mute", muted: true },
      { command: "microphone_mute", muted: false },
    ]);
  });

  it("minimises to a floating card over the conversation, and opens again", () => {
    const { sent } = show(call());
    fireEvent.click(screen.getByRole("button", { name: "Minimizar" }));
    const card = screen.getByRole("region", { name: "Llamada minimizada" });
    expect(card.textContent).toContain("En llamada con Atlas");
    expect(within(card).getByRole("timer").textContent).toBe("04:12");
    act(() => vi.advanceTimersByTime(1_000));
    expect(within(card).getByRole("timer").textContent).toBe("04:13");
    expect(screen.getByRole("textbox", { name: "Mensaje para Atlas" })).toBeTruthy();
    fireEvent.click(within(card).getByRole("button", { name: "Abrir" }));
    expect(screen.getByRole("heading", { name: "Llamada con Atlas" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Minimizar" }));
    fireEvent.click(
      within(screen.getByRole("region", { name: "Llamada minimizada" })).getByRole("button", {
        name: "Colgar",
      }),
    );
    expect(sent).toEqual([{ command: "call_stop" }]);
  });

  it("goes back to the conversation when the call ends", () => {
    const { rerender } = show(call());
    rerender(null);
    expect(screen.queryByRole("heading", { name: "Llamada con Atlas" })).toBeNull();
    expect(screen.getByRole("textbox", { name: "Mensaje para Atlas" })).toBeTruthy();
  });
});
