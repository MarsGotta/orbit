import type { PermissionSnapshot } from "../../bridge/generated/PermissionSnapshot";
import type { Views } from "../scenario";

// The Macs the first run's scenarios start from.

/** A Mac that has just installed Orbit: not paired, no permission asked yet, no assistant known. */
export function newMac(base: Views, granted: Partial<PermissionSnapshot> = {}): Views {
  const unasked = "not_determined" as const;
  const permissions = {
    ...base.permissions,
    microphone: unasked,
    screen: unasked,
    // macOS never asks for accessibility: it is switched on in System Settings.
    accessibility: "not_granted" as const,
    calendars: unasked,
    ...granted,
  };
  return {
    ...base,
    permissions,
    status: {
      ...base.status,
      connection: { daemon: "ready", hub: "unpaired" },
      settings: {
        ...base.status.settings,
        agents: [],
        default_agent_id: undefined,
        hub: {
          url: null,
          paired: false,
          device_name: null,
          device_id_short: null,
          connection: "offline",
        },
      },
    },
    local: { ...base.local, onboarding_done: false, profile: null },
    agenda: { ...base.agenda, authorization: "not_determined", events: [] },
    notes: { list: [], open: null, index: { state: "needs_hub", more: false, detail: null } },
    briefing: null,
  };
}

/**
 * The annex's Mac once paired with Nubaria's hub: the three assistants,
 * Phobos signed in and Ares's session expired; three of the four
 * permissions given, Calendarios refused.
 */
export function pairedMac(base: Views): Views {
  const fresh = newMac(base, {
    microphone: "granted",
    screen: "granted",
    accessibility: "granted",
    calendars: "denied",
  });
  return {
    ...fresh,
    status: {
      ...fresh.status,
      connection: { daemon: "ready", hub: "paired" },
      settings: {
        ...fresh.status.settings,
        agents: base.status.settings.agents,
        default_agent_id: base.status.settings.default_agent_id,
        hub: {
          url: "https://hub.nubaria.example",
          paired: true,
          device_name: "Mac de Lucía",
          device_id_short: "7f3a2c",
          connection: "online",
        },
      },
    },
  };
}
