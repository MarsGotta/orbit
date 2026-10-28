import type { Refusal } from "../../bridge/refusal";
import { lookupCopy } from "../lookup";

import type { AppOperationCode } from "../es/errors";

const appOperations = {
  link_invalid: "Can’t open the link: it isn’t a valid web address.",
  link_scheme: "Can’t open the link: only http and https links can be opened.",
  link_credentials:
    "Can’t open the link: a username or password before the site can hide where it leads.",
  link_no_host: "Can’t open the link: no destination site is specified.",
  note_unavailable: "The note isn’t available yet. Refresh it and try again.",
  speaker_undo_unavailable: "There’s no speaker name change to undo.",
  reminder_without_title: "The meeting needs a title before a reminder can be saved.",
  desktop_command: "This action needs the Orbit app.",
  client_command: "The app doesn’t support this command.",
  internal_command: "The interface can’t send that internal command.",
} satisfies Record<AppOperationCode, string>;

export const errors = {
  errors: {
    title: "Couldn’t complete this",
  },
} as const;

/** App-core and desktop errors carry codes; diagnostic text stays out of copy. */
export function appOperationText(refusal: Refusal): string | undefined {
  if (typeof refusal.cause !== "object" || refusal.cause === null) return undefined;
  const cause = refusal.cause as Record<string, unknown>;
  if (cause.kind !== "app_operation" || typeof cause.code !== "string") return undefined;
  if (cause.code === "meeting_event_not_found") {
    const facts: unknown = typeof cause.error === "string" ? safeFacts(cause.error) : undefined;
    const count =
      typeof facts === "object" && facts !== null && "count" in facts ? facts.count : undefined;
    return typeof count === "number"
      ? `The event wasn’t found among the ${count} loaded events. Refresh the calendar.`
      : "The event wasn’t found. Refresh the calendar.";
  }
  return lookupCopy(appOperations, cause.code);
}

function safeFacts(value: string): unknown {
  try {
    return JSON.parse(value) as unknown;
  } catch {
    return undefined;
  }
}
