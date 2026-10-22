import type { Attachment } from "../../bridge/generated/Attachment";
import type { AttachmentLimits } from "../../bridge/generated/AttachmentLimits";
import type { Refusal } from "../../bridge/refusal";
import { es } from "../../i18n/es";
import { attachmentRefusalText } from "../../i18n/es/attachments";
import { refusalEntries } from "../../i18n/es/refusalEntries";

/** A file on its way into the next message, as its chip shows it. */
export type DraftAttachment =
  | { key: number; state: "uploading"; name: string; bytes?: number }
  | { key: number; state: "ready"; name: string; attachment: Attachment }
  | { key: number; state: "failed"; name: string; refusal: Refusal };

function refusal(cause: Record<string, unknown>): Refusal {
  return { code: "attachment", cause, retry: { kind: "after_fix" } };
}

/** What a chip that will be sent weighs; an upload of unknown size, nothing yet. */
function weight(draft: DraftAttachment): number {
  if (draft.state === "ready") return draft.attachment.bytes;
  return draft.state === "uploading" ? (draft.bytes ?? 0) : 0;
}

function messageCeiling(
  limits: AttachmentLimits,
  others: DraftAttachment[],
  name: string,
  bytes: number,
): Refusal | null {
  const kept = others.filter((draft) => draft.state !== "failed");
  const total = kept.reduce((sum, draft) => sum + weight(draft), bytes);
  if (total <= limits.max_message_bytes) return null;
  return refusal({
    kind: "too_large",
    ceiling: "message",
    name,
    files: kept.length + 1,
    bytes: total,
    max_bytes: limits.max_message_bytes,
  });
}

/**
 * The hub's rule a new file would break, checked here before uploading it:
 * a hub without attachments, the files and bytes of one message, and the
 * size of one file when it is known (pasted bytes). The hub checks again.
 */
export function localRefusal(
  limits: AttachmentLimits | undefined,
  drafts: DraftAttachment[],
  name: string,
  bytes?: number,
): Refusal | null {
  if (limits === undefined) return refusal({ kind: "unsupported" });
  const files = drafts.filter((draft) => draft.state !== "failed").length + 1;
  if (files > limits.max_message_files)
    return refusal({ kind: "too_many", files, max_files: limits.max_message_files });
  if (bytes === undefined) return null;
  if (bytes > limits.max_file_bytes)
    return refusal({
      kind: "too_large",
      ceiling: "file",
      name,
      bytes,
      max_bytes: limits.max_file_bytes,
    });
  return messageCeiling(limits, drafts, name, bytes);
}

/** Once the hub said a file's size: whether the message still fits. */
export function uploadedRefusal(
  limits: AttachmentLimits | undefined,
  others: DraftAttachment[],
  attachment: Attachment,
): Refusal | null {
  if (limits === undefined) return null;
  return messageCeiling(limits, others, attachment.name, attachment.bytes);
}

/** Why a file could not be attached or loaded, naming it and its numbers. */
export function attachmentFailure(failure: Refusal, name: string): string {
  if (failure.code === "attachment") return attachmentRefusalText(failure);
  const entry = Object.hasOwn(refusalEntries, failure.code)
    ? refusalEntries[failure.code]
    : undefined;
  if (entry !== undefined && entry.class !== "failed") return entry.text(failure);
  return entry?.text?.(failure) ?? es.attachments.failed(name, failure.code);
}
