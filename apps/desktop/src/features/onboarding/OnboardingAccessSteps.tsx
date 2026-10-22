import type { AppCommand } from "../../bridge/generated";
import type { PermissionSnapshot } from "../../bridge/generated/PermissionSnapshot";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { permissionRows, permissionStateLabel } from "../../i18n/es/permissions";
import { Actions, Step } from "./OnboardingSteps";
import styles from "./OnboardingView.module.css";

interface AccessProps {
  onContinue: () => void;
  send: (command: AppCommand) => Promise<unknown>;
}

export function PermissionsStep({
  onContinue,
  permissions,
  send,
}: AccessProps & { permissions: PermissionSnapshot }) {
  return (
    <Step region title={es.onboarding.permissions}>
      <p>{es.onboarding.permissionsBody}</p>
      <ul className={styles.permissionList}>
        {permissionRows.map(([key, { label, pane }]) => (
          <li key={key}>
            <span>
              <strong>{label}</strong>
              <small>{permissionStateLabel(permissions[key])}</small>
            </span>
            {permissions[key] === "not_determined" && (
              <Button
                onClick={() => void send({ command: "permissions", request: [key] })}
                size={28}
              >
                {es.settings.request}
              </Button>
            )}
            {(permissions[key] === "granted" ||
              permissions[key] === "denied" ||
              permissions[key] === "not_granted") && (
              <Button onClick={() => void send({ command: "open_system_privacy", pane })} size={28}>
                {es.settings.openSystemSettings}
              </Button>
            )}
          </li>
        ))}
      </ul>
      <Actions onContinue={onContinue} onSkip={onContinue} />
    </Step>
  );
}
