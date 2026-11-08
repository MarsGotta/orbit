import { useEffect } from "react";
import { create } from "zustand";
import type { ChatSnapshot } from "../../bridge/generated/ChatSnapshot";

/**
 * What this run of Orbit knows about conversations passed to another
 * assistant at its plan's limit (proposal 30), and the questions waiting for
 * a plan to come back. Kept in memory only: the hub keeps the conversations.
 */
interface Handovers {
  /** Asked, not yet opened: the conversation it leaves and who takes it. */
  pending: { fromSession: string; fromAgent: string; toAgent: string } | null;
  /** Each conversation that continues another, by the agent it continues from. */
  from: Record<string, string>;
  /** «Esperar a las 18:40»: the conversation whose question goes once its agent is back. */
  waiting: Record<string, { agentId: string; until: number; question: string }>;
}

export const useHandovers = create<Handovers>(() => ({ pending: null, from: {}, waiting: {} }));

export function handingOver(fromSession: string, fromAgent: string, toAgent: string): void {
  useHandovers.setState({ pending: { fromSession, fromAgent, toAgent } });
}

export function waitFor(
  sessionId: string,
  wait: { agentId: string; until: number; question: string },
): void {
  useHandovers.setState((state) => ({ waiting: { ...state.waiting, [sessionId]: wait } }));
}

export function stopWaiting(sessionId: string): void {
  useHandovers.setState((state) => ({
    waiting: Object.fromEntries(
      Object.entries(state.waiting).filter(([session]) => session !== sessionId),
    ),
  }));
}

/**
 * Binds the conversation a handover opened to the one it continues, once the
 * daemon says it is open: a new conversation, with the agent that took it.
 */
export function useHandoverBinding(chat: Pick<ChatSnapshot, "session_id" | "agent_id">): void {
  const pending = useHandovers((state) => state.pending);
  useEffect(() => {
    const session = chat.session_id;
    if (pending === null || session === null || session === pending.fromSession) return;
    if (chat.agent_id !== pending.toAgent) return;
    useHandovers.setState((state) => ({
      pending: null,
      from: { ...state.from, [session]: pending.fromAgent },
    }));
  }, [chat.agent_id, chat.session_id, pending]);
}

/** Refresh the in-memory hint from the hub when this conversation opens. */
export function rememberHandover(sessionId: string, fromAgent: string | undefined): void {
  if (fromAgent === undefined) return;
  useHandovers.setState((state) => ({ from: { ...state.from, [sessionId]: fromAgent } }));
}

/** The agent the open conversation continues from, read from the hub or this run's handover. */
export function useHandedOverFrom(sessionId: string | null): string | undefined {
  return useHandovers((state) => (sessionId === null ? undefined : state.from[sessionId]));
}
