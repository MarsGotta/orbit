import type { ComputerControlSnapshot } from "../../bridge/generated/ComputerControlSnapshot";
import type { EngineSnapshot } from "../../bridge/generated/EngineSnapshot";
import { useSpanish } from "../../i18n/AssistantCopy";
import { es as base } from "../../i18n/es";
import { List, SkeletonRows } from "./layout";
import styles from "./SettingsView.module.css";

type Copy = ReturnType<typeof useSpanish>;

/// Nothing here switches anything on: what lets the assistant see and use
/// this Mac is the permission the user grants in macOS. With the engine
/// ready, starting or not known yet, nothing is said; otherwise what is
/// wrong and what to do. The cause and its numbers stay in the log.
export function engineProblem(engine: EngineSnapshot | undefined, copy: Copy): string | null {
  switch (engine?.state) {
    case "helper_not_connected":
      return copy.settings.engine.helperNotConnected;
    case "permission_missing": {
      const names: string[] = [];
      if (engine.accessibility) names.push(copy.onboarding.accessibility.toLowerCase());
      if (engine.screen_recording) names.push(copy.settings.engine.screenRecording);
      return copy.settings.engine.missing(names.join(copy.settings.engine.and));
    }
    case "failed":
      return copy.settings.engine.failed;
    // The privacy stop is said at the top of the window: not twice.
    case "privacy_stopped":
    case "ready":
    case "starting":
    case "permission_unknown":
    case "unknown":
    case undefined:
      return null;
  }
}

/// An action is performed, cancelled, failed or of uncertain end. Nothing
/// on this path waits for an approval, so no word here says so; any word the
/// daemon does not send today reads as an end nobody saw.
function outcomeOf(outcome: string): { text: string; tone?: "ok" | "warn" } {
  if (outcome === "performed") return { text: base.fullControl.performed, tone: "ok" };
  if (outcome === "cancelled") return { text: base.fullControl.cancelled };
  if (outcome === "failed") return { text: base.fullControl.failed, tone: "warn" };
  return { text: base.fullControl.unknown, tone: "warn" };
}

const clock = new Intl.DateTimeFormat("es-ES", { hour: "numeric", minute: "2-digit" });
const day = new Intl.DateTimeFormat("es-ES", { day: "numeric", month: "short" });

/** «Hoy, 9:52», «Ayer, 18:05» or «21 oct, 9:00», by the day on this Mac. */
export function historyTime(atMs: number, nowMs: number): string {
  const at = new Date(atMs);
  const today = new Date(nowMs);
  today.setHours(0, 0, 0, 0);
  const days = Math.round((today.getTime() - new Date(at).setHours(0, 0, 0, 0)) / 86_400_000);
  const date =
    days === 0
      ? base.settings.today
      : days === 1
        ? base.settings.yesterday
        : day.format(at).replace(".", "");
  return `${date}, ${clock.format(at)}`;
}

/** What the assistant did on this Mac, newest as the daemon sends it. */
export function ControlHistory({ control }: { control?: ComputerControlSnapshot }) {
  const es = useSpanish();
  const problem = engineProblem(control?.engine, es);
  if (control === undefined || control.engine.state === "unknown") return <SkeletonRows rows={2} />;
  const unreadable =
    control.history.length === 0 && control.history_error != null && control.history_error !== "";
  const now = Date.now();
  return (
    <List>
      {problem !== null && (
        <div className={styles.folder} data-warn>
          <div className={styles.rowTitle}>{problem}</div>
        </div>
      )}
      {control.history.length === 0 && (
        <p className={styles.empty}>
          {unreadable ? es.settings.historyUnreadable : es.fullControl.noHistory}
        </p>
      )}
      {control.history.map((entry) => {
        const outcome = outcomeOf(entry.outcome);
        return (
          <div className={styles.history} key={entry.operation_id}>
            <span className={styles.historyApp}>{entry.application}</span>
            <span className={styles.historyAsked}>{entry.requested}</span>
            <span className={styles.meta}>{historyTime(entry.at_ms, now)}</span>
            <span className={styles.historyOutcome} data-tone={outcome.tone}>
              {outcome.text}
            </span>
          </div>
        );
      })}
    </List>
  );
}
