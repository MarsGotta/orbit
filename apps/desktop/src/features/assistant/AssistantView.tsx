import { useEffect, useState } from "react";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import type { DaemonPort } from "../../bridge/port";
import { currentHubConnection, hasSavedPairing } from "../../bridge/hub-state";
import { Icon } from "../../design/Icon";
import { Toast } from "../../design/Toast";
import { ViewError } from "../../design/ViewError";
import { AssistantCopy, useSpanish } from "../../i18n/AssistantCopy";
import { useNow } from "../../use-now";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { AddAssistant } from "./AddAssistant";
import { conversationAgentName } from "./agents";
import { UnknownAssistant, useAssistantCall } from "./AssistantAvailability";
import { AssistantSettingsPage } from "./AssistantSettingsPage";
import { goTo, useAssistantRoute, type AssistantRoute } from "./assistant-route";
import { useRouteFollowsChat, useWith, ViewComposer } from "./assistant-screen";
import { CallView, MinimisedCall } from "./CallView";
import { Conversation, currentTurn, type Decision } from "./OpenConversation";
import { ConversationHeader } from "./ConversationHeader";
import { useHandoverBinding } from "./handover";
import type { With } from "./thread-extras";
import { turnQuestion } from "./ThreadParts";
import styles from "./AssistantView.module.css";

// The limit's time and the dates turn over while the view is open: read the clock each half minute.
const CLOCK_MS = 30_000;

interface AssistantViewProps {
  error: string | null;
  send: (command: AppCommand) => unknown;
  snapshot: AppSnapshot;
  readMessages?: DaemonPort["readSessionMessages"];
  readSessions?: DaemonPort["readSessionsPage"];
  /** The assistants' own calls to the hub: profile, adding, signing in. */
  agent?: AgentPort;
}

/** A Mac without its hub: how to talk to the agent, and the way to pair. */
function Unpaired() {
  const es = useSpanish();
  return (
    <div className={styles.unpaired}>
      <span className={styles.unpairedIcon}>
        <Icon name="assistant" size={20} />
      </span>
      <p>{es.assistant.unpaired}</p>
      <a className={styles.pairLink} href="#/settings">
        {es.assistant.pairMac}
      </a>
    </div>
  );
}

/** What the user last said in the call: the turn's question, typed or spoken. */
function lastQuestion(chat: AppSnapshot["chat"]): string | null {
  const question = turnQuestion(chat);
  const said = question?.voice ? (question.voice.transcript ?? "") : (question?.text ?? "");
  return said === "" ? null : said;
}

type Calls = ReturnType<typeof useAssistantCall>;

/** The conversation, or how to pair this Mac; the composer under it; the call minimised. */
function ConversationArea({
  calling,
  calls,
  onOpenCall,
  talk,
  ...props
}: AssistantViewProps & { calling: boolean; calls: Calls; onOpenCall: () => void; talk: With }) {
  const { error, readMessages, send, snapshot } = props;
  const { call, chat } = snapshot;
  const [decision, setDecision] = useState<Decision | null>(null);
  const paired = hasSavedPairing(snapshot);
  const statusKnown =
    snapshot.status_availability?.state !== "unknown" &&
    snapshot.status_availability?.state !== "absent";
  const unpaired = snapshot.connection.daemon === "ready" && !paired && statusKnown;
  return (
    <>
      {unpaired ? (
        <>
          <ConversationHeader
            began={null}
            calling={false}
            canCall={false}
            onCall={calls.startCall}
            send={send}
            snapshot={snapshot}
          />
          <Unpaired />
        </>
      ) : (
        <Conversation
          calling={calling}
          canCall={calls.available && !calls.callRequested}
          decision={decision}
          key={chat.session_id ?? "no-session"}
          onCall={calling ? onOpenCall : calls.startCall}
          onDecided={(choice) => {
            const turn = currentTurn(chat);
            if (turn !== null) setDecision({ turn, choice });
          }}
          readMessages={readMessages}
          send={send}
          snapshot={snapshot}
          with={talk}
        />
      )}
      <ViewError error={error} />
      <ViewComposer send={send} snapshot={snapshot} talk={talk} />
      {calling && <MinimisedCall call={call} onHangUp={calls.endCall} onOpen={onOpenCall} />}
    </>
  );
}

/** The Assistant once it is known: an assistant's settings, the call in its own view, or the conversation. */
function AssistantScreen({
  route,
  nowMs,
  ...props
}: AssistantViewProps & { route: AssistantRoute; nowMs: number }) {
  const es = useSpanish();
  const { error, send, snapshot } = props;
  const { call, chat } = snapshot;
  const talk = useWith(props, route, nowMs);
  const calls = useAssistantCall({
    error,
    send,
    snapshot,
    paired: hasSavedPairing(snapshot),
    hubConnection: currentHubConnection(snapshot),
  });
  const [minimised, setMinimised] = useState(false);
  const calling = calls.callRequested || call !== null;
  useEffect(() => {
    if (!calling) setMinimised(false);
  }, [calling]);
  return (
    <section
      className={styles.view}
      data-call-screen={call?.seeing_screen ?? false}
      data-minimised-call={(calling && minimised) || undefined}
    >
      <h1 className={styles.visuallyHidden}>{es.shell.assistant}</h1>
      {calling && !minimised ? (
        <CallView
          call={call}
          lastQuestion={lastQuestion(chat)}
          onHangUp={calls.endCall}
          onMinimise={() => setMinimised(true)}
          onMute={(muted) => send({ command: "microphone_mute", muted })}
        />
      ) : (
        <ConversationArea
          {...props}
          calling={calling}
          calls={calls}
          onOpenCall={() => setMinimised(false)}
          talk={talk}
        />
      )}
      {route.screen === "add" && (
        <AddAssistant
          onClose={() => goTo({ screen: "conversation" })}
          port={props.agent}
          send={send}
          snapshot={snapshot}
        />
      )}
      {calls.toast !== null && calls.toast !== "" && (
        <Toast message={calls.toast} onClose={() => calls.setToast(null)} />
      )}
    </section>
  );
}

/**
 * The Assistant: the open conversation with its agent, the composer fixed
 * under it, and the call with the agent, in its own calm view or minimised
 * to a card over the conversation. Before the first message, «¿Con quién
 * quieres hablar?» or the chosen assistant's own page; each assistant's
 * settings behind its gear; «Añadir un asistente» over it all. Not known
 * yet, it shows that it reads.
 */
export default function AssistantView(props: AssistantViewProps) {
  // Outside this view every text names the primary; here, the conversation's agent.
  return (
    <AssistantCopy name={conversationAgentName(props.snapshot)}>
      {props.snapshot.chat.availability?.state === "unknown" ? (
        <UnknownAssistant snapshot={props.snapshot} />
      ) : (
        <Routed {...props} />
      )}
    </AssistantCopy>
  );
}

/** Where the route says: an assistant's settings, or the conversation; the route follows it. */
function Routed(props: AssistantViewProps) {
  const { send, snapshot } = props;
  const route = useAssistantRoute();
  const nowMs = useNow(CLOCK_MS);
  useRouteFollowsChat(route, snapshot, send);
  useHandoverBinding(snapshot.chat);
  if (route.screen !== "settings")
    return <AssistantScreen {...props} nowMs={nowMs} route={route} />;
  return (
    <section className={styles.view}>
      <AssistantSettingsPage
        agentId={route.agentId}
        nowMs={nowMs}
        port={props.agent}
        send={send}
        snapshot={snapshot}
      />
    </section>
  );
}
