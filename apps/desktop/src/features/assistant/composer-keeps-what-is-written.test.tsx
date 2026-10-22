import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { ChatSnapshot } from "../../bridge/generated/ChatSnapshot";
import type { DaemonPort } from "../../bridge/port";
import { createOrbitStore } from "../../state/orbit-store";
import { AssistantComposer } from "./AssistantComposer";

// What the person writes in the chat is never lost on its way to the daemon,
// nor by leaving the view, nor by Orbit closing before it was sent. The port
// refuses as the daemon does (`orbit-core/src/transition.rs`,
// `decide_chat_submit`): busy while a turn is in flight, and a text past
// `MAX_TEXT_BYTES` (32 KiB).

const idle: ChatSnapshot = {
  turn: [],
  pending: null,
  session_id: null,
  voice: { level: 0, state: "idle" },
};

function refusingStore(refusal: Record<string, unknown>) {
  const sent: AppCommand[] = [];
  const port: DaemonPort = {
    snapshot: () => Promise.reject(new Error("sin arranque en esta prueba")),
    subscribeFrames: () => Promise.resolve(() => undefined),
    send: (command) => {
      sent.push(command);
      return Promise.reject({ refusal });
    },
    report: () => Promise.resolve(),
  };
  return { store: createOrbitStore(port), sent };
}

type Composer = typeof AssistantComposer;

function composer(
  chat: ChatSnapshot,
  send: (command: AppCommand) => unknown,
  Shown: Composer = AssistantComposer,
) {
  return render(<Shown chat={chat} daemonReady hub="online" paired send={send} />);
}

function box(): HTMLTextAreaElement {
  return screen.getByLabelText("Mensaje para Deimos") as HTMLTextAreaElement;
}

async function pressEnter() {
  await act(async () => {
    fireEvent.keyDown(box(), { key: "Enter" });
    await Promise.resolve();
  });
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("a chat message is never lost on its way to the daemon", () => {
  it("a message the daemon refuses as busy is still in the composer", async () => {
    const { store, sent } = refusingStore({
      code: "busy",
      operation: "interaction",
      retry: { kind: "never" },
    });
    composer(idle, store.getState().send);
    fireEvent.change(box(), { target: { value: "una pregunta escrita mientras responde" } });
    await pressEnter();
    expect(sent.map((command) => command.command)).toEqual(["chat"]);
    expect(store.getState().messages.length).toBeGreaterThan(0);
    expect(box().value).toBe("una pregunta escrita mientras responde");
  });

  it("while Deimos is still answering, Enter sends nothing and the text stays", async () => {
    const { store, sent } = refusingStore({ code: "busy", retry: { kind: "never" } });
    composer({ ...idle, activity: "thinking" }, store.getState().send);
    fireEvent.change(box(), { target: { value: "otra pregunta" } });
    await pressEnter();
    expect(sent).toEqual([]);
    expect(box().value).toBe("otra pregunta");
  });

  it("a message longer than the daemon takes is still in the composer after its refusal", async () => {
    const long = "palabra ".repeat(5_000); // 40,000 bytes, past the 32 KiB the daemon takes
    const { store } = refusingStore({
      code: "invalid_parameter",
      parameter: "text",
      expected: { kind: "text_length", bytes: long.length, max_bytes: 32 * 1024 },
      retry: { kind: "never" },
    });
    composer(idle, store.getState().send);
    fireEvent.change(box(), { target: { value: long } });
    await pressEnter();
    expect(box().value).toBe(long);
  });
});

describe("a message typed and not sent yet", () => {
  it("is still in the composer after visiting another view", () => {
    const send = vi.fn();
    const first = composer(idle, send);
    fireEvent.change(box(), { target: { value: "borrador a medias" } });
    // `app/Main.tsx` renders only the current view: leaving unmounts the composer.
    first.unmount();
    composer(idle, send);
    expect(box().value).toBe("borrador a medias");
    // The person's own draft of this session is not news to them.
    expect(screen.queryByText(/borrador que no llegaste a enviar/)).toBeNull();
  });

  it("is back, and says so, when Orbit opens again after it closed or fell", async () => {
    // What an earlier run of the webview left in this Mac's local storage.
    window.localStorage.setItem(
      "orbit.draft.chat",
      JSON.stringify("lo que escribí antes de cerrar"),
    );
    vi.resetModules();
    const reopened = (await import("./AssistantComposer")).AssistantComposer;
    composer(idle, vi.fn(), reopened);
    expect(box().value).toBe("lo que escribí antes de cerrar");
    expect(screen.getByRole("status").textContent).toContain(
      "Recuperado el borrador que no llegaste a enviar",
    );
  });

  it("is let go of once the daemon takes it", async () => {
    const send = vi.fn(() => Promise.resolve(true));
    const first = composer(idle, send);
    fireEvent.change(box(), { target: { value: "enviado" } });
    await pressEnter();
    expect(box().value).toBe("");
    first.unmount();
    composer(idle, send);
    expect(box().value).toBe("");
    expect(window.localStorage.getItem("orbit.draft.chat")).toBeNull();
  });
});
