import type { AppBootstrap, AppCommand, CommandReceipt, FeatureFrame } from "./generated";
import type { NoticeClass } from "./generated/NoticeClass";
import type { NoticeData } from "../i18n/es/notices";
import type { AssistantCursor, MessagesPage, SessionPage } from "./reads";

export type FrameListener = (frame: FeatureFrame) => void;
export type Unsubscribe = () => void;

/**
 * One line of the app's local report log (SIM-21), as `orbit_report` takes
 * it: a code token, the class, who met it and closed data, never text.
 */
export interface Report {
  op: string;
  code: string;
  class: NoticeClass;
  source: "ui" | "app";
  data: NoticeData;
}

export interface DaemonPort {
  snapshot(): Promise<AppBootstrap>;
  /** `op` names the action from end to end: the daemon, the hub, the notices. */
  send(command: AppCommand, op: string): Promise<CommandReceipt>;
  report(report: Report): Promise<void>;
  subscribeFrames(listener: FrameListener): Promise<Unsubscribe>;
  readSessionsPage?(cursor: AssistantCursor | null): Promise<SessionPage>;
  /** SIM-30: one page of a conversation; without a cursor, the newest. */
  readSessionMessages?(sessionId: string, cursor: AssistantCursor | null): Promise<MessagesPage>;
  /** P9-12: the floating panel opens a link of Deimos's answer. */
  openUrl?(url: string): Promise<CommandReceipt>;
}
