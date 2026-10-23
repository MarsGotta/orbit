import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { PermissionSnapshot } from "../../bridge/generated/PermissionSnapshot";
import { Button } from "../../design/Button";
import { Loading } from "../../design/Spinner";
import { useSpanish } from "../../i18n/AssistantCopy";
import { permissionRows } from "../../i18n/es/permissions";
import { useRereadOnReturn } from "../settings/permission-reread";
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
  /** macOS was asked, or System Settings opened, for this one. */
  asked: "macos" | "settings" | null;
  onAsk: () => void;
  onOpenSettings: () => void;
  permission: PermissionSnapshot[Key];
  permissionKey: Key;
}

/**
 * Asking stays «Esperando a macOS…» until macOS answers, and a permission
 * macOS has not reported yet waits the same way: neither is a refusal.
 * Accessibility, which macOS never asks for, and anything refused are
 * changed in System Settings; once opened, the row waits for it there until
 * Orbit's window has the focus again.
 */
function PermissionAction({
  asked,
  onAsk,
  onOpenSettings,
  permission,
  permissionKey,
}: ActionProps) {
  const es = useSpanish();
  const copy = es.onboarding;
  if (permission === "granted") return <span className={styles.granted}>{copy.allowed}</span>;
  if (permission === "unknown" || (permission === "not_determined" && asked === "macos"))
    return <Loading size={12}>{es.controls.waitingForMacos}</Loading>;
  if (asked === "settings") return <Loading size={12}>{copy.waitingForSettings}</Loading>;
  if (permission === "not_determined" && permissionKey !== "accessibility")
    return (
      <Button onClick={onAsk} size={32} variant="outline">
        {copy.allow}
      </Button>
    );
  // Accessibility not granted yet is what macOS says before it was ever switched on.
  if (permissionKey === "accessibility" && permission !== "denied")
    return (
      <Button onClick={onOpenSettings} size={32} variant="outline">
        {copy.openSettings}
      </Button>
    );
  return (
    <span className={styles.rowActions}>
      <span className={styles.missing}>
        <span aria-hidden="true" className={styles.missingDot} />
        {copy.notGranted}
      </span>
      <Button onClick={onOpenSettings} size={28} variant="outline">
        {copy.openSystemSettings}
      </Button>
    </span>
  );
}

export function PermissionsStep({ error = null, permissions, send }: Props) {
  const copy = useSpanish().onboarding;
  // What was asked, and the failure on screen when it was: a new one is this step's.
  const [asked, setAsked] = useState<{
    keys: Partial<Record<Key, "macos" | "settings">>;
    before: string | null;
  }>({ keys: {}, before: error });
  const failed = Object.keys(asked.keys).length > 0 && error !== null && error !== asked.before;
  const ask = (key: Key, how: "macos" | "settings") =>
    setAsked({ keys: { ...asked.keys, [key]: how }, before: error });
  // Back from System Settings, macOS is asked again; one still not granted
  // offers its button again instead of waiting for ever.
  useRereadOnReturn(send, Object.values(asked.keys).includes("settings"), () =>
    setAsked((now) => ({
      ...now,
      keys: Object.fromEntries(
        Object.entries(now.keys).filter(([, how]) => how !== "settings"),
      ) as typeof now.keys,
    })),
  );
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
              asked={failed ? null : (asked.keys[key] ?? null)}
              onAsk={() => {
                ask(key, "macos");
                void send({ command: "permissions", request: [key] });
              }}
              onOpenSettings={() => {
                ask(key, "settings");
                void send({ command: "open_system_privacy", pane });
              }}
              permission={permissions[key]}
              permissionKey={key}
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
