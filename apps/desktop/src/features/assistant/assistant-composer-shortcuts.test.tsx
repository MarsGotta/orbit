import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { ChatSnapshot } from "../../bridge/generated/ChatSnapshot";
import { AssistantComposer } from "./AssistantComposer";

// REF-07 A5: with the focus on the chat box, a modifier on its way to another
// chord — ⌃⌥Esc is the privacy stop — or a composed character (⌥2 is «@» on
// the Spanish keyboard) must not send `voice_start` or `voice_end` to the
// daemon, no matter how long the chord is held. ⌥ alone is never dictation.
//
// The second key may arrive after a long hold; neither the chord nor Option
// alone is a voice command.

const chat: ChatSnapshot = {
  turn: [],
  pending: null,
  session_id: null,
  voice: { level: 0, state: "idle" },
};

function renderComposer() {
  const send = vi.fn<(command: AppCommand) => void>();
  render(
    <AssistantComposer
      callActive={false}
      callPending={false}
      chat={chat}
      daemonReady
      hub="online"
      onCallToggle={() => undefined}
      paired
      send={send}
    />,
  );
  return { input: screen.getByLabelText("Mensaje"), send };
}

function voiceCalls(send: ReturnType<typeof renderComposer>["send"]) {
  return send.mock.calls
    .map(([command]) => command.command as string)
    .filter((name) => name === "voice_start" || name === "voice_end");
}

function keyDown(input: HTMLElement) {
  fireEvent.keyDown(input, { key: "Alt" });
}

afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe("REF-07 A5 una tecla modificadora no abre el micrófono", () => {
  it("mantener ⌥ y escribir @ nunca abre el micrófono", () => {
    vi.useFakeTimers();
    const { input, send } = renderComposer();

    keyDown(input);
    expect(voiceCalls(send)).toEqual([]);
    act(() => vi.advanceTimersByTime(1_000));
    expect(voiceCalls(send)).toEqual([]);
    fireEvent.keyDown(input, { key: "2", altKey: true, code: "Digit2" });
    fireEvent.keyUp(input, { key: "Alt" });
    expect(voiceCalls(send)).toEqual([]);
    expect(send).not.toHaveBeenCalled();
  });
});
