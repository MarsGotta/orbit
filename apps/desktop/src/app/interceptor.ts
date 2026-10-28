import { presentExceeded } from "./exceeded";
export { exceededReport } from "./exceeded";
import { getCopy } from "../i18n";
import type { AppCommand, FeatureId } from "../bridge/generated";
import type { Notice } from "../bridge/generated/Notice";
import type { NoticeCode } from "../bridge/generated/NoticeCode";
import { noticeClass } from "../bridge/generated/noticeClasses";
import type { Report } from "../bridge/port";
import type { Refusal } from "../bridge/refusal";
import type { NoticeAction, NoticeData } from "../i18n";
import type { FailedNoticeCode } from "../i18n";
import { failureCode, genericFailure, repeatable, failure, worded, type Failure } from "./failure";

import { closedData, token } from "./report-data";
import type { ViewId } from "../state/ui";

/** Where a command's refusal is shown: the view, overlay or panel that sent it. */
export type ErrorOrigin = ViewId | "onboarding" | "recording";

/** A view's boundary shows its own failure; nothing else repeats it. */
export type Place = ErrorOrigin | "boundary";

export type VisibleClass = "fixable" | "failed" | "working" | "done";

/** One message on screen. At most one per op. */
export interface Message {
  op: string;
  class: VisibleClass;
  text: string;
  actions?: readonly NoticeAction[];
  action?: NoticeAction;
  place?: Place;
  /** A refused order's function and command; the next accepted order clears it. */
  feature?: FeatureId;
  command?: AppCommand;
  /** An accepted rearm clears privacy refusals across functions. */
  refusalCode?: Refusal["code"];
  /** A failure the person cannot fix: see {@link Failure}. */
  failure?: Failure;
}

export { failureCode, genericFailure, type Failure };

export type CrashCode = "ui_uncaught_error" | "ui_render_failed";

/**
 * A wait past its ceiling (`state/ceilings.ts`): which part of it (`stage`),
 * what it waited on (`subject`: the order, the call or the feature) and how
 * long, against which ceiling.
 */
export interface Exceeded {
  kind: "exceeded";
  op: string;
  stage: "receipt" | "frame" | "call" | "first_state" | "working";
  subject: string;
  feature: FeatureId;
  elapsedMs: number;
  ceilingMs: number;
}

export type Incoming =
  | { kind: "notice"; notice: Notice }
  | { kind: "refusal"; op: string; refusal: Refusal }
  | { kind: "crash"; op: string; code: CrashCode }
  | Exceeded;

export type Context = Pick<Message, "place" | "feature" | "command">;

export function newOp(): string {
  return crypto.randomUUID();
}

export interface Presented {
  op: string;
  class: VisibleClass | "silent";
  text: string;
  actions?: readonly NoticeAction[];
  action?: NoticeAction;
  failure?: Failure;
  report: Report | null;
}

function noticeFailure(code: FailedNoticeCode): string {
  return getCopy().extras.failures.failure.notices[code];
}

function presentNotice(notice: Notice, assistantName?: string): Presented {
  const op = notice.op ?? newOp();
  const notices = getCopy().extras.noticesForAssistant(
    assistantName ?? getCopy().shell.assistantName,
  );
  const entry =
    notice.code !== "unknown" && Object.hasOwn(notices, notice.code)
      ? notices[notice.code]
      : undefined;
  if (!entry) {
    const data = closedData(notice.data);
    const report: Report = { op, code: token(notice.code), class: "failed", source: "ui", data };
    return failure(op, report, noticeFailure("unknown"));
  }
  if (entry.class === "silent") return { op, class: "silent", text: "", report: null };
  const data = notice.data ?? {};
  if (entry.class === "failed") {
    const what = noticeFailure(notice.code as FailedNoticeCode);
    return worded(failure(op, null, what), entry.text?.(data));
  }
  return {
    op,
    class: entry.class,
    text: entry.text(data),
    action: entry.action,
    actions: entry.actions,
    report: null,
  };
}

function refusalEntry(refusal: Refusal) {
  return Object.hasOwn(getCopy().extras.refusalEntries, refusal.code)
    ? getCopy().extras.refusalEntries[refusal.code]
    : undefined;
}

