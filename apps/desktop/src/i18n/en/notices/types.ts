import type { AppCommand } from "../../../bridge/generated";
import type { Notice } from "../../../bridge/generated/Notice";
import type { NoticeClass } from "../../../bridge/generated/NoticeClass";
import type { NoticeCode } from "../../../bridge/generated/NoticeCode";
import { noticeClass } from "../../../bridge/generated/noticeClasses";
import type { ViewId } from "../../../state/ui";

export type NoticeData = NonNullable<Notice["data"]>;

/** What the person can do about a message: an order, or a view to open. */
export type NoticeAction = { label: string } & ({ command: AppCommand } | { view: ViewId });

/** The classes the person sees with words of their own. */
export type Worded = "fixable" | "working" | "done";

/**
 * One entry of the catalogue. A silent code has no words and is only logged.
 * A failed code is the generic message with its code, unless its data names
 * a cause the person can act on (`text` answers only for those).
 */
export type Entry<C extends NoticeClass> = C extends Worded
  ? {
      class: C;
      text: (data: NoticeData) => string;
      actions?: readonly NoticeAction[];
      action?: NoticeAction;
    }
  : C extends "failed"
    ? { class: C; text?: (data: NoticeData) => string | undefined }
    : { class: C };

/**
 * Entries for some codes, each with the class Rust gave it
 * (`bridge/generated/noticeClasses.ts`): a disagreeing class does not compile.
 */
export type Entries<K extends NoticeCode = NoticeCode> = {
  [P in K]: Entry<(typeof noticeClass)[P]>;
};

export type WordedCode = {
  [P in NoticeCode]: (typeof noticeClass)[P] extends Worded ? P : never;
}[NoticeCode];

export function worded<C extends Worded>(
  kind: C,
  text: string,
  action?: NoticeAction,
): { class: C; text: () => string; actions?: readonly NoticeAction[]; action?: NoticeAction } {
  return action ? { class: kind, text: () => text, action } : { class: kind, text: () => text };
}

export const rearm: NoticeAction = { label: "Rearm Orbit", command: { command: "rearm" } };
export const openSettings: NoticeAction = { label: "Open Settings", view: "settings" };

export function systemPrivacy(pane: string): NoticeAction {
  return {
    label: "Open System Settings",
    command: { command: "open_system_privacy", pane },
  };
}
