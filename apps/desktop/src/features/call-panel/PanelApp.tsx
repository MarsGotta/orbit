import { useEffect, useState } from "react";
import { useStore } from "zustand";
import { useCrashReporting } from "../../app/use-crash-reporting";
import { useOrbitConnection } from "../../app/useOrbitConnection";
import { messageFor } from "../../app/visible-error";
import { ViewBoundary } from "../../app/ViewBoundary";
import type { AppCommand } from "../../bridge/generated";
import type { VoiceStatusSnapshot } from "../../bridge/generated/VoiceStatusSnapshot";
import { panelFocus } from "../../bridge/panel-port";
import type { DaemonPort } from "../../bridge/port";
import { CopyProvider, useCopy } from "../../i18n/CopyProvider";
import { createOrbitStore, type OrbitStore } from "../../state/orbit-store";
import { OrbitStoreProvider } from "../../state/use-in-flight";
import { voiceStatusText } from "../../app/meeting-question/copy";
import { primaryAgentName } from "../assistant/agents";
import { EndingCard, OfferCard, RecordingCard } from "./CallPanel";
import { PanelAnswer } from "./PanelAnswer";
import { PanelAsk } from "./PanelAsk";
import { openPanelLink } from "./panel-links";
import styles from "./CallPanel.module.css";

/**
 * The root of the floating panel's window. It follows the same
 * frames as the main window and renders only what the panel has to say:
 * a call, a recording to ask Deimos about, and a question with its answer.
 * The window itself is shown, hidden and sized by the app from the same frames.
 */
export function PanelApp({ port }: { port: DaemonPort }) {
  const [store] = useState(() => createOrbitStore(port, { notices: false }));
  const crash = useStore(store, (state) => state.crash);
  const assistantName = useStore(store, (state) => primaryAgentName(state.snapshot));
  const language = useStore(store, (state) => state.snapshot?.settings.language);
  useOrbitConnection(useStore(store, (state) => state.connect));
  useCrashReporting(crash);
  return (
    <ViewBoundary onCrash={() => crash("ui_render_failed")}>
      <OrbitStoreProvider store={store}>
        <CopyProvider name={assistantName} choice={language}>
          <PanelContent port={port} store={store} />
        </CopyProvider>
      </OrbitStoreProvider>
    </ViewBoundary>
  );
}

/**
 * One card at a time, the most pressing first: a call to answer, the
 * countdown of a call that ended, the question being written, the question
 * and its answer, and otherwise the recording in progress.
 */
function PanelContent({ port, store }: { port: DaemonPort; store: OrbitStore }) {
  const es = useCopy();
  const [asking, setAsking] = useState(false);
  const links = usePanelLinks(port);
  const snapshot = useStore(store, (state) => state.snapshot);
  const send = useStore(store, (state) => state.send);
  const said = useSaid(store, es.connectFailed);
  const order = (command: AppCommand) => send(command, "recording");
  const dispatch = (command: AppCommand) => void order(command);
  const stopped = snapshot?.privacy_stopped === true;
  const meeting = snapshot?.meeting;
  const call = stopped ? null : (meeting?.call_panel ?? null);
  const question = stopped ? null : (meeting?.question ?? null);
  // «Preguntar a Deimos» during any recording, started by a call or by hand.
  const ask = stopped ? null : (meeting?.ask ?? null);
  const focus = (on: boolean) => {
    setAsking(on);
    void panelFocus(port, on);
  };
  // The recording ended while the question was being written: the window
  // hides and gives the keyboard back; the field must not reopen with the next.
  useEffect(() => {
    if (ask === null && asking) setAsking(false);
  }, [ask, asking]);
  const onAsk = ask ? () => focus(true) : undefined;
  let card = null;
  if (call?.state === "offer") {
    card = <OfferCard panel={call} send={order} />;
  } else if (call?.state === "ending") {
    card = <EndingCard key={call.recording_id} panel={call} send={order} />;
  } else if (asking && ask) {
    card = (
      <PanelAsk aboutCall={call !== null} ask={ask} onDone={() => focus(false)} send={dispatch} />
    );
  } else if (question) {
    card = (
      <PanelAnswer
        key={question.id}
        onAsk={onAsk}
        openUrl={links.open}
        pending={snapshot?.chat?.pending}
        question={question}
        send={dispatch}
      />
    );
  } else if (meeting && (ask || call?.state === "recording")) {
    card = (
      <RecordingCard
        call={call?.state === "recording" ? call : null}
        meeting={meeting}
        onAsk={onAsk}
        send={order}
        voice={voiceLine(meeting.voice_trigger, es)}
      />
    );
  }
  return (
    <div className={styles.root} data-panel-root>
      {card}
      <Refusals texts={[...said, links.refused]} />
    </div>
  );
}

/** «Di «Oye Deimos» y tu pregunta» while it listens; otherwise why not. */
function voiceLine(
  status: VoiceStatusSnapshot | undefined,
  es: ReturnType<typeof useCopy>,
): string | null {
  if (status?.state === "listening") return es.floatingPanel.voiceHint;
  return voiceStatusText(status, es);
}

/**
 * What the panel must say beside its card: that it could not connect (as the
 * window does; the cause stays in the log) and what the daemon refused.
 */
function useSaid(store: OrbitStore, connectFailed: string): (string | null | undefined)[] {
  const unconnected = useStore(store, (state) => (state.error === null ? null : connectFailed));
  const refused = useStore(store, (state) => messageFor("recording", state.messages)?.text);
  return [unconnected, refused];
}

/** What the daemon or Rust refused, each said with its cause. */
function Refusals({ texts }: { texts: (string | null | undefined)[] }) {
  return texts.map(
    (text, index) =>
      text != null && (
        <p className={styles.refusal} key={index} role="alert">
          {text}
        </p>
      ),
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
