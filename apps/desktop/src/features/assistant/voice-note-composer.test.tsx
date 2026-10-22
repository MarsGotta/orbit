import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { act } from "react";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { ChatSnapshot } from "../../bridge/generated/ChatSnapshot";
import type { AppView } from "../../state/app-view";
import { AssistantComposer } from "./AssistantComposer";
import AssistantView from "./AssistantView";

// The composer's voice note: a click starts it and another sends it, holding
// the button talks while held, Esc cancels it, and while it records the
// composer says so with its time. A note the daemon found too short says its
// length and the minimum here, where it was recorded.

type VoiceState = ChatSnapshot["voice"]["state"];

function chat(state: VoiceState, too_short?: { duration_ms: number; minimum_ms: number }) {
  return {
    turn: [],
    pending: null,
    session_id: null,
    voice: { level: 0, state, ...(too_short ? { too_short } : {}) },
  } satisfies ChatSnapshot;
}

/**
 * A composer whose daemon answers `voice_start` only when the test says so,
 * as the real one does after its decision reaches the screen.
 */
function renderComposer(initial: ChatSnapshot = chat("idle")) {
  let answer: () => void = () => undefined;
  const send = vi.fn<(command: AppCommand) => unknown>((command) =>
    command.command === "voice_start"
      ? new Promise<void>((resolve) => {
          answer = resolve;
        })
      : undefined,
  );
  const view = (snapshot: ChatSnapshot) => (
    <AssistantComposer chat={snapshot} daemonReady hub="online" paired send={send} />
  );
  const rendered = render(view(initial));
  return {
    microphone: () => screen.getByRole("button", { name: /^(Grabar nota de voz|Grabando)/ }),
    send,
    /** The daemon's state reaches the screen, then its answer to the order. */
    daemon: async (state: VoiceState) => {
      rendered.rerender(view(chat(state)));
      await act(async () => answer());
    },
    rerender: (snapshot: ChatSnapshot) => rendered.rerender(view(snapshot)),
  };
}

function voiceOrders(send: ReturnType<typeof renderComposer>["send"]) {
  return send.mock.calls
    .map(([command]) => command.command as string)
    .filter((name) => name.startsWith("voice_"));
}

const capture = vi.fn();

// jsdom has no PointerEvent: this one carries the button and pointer id a
// WebView's does, so the composer reads them as it does on the Mac.
class TestPointerEvent extends MouseEvent {
  readonly pointerId: number;
  constructor(type: string, init: PointerEventInit = {}) {
    super(type, init);
    this.pointerId = init.pointerId ?? 0;
  }
}

beforeAll(() => {
  vi.stubGlobal("PointerEvent", TestPointerEvent);
});

afterAll(() => {
  vi.unstubAllGlobals();
});

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval", "setTimeout"] });
  capture.mockReset();
  HTMLElement.prototype.setPointerCapture = capture;
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe("a click starts the note and another sends it", () => {
  it("a quick click does not end the note it starts", async () => {
    const { daemon, microphone, send } = renderComposer();
    fireEvent.pointerDown(microphone(), { button: 0, pointerId: 1 });
    act(() => vi.advanceTimersByTime(80));
    fireEvent.pointerUp(microphone(), { button: 0, pointerId: 1 });
    await daemon("listening");
    expect(voiceOrders(send)).toEqual(["voice_start"]);
  });

  it("the second click sends it", async () => {
    const { daemon, microphone, send } = renderComposer();
    fireEvent.pointerDown(microphone(), { button: 0, pointerId: 1 });
    fireEvent.pointerUp(microphone(), { button: 0, pointerId: 1 });
    await daemon("listening");
    act(() => vi.advanceTimersByTime(4_000));
    fireEvent.pointerDown(microphone(), { button: 0, pointerId: 2 });
    fireEvent.pointerUp(microphone(), { button: 0, pointerId: 2 });
    expect(voiceOrders(send)).toEqual(["voice_start", "voice_end"]);
  });

  it("«Enviar nota de voz» sends it too", async () => {
    const { daemon, microphone, send } = renderComposer();
    fireEvent.pointerDown(microphone(), { button: 0, pointerId: 1 });
    fireEvent.pointerUp(microphone(), { button: 0, pointerId: 1 });
    await daemon("listening");
    fireEvent.click(screen.getByRole("button", { name: "Enviar nota de voz" }));
    expect(voiceOrders(send)).toEqual(["voice_start", "voice_end"]);
  });

  it("Enter on the focused button starts and sends, as a click", async () => {
    const { daemon, microphone, send } = renderComposer();
    fireEvent.click(microphone(), { detail: 0 });
    await daemon("listening");
    fireEvent.click(microphone(), { detail: 0 });
    expect(voiceOrders(send)).toEqual(["voice_start", "voice_end"]);
  });
});

