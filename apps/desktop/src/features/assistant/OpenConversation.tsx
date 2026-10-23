import type { AppCommand } from "../../bridge/generated";
import type { DaemonPort } from "../../bridge/port";
import { useSpanish } from "../../i18n/AssistantCopy";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { StepWait } from "./AgentWork";
import { ConversationHeader } from "./ConversationHeader";
import { ConversationTools } from "./ConversationTools";
import { turnInFlight } from "./conversation";
import { MessageList } from "./MessageList";
import { PendingCard } from "./PendingCard";
import { ActivityLine, turnFailure, turnQuestion } from "./ThreadParts";
import { readsAloud } from "./agents";
import { ThreadHead, TurnEnd, type With } from "./thread-extras";
import { useConversation } from "./useConversation";
import styles from "./AssistantView.module.css";

type Send = (command: AppCommand) => unknown;

/** The answer given to the approval of a turn, remembered once the card has gone. */
export type Decision = { turn: string; choice: string };

/** The turn the snapshot carries: its work's, or its question's. */
export function currentTurn(chat: AppSnapshot["chat"]): string | null {
  return chat.work?.turn_id ?? turnQuestion(chat)?.id ?? null;
}

type History = ReturnType<typeof useConversation>;

/** The thread hangs from the bottom: its top is the farthest scroll. */
const atTop = (box: HTMLElement) =>
  Math.abs(box.scrollTop) >= box.scrollHeight - box.clientHeight - 16;

/** The answer given in this turn, and what its running step waits for. */
function waitOf(chat: AppSnapshot["chat"], decision: Decision | null) {
  const decided = decision !== null && decision.turn === currentTurn(chat) ? decision.choice : null;
  const wait: StepWait =
    chat.pending?.kind === "approval" ? "approval" : decided === "deny" ? "denied" : null;
  return { decided, wait };
}

/** After the messages: what the agent does or waits for, what was answered, a failure. */
function Tail({
  decision,
  history,
  onDecided,
  send,
  snapshot,
  with: talk,
  empty,
}: {
  decision: string | null;
  history: History;
  onDecided: (choice: string) => void;
  send: Send;
  snapshot: AppSnapshot;
  with: With;
  /** The thread is empty: the assistant's page already says what its session needs. */
  empty: boolean;
}) {
  const es = useSpanish();
  const { chat } = snapshot;
  const { messages } = history;
  const working = chat.work != null && chat.work.ended_at_ms == null && chat.work.items.length > 0;
  const streaming = messages.some((message) => message.streaming);
  const failure = turnFailure(snapshot, messages, es);
  return (
    <>
      {(chat.activity === "transcribing" || (!working && !streaming)) && (
        <ActivityLine activity={chat.activity} />
      )}
      {chat.pending && <PendingCard onDecided={onDecided} pending={chat.pending} send={send} />}
      {decision !== null && chat.pending === null && (
        <p className={styles.decision}>{es.assistant.decided[decision]}</p>
      )}
      <TurnEnd
        empty={empty}
        failure={failure}
        messages={messages}
        send={send}
        snapshot={snapshot}
        with={talk}
      />
    </>
  );
}

/** Nothing in the thread once its first page is read, and nothing under way. */
const isEmpty = (history: History, chat: AppSnapshot["chat"]) =>
  history.firstPage === "read" && history.messages.length === 0 && chat.activity === undefined;

/** The thread, hanging from the bottom: older pages above, the turn and what it waits for below. */
function Thread({
  decision,
  history,
  onDecided,
  send,
  snapshot,
  with: talk,
}: {
  decision: Decision | null;
  history: History;
  onDecided: (choice: string) => void;
  send: Send;
  snapshot: AppSnapshot;
  with: With;
}) {
  const es = useSpanish();
  const { chat } = snapshot;
  const { messages } = history;
  const inFlight = turnInFlight(chat);
  const { decided, wait } = waitOf(chat, decision);
  const empty = isEmpty(history, chat);
  return (
    <div
      className={styles.scroller}
      data-empty={empty || undefined}
      data-testid="assistant-conversation"
      onScroll={(event) => atTop(event.currentTarget) && history.loadOlder()}
    >
      <div
        aria-busy={history.firstPage === "reading" || undefined}
        aria-label={es.assistant.thread}
        className={styles.thread}
        role="log"
      >
        <ThreadHead empty={empty} history={history} send={send} snapshot={snapshot} with={talk} />
        <MessageList
          busy={inFlight}
          listenable={readsAloud(snapshot, chat.agent_id)}
          messages={messages}
          send={send}
          speaking={chat.speaking ?? null}
          voiceAudio={chat.voice_audio}
          wait={wait}
          work={chat.work}
          workInFlight={inFlight}
        >
          <Tail
            decision={decided}
            history={history}
            onDecided={onDecided}
            send={send}
            empty={empty}
            snapshot={snapshot}
            with={talk}
          />
        </MessageList>
      </div>
    </div>
  );
}

/** The open conversation: its header and its thread, read from the hub. */
export function Conversation({
  calling,
  canCall,
  onCall,
  readMessages,
  ...thread
}: {
  calling: boolean;
  canCall: boolean;
  decision: Decision | null;
  onCall: () => void;
  onDecided: (choice: string) => void;
  readMessages?: DaemonPort["readSessionMessages"];
  send: Send;
  snapshot: AppSnapshot;
  with: With;
}) {
  const { chat } = thread.snapshot;
  const history = useConversation(chat, readMessages);
  const { messages } = history;
  const began =
    history.firstPage === "read" && !history.hasOlder ? (messages[0]?.at_ms ?? null) : null;
  return (
    <>
      <ConversationHeader
        agent={thread.with.agent}
        began={began}
        calling={calling}
        canCall={canCall}
        choosing={
          thread.with.empty === "choose" &&
          history.messages.length === 0 &&
          thread.snapshot.chat.activity === undefined
        }
        handedFrom={thread.with.handedFrom}
        onCall={onCall}
        port={thread.with.port}
        send={thread.send}
        snapshot={thread.snapshot}
      >
        <ConversationTools
          inFlight={turnInFlight(chat)}
          messages={messages}
          read={readMessages}
          sessionId={chat.session_id}
          turn={chat.turn}
        />
      </ConversationHeader>
      <Thread history={history} {...thread} />
    </>
  );
}
