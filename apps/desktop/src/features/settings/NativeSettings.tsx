import { useEffect } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { AppView } from "../../state/app-view";
import { useCopy } from "../../i18n/CopyProvider";
import { Button } from "../../design/Button";
import { Switch } from "../../design/Switch";
import { Row } from "./layout";
import styles from "./SettingsView.module.css";

type Props = { send: (command: AppCommand) => unknown; settings: AppView["settings"] };

function useSystemStatus(send: Props["send"]) {
  useEffect(() => {
    const refresh = () => {
      void send({ command: "desktop_status" });
    };
    refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [send]);
}

export function LoginItem({ send, settings }: Props) {
  const copy = useCopy().settings;
  useSystemStatus(send);
  const approval = settings.login_item === "requires_approval";
  const status =
    settings.login_item === "on"
      ? copy.loginEnabled
      : settings.login_item === "off"
        ? copy.loginNotRegistered
        : approval
          ? copy.loginApproval
          : copy.loginUnknown;
  return (
    <Row
      title={copy.openAtLogin}
      help={status}
      aside={
        <Switch
          checked={settings.login_item === "on"}
          label={copy.openAtLogin}
          onCheckedChange={(on) => Promise.resolve(send({ command: "login_item", on }))}
        />
      }
    >
      {approval && (
        <div className={styles.inRow}>
          <Button
            onClick={() => void send({ command: "open_system_privacy", pane: "LoginItems" })}
            size={32}
          >
            {copy.openSystemSettings}
          </Button>
        </div>
      )}
    </Row>
  );
}

export function NotificationPermission({ send, settings }: Props) {
  const copy = useCopy().settings;
  useSystemStatus(send);
  const { state = "unknown", error } = settings.notifications ?? {};
  const failed = error !== undefined && error !== null;
  const missing = ["denied", "not_granted", "not_determined"].includes(state);
  const request = state === "not_determined";
  const action = request ? copy.requestPermission : copy.openSystemSettings;
  return (
    <Row
      title={copy.notifications}
      help={copy.notificationsHelp}
      warn={missing || failed}
      aside={
        <span
          className={
            missing
              ? styles.statusWarn
              : state === "granted"
                ? styles.statusOk
                : styles.statusUnknown
          }
        >
          {state === "granted" ? copy.granted : missing ? copy.notGranted : copy.checkingPermission}
        </span>
      }
    >
      {(missing || failed) && (
        <div className={styles.inRow}>
          <Button
            aria-label={`${action}: ${copy.notifications}`}
            size={32}
            variant="accent"
            onClick={() =>
              void send(
                request
                  ? { command: "notifications_request" }
                  : { command: "open_system_privacy", pane: "Notifications" },
              )
            }
          >
            {action}
          </Button>
        </div>
      )}
      {failed && (
        <p role="status">
          {copy.notificationFailed} {error}
        </p>
      )}
    </Row>
  );
}
