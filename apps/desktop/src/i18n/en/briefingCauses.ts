import { decodeBriefingCause, hasNumbers } from "../briefing-cause";
import { formatQuantity } from "../locale";
import { lookupCopy } from "../lookup";
import { refusalEntries } from "./refusalEntries";

const states = {
  local_zone_unavailable: "Couldn’t read this Mac’s time zone.",
  agenda_page_pending: "The agenda page is still being read.",
  agenda_window_missing: "The agenda doesn’t say which time range was read.",
  notes_list_pending: "The meeting list is still being read.",
  notes_list_failed: "Couldn’t read the meeting list.",
  topic_not_observed: "This state hasn’t been read yet.",
  topic_stale: "This state is out of date.",
  helper_starting: "The Mac helper is starting.",
  daemon_starting: "The Orbit service is starting.",
  series_not_read: "The meeting series is still being read.",
  read_series_empty: "The meeting series was empty.",
  read_series_invalid: "The meeting series has an invalid format.",
  read_list_invalid: "A note in the list has an invalid format.",
  read_calendars_invalid: "A calendar in the list has an invalid format.",
  read_calendars_cursor: "The calendar cursor repeated or belongs to another read.",
  read_mismatch: "The response belongs to a different read.",
  read_missing: "The Orbit service did not return the read.",
  read_reminder_title: "The meeting has no title for saving a reminder.",
  read_reminder_empty: "The reminder was empty.",
  read_reminder_invalid: "The reminder has an invalid format.",
};

/** Wording is selected when displayed, including after a live language change. */
export function briefingCause(value: string): string {
  const { code, facts, refusal } = decodeBriefingCause(value);
  if (code === "topic_stale" && hasNumbers(facts, "since_ms"))
    return `This state has been out of date since ${formatQuantity(facts.since_ms)} ms Unix.`;
  if (code === "reading_since" && hasNumbers(facts, "since_ms"))
    return `The read has been in progress since ${formatQuantity(facts.since_ms)} ms Unix.`;
  if (
    code === "transcription" &&
    hasNumbers(facts, "completed", "pending", "failed") &&
    typeof facts.queued === "boolean"
  )
    return `Transcription: ${formatQuantity(facts.completed)} completed, ${formatQuantity(facts.pending)} pending, ${formatQuantity(facts.failed)} failed; ${facts.queued === true ? "another is queued" : "none queued"}.`;
  if (
    code === "agenda_window_excludes_today" &&
    hasNumbers(facts, "window_start_ms", "window_end_ms", "today_start_ms", "today_end_ms")
  )
    return `The time range read (${formatQuantity(facts.window_start_ms)}–${formatQuantity(facts.window_end_ms)} ms Unix) doesn’t cover today (${formatQuantity(facts.today_start_ms)}–${formatQuantity(facts.today_end_ms)} ms Unix).`;
  if (code === "read_calendars_count" && hasNumbers(facts, "count", "total"))
    return `Read ${formatQuantity(facts.count)} of ${formatQuantity(facts.total)} advertised calendars.`;
  if (code === "read_calendars_invalid" && hasNumbers(facts, "index"))
    return `Calendar entry ${formatQuantity(facts.index)} has an invalid format.`;
  const state = lookupCopy(states, code);
  if (state) return state;
  const entry = lookupCopy(refusalEntries, code);
  if (entry?.text && refusal) return entry.text(refusal) ?? "Couldn’t read this Orbit state.";
  return "Couldn’t read this Orbit state.";
}
