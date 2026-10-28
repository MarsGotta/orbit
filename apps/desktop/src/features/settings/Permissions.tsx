import { getCopy, type Catalogue } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import { useEffect, useRef } from "react";
import type { AppCommand, PermissionState } from "../../bridge/generated";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { Button } from "../../design/Button";
import { Switch } from "../../design/Switch";

import { Group, List, Row } from "./layout";
import { useRereadOnReturn } from "./permission-reread";
import styles from "./SettingsView.module.css";

type Send = (command: AppCommand) => unknown;
type Settings = AppSnapshot["settings"];
type PermissionKey = Catalogue["extras"]["permissionRows"][number][0];

const MISSING: readonly PermissionState[] = ["denied", "not_granted", "not_determined"];

/** Whether macOS has said anything yet: before the helper is ready, nothing is known. */
function permissionsKnown(settings: Settings): boolean {
  const state = settings.permissions.availability?.state;
  return settings.helper_connected && state !== "unknown" && state !== "absent";
}

/**
 * How many of the four permissions macOS has not given, for «● Falta 1»
 * in the index. One never asked counts; one not known yet does not.
 */
export function missingPermissions(settings: Settings): number {
  if (!permissionsKnown(settings)) return 0;
  return getCopy().extras.permissionRows.filter(([key]) =>
    MISSING.includes(settings.permissions[key]),
  ).length;
}

/**
 * Asks macOS for the microphone. The switch waits («Esperando a macOS…»)
 * until macOS gives it, and stops waiting when it is refused: by the daemon
 * (`send` resolves to `false`) or by macOS (the permission reads as not
 * granted).
 */
function useMicrophoneRequest(send: Send, state: PermissionState) {
  const refuse = useRef<(() => void) | null>(null);
  useEffect(() => {
    if (state !== "denied" && state !== "not_granted") return;
    refuse.current?.();
    refuse.current = null;
  }, [state]);
  return () => {
    const asked = new Promise<void>((_, reject) => {
      refuse.current = () => reject(new Error("microphone_refused"));
    });
    void Promise.resolve(send({ command: "permissions", request: ["microphone"] })).then(
      (taken) => {
        if (taken === false) refuse.current?.();
      },
    );
    return asked;
  };
}

function MicrophoneSwitch({
  known,
  send,
  state,
}: {
  known: boolean;
  send: Send;
  state: PermissionState;
}) {
  const base = useCopy();
  const request = useMicrophoneRequest(send, state);
  return (
    <div className={styles.inRow}>
      <Switch
        checked={state === "granted"}
        disabled={!known || state !== "not_determined"}
        label={base.settings.microphoneReady}
        onCheckedChange={request}
        waitsForSystem
      >
        <span>
          {base.settings.microphoneReady}{" "}
          <span className={styles.meta}>· {base.settings.microphoneHint}</span>
        </span>
      </Switch>
    </div>
  );
}

function PermissionRow({
  name,
  known,
  send,
  state,
}: {
  name: PermissionKey;
  known: boolean;
  send: Send;
  state: PermissionState;
}) {
  const permissionRows = useCopy().extras.permissionRows;
  const es = useCopy();
  const copy = es.permissionRows[name];
  const pane = permissionRows.find(([key]) => key === name)?.[1].pane;
  const missing = known && MISSING.includes(state);
  const status = !known ? (
    <span className={styles.statusUnknown}>{es.settings.checkingPermission}</span>
  ) : state === "granted" ? (
    <span className={styles.statusOk}>{es.settings.granted}</span>
  ) : missing ? (
    <span className={styles.statusWarn}>{es.settings.notGranted}</span>
  ) : (
    <span className={styles.statusUnknown}>{es.settings.checkingPermission}</span>
  );
  return (
    <Row aside={status} help={copy.help} title={copy.title} warn={missing}>
      {name === "microphone" && <MicrophoneSwitch known={known} send={send} state={state} />}
      {/* The microphone is asked for by its own switch. */}
      {missing && state === "not_determined" && name !== "microphone" && (
        <div className={styles.inRow}>
          <Button
            aria-label={`${es.settings.requestPermission}: ${copy.title}`}
            onClick={() => void send({ command: "permissions", request: [name] })}
            size={32}
            variant="accent"
          >
            {es.settings.requestPermission}
          </Button>
        </div>
      )}
      {missing && state !== "not_determined" && pane !== undefined && (
        <div className={styles.inRow}>
          <Button
            aria-label={`${es.settings.openSystemSettings}: ${copy.title}`}
            onClick={() => void send({ command: "open_system_privacy", pane })}
            size={32}
            variant="accent"
          >
            {es.settings.openSystemSettings}
          </Button>
        </div>
      )}
    </Row>
  );
}

/**
 * «Permisos de macOS»: each of the four, «Permitido», or what is missing as
 * an attention row with the button that fixes it. Not known yet is
 * «Comprobando…», never «Sin permiso».
 */
export function SystemPermissions({ send, settings }: { send: Send; settings: Settings }) {
  const base = useCopy();
  const permissionRows = useCopy().extras.permissionRows;
  const known = permissionsKnown(settings);
  // Back from System Settings, what is missing is read again from macOS.
  useRereadOnReturn(send, missingPermissions(settings) > 0);
  return (
    <Group label={base.settings.systemPermissions}>
      <List busy={settings.helper_connected && !known}>
        {permissionRows.map(([key]) => (
          <PermissionRow
            key={key}
            known={known}
            name={key}
            send={send}
            state={settings.permissions[key]}
          />
        ))}
        {!settings.helper_connected && (
          <Row
            aside={
              <Button onClick={() => void send({ command: "restart" })} size={32}>
                {base.settings.restart}
              </Button>
            }
            title={base.settings.helperMissing}
            warn
          />
        )}
      </List>
    </Group>
  );
}
