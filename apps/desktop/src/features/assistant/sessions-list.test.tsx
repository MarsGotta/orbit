import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { ChatSessionSummary } from "../../bridge/generated/ChatSessionSummary";
import type { SessionPage } from "../../bridge/reads";
import { AssistantCopy } from "../../i18n/AssistantCopy";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { ShellSlots, useShellSlots } from "../../app/shell-slots";
import { AssistantSessions } from "./AssistantSessions";
import { conversationAgentName } from "./agents";
import { rememberFirstLine } from "./first-lines";
import { sessionWhen } from "./session-title";

/** The window's list column, where the conversations are drawn. */
function ColumnHost() {
  return <div ref={useShellSlots()?.setColumn} />;
}

// The list of conversations: «+ Nueva», the open one on top, then «Recientes»,
// each named by when it began, its agent on the right and its first sentence
// below, never its date again nor «Conversación nueva».

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

/** The open conversation's row: marked current, not a button to press again. */
async function openRow(): Promise<HTMLElement> {
  return waitFor(() => {
    const row = document.querySelector<HTMLElement>('[aria-current="page"]');
    expect(row).not.toBeNull();
    return row as HTMLElement;
  });
}

// Wednesday 21 October 2026, 12:00 on the Mac's clock.
const NOW = new Date(2026, 9, 21, 12, 0).getTime();
const at = (day: number, hour: number, minute: number) =>
  new Date(2026, 9, day, hour, minute).getTime();

beforeEach(() => {
  vi.useFakeTimers({ now: NOW, toFake: ["Date"] });
});

const AGENTS: Agent[] = [
  { id: "deimos", name: "Deimos" },
  { id: "claude-code", name: "Claude Code" },
];

function summary(id: string, created: number, count: number, agent_id?: string) {
  return {
    id,
    created_at_ms: created,
    last_at_ms: created,
    message_count: count,
    ...(agent_id !== undefined ? { agent_id } : {}),
  } satisfies ChatSessionSummary;
}

const SESSIONS = [
  summary("s-open", at(21, 9, 31), 6, "deimos"),
  summary("s-code", at(20, 16, 0), 12, "claude-code"),
  summary("s-old", at(18, 18, 2), 1, "deimos"),
];

function page(items: ChatSessionSummary[]): SessionPage {
  return {
    state: "present",
    value: { items, next: null, total: items.length, revision: 1 },
    as_of_ms: null,
  };
}

function snapshot(chat: Partial<AppSnapshot["chat"]>, agents: Agent[] = AGENTS): AppSnapshot {
  return {
    connection: { daemon: "ready", hub: "paired" },
    settings: {
      hub: { paired: true, connection: "online" },
      agents,
      default_agent_id: agents[0]?.id,
    },
    chat: {
      session_id: null,
      turn: [],
      pending: null,
      voice: { level: 0, state: "idle" },
      ...chat,
    },
  } as unknown as AppSnapshot;
}

function list(
  chat: Partial<AppSnapshot["chat"]>,
  read: (cursor: unknown) => Promise<SessionPage> = () => Promise.resolve(page(SESSIONS)),
  agents?: Agent[],
) {
  const sent: AppCommand[] = [];
  const snap = snapshot(chat, agents);
  render(
    <ShellSlots>
      <ColumnHost />
      <AssistantCopy name={conversationAgentName(snap)}>
        <AssistantSessions read={read} send={(command) => sent.push(command)} snapshot={snap} />
      </AssistantCopy>
    </ShellSlots>,
  );
  return { sent };
}

describe("when a conversation began", () => {
  it("reads today and yesterday as words, an older day as its date", () => {
    expect(sessionWhen(at(21, 9, 31), NOW)).toBe("Hoy, 09:31");
    expect(sessionWhen(at(20, 16, 0), NOW)).toBe("Ayer, 16:00");
    expect(sessionWhen(at(18, 18, 2), NOW)).toBe("18 oct, 18:02");
    expect(sessionWhen(new Date(2025, 9, 15, 3, 46).getTime(), NOW)).toBe("15 oct 2025, 03:46");
  });
});

