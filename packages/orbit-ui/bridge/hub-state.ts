import type { AppView as AppSnapshot } from "../state/app-view";
import type { HubLinkConnection } from "./generated/HubLinkConnection";

export function currentHubConnection(snapshot: AppSnapshot): HubLinkConnection {
  const projected = snapshot.settings.hub.connection;
  if (projected) return projected;
  return snapshot.connection.hub === "paired" ? "online" : "offline";
}

export function hasSavedPairing(snapshot: AppSnapshot): boolean {
  if (snapshot.settings.hub.connection) return snapshot.settings.hub.paired;
  return snapshot.connection.hub === "paired";
}
