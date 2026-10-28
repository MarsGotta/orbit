import { useCopy } from "../../i18n/CopyProvider";
import { useEffect, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { HubLinkConnection } from "../../bridge/generated/HubLinkConnection";
import { Skeleton } from "../../design/Skeleton";
import { Loading } from "../../design/Spinner";

import type { AppView } from "../../state/app-view";
import styles from "./AssistantView.module.css";

/**
 * An assistant not known yet waits with the shape of a conversation and
 * says what it reads: an unknown is neither empty nor a failure.
 */
export function UnknownAssistant({ snapshot }: { snapshot: AppView }) {
  const copy = useCopy();
  if (snapshot.chat.availability?.state !== "unknown") return null;
  return (
    <section aria-busy="true" className={styles.view}>
      <div className={styles.waiting}>
        <Loading>{copy.assistant.loadingAssistant}</Loading>
        <Skeleton widths={["40%", "90%", "84%", "60%"]} />
      </div>
    </section>
  );
}

export function useAssistantCall({
  error,
  send,
  snapshot,
  paired,
  hubConnection,
}: {
  error: string | null;
  send: (command: AppCommand) => unknown;
  snapshot: AppView;
  paired: boolean;
  hubConnection: HubLinkConnection;
}) {
  const es = useCopy();
  const [callRequested, setCallRequested] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const { call, chat } = snapshot;
  const available =
    snapshot.connection.daemon === "ready" &&
    paired &&
    hubConnection === "online" &&
    chat.availability?.state !== "unknown" &&
    snapshot.call_availability?.state !== "unknown";
  useEffect(() => {
    if (call) setCallRequested(false);
  }, [call]);
  useEffect(() => {
    if (error === null || call !== null) return;
    if (callRequested) setCallRequested(false);
  }, [call, callRequested, error]);
  const startCall = () => {
    if (!available) return;
    setCallRequested(true);
    send({ command: "call_start" });
  };
  const endCall = () => {
    setCallRequested(false);
    send({ command: "call_stop" });
    setToast(es.assistant.callEnded);
  };
  return { available, callRequested, toast, setToast, startCall, endCall };
}