describe("the list of conversations", () => {
  it("puts the open one on top, then the recent ones with their date and agent", async () => {
    list({ session_id: "s-open", agent_id: "deimos" });
    const open = await openRow();
    expect(open.textContent).toContain("Hoy, 09:31");
    expect(open.textContent).toContain("Deimos");
    const recent = screen.getByRole("list", { name: "Recientes" });
    const rows = within(recent).getAllByRole("button");
    expect(rows.map((row) => row.textContent)).toEqual([
      expect.stringContaining("Ayer, 16:00"),
      expect.stringContaining("18 oct, 18:02"),
    ]);
    expect(rows[0]?.textContent).toContain("Claude Code");
  });

  it("shows each conversation's first sentence, never its date twice", async () => {
    rememberFirstLine("s-code", "Revisa el cambio del calendario semanal. Y dime si rompe algo.");
    list({ session_id: "s-open" });
    const recent = await screen.findByRole("list", { name: "Recientes" });
    const [code, old] = within(recent).getAllByRole("button");
    expect(code?.textContent).toContain("Revisa el cambio del calendario semanal.");
    expect(code?.textContent).not.toContain("Y dime si rompe algo");
    // Not read yet: how many messages it holds, not its title again.
    expect(old?.textContent).toContain("1 mensaje");
    expect(old?.textContent?.match(/18 oct/g)).toHaveLength(1);
  });

  it("names a conversation in progress by its question, never «Conversación nueva»", async () => {
    const question: ChatMessage = {
      id: "t1",
      role: "user",
      text: "Prepárame la llamada con Ribera. ¿Qué quedó pendiente?",
      at_ms: at(21, 11, 58),
      streaming: false,
      voice: null,
      receipts: [],
    };
    // Begun a moment ago: the hub does not list it yet.
    list({ session_id: "s-new", turn: [question], activity: "thinking" });
    const open = await openRow();
    expect(open.textContent).toContain("Hoy, 11:58");
    expect(open.textContent).toContain("Prepárame la llamada con Ribera.");
    expect(screen.queryByText(/Conversación nueva/)).toBeNull();
    // The list is read while the turn is in flight, not only once it ends.
    expect(await screen.findByRole("list", { name: "Recientes" })).toBeTruthy();
  });

  it("opens a recent conversation with its agent", async () => {
    const { sent } = list({ session_id: "s-open" });
    const recent = await screen.findByRole("list", { name: "Recientes" });
    await userEvent.click(within(recent).getAllByRole("button")[0] as HTMLElement);
    expect(sent).toEqual([{ command: "session_open", id: "s-code", agent_id: "claude-code" }]);
  });

  it("starts a new conversation from «Nueva»", async () => {
    const { sent } = list({ session_id: "s-open" });
    await userEvent.click(screen.getByRole("button", { name: "Nueva conversación" }));
    expect(sent).toEqual([{ command: "new_session" }]);
  });

  it("names no agent on the rows when the hub has one", async () => {
    list({ session_id: "s-open" }, undefined, [{ id: "default", name: "Vega" }]);
    const recent = await screen.findByRole("list", { name: "Recientes" });
    expect(recent.textContent).not.toContain("Vega");
  });

  it("shows the shape of the rows while the first page loads, and says nothing is missing", async () => {
    list({ session_id: null }, () => new Promise(() => undefined));
    const busy = await screen.findByRole("list", { busy: true });
    expect(busy).toBeTruthy();
    expect(screen.queryByText(/Aún no hay/)).toBeNull();
  });

  it("says why the list could not be read and retries it", async () => {
    const read = vi
      .fn<(cursor: unknown) => Promise<SessionPage>>()
      .mockResolvedValueOnce({
        state: "absent",
        refusal: { code: "hub_unreachable" },
      } as unknown as SessionPage)
      .mockResolvedValue(page(SESSIONS));
    list({ session_id: null }, read);
    expect(await screen.findByText("No se pudieron cargar las conversaciones.")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Reintentar" }));
    expect(await screen.findByRole("list", { name: "Recientes" })).toBeTruthy();
  });
});
