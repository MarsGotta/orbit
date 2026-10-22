import { useEffect, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { HubLinkConnection } from "../../bridge/generated/HubLinkConnection";
import { es } from "../../i18n/es";
import type { AppView } from "../../state/app-view";
import styles from "./AssistantView.module.css";

/** An assistant not known yet is an empty view: an unknown is not a failure. */
export function UnknownAssistant({ snapshot }: { snapshot: AppView }) {
  if (snapshot.chat.availability?.state !== "unknown") return null;
  return <section className={styles.view} />;
}

export function useAssistantCall({
  error,
  send,
  snapshot,
  paired,
  hubConnection,
}: {
  error: string | null;
  send: (command: AppCommand) => void;
  snapshot: AppView;
  paired: boolean;
  hubConnection: HubLinkConnection;
}) {
  const [callRequested, setCallRequested] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const { call, chat } = snapshot;
  const assistantAvailable =
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
    if (!assistantAvailable) return;
    setCallRequested(true);
    send({ command: "call_start" });
  };
  const endCall = () => {
    setCallRequested(false);
    send({ command: "call_stop" });
    setToast(es.assistant.callEnded);
  };
  return { callRequested, toast, setToast, startCall, endCall };
}
