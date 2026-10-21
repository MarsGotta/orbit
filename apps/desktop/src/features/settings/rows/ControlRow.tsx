import type { ComputerControlSnapshot } from "../../../bridge/generated/ComputerControlSnapshot";
import { es } from "../../../i18n/es";
import { useSpanish } from "../../../i18n/AssistantCopy";
import styles from "../SettingsView.module.css";

/// An action is performed, cancelled, failed or of uncertain end.
/// Nothing on this path waits for an approval, so no word here says so; any
/// word the daemon does not send today reads as an end nobody saw.
function historyOutcome(outcome: string): string {
  if (outcome === "performed") return es.fullControl.performed;
  if (outcome === "cancelled") return es.fullControl.cancelled;
  if (outcome === "failed") return es.fullControl.failed;
  return es.fullControl.unknown;
}

function historyTime(atMs: number): string {
  return new Intl.DateTimeFormat("es-ES", {
    dateStyle: "short",
    timeStyle: "short",
  }).format(new Date(atMs));
}

/**
 * What Deimos did on this Mac, newest as the daemon sends it. A history not
 * known yet, or one the daemon could not read, says nothing.
 */
export function ControlHistory({ control }: { control?: ComputerControlSnapshot }) {
  const es = useSpanish();
  if (control == null || control.engine.state === "unknown") return null;
  if (control.history.length === 0) {
    const failed = control.history_error != null && control.history_error.length > 0;
    return failed ? null : <p>{es.fullControl.noHistory}</p>;
  }
  return (
    <div className={styles.compactList}>
      {control.history.map((entry) => (
        <div className={styles.historyRow} key={entry.operation_id}>
          <div>
            <strong>{entry.application}</strong>
            <span>{entry.requested}</span>
          </div>
          <span>
            {historyTime(entry.at_ms)} · {historyOutcome(entry.outcome)}
          </span>
        </div>
      ))}
    </div>
  );
}
