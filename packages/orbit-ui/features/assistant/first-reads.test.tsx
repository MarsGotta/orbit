import { render, renderHook, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { MessagesPage, SessionPage } from "../../bridge/reads";
import type { AppView as AppSnapshot } from "../../state/app-view";
import AssistantView from "./AssistantView";
import { useAssistantSessions } from "./useAssistantSessions";

// A conversation's first page and the list of conversations are read from
// the hub. Until they answer, nothing is known: the screen says it is reading,
// never that there is nothing. An answer that says nothing is not «nothing».

function snapshot(): AppSnapshot {
  return {
    connection: { daemon: "ready", hub: "paired" },
    status_availability: { state: "current" },
    call_availability: { state: "current" },
    call: null,
    settings: { hub: { paired: true, connection: "online" } },
    chat: {
      session_id: "session-taller",
      turn: [],
      pending: null,
      voice: { level: 0, state: "idle", availability: { state: "current" } },
      availability: { state: "current" },
    },
  } as unknown as AppSnapshot;
}

const unknown = { state: "unknown", why: "not_observed_yet" } as const;

describe("a conversation's first page", () => {
  it("while it is read, says so and never that there are no messages", async () => {
    const read = vi.fn(() => new Promise<MessagesPage>(() => undefined));
    render(
      <AssistantView
        error={null}
        readMessages={read}
        send={() => undefined}
        snapshot={snapshot()}
      />,
    );
    expect(await screen.findByText("Leyendo la conversación…")).toBeTruthy();
    expect(screen.queryByRole("heading", { level: 1, name: "Deimos" })).toBeNull();
  });

  it("an answer that says nothing is a failed read, not an empty conversation", async () => {
    const read = vi.fn(async (): Promise<MessagesPage> => unknown);
    render(
      <AssistantView
        error={null}
        readMessages={read}
        send={() => undefined}
        snapshot={snapshot()}
      />,
    );
    expect(await screen.findByText("No se pudieron cargar los mensajes.")).toBeTruthy();
    expect(screen.queryByRole("heading", { level: 1, name: "Deimos" })).toBeNull();
  });

  it("a failed first page can be read again", async () => {
    const read = vi
      .fn<(sessionId: string, cursor: unknown) => Promise<MessagesPage>>()
      .mockResolvedValueOnce(unknown)
      .mockResolvedValueOnce({
        state: "present",
        value: { items: [], next: null, total: 0, revision: 1 },
        as_of_ms: null,
      });
    render(
      <AssistantView
        error={null}
        readMessages={read}
        send={() => undefined}
        snapshot={snapshot()}
      />,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Reintentar" }));
    // The empty conversation invites to write to its agent: its own page.
    expect(await screen.findByRole("heading", { level: 1, name: "Deimos" })).toBeTruthy();
    expect(read).toHaveBeenCalledTimes(2);
  });

  it("an empty page is an empty conversation", async () => {
    const read = vi.fn(async (): Promise<MessagesPage> => ({
      state: "present",
      value: { items: [], next: null, total: 0, revision: 1 },
      as_of_ms: null,
    }));
    render(
      <AssistantView
        error={null}
        readMessages={read}
        send={() => undefined}
        snapshot={snapshot()}
      />,
    );
    // The empty conversation invites to write to its agent: its own page.
    expect(await screen.findByRole("heading", { level: 1, name: "Deimos" })).toBeTruthy();
    expect(screen.queryByText("Leyendo la conversación…")).toBeNull();
  });
});

describe("the list of conversations", () => {
  it("an answer that says nothing is a failed read, which can be retried", async () => {
    const read = vi.fn(async (): Promise<SessionPage> => unknown);
    const { result } = renderHook(() => useAssistantSessions("session-taller", false, read));
    await waitFor(() =>
      expect(result.current.state.error).toBe("No se pudieron cargar las conversaciones."),
    );
    expect(result.current.state.loading).toBe(false);
  });
});
