import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { PermissionSnapshot } from "../../bridge/generated/PermissionSnapshot";
import { Button } from "../../design/Button";
import { Loading } from "../../design/Spinner";
import { useSpanish } from "../../i18n/AssistantCopy";
import { permissionRows } from "../../i18n/es/permissions";
import { Step } from "./OnboardingSteps";
import styles from "./OnboardingView.module.css";

type Key = keyof PermissionSnapshot;

interface Props {
  /** The refusal of an order of this first run, if any. */
  error?: string | null;
  permissions: PermissionSnapshot;
  send: (command: AppCommand) => Promise<unknown>;
}

interface ActionProps {
  asking: boolean;
  onAsk: () => void;
  pane: string;
  permission: PermissionSnapshot[Key];
  permissionKey: Key;
  send: Props["send"];
}

/**
 * Asking stays «Esperando a macOS…» until macOS answers, and a permission
 * macOS has not reported yet waits the same way: neither is a refusal.
 * Accessibility, and anything already refused, is changed in System Settings.
 */
function PermissionAction({ asking, onAsk, pane, permission, permissionKey, send }: ActionProps) {
  const es = useSpanish();
  const copy = es.onboarding;
  if (permission === "granted") return <span className={styles.granted}>{copy.allowed}</span>;
  if (permission === "unknown" || (permission === "not_determined" && asking))
    return <Loading size={12}>{es.controls.waitingForMacos}</Loading>;
  if (permission === "not_determined" && permissionKey !== "accessibility")
    return (
      <Button onClick={onAsk} size={32} variant="outline">
        {copy.allow}
      </Button>
    );
  return (
    <span className={styles.rowActions}>
      {permission !== "not_determined" && (
        <span className={styles.missing}>{es.permissions.notGranted}</span>
      )}
      <Button
        onClick={() => void send({ command: "open_system_privacy", pane })}
        size={32}
        variant="outline"
      >
        {copy.openSettings}
      </Button>
    </span>
  );
}

export function PermissionsStep({ error = null, permissions, send }: Props) {
  const copy = useSpanish().onboarding;
  // What was asked, and the failure on screen when it was: a new one is this step's.
  const [asked, setAsked] = useState<{ keys: Key[]; before: string | null }>({
    keys: [],
    before: error,
  });
  const failed = asked.keys.length > 0 && error !== null && error !== asked.before;
  return (
    <Step lead={copy.permissionsBody} title={copy.permissions}>
      <ul className={styles.rows}>
        {permissionRows.map(([key, { label, pane }]) => (
          <li className={styles.permissionRow} key={key}>
            <span className={styles.rowText}>
              <strong>{label}</strong>
              <span>{copy.permissionWhy[key]}</span>
            </span>
            <PermissionAction
              asking={asked.keys.includes(key) && !failed}
              onAsk={() => {
                setAsked({ keys: [...asked.keys, key], before: error });
                void send({ command: "permissions", request: [key] });
              }}
              pane={pane}
              permission={permissions[key]}
              permissionKey={key}
              send={send}
            />
          </li>
        ))}
      </ul>
      {failed && (
        <p className={`${styles.result} ${styles.failed}`} role="alert">
          {error}
        </p>
      )}
      <p className={styles.note}>{copy.permissionsLater}</p>
    </Step>
  );
}
