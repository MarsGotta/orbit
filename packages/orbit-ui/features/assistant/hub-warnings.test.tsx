import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { MessagesPage } from "../../bridge/reads";
import { CopyProvider } from "../../i18n/CopyProvider";
import { en } from "../../i18n/en";
import { es } from "../../i18n/es";
import type { AppView as AppSnapshot } from "../../state/app-view";
import AssistantView from "./AssistantView";

// The hub's warnings about the open conversation are painted in its chat,
// below its turn, with the hub's own words and the app's label saying whose
// words they are.

afterEach(cleanup);

const WITHDRAWN = "Hermes withdrew its approval question (timeout); it can no longer be answered";

function message(id: string, role: ChatMessage["role"], text: string): ChatMessage {
  return { id, role, text, at_ms: 1_700_000_000_000, streaming: false, voice: null, receipts: [] };
}

function page(items: ChatMessage[]): MessagesPage {
  return {
    state: "present",
    value: { items, next: null, total: items.length, revision: 1 },
    as_of_ms: null,
  };
}

function snapshot(chat: Record<string, unknown>): AppSnapshot {
  return {
    connection: { daemon: "ready", hub: "paired" },
    status_availability: { state: "current" },
    call_availability: { state: "current" },
    call: null,
    notice: null,
    settings: {
      hub: { paired: true, connection: "online" },
      agents: [{ id: "deimos", name: "Deimos" }],
      default_agent_id: "deimos",
    },
    chat: {
      session_id: "s-warnings",
      turn: [],
      pending: null,
      voice: { level: 0, state: "idle", availability: { state: "current" } },
      availability: { state: "current" },
      ...chat,
    },
  } as unknown as AppSnapshot;
}

function show(snap: AppSnapshot) {
  render(
    <CopyProvider name="Deimos">
      <AssistantView
        error={null}
        readMessages={() => Promise.resolve(page([]))}
        send={() => undefined}
        snapshot={snap}
      />
    </CopyProvider>,
  );
}

describe("the hub's warnings", () => {
  it("both languages have the label that says the words are the hub's", () => {
    expect(typeof es.assistant.hubWarning).toBe("string");
    expect(es.assistant.hubWarning.length).toBeGreaterThan(0);
    expect(typeof en.assistant.hubWarning).toBe("string");
    expect(en.assistant.hubWarning.length).toBeGreaterThan(0);
  });

  it("are painted under the turn, in the hub's words, with their label", async () => {
    const question = message("t1", "user", "Borra la carpeta de pruebas.");
    show(
      snapshot({
        turn: [question],
        activity: "thinking",
        warnings: [WITHDRAWN, "Hermes asked for sudo, which Orbit cannot answer"],
      }),
    );
    const first = await screen.findByText(WITHDRAWN);
    expect(screen.getByText("Hermes asked for sudo, which Orbit cannot answer")).toBeTruthy();
    expect(screen.getAllByText(es.assistant.hubWarning, { exact: false }).length).toBeGreaterThan(
      0,
    );
    // Below the question the warning is about.
    const asked = screen.getByText("Borra la carpeta de pruebas.");
    expect(asked.compareDocumentPosition(first) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // Shown in the chat, never as an alert that interrupts.
    expect(first.closest('[role="alert"]')).toBeNull();
  });

  it("stay painted after the turn ended", async () => {
    show(
      snapshot({
        turn: [message("t1", "user", "¿Y ahora?"), message("t1:assistant", "assistant", "Hecho.")],
        warnings: [WITHDRAWN],
      }),
    );
    expect(await screen.findByText(WITHDRAWN)).toBeTruthy();
  });

  it("a chat without warnings paints no label", async () => {
    show(snapshot({ turn: [message("t1", "user", "Hola.")], activity: "thinking" }));
    await screen.findByText("Hola.");
    expect(screen.queryByText(es.assistant.hubWarning, { exact: false })).toBeNull();
  });
});
