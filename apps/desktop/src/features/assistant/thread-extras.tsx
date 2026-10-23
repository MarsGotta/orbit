import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import type { DaemonPort } from "../../bridge/port";
import { Button } from "../../design/Button";
import { Loading } from "../../design/Spinner";
import { useSpanish } from "../../i18n/AssistantCopy";
import { assistantsCopy } from "../../i18n/es/assistants";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { agentsOf } from "./agents";
import { AssistantHome } from "./AssistantHome";
import { healthOf } from "./assistants";
import { ChooseAssistant } from "./ChooseAssistant";
import { LimitCard } from "./LimitCard";
import { ThreadSkeleton, TurnFailure, turnFailureCode } from "./ThreadParts";
import { retryText } from "./conversation-markdown";
import { SignInNotice } from "./SignInNotice";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { useConversation } from "./useConversation";
import styles from "./AssistantView.module.css";

type Send = (command: AppCommand) => unknown;
type History = ReturnType<typeof useConversation>;

/** Whom the conversation is with, and what the thread needs to say about it. */
export interface With {
  agent?: Agent;
  port?: AgentPort;
  readSessions?: DaemonPort["readSessionsPage"];
  nowMs: number;
  /**
   * What an empty thread shows: «¿Con quién quieres hablar?» while a new
   * conversation can still change assistant, or the assistant's own page.
   */
  empty: "choose" | "home";
  handedFrom?: Agent;
}

/** A turn stopped at the plan's limit: the card, with the hub's time or the agent's. */
export function Limit({
  snapshot,
  question,
  with: talk,
  send,
}: {
  snapshot: AppSnapshot;
  /** The question left without its answer, sent again once the plan is back. */
  question: string | null;
  with: With;
  send: Send;
}) {
  const { agent } = talk;
  if (agent === undefined) return null;
  const said = Number(snapshot.notice?.data?.resets_at_ms);
  const health = healthOf(agent, talk.nowMs);
  const until =
    Number.isFinite(said) && said > 0 ? said : health.kind === "limit" ? health.until : undefined;
  return (
    <LimitCard
      agent={agent}
      agents={agentsOf(snapshot)}
      canHandOver={snapshot.settings.addable_agents !== undefined}
      nowMs={talk.nowMs}
      primaryId={snapshot.settings.default_agent_id}
      question={question}
      send={send}
      sessionId={snapshot.chat.session_id}
      until={until}
    />
  );
}

/** A thread with nothing said: «¿Con quién quieres hablar?», or the assistant's own page. */
export function EmptyPage({
  snapshot,
  send,
  with: talk,
}: {
  snapshot: AppSnapshot;
  send: Send;
  with: With;
}) {
  const es = useSpanish();
  if (talk.empty === "choose")
    return <ChooseAssistant nowMs={talk.nowMs} port={talk.port} send={send} snapshot={snapshot} />;
  return (
    <AssistantHome
      // Before the hub says its agents, the one it speaks with by name.
      agent={talk.agent ?? { id: "", name: es.chat.roles.assistant }}
      nowMs={talk.nowMs}
      port={talk.port}
      readSessions={talk.readSessions}
      send={send}
      snapshot={snapshot}
    />
  );
}

/** «Sigues con Deimos · le hemos pasado esta conversación», across the thread. */
export function HandoverLine({ with: talk }: { with: With }) {
  if (talk.handedFrom === undefined || talk.agent === undefined) return null;
  return (
    <p className={styles.handover} role="note">
      <span>{assistantsCopy.handedOver(talk.agent.name)}</span>
    </p>
  );
}

/** «Cargar mensajes anteriores», while they load, or why they could not. */
function OlderMessages({ history }: { history: History }) {
  const es = useSpanish();
  return (
    <>
      {history.hasOlder && (
        <Button className={styles.older} onClick={history.loadOlder} size={28} variant="ghost">
          {es.assistant.loadOlder}
        </Button>
      )}
      {history.loading && <Loading size={12}>{es.assistant.loadingOlder}</Loading>}
      {history.error !== null && (
        <p className={styles.historyError} role="alert">
          {history.error}
        </p>
      )}
      {history.error !== null && history.firstPage === "failed" && (
        <Button className={styles.older} onClick={history.retry} size={28}>
          {es.retry}
        </Button>
      )}
    </>
  );
}

/** Above the messages: older pages, the shape while reading, the page of an empty thread, a handover. */
export function ThreadHead({
  history,
  empty,
  snapshot,
  send,
  with: talk,
}: {
  history: History;
  empty: boolean;
  snapshot: AppSnapshot;
  send: Send;
  with: With;
}) {
  return (
    <>
      <OlderMessages history={history} />
      {history.firstPage === "reading" && <ThreadSkeleton />}
      {empty && <EmptyPage send={send} snapshot={snapshot} with={talk} />}
      <HandoverLine with={talk} />
    </>
  );
}

/**
 * How the last turn ended when it did not answer: the plan's limit with its
 * offers, or the cause with «Volver a preguntar»; and the session the
 * assistant needs, unless its own page already says so.
 */
export function TurnEnd({
  snapshot,
  messages,
  failure,
  empty,
  send,
  with: talk,
}: {
  snapshot: AppSnapshot;
  messages: ChatMessage[];
  failure: string | null;
  empty: boolean;
  send: Send;
  with: With;
}) {
  const question = retryText(messages, messages.length);
  const limited = failure !== null && turnFailureCode(snapshot) === "agent_limit_reached";
  return (
    <>
      {limited ? (
        <Limit question={question} send={send} snapshot={snapshot} with={talk} />
      ) : (
        failure !== null && <TurnFailure retry={question} send={send} text={failure} />
      )}
      {talk.agent !== undefined && !empty && (
        <SignInNotice
          agent={talk.agent}
          agents={agentsOf(snapshot)}
          nowMs={talk.nowMs}
          port={talk.port}
          send={send}
        />
      )}
    </>
  );
}
