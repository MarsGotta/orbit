import type { AppCommand, FeatureId } from "../bridge/generated";
import type { Notice } from "../bridge/generated/Notice";
import type { NoticeCode } from "../bridge/generated/NoticeCode";
import { noticeClass } from "../bridge/generated/noticeClasses";
import type { Report } from "../bridge/port";
import type { Refusal } from "../bridge/refusal";
import { noticesForAssistant, type NoticeAction, type NoticeData } from "../i18n/es/notices";
import { refusalEntries } from "../i18n/es/refusalEntries";
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
  action?: NoticeAction;
  place?: Place;
  /** A refused order's function and command; the next accepted order clears it. */
  feature?: FeatureId;
  command?: AppCommand;
  /** An accepted rearm clears privacy refusals across functions. */
  refusalCode?: Refusal["code"];
}

export type CrashCode = "ui_uncaught_error" | "ui_render_failed";

export type Incoming =
  | { kind: "notice"; notice: Notice }
  | { kind: "refusal"; op: string; refusal: Refusal }
  | { kind: "crash"; op: string; code: CrashCode };

export type Context = Pick<Message, "place" | "feature" | "command">;

const TOKEN = /^[a-z0-9_.-]{1,64}$/;

export function newOp(): string {
  return crypto.randomUUID();
}

/** The message of a failure the person cannot fix: the code of its op. */
export function genericFailure(op: string): string {
  return `Algo ha fallado. Contacta con el administrador (código ${op.slice(0, 6).toUpperCase()}).`;
}

/** Only numbers, booleans and tokens leave for the log; anything else is dropped. */
function closedData(data: NoticeData | undefined): NoticeData {
  const kept: NoticeData = {};
  for (const [key, value] of Object.entries(data ?? {}) as [keyof NoticeData, unknown][]) {
    if (typeof value === "number" || typeof value === "boolean") kept[key] = value;
    else if (typeof value === "string" && TOKEN.test(value)) kept[key] = value;
  }
  return kept;
}

function token(code: string): string {
  return TOKEN.test(code) ? code : "unrecognized";
}

interface Presented {
  op: string;
  class: VisibleClass | "silent";
  text: string;
  action?: NoticeAction;
  report: Report | null;
}

function failure(op: string, report: Report | null): Presented {
  return { op, class: "failed", text: genericFailure(op), report };
}

function presentNotice(notice: Notice, assistantName?: string): Presented {
  const op = notice.op ?? newOp();
  const notices = noticesForAssistant(assistantName);
  const entry =
    notice.code !== "unknown" && Object.hasOwn(notices, notice.code)
      ? notices[notice.code]
      : undefined;
  if (!entry) {
    const data = closedData(notice.data);
    return failure(op, { op, code: token(notice.code), class: "failed", source: "ui", data });
  }
  if (entry.class === "silent") return { op, class: "silent", text: "", report: null };
  if (entry.class === "failed") return failure(op, null);
  const data = notice.data ?? {};
  return { op, class: entry.class, text: entry.text(data), action: entry.action, report: null };
}

function refusalEntry(refusal: Refusal) {
  return Object.hasOwn(refusalEntries, refusal.code) ? refusalEntries[refusal.code] : undefined;
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

function presentRefusal(op: string, refusal: Refusal): Presented {
  const entry = refusalEntry(refusal);
  const report = refusalReport(op, refusal);
  if (!entry) return failure(op, report);
  if (entry.class === "failed") {
    const result = failure(op, report);
    return entry.text ? { ...result, text: entry.text(refusal) } : result;
  }
  return {
    op,
    class: entry.class,
    text: entry.text(refusal),
    action: entry.action?.(refusal),
    report,
  };
}

function present(incoming: Incoming, assistantName?: string): Presented {
  if (incoming.kind === "notice") return presentNotice(incoming.notice, assistantName);
  if (incoming.kind === "refusal") return presentRefusal(incoming.op, incoming.refusal);
  const { op, code } = incoming;
  return failure(op, { op, code, class: "failed", source: "ui", data: {} });
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
  const shown = present(incoming, assistantName);
  if (shown.class === "silent") {
    const kept = messages.filter(
      (message) => message.op !== shown.op || message.class !== "working",
    );
    return { messages: kept, report: shown.report };
  }
  const message: Message = { op: shown.op, class: shown.class, text: shown.text, ...context };
  if (incoming.kind === "refusal") message.refusalCode = incoming.refusal.code;
  if (shown.action) message.action = shown.action;
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
