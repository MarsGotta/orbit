import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
import type { Refusal } from "../../bridge/refusal";
import { fields } from "./refusalDetails";

const KIB = 1024;
const MIB = 1024 * KIB;
const GIB = 1024 * MIB;
const decimals = numberFormat({ maximumFractionDigits: 1 });

/**
 * A size as the person reads it: «512 B», «1,5 KB», «20 MB», «82,3 GB». In
 * units of 1024, as the hub's ceilings are set, so a ceiling reads as a
 * round number.
 */
export function fileSize(bytes: number): string {
  if (bytes < KIB) return `${numberFormat().format(bytes)} B`;
  if (bytes < MIB) return `${decimals.format(bytes / KIB)} KB`;
  if (bytes < GIB) return `${decimals.format(bytes / MIB)} MB`;
  return `${decimals.format(bytes / GIB)} GB`;
}

function size(value: unknown): string {
  return typeof value === "number" ? fileSize(value) : "an unknown size";
}

/** The file between Spanish quotes «», or «el fichero» when the refusal has no name. */
function quoted(name: unknown): string {
  return typeof name === "string" && name !== "" ? `“${name}”` : "the file";
}

/** «de «plano.pdf»», or «del fichero»: Spanish contracts «de el». */
function ofFile(name: unknown): string {
  const file = quoted(name);
  return file === "the file" ? "of the file" : `of ${file}`;
}

function capitalised(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** The conversation's ceiling: what it already holds, what the message adds, the maximum. */
export function sessionCeilingText(stored: unknown, bytes: unknown, maxBytes: unknown): string {
  const held =
    typeof stored === "number"
      ? `This conversation already stores ${size(stored)} of attachments and this message adds ${size(bytes)}`
      : `Adding ${size(bytes)} of attachments would put this conversation over the limit`;
  return `${held}; the limit per conversation is ${size(maxBytes)}. Start a new conversation to attach more.`;
}

export function tooManyText(files: unknown, maxFiles: unknown): string {
  return `This message would have ${String(files)} attachments; the maximum per message is ${String(maxFiles)}.`;
}

export const ATTACHMENT_NOT_FOUND =
  "That attachment is no longer on the hub (unsent attachments expire after an hour). Attach it again.";

function tooLarge(cause: Record<string, unknown>): string {
  const max = size(cause.max_bytes);
  if (cause.ceiling === "file")
    return `${capitalised(quoted(cause.name))} is ${size(cause.bytes)}; the limit per file is ${max}.`;
  if (cause.ceiling === "session")
    return sessionCeilingText(cause.session_bytes, cause.bytes, cause.max_bytes);
  const files =
    typeof cause.files === "number"
      ? `The ${numberFormat().format(cause.files)} attachments`
      : "The attachments";
  return `${files} in this message take ${size(cause.bytes)}; the limit per message is ${max}.`;
}

function unreadable(cause: Record<string, unknown>): string {
  const file = quoted(cause.name);
  const reasons: Record<string, string> = {
    missing: `${capitalised(file)} is no longer at that location.`,
    permission: `Orbit doesn’t have permission to read ${file}.`,
    not_a_file: `${capitalised(file)} isn’t a file. Folders can’t be attached.`,
    io: `macOS couldn’t read ${file}.`,
    invalid_bytes: `The content ${ofFile(cause.name)} is damaged or doesn’t match its type.`,
    pdf_locked: `${capitalised(file)} is password-protected.`,
    pdf_damaged: `${capitalised(file)} is damaged: the hub couldn’t read the PDF.`,
    render_failed: `The hub couldn’t convert the pages ${ofFile(cause.name)} to images.`,
    // A HEIC photo, which the Mac turns into JPEG before uploading it.
    photo_unreadable: `macOS couldn’t read photo ${file} to convert it to JPEG.`,
    photo_jpeg_failed: `macOS couldn’t convert ${file} to JPEG.`,
    helper_restarted: `Orbit was interrupted while converting ${file} to JPEG. Attach it again.`,
  };
  return reasons[String(cause.reason)] ?? `Couldn’t read ${file} (cause ${String(cause.reason)}).`;
}

function refusedByAgent(cause: Record<string, unknown>): string {
  const name = typeof cause.name === "string" ? ` “${cause.name}”` : "";
  const code =
    typeof cause.upstream_code === "number"
      ? ` (code ${numberFormat().format(cause.upstream_code)})`
      : "";
  return `The agent rejected the image${name}${code}.`;
}

/** An attachment refusal in words: the file, and the numbers of the rule that fired. */
export function attachmentRefusalText(refusal: Refusal): string {
  const cause = fields(refusal.cause);
  switch (cause.kind) {
    case "unsupported":
      return "This hub doesn’t support attachments. Update it.";
    case "too_large":
      return tooLarge(cause);
    case "too_many":
      return tooManyText(cause.files, cause.max_files);
    case "type_unsupported":
      return `${capitalised(quoted(cause.name))} is ${String(cause.mime)}, which no agent can read. Supported attachments: PNG, JPEG, GIF or WebP images, PDFs and text files.`;
    case "unreadable":
      return unreadable(cause);
    case "refused":
      return refusedByAgent(cause);
    case "not_found":
      return ATTACHMENT_NOT_FOUND;
    default:
      return `Couldn’t attach due to a cause this version doesn’t recognize (${String(cause.kind)}).`;
  }
}

/** Why «Adjuntar ventana» sent the capture without the window's text. */
const WINDOW_TEXT_MISSING: Record<string, string> = {
  empty: "The window has no readable text. Only the screenshot is attached.",
  accessibility_permission_required:
    "Without Accessibility permission, Orbit can’t read the window’s text. Only the screenshot is attached.",
  timeout: "The app didn’t provide text in time. Only the screenshot is attached.",
};

/** What the composer and the messages say about attachments. */
export const attachmentsCopy = {
  attachments: {
    attach: "Attach",
    attachWindow: "Attach window",
    unsupported: "This hub doesn’t support attachments",
    window: "Window",
    remove: (name: string) => `Remove ${name}`,
    uploading: "Uploading…",
    pastedImage: "Pasted image.png",
    pastedText: "Pasted text.txt",
    failed: (name: string, code: string) => `Couldn’t attach “${name}” (${code}).`,
    // Uploading, «Ver» or «Abrir» with the hub stopped or out of reach.
    hubUnreachable: (name: string) =>
      `No connection to the hub. Try “${name}” again when it reconnects.`,
    windowTextMissing: (cause: string) =>
      WINDOW_TEXT_MISSING[cause] ??
      `Couldn’t read its text (cause ${cause}). Only the screenshot is attached.`,
    windowTextRefused: (reason: string) => `Its text wasn’t attached: ${reason}`,
    pages: (pages: number) =>
      pluralRules().select(pages) === "one" ? "1 page" : `${numberFormat().format(pages)} pages`,
    view: "View",
    viewImage: (name: string) => `View ${name}`,
    open: "Open",
    reveal: "Show in Finder",
    loading: "Loading…",
    previous: "Previous",
    next: "Next",
    copyImage: "Copy image",
    copiedImage: "✓ Copied",
    copying: "Copying…",
    all: "Conversation attachments",
    opening: "Opening…",
    close: "Close",
    escHint: "esc",
    image: "Image",
    pdf: "PDF",
    file: "File",
    position: (index: number, total: number) =>
      `${numberFormat().format(index)} of ${numberFormat().format(total)}`,
  },
} as const;
