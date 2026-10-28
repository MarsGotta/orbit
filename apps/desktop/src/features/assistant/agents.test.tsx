import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { ChatSessionSummary } from "../../bridge/generated/ChatSessionSummary";
import { AssistantCopy } from "../../i18n/AssistantCopy";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { FeatureFrame } from "../../bridge/generated";
import { applyFrame } from "../../state/feature-frame";
import AssistantView from "./AssistantView";
import { ShellSlots, useShellSlots } from "../../app/shell-slots";
import { AssistantSessions } from "./AssistantSessions";
import { conversationAgentName, primaryAgentName } from "./agents";

/** The window's list column, where the conversations are drawn. */
function ColumnHost() {
  return <div ref={useShellSlots()?.setColumn} />;
}

// Several agents on one hub: a new conversation picks one, an open one says
// whose it is, and every text names the open conversation's agent.

afterEach(cleanup);

const TWO = [
  { id: "deimos", name: "Deimos" },
  { id: "atlas", name: "Atlas" },
];

function snapshot(
  chat: Partial<AppSnapshot["chat"]>,
  agents: Agent[] = TWO,
  defaultAgent: string | undefined = "deimos",
): AppSnapshot {
  return {
    connection: { daemon: "ready", hub: "paired" },
    status_availability: { state: "current" },
    call_availability: { state: "current" },
    call: null,
    settings: {
      hub: { paired: true, connection: "online" },
      assistant_name: "Deimos",
      agents,
      default_agent_id: defaultAgent,
    },
    chat: {
      session_id: null,
      turn: [],
      pending: null,
      voice: { level: 0, state: "idle", availability: { state: "current" } },
      availability: { state: "current" },
      ...chat,
    },
  } as unknown as AppSnapshot;
}

function view(snap: AppSnapshot, sent: AppCommand[] = []) {
  return render(
    <AssistantCopy name={conversationAgentName(snap)}>
      <AssistantView
        error={null}
        readMessages={() => new Promise(() => undefined)}
        send={(command) => sent.push(command)}
        snapshot={snap}
      />
    </AssistantCopy>,
  );
}

describe("the agent of a conversation", () => {
  it("offers no choice with one agent, as before", () => {
    view(snapshot({}, [{ id: "default", name: "Vega" }], "default"));
    expect(screen.queryByRole("radiogroup")).toBeNull();
    expect(screen.queryByText(/Conversación con/)).toBeNull();
  });

  it("lets a new conversation pick its agent, the default one first", async () => {
    const sent: AppCommand[] = [];
    view(snapshot({}), sent);
    const picker = screen.getByRole("radiogroup", { name: "Hablar con" });
    const options = within(picker).getAllByRole("radio");
    expect(options).toEqual([
      within(picker).getByRole("radio", { name: "Deimos" }),
      within(picker).getByRole("radio", { name: "Atlas" }),
    ]);
    expect(options[0]?.getAttribute("aria-checked")).toBe("true");
    await userEvent.click(within(picker).getByRole("radio", { name: "Atlas" }));
    expect(sent).toEqual([{ command: "new_session", agent_id: "atlas" }]);
  });

  it("says whose an open conversation is and does not change its agent", () => {
    view(snapshot({ session_id: "session-atlas", agent_id: "atlas", activity: "thinking" }));
    expect(screen.queryByRole("radiogroup")).toBeNull();
    expect(screen.getByRole("heading", { name: "Atlas" })).toBeTruthy();
    expect(screen.getByText("Atlas está pensando…")).toBeTruthy();
  });

  it("puts the conversation's agent's name above its answer", async () => {
    const answer: ChatMessage = {
      id: "answer-atlas",
      role: "assistant",
      text: "La sala Sur está libre.",
      at_ms: 1_700_000_000_000,
      streaming: false,
      voice: null,
      receipts: [],
    };
    render(
      <AssistantCopy name="Atlas">
        <AssistantView
          error={null}
          readMessages={() =>
            Promise.resolve({
              state: "present",
              value: { items: [answer], next: null, total: 1, revision: 1 },
              as_of_ms: null,
            })
          }
          send={() => undefined}
          snapshot={snapshot({ session_id: "session-atlas", agent_id: "atlas" })}
        />
      </AssistantCopy>,
    );
    const article = (await screen.findByText(/sala Sur/)).closest("article") as HTMLElement;
    expect(within(article).getByText("Atlas")).toBeTruthy();
    expect(within(article).queryByText("ORBIT")).toBeNull();
  });

  it("names the open conversation's agent, or the default one, or the hub's name", () => {
    expect(conversationAgentName(snapshot({ agent_id: "atlas" }))).toBe("Atlas");
    expect(conversationAgentName(snapshot({}))).toBe("Deimos");
    expect(conversationAgentName(snapshot({}, TWO, "atlas"))).toBe("Atlas");
    // Before the daemon says which agents there are.
    expect(conversationAgentName(snapshot({}, [], undefined))).toBe("Deimos");
  });

  it("outside a conversation names the primary, whoever's conversation is open", () => {
    expect(primaryAgentName(snapshot({ agent_id: "atlas" }))).toBe("Deimos");
    expect(primaryAgentName(snapshot({ agent_id: "deimos" }, TWO, "atlas"))).toBe("Atlas");
    // Before the daemon says which agents there are.
    expect(primaryAgentName(snapshot({}, [], undefined))).toBe("Deimos");
  });
});

