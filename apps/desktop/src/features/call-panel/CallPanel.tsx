import type { AppCommand } from "../../bridge/generated";
import type { CallPanelSnapshot } from "../../bridge/generated/CallPanelSnapshot";
import { Button } from "../../design/Button";
import { endingLabel } from "../../app/call-countdown";
import { useNow } from "../../use-now";
import { es } from "../../i18n/es";
import styles from "./CallPanel.module.css";

export interface CallPanelProps {
  panel: CallPanelSnapshot | null;
  send: (command: AppCommand) => void;
}

/**
 * What the floating panel says — an offer, a recording that started
 * on its own, or the countdown after the call ended — and nothing else. Each
 * button names what it answers.
 */
export function CallPanel({ panel, send }: CallPanelProps) {
  const now = useNow(panel?.state === "ending" ? 1_000 : null);
  if (!panel) return null;
  return (
    <section aria-label={es.callDetection.panelLabel} className={styles.panel}>
      <span aria-hidden="true" className={styles.dot} />
      <div className={styles.copy}>{copy(panel, now)}</div>
      <div className={styles.actions}>{actions(panel, send)}</div>
    </section>
  );
}

function copy(panel: CallPanelSnapshot, now: number) {
  switch (panel.state) {
    case "offer":
      return (
        <>
          <strong>{es.callDetection.offer(panel.application)}</strong>
          {panel.event_title != null && panel.event_title !== "" && (
            <span className={styles.event}>{panel.event_title}</span>
          )}
        </>
      );
    case "recording":
      return (
        <>
          <strong>{es.callDetection.recording(panel.application)}</strong>
          {panel.event_title != null && panel.event_title !== "" && (
            <span className={styles.event}>{panel.event_title}</span>
          )}
        </>
      );
    case "ending":
      return <strong>{endingLabel(panel.ends_at_ms, now)}</strong>;
  }
}

function actions(panel: CallPanelSnapshot, send: CallPanelProps["send"]) {
  switch (panel.state) {
    case "offer":
      return (
        <>
          <Button
            onClick={() => send({ command: "call_record", call_id: panel.call_id })}
            size={28}
            variant="accent"
          >
            {es.callDetection.record}
          </Button>
          <Button
            onClick={() => send({ command: "call_dismiss", call_id: panel.call_id })}
            size={28}
            variant="ghost"
          >
            {es.callDetection.notNow}
          </Button>
        </>
      );
    case "recording":
      return (
        <Button onClick={() => send({ command: "meeting_finish" })} size={28}>
          {es.callDetection.stop}
        </Button>
      );
    case "ending":
      return (
        <Button
          onClick={() => send({ command: "recording_keep", recording_id: panel.recording_id })}
          size={28}
          variant="accent"
        >
          {es.callDetection.keep}
        </Button>
      );
  }
}