describe("holding the button talks while held", () => {
  it("captures the pointer, so releasing outside the button still reaches it", () => {
    const { microphone } = renderComposer();
    fireEvent.pointerDown(microphone(), { button: 0, pointerId: 7 });
    expect(capture).toHaveBeenCalledWith(7);
  });

  it("releasing after holding sends the note", async () => {
    const { daemon, microphone, send } = renderComposer();
    fireEvent.pointerDown(microphone(), { button: 0, pointerId: 1 });
    await daemon("listening");
    act(() => vi.advanceTimersByTime(2_500));
    // With the pointer captured, a release anywhere is delivered to the button.
    fireEvent.pointerUp(microphone(), { button: 0, pointerId: 1 });
    expect(voiceOrders(send)).toEqual(["voice_start", "voice_end"]);
  });

  it("a pointer the system takes away leaves the note recording, and saying so", async () => {
    const { daemon, microphone, send } = renderComposer();
    fireEvent.pointerDown(microphone(), { button: 0, pointerId: 1 });
    await daemon("listening");
    act(() => vi.advanceTimersByTime(2_500));
    fireEvent.pointerCancel(microphone(), { pointerId: 1 });
    expect(voiceOrders(send)).toEqual(["voice_start"]);
    expect(screen.getByRole("status").textContent).toContain("Grabando");
  });
});

describe("while it records, the composer says so", () => {
  it("says it records from the press, before the daemon answers, with mm:ss", () => {
    const { microphone } = renderComposer();
    fireEvent.pointerDown(microphone(), { button: 0, pointerId: 1 });
    fireEvent.pointerUp(microphone(), { button: 0, pointerId: 1 });
    expect(screen.getByRole("status").textContent).toContain("Grabando nota de voz");
    expect(screen.getByRole("status").textContent).toContain("0:00");
    expect(screen.getByRole("status").textContent).toContain("esc cancela");
    act(() => vi.advanceTimersByTime(65_000));
    expect(screen.getByRole("status").textContent).toContain("1:05");
    expect(microphone().getAttribute("aria-label")).toContain("1:05");
    expect(microphone().getAttribute("aria-pressed")).toBe("true");
  });

  it("a start the daemon refused stops saying it records", async () => {
    const { daemon, microphone } = renderComposer();
    fireEvent.pointerDown(microphone(), { button: 0, pointerId: 1 });
    fireEvent.pointerUp(microphone(), { button: 0, pointerId: 1 });
    await daemon("idle");
    expect(screen.queryByRole("status")).toBeNull();
    expect(microphone().getAttribute("aria-pressed")).toBe("false");
  });

  it("a note the daemon ended by itself ends in the composer too", async () => {
    const { daemon, microphone, rerender } = renderComposer();
    fireEvent.pointerDown(microphone(), { button: 0, pointerId: 1 });
    fireEvent.pointerUp(microphone(), { button: 0, pointerId: 1 });
    await daemon("listening");
    rerender(chat("idle"));
    expect(screen.queryByRole("status")).toBeNull();
  });
});

describe("Esc cancels the note without sending it", () => {
  it("sends voice_cancel and never voice_end", async () => {
    const { daemon, microphone, send } = renderComposer();
    fireEvent.pointerDown(microphone(), { button: 0, pointerId: 1 });
    fireEvent.pointerUp(microphone(), { button: 0, pointerId: 1 });
    await daemon("listening");
    fireEvent.keyDown(window, { key: "Escape" });
    expect(voiceOrders(send)).toEqual(["voice_start", "voice_cancel"]);
  });

  it("while holding, Esc cancels and the release sends nothing more", async () => {
    const { daemon, microphone, send } = renderComposer();
    fireEvent.pointerDown(microphone(), { button: 0, pointerId: 1 });
    await daemon("listening");
    act(() => vi.advanceTimersByTime(2_000));
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.pointerUp(microphone(), { button: 0, pointerId: 1 });
    expect(voiceOrders(send)).toEqual(["voice_start", "voice_cancel"]);
  });
});

describe("a note too short to send", () => {
  it("says its length and the daemon's minimum in the composer", () => {
    renderComposer(chat("idle", { duration_ms: 90, minimum_ms: 160 }));
    expect(screen.getByRole("status").textContent).toBe(
      "La nota de voz duró 0,09 s y el mínimo es 0,16 s: no se ha enviado.",
    );
  });
});

describe("the microphone does not wait for read-aloud", () => {
  // Dictation is the user's input; whether the hub's voice can read an
  // answer aloud is another matter, unknown until something is read.
  function assistant(availability: NonNullable<ChatSnapshot["voice"]["availability"]>) {
    const send = vi.fn<(command: AppCommand) => unknown>();
    const snapshot = {
      connection: { daemon: "ready", hub: "paired" },
      status_availability: { state: "current" },
      call_availability: { state: "current" },
      call: null,
      settings: { hub: { paired: true, connection: "online" } },
      chat: {
        ...chat("idle"),
        availability: { state: "current" },
        voice: { level: 0, state: "idle", availability },
      },
    } as unknown as AppView;
    render(
      <AssistantView
        error={null}
        readMessages={() => new Promise(() => undefined)}
        send={send}
        snapshot={snapshot}
      />,
    );
    return { microphone: screen.getByRole("button", { name: "Grabar nota de voz" }), send };
  }

  it("records a note while the read-aloud voice is not known yet", () => {
    const { microphone, send } = assistant({ state: "unknown", cause: "pending:not_observed_yet" });
    expect((microphone as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(microphone, { detail: 0 });
    expect(voiceOrders(send)).toEqual(["voice_start"]);
  });

  it("records a note when the read-aloud voice did not arrive", () => {
    const { microphone } = assistant({ state: "absent", cause: "speech_unavailable" });
    expect((microphone as HTMLButtonElement).disabled).toBe(false);
  });
});
