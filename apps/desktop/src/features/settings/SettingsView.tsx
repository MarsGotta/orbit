import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { Card } from "../../design/Card";
import { SectionLabel } from "../../design/SectionLabel";
import { ViewError } from "../../design/ViewError";
import { es } from "../../i18n/es";
import { AutonomySettings } from "./AutonomySettings";
import { CallPreferenceSection } from "./CallPreferenceSection";
import { DeimosSettings } from "./DeimosSettings";
import { GeneralSettings } from "./GeneralSettings";
import { SystemPermissions } from "./SystemPermissions";
import { MeetingDictionaries } from "./MeetingDictionaries";
import { NoteTemplates } from "./templates/NoteTemplates";
import styles from "./SettingsView.module.css";

export interface SettingsViewProps {
  error: string | null;
  onReopenOnboarding: (mode?: "pair") => void;
  send: (command: AppCommand) => void | Promise<void>;
  snapshot: AppSnapshot;
}

export default function SettingsView(props: SettingsViewProps) {
  return <SettingsPage {...props} />;
}

function SettingsPage({ error, onReopenOnboarding, send, snapshot }: SettingsViewProps) {
  const settings = snapshot.settings;
  return (
    <div className={styles.page}>
      <ViewError error={error} />
      <div className={styles.content}>
        <header className={styles.hero}>
          <h1>{es.shell.settings}</h1>
        </header>
        <AutonomySettings
          daemonReady={snapshot.connection.daemon === "ready"}
          send={send}
          settings={settings}
        />
        <CallPreferenceSection preference={snapshot.meeting.call_preference} send={send} />
        <DeimosSettings control={snapshot.computer_control} />

        <NoteTemplates
          daemon={snapshot.connection.daemon}
          error={error}
          library={snapshot.notes?.library}
          send={send}
        />

        <MeetingDictionaries library={snapshot.notes?.library} send={send} />

        <GeneralSettings
          onReopenOnboarding={onReopenOnboarding}
          send={send}
          settings={settings}
          statusAvailability={snapshot.status_availability}
        />
        <SystemPermissions send={send} settings={settings} />

        <section className={styles.section}>
          <SectionLabel>{es.settings.exitSection}</SectionLabel>
          <Card>
            <Button onClick={() => send({ command: "quit" })}>{es.settings.exit}</Button>
          </Card>
        </section>
      </div>
    </div>
  );
}
