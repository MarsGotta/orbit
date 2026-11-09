import type { AddedAttachment } from "./generated/AddedAttachment";

/** Where a new attachment's bytes come from; the daemon reads or captures them. */
export type AttachmentSource =
  | { from: "path"; path: string }
  | { from: "bytes"; name: string; data_base64: string }
  | { from: "window" };

/** What a frame provides so the assistant's views can handle attachments. */
export interface AttachmentsFrame {
  addAttachment(source: AttachmentSource): Promise<AddedAttachment>;
  attachmentBytes(attachmentId: string): Promise<ArrayBuffer>;
  openAttachment(attachmentId: string, reveal: boolean): Promise<void>;
  chooseFiles(): Promise<string[]>;
  onFilesDropped(handler: (paths: string[]) => void): Promise<() => void>;
  copyImage(bytes: ArrayBuffer, mime: string): Promise<void>;
}

let frame: AttachmentsFrame | null = null;

/** Called once by each frame, before it mounts anything. */
export function installAttachments(implementation: AttachmentsFrame): void {
  frame = implementation;
}

function installed(): AttachmentsFrame {
  if (frame === null) throw new Error("No frame has installed attachments (installAttachments).");
  return frame;
}

/**
 * Uploads a file to the hub, which keeps it for this device until it is
 * sent; for a window, also what became of its text.
 */
export async function addAttachment(source: AttachmentSource): Promise<AddedAttachment> {
  return installed().addAttachment(source);
}

/** A stored attachment's bytes, for its thumbnail or the viewer. */
export async function attachmentBytes(attachmentId: string): Promise<ArrayBuffer> {
  return installed().attachmentBytes(attachmentId);
}

/** «Abrir» with the Mac's app, or «Mostrar en Finder» when `reveal`. */
export async function openAttachment(attachmentId: string, reveal: boolean): Promise<void> {
  return installed().openAttachment(attachmentId, reveal);
}

/** The files chosen in the open dialog; none when it was cancelled. */
export async function chooseFiles(): Promise<string[]> {
  return installed().chooseFiles();
}

/** Files dropped onto the window, by path; hovering over it is not a drop. */
export async function onFilesDropped(handler: (paths: string[]) => void): Promise<() => void> {
  return installed().onFilesDropped(handler);
}

/** Puts an image on the clipboard. */
export async function copyImage(bytes: ArrayBuffer, mime: string): Promise<void> {
  return installed().copyImage(bytes, mime);
}
