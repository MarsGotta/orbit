import { Button } from "../../../design/Button";
import { Toggle } from "../../../design/Toggle";
import { macPermissions } from "../../../i18n/es/permissions";
import { es } from "../../../i18n/es";
import styles from "../SettingsView.module.css";
import { SettingsRow, type ControlProps } from "./RowLayout";

export function MicrophoneRow({ daemonReady, send, settings }: ControlProps) {
  const granted = settings.permissions.microphone === "granted";
  const action = (
    <div className={styles.rowAction}>
      {granted && (
        <Button
          onClick={() =>
            send({ command: "open_system_privacy", pane: macPermissions.microphone.pane })
          }
          size={26}
          variant="ghost"
        >
          {es.settings.openSystemSettings}
        </Button>
      )}
      <Toggle
        checked={granted}
        disabled={granted || !daemonReady}
        label={es.settings.microphoneReady}
        onCheckedChange={() => send({ command: "permissions", request: ["microphone"] })}
      />
    </div>
  );
  return (
    <SettingsRow
      action={action}
      description={es.settings.microphoneHelp}
      title={es.settings.microphoneReady}
    />
  );
}