describe("the list of conversations", () => {
  const sessions: ChatSessionSummary[] = [
    { id: "a", created_at_ms: 1_789_000_000_000, last_at_ms: 1_789_000_500_000, message_count: 4 },
    {
      id: "b",
      agent_id: "atlas",
      created_at_ms: 1_788_000_000_000,
      last_at_ms: 1_788_000_000_000,
      message_count: 2,
    },
    { id: "c", created_at_ms: 1_787_000_000_000, last_at_ms: 1_787_000_000_000, message_count: 1 },
  ];
  const read = () =>
    Promise.resolve({
      state: "present" as const,
      value: { items: sessions, next: null, total: sessions.length, revision: 1 },
      as_of_ms: null,
    });

  it("says each conversation's agent and opens it with that agent", async () => {
    const sent: AppCommand[] = [];
    render(
      <ShellSlots>
        <ColumnHost />
        <AssistantSessions
          read={read}
          send={(command) => sent.push(command)}
          snapshot={snapshot({ session_id: "a" })}
        />
      </ShellSlots>,
    );
    await screen.findAllByRole("link", { name: /mensaje/ });
    const recent = screen
      .getAllByRole("link", { name: /mensaje/ })
      .filter((row) => row.getAttribute("aria-current") !== "page");
    expect(recent.map((row) => row.textContent)).toEqual([
      expect.stringContaining("Atlas"),
      expect.stringContaining("Deimos"),
    ]);
    await userEvent.click(recent[0] as HTMLElement);
    await userEvent.click(recent[1] as HTMLElement);
    expect(sent).toEqual([
      { command: "session_open", id: "b", agent_id: "atlas" },
      { command: "session_open", id: "c" },
    ]);
  });

  it("names no agent when the hub has one", async () => {
    render(
      <ShellSlots>
        <ColumnHost />
        <AssistantSessions
          read={read}
          send={() => undefined}
          snapshot={snapshot({ session_id: "a" }, [{ id: "default", name: "Vega" }], "default")}
        />
      </ShellSlots>,
    );
    const rows = await screen.findAllByRole("link", { name: /mensaje/ });
    for (const row of rows) {
      expect(row.textContent).not.toContain("Vega");
      expect(row.getAttribute("aria-label")).not.toContain("Vega");
    }
  });
});

describe("the agents as frames arrive", () => {
  it("keeps the last agents when a status frame says none", () => {
    const status = (settings: object) =>
      ({
        feature: "status",
        revision: { epoch: 1, counter: 2 },
        view: { settings, notices: [], notice: null },
      }) as unknown as FeatureFrame;
    const known = applyFrame(snapshot({}), status({ agents: TWO, default_agent_id: "atlas" }));
    expect(known.settings.agents).toEqual(TWO);
    expect(known.settings.default_agent_id).toBe("atlas");
    const kept = applyFrame(known, status({ agents: [] }));
    expect(kept.settings.agents).toEqual(TWO);
    expect(kept.settings.default_agent_id).toBe("atlas");
  });
});

describe("«Escuchar» and the agents' voices", () => {
  // The hub reads an agent without a voice of its own in the default agent's.
  const answer: ChatMessage = {
    id: "answer-voz",
    role: "assistant",
    text: "La sala Norte tiene 8 plazas.",
    at_ms: 1_700_000_000_000,
    streaming: false,
    voice: null,
    receipts: [],
  };

  async function offersListening(agents: Agent[], agentId?: string): Promise<boolean> {
    render(
      <AssistantView
        error={null}
        readMessages={() =>
          Promise.resolve({
            state: "present",
            value: { items: [answer], next: null, total: 1, revision: 1 },
            as_of_ms: null,
          })
        }
        send={() => undefined}
        snapshot={snapshot({ session_id: "session-voz", agent_id: agentId }, agents)}
      />,
    );
    const article = (await screen.findByText(/sala Norte/)).closest("article") as HTMLElement;
    const offered = within(article).queryByRole("button", { name: "Escuchar" }) !== null;
    cleanup();
    return offered;
  }

  it("is not offered when neither the conversation's agent nor the default one has a voice", async () => {
    const mute = [
      { id: "deimos", name: "Deimos", voice: false },
      { id: "atlas", name: "Atlas", voice: false },
    ];
    expect(await offersListening(mute, "atlas")).toBe(false);
    expect(await offersListening(mute)).toBe(false);
  });

  it("is offered with the conversation's own voice or the default agent's", async () => {
    const own = [
      { id: "deimos", name: "Deimos", voice: false },
      { id: "atlas", name: "Atlas", voice: true },
    ];
    expect(await offersListening(own, "atlas")).toBe(true);
    const borrowed = [
      { id: "deimos", name: "Deimos", voice: true },
      { id: "atlas", name: "Atlas", voice: false },
    ];
    expect(await offersListening(borrowed, "atlas")).toBe(true);
  });

  it("is offered as before when the agents do not say", async () => {
    // A hub without agents (0.9.0) has one that says nothing of its voice;
    // before the daemon says, there are none.
    expect(await offersListening([{ id: "default", name: "Vega" }])).toBe(true);
    expect(await offersListening([])).toBe(true);
  });
});
