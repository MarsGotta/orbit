import type { FeatureFrame, FrameRevision } from "../bridge/generated";
import type { AppView } from "./app-view";

export function newer(next: FrameRevision, previous: FrameRevision | undefined): boolean {
  return (
    previous === undefined ||
    next.epoch > previous.epoch ||
    (next.epoch === previous.epoch && next.counter > previous.counter)
  );
}

function applyStatusFrame(
  snapshot: AppView,
  view: Extract<FeatureFrame, { feature: "status" }>["view"],
): AppView {
  return {
    ...snapshot,
    connection: view.connection,
    status_availability: view.availability,
    privacy_stopped: view.privacy_stopped,
    system_timezone: view.system_timezone,
    system_timezone_error: view.system_timezone_error,
    settings: {
      ...snapshot.settings,
      ...view.settings,
      assistant_name: view.settings.assistant_name ?? snapshot.settings.assistant_name,
      // A status without agents (before the daemon says) keeps the last ones.
      agents: view.settings.agents?.length ? view.settings.agents : snapshot.settings.agents,
      default_agent_id: view.settings.default_agent_id ?? snapshot.settings.default_agent_id,
    },
    notice: view.notice,
  };
}

export function applyFrame(snapshot: AppView, frame: FeatureFrame): AppView {
  const base = { ...snapshot, revision: frame.revision.counter };
  switch (frame.feature) {
    case "status":
      return applyStatusFrame(base, frame.view);
    case "permissions":
      return {
        ...base,
        permissions: frame.view,
        settings: { ...snapshot.settings, permissions: frame.view },
      };
    case "agenda":
      return { ...base, calendar: frame.view };
    case "notes":
      return { ...base, notes: frame.view ?? undefined };
    case "note":
      return {
        ...base,
        notes: snapshot.notes ? { ...snapshot.notes, open: frame.view } : snapshot.notes,
      };
    case "recording":
      return {
        ...base,
        meeting: frame.view.meeting,
        call: frame.view.call,
        recording_availability: frame.view.availability,
        call_availability: frame.view.call_availability,
      };
    case "assistant":
      return { ...base, chat: frame.view };
    case "control":
      return { ...base, computer_control: frame.view ?? undefined };
    case "briefing":
      return { ...base, briefing: frame.view ?? undefined };
    case "local":
      return {
        ...base,
        settings: {
          ...snapshot.settings,
          profile: frame.view.profile ?? undefined,
          login_item: frame.view.login_item,
          notifications: frame.view.notifications,
          update: frame.view.update,
          onboarding_done: frame.view.onboarding_done,
        },
      };
  }
}
