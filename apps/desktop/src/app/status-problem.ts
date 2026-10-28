import { getCopy } from "../i18n";

import type { AppCommand } from "../bridge/generated";
import type { HubLinkConnection } from "../bridge/generated/HubLinkConnection";
import { hasSavedPairing } from "../bridge/hub-state";

import type { AppView as AppSnapshot } from "../state/app-view";
import { settingsRoute } from "../state/ui";

/** What fixes a notice: a section of Settings, an order, or pairing again. */
export type NoticeFix =
  | { kind: "route"; label: string; route: string }
  | { kind: "command"; label: string; command: AppCommand; pending: string }
  | { kind: "pair"; label: string };

/**
 * One notice of the top bar. `working` is something under way that ends by
 * itself (a spinner and its words); `warn`, something the person can fix.
 */
export interface TopBarNotice {
  id: string;
  tone: "warn" | "working";
  text: string;
  detail?: string;
  fix?: NoticeFix;
}

const pair = (): NoticeFix => ({ kind: "pair", label: getCopy().problem.pair });

function privacyNotice(snapshot: AppSnapshot): TopBarNotice | null {
  if (!snapshot.privacy_stopped) return null;
  return {
    id: "privacy",
    tone: "warn",
    text: getCopy().problem.privacyStopped,
    fix: {
      kind: "command",
      label: getCopy().problem.rearm,
      command: { command: "rearm" },
      pending: getCopy().problem.rearming,
    },
  };
}

function hubNotice(snapshot: AppSnapshot, settled: HubLinkConnection | null): TopBarNotice | null {
  const known = snapshot.status_availability?.state;
  if (known === "unknown" || known === "absent") return null;
  if (!hasSavedPairing(snapshot))
    return { id: "unpaired", tone: "warn", text: getCopy().problem.unpaired, fix: pair() };
  if (settled === "revoked")
    return { id: "revoked", tone: "warn", text: getCopy().problem.revoked, fix: pair() };
  if (settled === "offline")
    return {
      id: "offline",
      tone: "working",
      text: getCopy().problem.offline,
      detail: getCopy().problem.retrying,
    };
  // Connecting for the first time: on its way, never a failure.
  if (settled === null)
    return { id: "hub", tone: "working", text: getCopy().problem.connectingHub };
  return null;
}

/** A permission macOS has not granted. «unknown» is not missing: it says nothing. */
function permissionNotice(permissions: AppSnapshot["permissions"]): TopBarNotice | null {
  const known = permissions.availability?.state;
  if (known === "unknown" || known === "absent") return null;
  const missing = getCopy().extras.permissionRows.filter(([key]) => {
    const state = permissions[key];
    return state === "denied" || state === "not_granted" || state === "not_determined";
  });
  if (missing.length === 0) return null;
  const [only] = missing;
  const text =
    missing.length === 1 && only
      ? getCopy().problem.missingPermission(only[1].name)
      : getCopy().problem.missingPermissions;
  return {
    id: "permission",
    tone: "warn",
    text,
    fix: { kind: "route", label: getCopy().problem.fix, route: settingsRoute("privacidad") },
  };
}

/**
 * The notices of the top bar, the most important first: everything
 * stopped, the hub (unpaired, removed, offline, connecting), a missing
 * permission. While Orbit gets ready after opening, only that. Empty when
 * all is well. Orbit not answering at all is the whole window's, not the
 * bar's. `settled` is the hub link's last state other than «connecting», so
 * a retry does not make the notice blink.
 */
export function topBarNotices(
  snapshot: AppSnapshot,
  settled: HubLinkConnection | null,
): TopBarNotice[] {
  if (snapshot.connection.daemon === "connecting")
    return [{ id: "daemon", tone: "working", text: getCopy().problem.preparing }];
  if (snapshot.connection.daemon === "lost") return [];
  return [
    privacyNotice(snapshot),
    hubNotice(snapshot, settled),
    permissionNotice(snapshot.permissions),
  ].filter((notice): notice is TopBarNotice => notice !== null);
}

/** The hub link as the bar reads it: «connecting» keeps the state before it. */
export function settleHub(
  previous: HubLinkConnection | null,
  connection: HubLinkConnection,
): HubLinkConnection | null {
  return connection === "connecting" ? previous : connection;
}
