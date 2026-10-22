import type { FeatureFrame } from "../bridge/generated";
import type { DaemonPort } from "../bridge/port";
import {
  freshNotices,
  newOp,
  receive,
  uiReport,
  type Context,
  type Incoming,
} from "../app/interceptor";
import { conversationAgentName } from "../features/assistant/agents";
import { watchFailures } from "./state-failures";
import type { Get, Set, StoreOptions } from "./orbit-store";

/** Every message passes here: shown as the interceptor decides, reported when it says. */
export function intercept(
  port: DaemonPort,
  get: Get,
  set: Set,
  incoming: Incoming,
  context?: Context,
) {
  const { messages, report } = receive(
    get().messages,
    incoming,
    context,
    conversationAgentName(get().snapshot),
  );
  set({ messages });
  if (report) void port.report(report).catch(() => undefined);
  return messages;
}

/**
 * The failures a frame shows whose cause the screen no longer says,
 * each reported once when it appears. Only the main window does
 * it, as with the notices, so a failure is not written twice.
 */
function failureWatcher(port: DaemonPort, get: Get, options: StoreOptions, lifecycle: Lifecycle) {
  const watch = watchFailures();
  return () => {
    const view = get().snapshot;
    if (!options.notices || !view) return;
    for (const failure of watch(view, lifecycle.closing))
      void port.report(uiReport(newOp(), failure.code, failure.data)).catch(() => undefined);
  };
}

/**
 * Whether the daemon this window watches has said it is closing. Only a notice
 * that arrives while the window watches counts: the log is durable, so the
 * last session's `orbit_closing` is still its newest notice at the next start.
 */
interface Lifecycle {
  closing: boolean;
}

/** The daemon's notices, each once and in order. */
/** Told of each notice shown, by code and op, to bound the working ones. */
type NoticeFollower = (code: string, op: string | undefined) => void;

function noticeWatcher(
  port: DaemonPort,
  get: Get,
  set: Set,
  options: StoreOptions,
  lifecycle: Lifecycle,
  follow: NoticeFollower,
) {
  let last: number | null = null;
  let epoch: string | undefined;
  // The first status of a daemon is what it had before this window watched.
  let started = false;
  return (frames: FeatureFrame[]) => {
    if (!options.notices) return;
    for (const frame of frames) {
      if (frame.feature !== "status") continue;
      if (frame.view.notice_epoch !== undefined && frame.view.notice_epoch !== epoch) {
        epoch = frame.view.notice_epoch;
        last = null;
        started = false;
        lifecycle.closing = false;
      }
      const watching = started;
      started = true;
      const taken = freshNotices(frame.view.notices, last);
      last = taken.last;
      for (const notice of taken.fresh) {
        if (watching && notice.code === "orbit_closing") lifecycle.closing = true;
        intercept(port, get, set, { kind: "notice", notice });
        follow(notice.code, notice.op);
      }
    }
  };
}

/** The notices and the failures a window reports, sharing what the notices say. */
export function daemonWatchers(
  port: DaemonPort,
  get: Get,
  set: Set,
  options: StoreOptions,
  follow: NoticeFollower = () => undefined,
) {
  const lifecycle: Lifecycle = { closing: false };
  return {
    notices: noticeWatcher(port, get, set, options, lifecycle, follow),
    failures: failureWatcher(port, get, options, lifecycle),
  };
}
