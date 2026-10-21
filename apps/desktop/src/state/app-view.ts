import type { FeatureFrame, FrameRevision } from "../bridge/generated";

type ViewOf<F extends FeatureFrame["feature"]> = Extract<FeatureFrame, { feature: F }>["view"];

/** The desktop's composed view is derived from the function frames on the wire. */
export type AppView = {
  revision: FrameRevision["counter"];
  connection: ViewOf<"status">["connection"];
  status_availability?: ViewOf<"status">["availability"];
  privacy_stopped: ViewOf<"status">["privacy_stopped"];
  system_timezone: ViewOf<"status">["system_timezone"];
  system_timezone_error?: ViewOf<"status">["system_timezone_error"];
  settings: ViewOf<"status">["settings"] & {
    permissions: ViewOf<"permissions">;
    login_item: ViewOf<"local">["login_item"];
    update: ViewOf<"local">["update"];
    onboarding_done: ViewOf<"local">["onboarding_done"];
    profile?: NonNullable<ViewOf<"local">["profile"]>;
  };
  permissions: ViewOf<"permissions">;
  calendar: ViewOf<"agenda">;
  notes?: NonNullable<ViewOf<"notes">>;
  meeting: ViewOf<"recording">["meeting"];
  call: ViewOf<"recording">["call"];
  recording_availability?: ViewOf<"recording">["availability"];
  call_availability?: ViewOf<"recording">["call_availability"];
  chat: ViewOf<"assistant">;
  computer_control?: NonNullable<ViewOf<"control">>;
  notice: ViewOf<"status">["notice"];
  briefing?: ViewOf<"briefing">;
};

export function composeView(frames: FeatureFrame[], revision: FrameRevision): AppView {
  const byFeature = new Map(frames.map((frame) => [frame.feature, frame]));
  const view = <F extends FeatureFrame["feature"]>(feature: F): ViewOf<F> => {
    const frame = byFeature.get(feature);
    if (!frame) throw new Error(`Falta el marco de ${feature} en el arranque`);
    return frame.view as ViewOf<F>;
  };
  const status = view("status");
  const local = view("local");
  const recording = view("recording");
  const notes = view("notes");
  return {
    revision: revision.counter,
    connection: status.connection,
    status_availability: status.availability,
    privacy_stopped: status.privacy_stopped,
    system_timezone: status.system_timezone,
    system_timezone_error: status.system_timezone_error,
    settings: {
      ...status.settings,
      permissions: view("permissions"),
      login_item: local.login_item,
      update: local.update,
      onboarding_done: local.onboarding_done,
      profile: local.profile ?? undefined,
    },
    permissions: view("permissions"),
    calendar: view("agenda"),
    notes: notes ? { ...notes, open: view("note") } : undefined,
    meeting: recording.meeting,
    call: recording.call,
    recording_availability: recording.availability,
    call_availability: recording.call_availability,
    chat: view("assistant"),
    computer_control: view("control") ?? undefined,
    notice: status.notice,
    briefing: view("briefing"),
  };
}
