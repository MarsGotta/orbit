import { invoke } from "@tauri-apps/api/core";
import { Image } from "@tauri-apps/api/image";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import type { AddedAttachment } from "./generated/AddedAttachment";

/** Where a new attachment's bytes come from; the daemon reads or captures them. */
export type AttachmentSource =
  | { from: "path"; path: string }
  | { from: "bytes"; name: string; data_base64: string }
  | { from: "window" };

/**
 * Uploads a file to the hub, which keeps it for this device until it is
 * sent; for a window, also what became of its text.
 */
export function addAttachment(source: AttachmentSource): Promise<AddedAttachment> {
  return invoke<AddedAttachment>("orbit_attachment_add", { source });
}

/** A stored attachment's bytes, for its thumbnail or the viewer. */
export function attachmentBytes(attachmentId: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("orbit_attachment_content", { attachmentId });
}

/** «Abrir» with the Mac's app, or «Mostrar en Finder» when `reveal`. */
export function openAttachment(attachmentId: string, reveal: boolean): Promise<void> {
  return invoke<void>("orbit_attachment_open", { attachmentId, reveal });
}

/** The files chosen in the open dialog; none when it was cancelled. */
export async function chooseFiles(): Promise<string[]> {
  return (await open({ multiple: true, directory: false })) ?? [];
}

/** Files dropped onto the window, by path; hovering over it is not a drop. */
export async function onFilesDropped(handler: (paths: string[]) => void): Promise<() => void> {
  return getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type === "drop" && event.payload.paths.length > 0)
      handler(event.payload.paths);
  });
}

/**
 * Puts an image on the Mac's clipboard. Not through `navigator.clipboard`,
 * which makes WebKit ask each time (see `clipboard.ts`): the webview decodes
 * it to RGBA and the clipboard plugin writes those pixels.
 */
export async function copyImage(bytes: ArrayBuffer, mime: string): Promise<void> {
  const bitmap = await createImageBitmap(new Blob([bytes], { type: mime }));
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("canvas 2d unavailable");
  context.drawImage(bitmap, 0, 0);
  bitmap.close();
  const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
  const image = await Image.new(new Uint8Array(pixels.buffer), canvas.width, canvas.height);
  try {
    // The plugin's command, by the image's resource id: its JS wrapper only
    // recognises an `Image` of its own copy of @tauri-apps/api (2.12.4), not
    // the app's (2.11.1), and would send the object instead of the id.
    await invoke("plugin:clipboard-manager|write_image", { image: image.rid });
  } finally {
    await image.close();
  }
}
