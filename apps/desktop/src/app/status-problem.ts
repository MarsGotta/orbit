import { permissionRows } from "../i18n/es/permissions";
import type { HubLinkConnection } from "../bridge/generated/HubLinkConnection";
import { hasSavedPairing } from "../bridge/hub-state";
import type { PillStatus } from "../design/Pill";
import { es } from "../i18n/es";
import type { NoticeAction } from "../i18n/es/notices";
import type { AppView as AppSnapshot } from "../state/app-view";

/** What the status pill says, when it says anything: a problem and what to do. */
export interface Problem {
  tone: PillStatus;
  text: string;
  action?: NoticeAction;
}

const settings: NoticeAction = { label: es.problem.openSettings, view: "settings" };
const pair: NoticeAction = { label: es.problem.pair, view: "settings" };

function daemonProblem(connection: AppSnapshot["connection"]): Problem | null {
  if (connection.daemon === "ready") return null;
  // Connecting is Orbit on its way: nothing to say yet.
  if (connection.daemon === "lost") return { tone: "disconnected", text: es.problem.orbitLost };
  return null;
}

function hubProblem(snapshot: AppSnapshot, settled: HubLinkConnection | null): Problem | null {
  const known = snapshot.status_availability?.state;
  if (known === "unknown" || known === "absent") return null;
  if (!hasSavedPairing(snapshot))
    return { tone: "disconnected", text: es.problem.unpaired, action: pair };
  if (settled === "revoked")
    return { tone: "disconnected", text: es.problem.revoked, action: pair };
  if (settled === "offline") return { tone: "disconnected", text: es.problem.offline };
  return null;
}

/** A permission macOS has not granted. «unknown» is not missing: it says nothing. */
function permissionProblem(permissions: AppSnapshot["permissions"]): Problem | null {
  const known = permissions.availability?.state;
  if (known === "unknown" || known === "absent") return null;
  const missing = permissionRows.filter(([key]) => {
    const state = permissions[key];
    return state === "denied" || state === "not_granted" || state === "not_determined";
  });
  if (missing.length === 0) return null;
  const [only] = missing;
  const text =
    missing.length === 1 && only
      ? es.problem.missingPermission(only[1].name)
      : es.problem.missingPermissions;
  return { tone: "disconnected", text, action: settings };
}

/**
 * SIM-24: the one problem the top bar names, or nothing when all is well.
 * Orbit itself first, then the privacy stop, the hub and the permissions.
 * `settled` is the hub link's last state other than «connecting», so a
 * retry does not make the pill blink.
 */
export function statusProblem(
  snapshot: AppSnapshot,
  settled: HubLinkConnection | null,
): Problem | null {
  const daemon = daemonProblem(snapshot.connection);
  if (daemon) return daemon;
  if (snapshot.privacy_stopped) {
    return {
      tone: "privacy",
      text: es.problem.privacyStopped,
      action: { label: es.problem.rearm, command: { command: "rearm" } },
    };
  }
  return hubProblem(snapshot, settled) ?? permissionProblem(snapshot.permissions);
}

/** The hub link as the pill reads it: «connecting» keeps the state before it. */
export function settleHub(
  previous: HubLinkConnection | null,
  connection: HubLinkConnection,
): HubLinkConnection | null {
  return connection === "connecting" ? previous : connection;
}
