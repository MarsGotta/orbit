/** What a frame provides so a view can put text on the clipboard. */
export interface ClipboardFrame {
  copyText(text: string): Promise<void>;
}

let frame: ClipboardFrame | null = null;

/** Called once by each frame, before it mounts anything. */
export function installClipboard(implementation: ClipboardFrame): void {
  frame = implementation;
}

/** Puts `text` on the clipboard as plain text, through whatever the frame installed. */
export async function copyText(text: string): Promise<void> {
  if (frame === null) throw new Error("No frame has installed the clipboard (installClipboard).");
  return frame.copyText(text);
}
