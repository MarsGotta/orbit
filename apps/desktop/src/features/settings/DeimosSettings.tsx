import type { ComputerControlSnapshot } from "../../bridge/generated/ComputerControlSnapshot";
import type { EngineSnapshot } from "../../bridge/generated/EngineSnapshot";
import { Card } from "../../design/Card";
import { SectionLabel } from "../../design/SectionLabel";
import { es } from "../../i18n/es";
import { useSpanish } from "../../i18n/AssistantCopy";
import { ControlHistory } from "./rows/ControlRow";
import styles from "./SettingsView.module.css";

/// Nothing here switches anything on. What lets Deimos see and use
/// this Mac is the permission the user grants in macOS. With the engine
/// ready, starting or not known yet, the card says nothing; otherwise it says
/// what is wrong and what to do. The cause and its numbers stay in the log.
export function engineProblem(engine: EngineSnapshot | undefined, copy = es): string | null {
  switch (engine?.state) {
    case "helper_not_connected":
      return copy.settings.engine.helperNotConnected;
    case "permission_missing":
      return copy.settings.engine.missing(permissionNames(engine));
    case "failed":
      return copy.settings.engine.failed;
    // The privacy stop is the status pill's: it is not said twice.
    case "privacy_stopped":
    case "ready":
    case "starting":
    case "permission_unknown":
    case "unknown":
    case undefined:
      return null;
  }
}

function permissionNames(permissions: {
  accessibility: boolean;
  screen_recording: boolean;
}): string {
  const names: string[] = [];
  if (permissions.accessibility) names.push(es.onboarding.accessibility.toLowerCase());
  if (permissions.screen_recording) names.push(es.settings.engine.screenRecording);
  return names.join(es.settings.engine.and);
}

/** Ajustes › Deimos: what is wrong, if anything, and what Deimos did on this Mac. */
export function DeimosSettings({ control }: { control?: ComputerControlSnapshot }) {
  const es = useSpanish();
  const problem = engineProblem(control?.engine, es);
  if (problem === null && (control == null || control.engine.state === "unknown")) return null;
  return (
    <section aria-label={es.settings.deimos} className={styles.section}>
      <SectionLabel>{es.settings.deimos}</SectionLabel>
      <Card>
        {problem !== null && (
          <p>
            <strong>{problem}</strong>
          </p>
        )}
        <ControlHistory control={control} />
      </Card>
    </section>
  );
}
