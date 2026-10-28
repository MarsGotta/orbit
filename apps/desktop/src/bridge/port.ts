import type { AppBootstrap, AppCommand, CommandReceipt, FeatureFrame } from "./generated";
import type { AgentAnswer } from "./generated/AgentAnswer";
import type { AgentCall } from "./generated/AgentCall";
import type { VaultCall } from "./generated/VaultCall";
import type { VaultOutcome } from "./generated/VaultOutcome";
import type { NoticeClass } from "./generated/NoticeClass";
import type { NoticeData } from "../i18n";
import type { AssistantCursor, MessagesPage, SessionPage } from "./reads";

export type FrameListener = (frame: FeatureFrame) => void;
export type Unsubscribe = () => void;

/**
 * One line of the app's local report log, as `orbit_report` takes
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
  /** One page of a conversation; without a cursor, the newest. */
  readSessionMessages?(sessionId: string, cursor: AssistantCursor | null): Promise<MessagesPage>;
  /** Settings › Agents: one read or order of an agent's configuration on the hub. */
  agent?(call: AgentCall): Promise<AgentAnswer>;
  vault?(call: VaultCall): Promise<VaultOutcome>;
  /** The main window hands over what it holds before Orbit quits (`state/quit-flush.ts`). */
  onFlushBeforeQuit?(flush: () => Promise<void>): Promise<Unsubscribe>;
  /** The floating panel opens a link of Deimos's answer. */
  openUrl?(url: string): Promise<CommandReceipt>;
}
