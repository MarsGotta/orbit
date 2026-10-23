import { useEffect, useRef } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import type { DaemonPort } from "../../bridge/port";
import { currentHubConnection, hasSavedPairing } from "../../bridge/hub-state";
import { AssistantAvatar } from "../../design/AssistantAvatar";
import { assistantsCopy } from "../../i18n/es/assistants";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { AssistantComposer } from "./AssistantComposer";
import { goTo, parseAssistantRoute, type AssistantRoute } from "./assistant-route";
import { colorOf, composerBlock } from "./assistants";
import { agentFor, agentsOf } from "./agents";
import { useHandedOverFrom } from "./handover";
import type { With } from "./thread-extras";
import styles from "./AssistantView.module.css";

/** Nothing said yet in the open conversation. */
export function isFresh(chat: AppSnapshot["chat"]): boolean {
  return chat.session_id === null && chat.turn.length === 0 && chat.activity === undefined;
}

/**
 * The route and the open conversation agree. Arriving at an assistant's own
 * page (or at «Nueva») opens a new conversation with it, once; once a new
 * conversation has its id, the address names it.
 */
export function useRouteFollowsChat(
  route: AssistantRoute,
  snapshot: AppSnapshot,
  send: (command: AppCommand) => unknown,
) {
  const { chat } = snapshot;
  const latest = useRef(snapshot);
  latest.current = snapshot;
  const key = route.screen === "agent" ? `agent:${route.agentId}` : route.screen;
  useEffect(() => {
    const now = latest.current;
    const fresh = isFresh(now.chat);
    const talking = now.chat.agent_id ?? now.settings.default_agent_id;
    if (route.screen === "agent" && !(fresh && talking === route.agentId))
      void send({ command: "new_session", agent_id: route.agentId });
    if (route.screen === "new" && !fresh) void send({ command: "new_session" });
    // Only arriving here asks: what the conversation does next does not.
  }, [key]);
  const session = chat.session_id;
  const previous = useRef(session);
  useEffect(() => {
    if (previous.current === session) return;
    previous.current = session;
    const here = parseAssistantRoute(window.location.hash);
    if (session !== null && ["conversation", "agent", "new"].includes(here.screen))
      goTo({ screen: "conversation", sessionId: session }, { replace: true });
    if (session === null && here.screen === "conversation" && here.sessionId !== undefined)
      goTo({ screen: "conversation" }, { replace: true });
  }, [session]);
}

/** Who the thread is with, and what it shows before its first message. */
export function useWith(
  props: {
    snapshot: AppSnapshot;
    agent?: AgentPort;
    readSessions?: DaemonPort["readSessionsPage"];
  },
  route: AssistantRoute,
  nowMs: number,
): With {
  const { snapshot } = props;
  const agents = agentsOf(snapshot);
  const chatAgent = agentFor(snapshot, snapshot.chat.agent_id);
  const routeAgent =
    route.screen === "agent" ? agents.find((agent) => agent.id === route.agentId) : undefined;
  const handedFromId = useHandedOverFrom(snapshot.chat.session_id);
  // Only a conversation not begun can still change its assistant.
  const choose = agents.length > 1 && route.screen !== "agent" && snapshot.chat.session_id === null;
  return {
    agent: routeAgent ?? chatAgent,
    port: props.agent,
    readSessions: props.readSessions,
    nowMs,
    empty: choose ? "choose" : "home",
    handedFrom: agents.find((agent) => agent.id === handedFromId),
  };
}

/** «Hablar con [Deimos]» above the box while the assistant is being chosen. */
function TalkingTo({ agent, agents }: { agent: Agent; agents: readonly Agent[] }) {
  return (
    <p className={styles.talkingTo}>
      <span>{assistantsCopy.talkingTo}</span>
      <span className={styles.talkingChip}>
        <AssistantAvatar color={colorOf(agent, agents)} name={agent.name} size={18} />
        {agent.name}
      </span>
    </p>
  );
}

/** The composer of the view: what stops it sending to this assistant, «Hablar con» while choosing. */
export function ViewComposer({
  snapshot,
  send,
  talk,
}: {
  snapshot: AppSnapshot;
  send: (command: AppCommand) => unknown;
  talk: With;
}) {
  const { chat } = snapshot;
  return (
    <AssistantComposer
      agentBlocked={composerBlock(talk.agent, talk.nowMs)}
      attachmentLimits={snapshot.settings.attachments}
      chat={chat}
      daemonReady={snapshot.connection.daemon === "ready"}
      hub={currentHubConnection(snapshot)}
      paired={hasSavedPairing(snapshot)}
      send={send}
      talkingTo={
        talk.empty === "choose" && isFresh(chat) && talk.agent !== undefined ? (
          <TalkingTo agent={talk.agent} agents={agentsOf(snapshot)} />
        ) : undefined
      }
    />
  );
}
