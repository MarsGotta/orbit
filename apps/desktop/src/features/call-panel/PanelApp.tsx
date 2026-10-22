import { useState } from "react";
import { useStore } from "zustand";
import { useCrashReporting } from "../../app/use-crash-reporting";
import { useOrbitConnection } from "../../app/useOrbitConnection";
import { messageFor } from "../../app/visible-error";
import { ViewBoundary } from "../../app/ViewBoundary";
import type { AppCommand } from "../../bridge/generated";
import type { ChatPending } from "../../bridge/generated/ChatPending";
import type { MeetingQuestionSnapshot } from "../../bridge/generated/MeetingQuestionSnapshot";
import { panelFocus } from "../../bridge/panel-port";
import type { DaemonPort } from "../../bridge/port";
import { Button } from "../../design/Button";
import { AssistantCopy, useSpanish } from "../../i18n/AssistantCopy";
import { createOrbitStore, type OrbitStore } from "../../state/orbit-store";
import { AskDeimos } from "../../app/meeting-question/AskDeimos";
import { QuestionCard } from "../../app/meeting-question/QuestionCard";
import { voiceStatusText } from "../../app/meeting-question/copy";
import { conversationAgentName } from "../assistant/agents";
import { PendingCard } from "../assistant/PendingCard";
import { CallPanel } from "./CallPanel";
import { openPanelLink } from "./panel-links";
import styles from "./CallPanel.module.css";

/**
 * The root of the floating panel's window. It follows the same
 * frames as the main window and renders only what the panel has to say:
 * a call and a question to Deimos with its answer. The
 * window itself is shown, hidden and sized by the app from the same frames.
 */
export function PanelApp({ port }: { port: DaemonPort }) {
  const [store] = useState(() => createOrbitStore(port, { notices: false }));
  const crash = useStore(store, (state) => state.crash);
  const assistantName = useStore(store, (state) => conversationAgentName(state.snapshot));
  useOrbitConnection(useStore(store, (state) => state.connect));
  useCrashReporting(crash);
  return (
    <ViewBoundary onCrash={() => crash("ui_render_failed")}>
      <AssistantCopy name={assistantName}>
        <PanelContent port={port} store={store} />
      </AssistantCopy>
    </ViewBoundary>
  );
}

function PanelContent({ port, store }: { port: DaemonPort; store: OrbitStore }) {
  const es = useSpanish();
  const [asking, setAsking] = useState(false);
  const links = usePanelLinks(port);
  const snapshot = useStore(store, (state) => state.snapshot);
  const send = useStore(store, (state) => state.send);
  const refused = useStore(store, (state) => messageFor("recording", state.messages)?.text);
  const dispatch = (command: AppCommand) => void send(command, "recording");
  const stopped = snapshot?.privacy_stopped === true;
  const meeting = snapshot?.meeting;
  const call = stopped ? null : (meeting?.call_panel ?? null);
  const question = stopped ? null : (meeting?.question ?? null);
  const ask = stopped ? null : (meeting?.ask ?? null);
  // Whether «Oye Deimos» listens, while the panel shows anything at all.
  const voice =
    stopped || (call === null && question === null)
      ? null
      : voiceStatusText(meeting?.voice_trigger, es);
  const focus = (on: boolean) => {
    setAsking(on);
    void panelFocus(port, on);
  };
  // «Preguntar a Deimos» from the panel: while a call's recording runs, or
  // once Deimos has answered; never over a question still in flight.
  const offerAsk =
    ask !== null &&
    !asking &&
    question?.stage !== "thinking" &&
    question?.stage !== "responding" &&
    (call?.state === "recording" || question !== null);
  return (
    <div className={styles.root} data-panel-root>
      {question && !asking && (
        <PanelQuestion
          openUrl={links.open}
          pending={snapshot?.chat?.pending}
          question={question}
          send={dispatch}
        />
      )}
      {asking && ask && (
        <AskDeimos
          ask={ask}
          onDone={() => focus(false)}
          send={dispatch}
          voiceStatus={meeting?.voice_trigger}
        />
      )}
      {offerAsk && (
        <Button className={styles.askButton} onClick={() => focus(true)} size={26}>
          {question ? es.meetingQuestion.askAgain : es.meetingQuestion.ask}
        </Button>
      )}
      <CallPanel panel={call} send={dispatch} />
      <Refusals texts={[refused, links.refused]} />
      {voice !== null && <p className={styles.voice}>{voice}</p>}
    </div>
  );
}

/** What the daemon or Rust refused, each said with its cause. */
function Refusals({ texts }: { texts: (string | null | undefined)[] }) {
  return texts.map(
    (text, index) =>
      text != null && (
        <p className={styles.voice} key={index} role="alert">
          {text}
        </p>
      ),
  );
}

/**
 * The meeting question and, while Deimos answers it, what Deimos asks back:
 * answered here too, with the same choices as in the chat.
 */
function PanelQuestion({
  openUrl,
  pending,
  question,
  send,
}: {
  openUrl: (url: string) => void;
  pending: ChatPending | null | undefined;
  question: MeetingQuestionSnapshot;
  send: (command: AppCommand) => void;
}) {
  const inFlight = question.stage === "thinking" || question.stage === "responding";
  return (
    <>
      <QuestionCard openUrl={openUrl} question={question} send={send} />
      {inFlight && pending && <PendingCard pending={pending} send={send} />}
    </>
  );
}

/**
 * The answer's links open through the panel's own order; a link
 * Rust refuses (not http/https, credentials, no host) is said with its cause.
 */
function usePanelLinks(port: DaemonPort) {
  const [refused, setRefused] = useState<string | null>(null);
  const open = (url: string) => {
    void openPanelLink(port, url).then(setRefused);
  };
  return { open, refused };
}
