import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import type { DaemonPort } from "../../bridge/port";
import { currentHubConnection, hasSavedPairing } from "../../bridge/hub-state";
import { Button } from "../../design/Button";
import { Orb } from "../../design/Orb";
import { Toast } from "../../design/Toast";
import { ViewError } from "../../design/ViewError";
import { es } from "../../i18n/es";
import { useSpanish } from "../../i18n/AssistantCopy";
import { AssistantComposer } from "./AssistantComposer";
import { ConversationTools } from "./ConversationTools";
import { turnInFlight } from "./conversation";
import { UnknownAssistant, useAssistantCall } from "./AssistantAvailability";
import { MessageList } from "./MessageList";
import { PendingCard } from "./PendingCard";
import { agentFor, agentsOf } from "./agents";
import { useConversation } from "./useConversation";
import styles from "./AssistantView.module.css";

interface AssistantViewProps {
  error: string | null;
  send: (command: AppCommand) => void;
  snapshot: AppSnapshot;
  readMessages?: DaemonPort["readSessionMessages"];
}

/**
 * With several agents, whose the conversation is: a new one, before its
 * first message, picks its agent; an open one says it and keeps it. With
 * one agent there is nothing to say.
 */
function ConversationAgent({
  snapshot,
  send,
}: {
  snapshot: AppSnapshot;
  send: AssistantViewProps["send"];
}) {
  const agents = agentsOf(snapshot);
  const { chat } = snapshot;
  const current = agentFor(snapshot, chat.agent_id);
  if (agents.length < 2 || !current) return null;
  if (chat.session_id !== null || chat.turn.length > 0)
    return <span>{es.assistant.conversationWith(current.name)}</span>;
  return (
    <label className={styles.agentPicker}>
      <span>{es.assistant.agentPicker}</span>
      <select
        onChange={(event) => send({ command: "new_session", agent_id: event.target.value })}
        value={current.id}
      >
        {agents.map((agent) => (
          <option key={agent.id} value={agent.id}>
            {agent.name}
          </option>
        ))}
      </select>
    </label>
  );
}

function AssistantHeader({
  snapshot,
  send,
}: {
  snapshot: AppSnapshot;
  send: AssistantViewProps["send"];
}) {
  const { chat } = snapshot;
  // The list names a conversation by its date; here it is the open one.
  const title =
    chat.session_id === null ? es.assistant.emptySessionTitle : es.assistant.openSessionTitle;
  const streaming = chat.turn.some((message) => message.streaming);
  return (
    <header className={styles.header}>
      <strong>{title}</strong>
      <ConversationAgent send={send} snapshot={snapshot} />
      {streaming && (
        <Button onClick={() => send({ command: "interrupt" })} size={28}>
          {es.chat.interrupt}
        </Button>
      )}
    </header>
  );
}

function ActivityLine({ activity }: { activity: AppSnapshot["chat"]["activity"] }) {
  const es = useSpanish();
  if (!activity) return null;
  const labels = {
    transcribing: es.assistant.transcribingVoice,
    thinking: es.assistant.thinking,
    responding: es.assistant.responding,
  };
  return (
    <div className={styles.activity} role="status">
      <span aria-hidden="true" className={styles.activityOrb}>
        <i />
        <i />
        <i />
      </span>
      {labels[activity]}
    </div>
  );
}

function CallOrb({ call, onEnd }: { call: AppSnapshot["call"]; onEnd: () => void }) {
  return (
    <Orb
      endLabel={es.design.endCallFromOrb}
      lastLine={call?.last_line}
      onEnd={onEnd}
      seeingScreen={call?.seeing_screen}
      startedAtMs={call?.started_at_ms}
      voiceUnavailable={call?.voice_unavailable}
    />
  );
}

function ConversationThread({
  chat,
  readMessages,
  send,
}: {
  chat: AppSnapshot["chat"];
  readMessages?: DaemonPort["readSessionMessages"];
  send: AssistantViewProps["send"];
}) {
  const history = useConversation(chat, readMessages);
  return (
    <div
      className={styles.conversation}
      data-testid="assistant-conversation"
      onScroll={(event) => {
        if (event.currentTarget.scrollTop <= 16) history.loadOlder();
      }}
    >
      <div className={styles.thread}>
        {history.hasOlder && <Button onClick={history.loadOlder}>{es.assistant.loadOlder}</Button>}
        {history.loading && <p role="status">{es.assistant.loadingOlder}</p>}
        {history.error !== null && <p role="alert">{history.error}</p>}
        <ConversationTools
          inFlight={turnInFlight(chat)}
          messages={history.messages}
          read={readMessages}
          sessionId={chat.session_id}
          turn={chat.turn}
        />
        <MessageList
          busy={turnInFlight(chat)}
          messages={history.messages}
          send={send}
          speaking={chat.speaking ?? null}
          voiceAudio={chat.voice_audio}
        />
        <ActivityLine activity={chat.activity} />
        {chat.pending && <PendingCard pending={chat.pending} send={send} />}
      </div>
    </div>
  );
}

export default function AssistantView({ error, readMessages, send, snapshot }: AssistantViewProps) {
  const { call, chat } = snapshot;
  const hubConnection = currentHubConnection(snapshot);
  const paired = hasSavedPairing(snapshot);
  const { callRequested, toast, setToast, startCall, endCall } = useAssistantCall({
    error,
    send,
    snapshot,
    paired,
    hubConnection,
  });
  if (chat.availability?.state === "unknown") return <UnknownAssistant snapshot={snapshot} />;
  return (
    <section className={styles.view} data-call-screen={call?.seeing_screen ?? false}>
      <h1 className={styles.visuallyHidden}>{es.shell.assistant}</h1>
      <AssistantHeader send={send} snapshot={snapshot} />
      <ViewError error={error} />
      <ConversationThread
        chat={chat}
        key={chat.session_id ?? "no-session"}
        readMessages={readMessages}
        send={send}
      />
      <AssistantComposer
        callActive={Boolean(call)}
        callPending={callRequested}
        chat={chat}
        daemonReady={snapshot.connection.daemon === "ready"}
        hub={hubConnection}
        onCallToggle={call ? endCall : startCall}
        paired={paired}
        callUnknown={snapshot.call_availability?.state === "unknown"}
        voiceUnknown={chat.voice.availability?.state === "unknown"}
        statusUnknown={
          snapshot.status_availability?.state === "unknown" ||
          snapshot.status_availability?.state === "absent"
        }
        send={send}
      />
      {(callRequested || call) && <CallOrb call={call} onEnd={endCall} />}
      {toast !== null && toast !== "" && <Toast message={toast} onClose={() => setToast(null)} />}
    </section>
  );
}
