import { formatQuantity } from "../locale";
import type { HelperCode } from "../../bridge/generated/HelperCode";
import type { Refusal } from "../../bridge/refusal";

const helperMessages: Record<HelperCode, string> = {
  accessibility_permission_required:
    "macOS hasn’t granted Orbit Accessibility permission on that Mac, so it can’t use the computer. Grant it in System Settings › Privacy & Security › Accessibility.",
  calendar_busy:
    "EventKit is still finishing an earlier agenda read on that Mac, which Orbit won’t interrupt. Ask again in a few seconds.",
  calendar_clock_unavailable:
    "Orbit couldn’t verify the clock for that agenda read. This does NOT mean the agenda is empty.",
  calendar_event_too_large:
    "An agenda page exceeds the reader’s largest message, even when oversized events are omitted by name, because the omitted-event list is too long. Nothing was truncated. This does NOT mean the agenda is empty. Request a shorter range.",
  calendar_invalidated:
    "The calendar changed while being read. Request the agenda again; its contents are still unknown.",
  calendar_limit_exceeded:
    "An event or calendar has a field longer than Orbit can read. Reading stopped rather than truncating it. This does NOT mean the agenda is empty.",
  calendar_permission_required:
    "Orbit doesn’t have full read access to macOS Calendars, so it can’t read the agenda. This does NOT mean the agenda is empty. Grant access in System Settings › Privacy & Security › Calendars.",
  calendar_read_expired:
    "EventKit didn’t finish reading the agenda within the deadline, even for the smallest range Orbit requests. Ask again in a few seconds.",
  calendar_read_failed:
    "EventKit returned an error while reading the Mac’s agenda. This does NOT mean the agenda is empty.",
  calendar_selection_unavailable:
    "The requested calendar isn’t available for this read. Its contents are still unknown.",
  capture_clock_unavailable:
    "The helper couldn’t verify the capture clock. No result was delivered. Request it again.",
  invalid_calendar_request:
    "The helper rejected the calendar read parameters. Correct them before requesting again.",
  invalid_screen_inventory:
    "The window inventory doesn’t match the helper’s contract. Request the windows again.",
  invalid_screenshot_dimensions:
    "The screenshot has invalid dimensions and wasn’t delivered. Request it again if still needed.",
  invalid_window_binding:
    "The requested window identity is invalid. Request the window list again and choose from it.",
  invalid_window_dimensions:
    "The window has invalid capture dimensions. Request the window list again and choose another.",
  privacy_stopped:
    "Everything was stopped using that Mac’s local controls. Orbit can’t see or act until rearmed on the Mac itself. This isn’t a Mac or connection failure. Ask again once rearmed.",
  screen_inventory_busy:
    "The helper is still listing windows for another request. Wait until it finishes before requesting again.",
  screen_inventory_too_large:
    "The window inventory exceeds the helper’s delivery size. No partial list was delivered.",
  screen_permission_required:
    "macOS hasn’t granted Orbit Screen Recording permission on that Mac. Grant it in System Settings › Privacy & Security › Screen Recording.",
  screen_selection_expired:
    "The window selection expired. Request the windows again before capturing.",
  screen_selection_unavailable:
    "The Mac can’t select that window right now. Request the windows again.",
  screenshot_busy:
    "The helper is finishing another screenshot. Wait until it finishes before requesting another.",
  screenshot_expired:
    "The screenshot didn’t finish within the helper’s deadline. Request it again if still needed.",
  screenshot_sdk_failed:
    "ScreenCaptureKit couldn’t capture that window’s image. Request the screenshot again if still needed.",
  screenshot_too_large:
    "The helper couldn’t encode the PNG within the capture size limit. No image was delivered.",
  selected_text_changed:
    "The text selection changed while being read. Request the selected text again.",
  selected_text_classification_incomplete:
    "The Mac couldn’t verify whether the selected field is secure. Its text wasn’t read.",
  selected_text_empty:
    "Accessibility confirms an empty selection range. No text is selected in that field.",
  selected_text_expired: "The selected text read expired. Request it again if still needed.",
  selected_text_identity_changed:
    "The window or selection changed while being read. Request the selected text again.",
  selected_text_invalid_range: "The selected text range is invalid. Request the selection again.",
  selected_text_invalid_text:
    "Accessibility returned invalid text. It wasn’t used. Request it again.",
  selected_text_limit: "The selection exceeds the helper’s read limit. Select less text.",
  selected_text_not_exposed:
    "The app doesn’t expose the Accessibility attributes needed to read the selection. Whether text is selected is unknown.",
  selected_text_secure_field:
    "The selection is in a protected field, which Orbit won’t read. Choose another field.",
  selected_text_unavailable:
    "Accessibility couldn’t return that window’s selection. Whether text is selected is unknown.",
  selected_text_window_ambiguous:
    "Accessibility couldn’t distinguish the requested window from others. Request the window list and choose an unambiguous one.",
  window_identity_changed:
    "The window changed between listing and reading. Request the window or screenshot again.",
};

export function fields(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {};
}

function expectedText(value: unknown): string {
  const expected = fields(value);
  const kind = expected.kind;
  if (kind === "wire_type") {
    const names: Record<string, string> = {
      string: "text",
      object: "an object",
      boolean: "true or false",
    };
    return names[String(expected.name)] ?? `the ${String(expected.name)} contract`;
  }
  if (kind === "calendar_event")
    return `a valid event in row ${formatQuantity(expected.position)} of ${formatQuantity(expected.rows)}`;
  if (kind === "signed_range" || kind === "unsigned_range")
    return `a value between ${formatQuantity(expected.min)} and ${formatQuantity(expected.max)}; received ${formatQuantity(expected.received)}`;
  if (kind === "text_length")
    return `nonempty text of up to ${formatQuantity(expected.max_bytes)} bytes; received ${formatQuantity(expected.bytes)}`;
  if (kind === "read_duration")
    return `a read shorter than ${formatQuantity(expected.max_exclusive_ns)} ns; it took ${formatQuantity(expected.read_ns)} ns`;
  if (kind === "read_times")
    return `ordered times; started at ${formatQuantity(expected.started_ns)} ns and finished at ${formatQuantity(expected.completed_ns)} ns`;
  if (kind === "current_generation")
    return `generation ${String(expected.required ?? "not observed")}; received ${formatQuantity(expected.received)} and the instance ${expected.instance_matches === true ? "matches" : "doesn’t match"}`;
  if (kind === "clock_domain")
    return `the helper’s clock; received ${formatQuantity(expected.received)}`;
  return "that read’s contract";
}

function receivedType(value: unknown): string {
  const names: Record<string, string> = {
    string: "text",
    number: "number",
    boolean: "boolean",
    object: "object",
    array: "list",
    null: "null",
    absent: "absent data",
  };
  return names[String(value)] ?? "invalid data";
}

export function helperRefused(refusal: Refusal): string {
  return (
    helperMessages[refusal.helper_code as HelperCode] ??
    "The helper rejected the read for a cause this version doesn’t recognize."
  );
}

export function helperReplyInvalid(refusal: Refusal): string {
  return `The helper response doesn’t match the contract: ${String(refusal.field)} must be ${expectedText(refusal.expected)}; received type: ${receivedType(refusal.received_type)}.`;
}
