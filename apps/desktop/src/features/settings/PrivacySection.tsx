import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { Button } from "../../design/Button";
import { Switch } from "../../design/Switch";

import { useHeldChoice } from "../../state/use-held-choice";
import { Group, List, Row, SectionHeading } from "./layout";
import { SystemPermissions } from "./Permissions";
import styles from "./SettingsView.module.css";

type Send = (command: AppCommand) => unknown;

/** «Parar todo» at the top; once stopped, «Todo está parado · Reactivar Orbit». */
function StopCard({ send, stopped }: { send: Send; stopped: boolean }) {
  const es = useCopy();
  const [rearming, setRearming] = useState(false);
  return (
    <section
      aria-labelledby="settings-stop-title"
      className={styles.stopCard}
      data-stopped={stopped || undefined}
    >
      <h3 id="settings-stop-title">{stopped ? es.settings.stoppedTitle : es.settings.stopTitle}</h3>
      <p>{stopped ? es.settings.stoppedText : es.settings.privacyArmed}</p>
      <div className={styles.actions}>
        {stopped ? (
          <Button
            feedback={rearming ? "pending" : "idle"}
            onClick={() => {
              setRearming(true);
              void Promise.resolve(send({ command: "rearm" })).finally(() => setRearming(false));
            }}
            pendingLabel={es.settings.rearming}
            size={36}
            variant="accent"
          >
            {es.settings.rearm}
          </Button>
        ) : (
          <Button
            onClick={() => void send({ command: "privacy_stop" })}
            size={36}
            variant="dangerSoft"
          >
            {es.settings.stopNow}
          </Button>
        )}
        <span className={styles.meta}>{es.settings.stopShortcut}</span>
      </div>
    </section>
  );
}

/** «Enviar informes de errores anónimos», shown as chosen until it is confirmed. */
function ErrorReports({ enabled, send }: { enabled: boolean; send: Send }) {
  const base = useCopy();
  const reports = useHeldChoice(enabled, send);
  return (
    <Group label={base.settings.improve}>
      <List>
        <Row
          aside={
            <Switch
              checked={reports.shown}
              label={base.settings.errorReports}
              onCheckedChange={(on) =>
                reports.choose(on, { command: "error_reports", enabled: on })
              }
            />
          }
          help={base.settings.errorReportsHelp}
          title={base.settings.errorReports}
        />
      </List>
    </Group>
  );
}

/** Ajustes › Privacidad y permisos: stop everything, the four macOS permissions, reports. */
export function PrivacySection({ send, snapshot }: { send: Send; snapshot: AppSnapshot }) {
  const base = useCopy();
  const settings = snapshot.settings;
  return (
    <>
      <SectionHeading
        lead={base.settings.leads.privacidad}
        title={base.shell.settingsSections.privacidad}
      />
      <StopCard send={send} stopped={settings.privacy_stopped} />
      <SystemPermissions send={send} settings={settings} />
      {/* Only a build that carries a Sentry address offers it. */}
      {settings.error_reports !== undefined && (
        <ErrorReports enabled={settings.error_reports} send={send} />
      )}
    </>
  );
}
