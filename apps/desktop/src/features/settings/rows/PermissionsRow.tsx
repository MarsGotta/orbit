import type { AppCommand, SettingsSnapshot } from "../../../bridge/generated";
import { Button } from "../../../design/Button";
import { Card } from "../../../design/Card";
import { SectionLabel } from "../../../design/SectionLabel";
import { es } from "../../../i18n/es";
import { permissionRows, permissionStateLabel } from "../../../i18n/es/permissions";
import styles from "../SettingsView.module.css";

interface Props {
  send: (command: AppCommand) => void;
  settings: SettingsSnapshot;
}

export function SystemPermissions({ send, settings }: Props) {
  return (
    <section aria-label={es.settings.systemPermissions} className={styles.section}>
      <SectionLabel>{es.settings.systemPermissions}</SectionLabel>
      <Card>
        <div className={styles.compactList}>
          {permissionRows.map(([key, { label, pane }]) => {
            const permission = settings.permissions[key];
            const canRequest = permission === "not_determined";
            const canOpenSettings =
              permission === "granted" || permission === "denied" || permission === "not_granted";
            return (
              <div className={styles.permissionRow} key={key}>
                <strong>{label}</strong>
                <span>{settings.helper_connected ? permissionStateLabel(permission) : null}</span>
                {settings.helper_connected && canRequest && (
                  <Button
                    aria-label={`${es.settings.request}: ${label}`}
                    onClick={() => send({ command: "permissions", request: [key] })}
                    size={28}
                    variant="ghost"
                  >
                    {es.settings.request}
                  </Button>
                )}
                {settings.helper_connected && canOpenSettings && (
                  <Button
                    aria-label={`${es.settings.openSystemSettings}: ${label}`}
                    onClick={() => send({ command: "open_system_privacy", pane })}
                    size={28}
                    variant="ghost"
                  >
                    {es.settings.openSystemSettings}
                  </Button>
                )}
              </div>
            );
          })}
        </div>
        {!settings.helper_connected && (
          <>
            <p>{es.settings.helperMissing}</p>
            <Button onClick={() => send({ command: "quit" })}>{es.settings.restart}</Button>
          </>
        )}
      </Card>
    </section>
  );
}
