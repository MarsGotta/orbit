import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { ViewError } from "../../design/ViewError";
import { es } from "../../i18n/es";
import { SETTINGS_SECTIONS, type SettingsSection } from "../../state/ui";
import type { AgentPort } from "./agents/use-agent-config";
import { AssistantSection } from "./AssistantSection";
import { GeneralSection } from "./GeneralSection";
import { SectionHeading } from "./layout";
import { MeetingDictionaries } from "./MeetingDictionaries";
import { missingPermissions } from "./Permissions";
import { PrivacySection } from "./PrivacySection";
import { RecordingSection } from "./RecordingSection";
import { NoteTemplates } from "./templates/NoteTemplates";
import styles from "./SettingsView.module.css";

export interface SettingsViewProps {
  /** Ajustes › Asistente's call to the hub; absent where it cannot be made. */
  agent?: AgentPort;
  error: string | null;
  onReopenOnboarding: (mode?: "pair") => void;
  send: (command: AppCommand) => unknown;
  snapshot: AppSnapshot;
  /** The section the route names (`#/settings/{id}`); `general` without one. */
  section?: SettingsSection;
  /** Opens another section; the router changes the route. Without it the view keeps its own. */
  onSection?: (section: SettingsSection) => void;
}

function Section({
  agent,
  error,
  onReopenOnboarding,
  send,
  snapshot,
  section,
}: SettingsViewProps & { section: SettingsSection }) {
  switch (section) {
    case "general":
      return (
        <GeneralSection onReopenOnboarding={onReopenOnboarding} send={send} snapshot={snapshot} />
      );
    case "privacidad":
      return <PrivacySection send={send} snapshot={snapshot} />;
    case "grabacion":
      return <RecordingSection send={send} snapshot={snapshot} />;
    case "asistente":
      return <AssistantSection agent={agent} send={send} snapshot={snapshot} />;
    case "reuniones":
      return (
        <>
          <SectionHeading
            lead={es.settings.leads.reuniones}
            title={es.shell.settingsSections.reuniones}
          />
          <NoteTemplates
            daemon={snapshot.connection.daemon}
            error={error}
            library={snapshot.notes?.library}
            send={send}
          />
          <MeetingDictionaries library={snapshot.notes?.library} send={send} />
        </>
      );
  }
}

/**
 * Ajustes: five sections with their index at the left and «Salir de Orbit»
 * with the version under it. The window's top bar names the one open
 * («Ajustes / Privacidad y permisos»), as the route does.
 */
export default function SettingsView(props: SettingsViewProps) {
  const [own, setOwn] = useState<SettingsSection>(props.section ?? "general");
  const current = props.section ?? own;
  const open = props.onSection ?? setOwn;
  const missing = missingPermissions(props.snapshot.settings);
  return (
    <div className={styles.page}>
      <div className={styles.index}>
        <h1 className={styles.indexTitle}>{es.settings.title}</h1>
        <nav aria-label={es.settings.index} className={styles.indexList}>
          {SETTINGS_SECTIONS.map((id) => (
            <button
              aria-current={id === current ? "page" : undefined}
              className={styles.indexItem}
              key={id}
              onClick={() => open(id)}
              type="button"
            >
              <span className={styles.indexLabel}>{es.shell.settingsSections[id]}</span>
              {id === "privacidad" && missing > 0 && (
                <span className={styles.indexBadge}>
                  <span aria-hidden="true" className={styles.dot} />
                  {es.settings.missing(missing)}
                </span>
              )}
            </button>
          ))}
        </nav>
        <div className={styles.indexEnd}>
          <button
            className={styles.exit}
            onClick={() => void props.send({ command: "quit" })}
            type="button"
          >
            {es.settings.exit}
          </button>
          <span className={styles.version}>
            {es.settings.appVersion(props.snapshot.settings.versions.app)}
          </span>
        </div>
      </div>
      <div className={styles.main}>
        <div className={styles.scroll}>
          <div className={styles.content} key={current}>
            <ViewError error={props.error} />
            <Section {...props} section={current} />
          </div>
        </div>
      </div>
    </div>
  );
}