/** The report line of a refusal: its own code and class, no text. */
export function refusalReport(op: string, refusal: Refusal): Report {
  const entry = refusalEntry(refusal);
  return {
    op,
    code: token(refusal.code),
    class: entry?.class ?? "failed",
    source: "app",
    data: {},
  };
}

/** A code of the interface's own, with closed data only. */
export function uiReport(op: string, code: NoticeCode, data: NoticeData = {}): Report {
  return { op, code: token(code), class: noticeClass[code], source: "ui", data: closedData(data) };
}

function presentRefusal(op: string, refusal: Refusal, command?: AppCommand): Presented {
  const entry = refusalEntry(refusal);
  const report = refusalReport(op, refusal);
  if (!entry || entry.class === "failed") {
    const what = command ? getCopy().extras.failures.failure.commands[command.command] : undefined;
    const result = worded(failure(op, report, what), entry?.text?.(refusal));
    if (!command || !repeatable(refusal)) return result;
    return { ...result, failure: { ...result.failure, retry: command } };
  }
  return {
    op,
    class: entry.class,
    text: entry.text(refusal),
    action: entry.action?.(refusal),
    report,
  };
}

function present(incoming: Incoming, context: Context, assistantName?: string): Presented {
  if (incoming.kind === "notice") return presentNotice(incoming.notice, assistantName);
  if (incoming.kind === "refusal")
    return presentRefusal(incoming.op, incoming.refusal, context.command);
  if (incoming.kind === "exceeded") return presentExceeded(incoming);
  const { op, code } = incoming;
  const what =
    code === "ui_render_failed" ? getCopy().extras.failures.failure.renderFailed : undefined;
  return failure(op, { op, code, class: "failed", source: "ui", data: {} }, what);
}

/**
 * The only place a notice, a refusal or an error becomes something on screen.
 * A silent code is only logged, and ends a working message of its
 * op; anything shown replaces the message of its op, so there is never more
 * than one per op. Identical visible notices also replace the previous notice
 * across operations. A code the catalogue does not know is the generic
 * failure. Returns what to write to the app's report log, if anything.
 */
export function receive(
  messages: readonly Message[],
  incoming: Incoming,
  context: Context = {},
  assistantName?: string,
): { messages: Message[]; report: Report | null } {
  const shown = present(incoming, context, assistantName);
  if (shown.class === "silent") {
    const kept = messages.filter(
      (message) =>
        message.op !== shown.op ||
        (message.class !== "working" && !["call-silence", "recording-silence"].includes(shown.op)),
    );
    return { messages: kept, report: shown.report };
  }
  const message: Message = { op: shown.op, class: shown.class, text: shown.text, ...context };
  const stable =
    incoming.kind === "notice"
      ? { ...incoming, notice: { ...incoming.notice, op: shown.op } }
      : incoming;
  wording.set(message, () => present(stable, context, assistantName));
  if (incoming.kind === "refusal") message.refusalCode = incoming.refusal.code;
  if (shown.action) message.action = shown.action;
  if (shown.actions) message.actions = shown.actions;
  if (shown.failure) message.failure = shown.failure;
  const others = messages.filter(
    (existing) =>
      existing.op !== shown.op &&
      !(
        incoming.kind === "notice" &&
        existing.class === message.class &&
        existing.text === message.text &&
        existing.place === message.place &&
        existing.action === message.action
      ),
  );
  return { messages: [...others, message], report: shown.report };
}

/** The daemon numbers log entries once, independently of their payload. */
export function freshNotices(recent: Notice[], last: number | null) {
  const numbered = recent.filter(
    (notice) => notice.sequence !== undefined && notice.op !== undefined,
  );
  const fresh =
    last === null ? numbered.slice(-1) : numbered.filter((notice) => (notice.sequence ?? 0) > last);
  const newest = numbered.at(-1)?.sequence;
  return { fresh, last: newest === undefined ? last : Math.max(last ?? 0, newest) };
}

// Retain the code while its message lives, without adding payloads to reports
// or extending their lifetime after dismissal. Rewording executes no effects.
const wording = new WeakMap<Message, () => Presented>();
export function localizeMessage(message: Message): Message {
  const shown = wording.get(message)?.();
  if (!shown) return message;
  return {
    ...message,
    text: shown.text,
    action: shown.action,
    actions: shown.actions,
    failure: shown.failure,
  };
}
