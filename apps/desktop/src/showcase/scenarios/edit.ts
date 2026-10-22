import type { Notice } from "../../bridge/generated/Notice";
import type { SettingsHubSnapshot } from "../../bridge/generated/SettingsHubSnapshot";
import type { Views } from "../scenario";

type Status = Views["status"];

/** `views` with the status changed, its settings' hub merged in. */
export function withStatus(
  views: Views,
  change: Partial<Status>,
  hub: Partial<SettingsHubSnapshot> = {},
): Views {
  const settings = { ...views.status.settings, ...change.settings };
  return {
    ...views,
    status: {
      ...views.status,
      ...change,
      settings: { ...settings, hub: { ...settings.hub, ...hub } },
    },
  };
}

/** The privacy stop, as every function the daemon publishes says it. */
export function privacyStopped(views: Views): Views {
  const stopped = withStatus(views, {
    privacy_stopped: true,
    settings: { ...views.status.settings, privacy_stopped: true },
  });
  return {
    ...stopped,
    agenda: { ...views.agenda, reading: { state: "unavailable", blocker: "privacy_stopped" } },
    control: views.control && { ...views.control, engine: { state: "privacy_stopped" } },
    recording: {
      ...views.recording,
      meeting: {
        ...views.recording.meeting,
        voice_trigger: { state: "not_listening", why: { cause: "privacy_stopped" } },
      },
    },
  };
}

let sequence = 100;

/** The notices the daemon adds to its log, each with its op and a new sequence. */
export function notices(views: Views, ...added: Notice[]): Partial<Views> {
  const numbered = added.map((notice) => ({ op: `op-${(sequence += 1)}`, sequence, ...notice }));
  return {
    status: {
      ...views.status,
      notices: [...views.status.notices, ...numbered],
      notice: numbered[numbered.length - 1] ?? null,
    },
  };
}
