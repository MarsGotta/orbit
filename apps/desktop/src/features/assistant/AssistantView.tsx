import { useEffect, useState } from "react";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import type { DaemonPort } from "../../bridge/port";
import { currentHubConnection, hasSavedPairing } from "../../bridge/hub-state";
import { Icon } from "../../design/Icon";
import { Toast } from "../../design/Toast";
import { ViewError } from "../../design/ViewError";
import { useSpanish } from "../../i18n/AssistantCopy";
import { AssistantComposer } from "./AssistantComposer";
import { UnknownAssistant, useAssistantCall } from "./AssistantAvailability";
import { CallView, MinimisedCall } from "./CallView";
import { Conversation, currentTurn, type Decision } from "./OpenConversation";
import { ConversationHeader } from "./ConversationHeader";
import { turnQuestion } from "./ThreadParts";
import styles from "./AssistantView.module.css";

interface AssistantViewProps {
  error: string | null;
  send: (command: AppCommand) => unknown;
  snapshot: AppSnapshot;
  readMessages?: DaemonPort["readSessionMessages"];
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
  error,
  onOpenCall,
  readMessages,
  send,
  snapshot,
}: AssistantViewProps & { calling: boolean; calls: Calls; onOpenCall: () => void }) {
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
        />
      )}
      <ViewError error={error} />
      <AssistantComposer
        attachmentLimits={snapshot.settings.attachments}
        chat={chat}
        daemonReady={snapshot.connection.daemon === "ready"}
        hub={currentHubConnection(snapshot)}
        paired={paired}
        send={send}
      />
      {calling && <MinimisedCall call={call} onHangUp={calls.endCall} onOpen={onOpenCall} />}
    </>
  );
}

/** The Assistant once it is known: the call in its own view, or the conversation. */
function AssistantScreen(props: AssistantViewProps) {
  const es = useSpanish();
  const { error, send, snapshot } = props;
  const { call, chat } = snapshot;
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
        />
      ) : (
        <ConversationArea
          {...props}
          calling={calling}
          calls={calls}
          onOpenCall={() => setMinimised(false)}
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
 * to a card over the conversation. Not known yet, it shows that it reads.
 */
export default function AssistantView(props: AssistantViewProps) {
  if (props.snapshot.chat.availability?.state === "unknown")
    return <UnknownAssistant snapshot={props.snapshot} />;
  return <AssistantScreen {...props} />;
}
