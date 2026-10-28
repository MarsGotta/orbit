import { formatQuantity } from "../locale";
import { interrupted, calendarLimit, daemonReply } from "./readRefusals";
import { numberFormat } from "../locale";
import { busyEntry } from "./busy";
import type { RecoveryStep } from "../../bridge/generated/RecoveryStep";
import { helperRefused, helperReplyInvalid } from "./refusalDetails";
import { attachmentRefusalText } from "./attachments";
import { macPermission } from "./permissions";
import type { Refusal, RefusalCode } from "../../bridge/refusal";
import { openSettings, rearm, systemPrivacy, NoticeAction } from "./notices/types";

/**
 * A refused order as the person sees it: something they can fix,
 * work still in progress, or a failure with the generic message and its code.
 * The interceptor reads this as it reads the notice catalogue.
 * The words say what to do; the refusal's numbers stay in the log.
 */
export type RefusalEntry =
  | { class: "failed"; text?: (refusal: Refusal) => string | undefined }
  | {
      class: "fixable" | "working";
      text: (refusal: Refusal) => string;
      action?: (refusal: Refusal) => NoticeAction;
    };

const failed = { class: "failed" } as const;

function privacyPane(refusal: Refusal): NoticeAction {
  const permission =
    typeof refusal.permission === "string" ? macPermission(refusal.permission) : undefined;
  return permission && "pane" in permission ? systemPrivacy(permission.pane) : openSettings;
}

const permission = {
  class: "fixable",
  text: (refusal: Refusal) => {
    const name =
      typeof refusal.permission === "string" ? macPermission(refusal.permission)?.name : undefined;
    return name
      ? `${name} permission is missing. Grant it in System Settings.`
      : "A macOS permission is missing. Grant it in System Settings.";
  },
  action: privacyPane,
} as const;

const recoverySteps: Record<RecoveryStep, string> = {
  chunks: "fragments",
  upload_jobs: "pending uploads",
  snapshot: "snapshot",
  environment: "service preparation",
  operations: "pending operations",
  call_preference: "call detection preference",
};

export const refusalEntries: Record<RefusalCode, RefusalEntry> = {
  attachment: { class: "fixable", text: attachmentRefusalText },
  encode_failed: {
    class: "failed",
    text: () => "Orbit couldn’t encode the response to that read.",
  },
  helper_reply_invalid: {
    class: "failed",
    text: helperReplyInvalid,
  },
  read_interrupted: { class: "fixable", text: interrupted },
  helper_refused: {
    class: "fixable",
    text: helperRefused,
  },
  internal: failed,
  protocol_version: {
    class: "fixable",
    text: (r) =>
      `Received protocol version ${formatQuantity(r.received)}; version ${formatQuantity(r.required)} is required.`,
  },
  hub_reply_invalid: {
    class: "failed",
    text: (r) =>
      `The hub response doesn’t match the contract: field ${String(r.field)}${typeof r.received_bytes === "number" ? `, ${numberFormat().format(r.received_bytes)} bytes received` : ""}.`,
  },
  daemon_reply_invalid: { class: "failed", text: daemonReply },
  calendar_read_limit: { class: "fixable", text: calendarLimit },

  busy: busyEntry,
  calendar_backend_unavailable: failed,
  device_reply_invalid: failed,
  device_reply_timeout: failed,
  engine_unavailable: failed,
  expired: failed,
  helper_not_running: {
    class: "fixable",
    text: () => "Orbit.app isn’t running. Open it to continue.",
  },
  hub_session_invalid_row: failed,
  hub_sessions_cursor_mismatch: failed,
  hub_sessions_unpaged: failed,
  hub_unreachable: failed,
  invalid_parameter: failed,
  legacy: failed,
  mac_disconnected: failed,
  // «Reintentar transcripción» on a note whose audio this Mac no longer has.
  not_found: {
    class: "failed",
    text: (refusal) =>
      refusal.resource === "audio"
        ? "That voice note’s audio is no longer on this Mac. Transcription can’t be retried. Record it again."
        : undefined,
  },
  not_paired: {
    class: "fixable",
    text: () => "This Mac isn’t paired with the hub. Pair it in Settings.",
    action: () => openSettings,
  },
  outcome_unknown: failed,
  pending_recording: failed,
  physical_work_finishing: failed,
  permission_denied: permission,
  permission_not_granted: permission,
  permission_unknown: {
    class: "working",
    text: (refusal) => {
      const name =
        typeof refusal.permission === "string"
          ? macPermission(refusal.permission)?.name
          : undefined;
      return `Orbit doesn’t yet know whether macOS grants ${name ?? "that permission"}. Wait for the Mac to report it; this isn’t a denial.`;
    },
  },
  privacy_stopped: {
    class: "fixable",
    text: () => "Everything is stopped. Rearm Orbit to continue.",
    action: () => rearm,
  },
  recovering: {
    class: "working",
    text: (refusal) => {
      const step =
        typeof refusal.step === "string" ? recoverySteps[refusal.step as RecoveryStep] : undefined;
      return `Orbit is recovering its store: step ${step ?? "unknown"}, ${formatQuantity(refusal.elapsed_ms)} ms elapsed. Wait until it finishes.`;
    },
  },
  revision_changed: failed,
  storage_failed: failed,
  timeout: failed,
  too_large: failed,
  too_many: failed,
  update: failed,
  upstream_failed: failed,
  work_expired: failed,
  work_in_flight: failed,
  // The assistant's folders: these reach Deimos with their paths; in the app
  // only the missing folder has something to fix.
  no_folders: {
    class: "fixable",
    text: () => "The assistant has no folders from this Mac. Add them in Settings.",
    action: () => openSettings,
  },
  outside_folders: failed,
  folder_unavailable: {
    class: "fixable",
    text: (refusal) =>
      `Folder ${String(refusal.path)} can no longer be opened. Remove it and add it again in Settings.`,
    action: () => openSettings,
  },
  file_unavailable: failed,
  file_too_large: failed,
};
