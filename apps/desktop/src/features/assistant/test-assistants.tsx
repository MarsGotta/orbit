// What the tests of several assistants share: Deimos, Phobos and Ares as the
// annex invents them, a snapshot with them, and the window's slots (its top
// bar and its list column) for the Assistant to fill.
import { render } from "@testing-library/react";
import type { ReactNode } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import type { AgentAnswer } from "../../bridge/generated/AgentAnswer";
import type { AgentCall } from "../../bridge/generated/AgentCall";
import type { AgentOutcome } from "../../bridge/generated/AgentOutcome";
import type { ChatSessionSummary } from "../../bridge/generated/ChatSessionSummary";
import type { Notice } from "../../bridge/generated/Notice";
import type { MessagesPage, SessionPage } from "../../bridge/reads";
import { ShellSlots, useShellSlots } from "../../app/shell-slots";
import { CopyProvider } from "../../i18n/CopyProvider";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { AssistantSessions } from "./AssistantSessions";
import AssistantView from "./AssistantView";
import { conversationAgentName } from "./agents";

// Wednesday 21 October 2026, 12:00 on the Mac's clock.
export const NOW = new Date(2026, 9, 21, 12, 0).getTime();
export const at = (day: number, hour: number, minute: number) =>
  new Date(2026, 9, day, hour, minute).getTime();

export const DEIMOS: Agent = {
  id: "deimos",
  name: "Deimos",
  voice: true,
  service: "hermes",
  description: "Tu día, tus reuniones y usar este Mac por ti",
  color: "c1",
  removable: false,
};
export const PHOBOS: Agent = {
  id: "codex-1a2b3c4d",
  name: "Phobos",
  voice: false,
  service: "codex",
  description: "Código, scripts y datos",
  color: "c2",
  removable: true,
  account: "signed_in",
  account_email: "lucia.ferrer@nubaria.example",
  plan: "ChatGPT Plus",
};
export const ARES: Agent = {
  id: "claude-5e6f7a8b",
  name: "Ares",
  voice: false,
  service: "claude-code",
  description: "Redactar, revisar contratos y documentos largos",
  color: "c3",
  removable: true,
  account: "expired",
  account_email: "lucia.ferrer@nubaria.example",
  plan: "Claude Pro",
};
export const THREE = [DEIMOS, PHOBOS, ARES];

export function summary(
  id: string,
  created: number,
  agent_id: string,
  count = 2,
): ChatSessionSummary {
  return { id, agent_id, created_at_ms: created, last_at_ms: created, message_count: count };
}

export function sessionsPage(items: ChatSessionSummary[]): SessionPage {
  return {
    state: "present",
    value: { items, next: null, total: items.length, revision: 1 },
    as_of_ms: null,
  };
}

export function assistantSnapshot(
  chat: Partial<AppSnapshot["chat"]> = {},
  options: {
    agents?: Agent[];
    primary?: string;
    notice?: Notice | null;
    addable?: string[] | null;
  } = {},
): AppSnapshot {
  const addable = options.addable === undefined ? ["codex", "claude-code"] : options.addable;
  return {
    connection: { daemon: "ready", hub: "paired" },
    status_availability: { state: "current" },
    call_availability: { state: "current" },
    call: null,
    notice: options.notice ?? null,
    computer_control: undefined,
    settings: {
      hub: { paired: true, connection: "online", url: "https://hub.example" },
      assistant_name: "Deimos",
      agents: options.agents ?? THREE,
      default_agent_id: options.primary ?? "deimos",
      ...(addable === null ? {} : { addable_agents: addable }),
      attachments: undefined,
      privacy_stopped: false,
    },
    chat: {
      session_id: null,
      agent_id: "deimos",
      turn: [],
      pending: null,
      voice: { level: 0, state: "idle", availability: { state: "current" } },
      availability: { state: "current" },
      ...chat,
    },
  } as unknown as AppSnapshot;
}

/** The window's top bar start and list column, where the Assistant draws its header and list. */
function Hosts() {
  const slots = useShellSlots();
  return (
    <>
      <div data-testid="top-bar" ref={slots?.setLeading} />
      <div data-testid="column" ref={slots?.setColumn} />
    </>
  );
}

export type AgentReply = (call: AgentCall) => AgentOutcome | Promise<AgentOutcome>;

export function renderAssistant(
  snapshot: AppSnapshot,
  options: {
    agent?: AgentReply;
    sessions?: ChatSessionSummary[];
    messages?: (session: string) => Promise<MessagesPage>;
    column?: boolean;
  } = {},
) {
  const sent: AppCommand[] = [];
  const calls: AgentCall[] = [];
  const reply = options.agent ?? (() => ({ outcome: "done" }) as AgentOutcome);
  const agent = async (call: AgentCall): Promise<AgentAnswer> => {
    calls.push(call);
    return { call, outcome: await reply(call) };
  };
  const readSessions = () => Promise.resolve(sessionsPage(options.sessions ?? []));
  const readMessages =
    options.messages ??
    (() =>
      Promise.resolve({
        state: "present",
        value: { items: [], next: null, total: 0, revision: 1 },
        as_of_ms: null,
      } as MessagesPage));
  const send = (command: AppCommand) => {
    sent.push(command);
    return Promise.resolve(true);
  };
  const tree = (current: AppSnapshot): ReactNode => (
    <ShellSlots>
      <Hosts />
      <CopyProvider name={conversationAgentName(current)}>
        <AssistantView
          agent={agent}
          error={null}
          readMessages={readMessages}
          readSessions={readSessions}
          send={send}
          snapshot={current}
        />
        {options.column !== false && (
          <AssistantSessions read={readSessions} send={send} snapshot={current} />
        )}
      </CopyProvider>
    </ShellSlots>
  );
  const view = render(tree(snapshot));
  return {
    sent,
    calls,
    rerender: (next: AppSnapshot) => view.rerender(tree(next)),
  };
}
