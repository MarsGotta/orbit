import { writeText } from "@tauri-apps/plugin-clipboard-manager";

/**
 * Puts `text` on the Mac's clipboard as plain text. Through the plugin and
 * not `navigator.clipboard`, which makes WebKit ask the user each time
 * (Tauri issue #12007).
 */
export function copyText(text: string): Promise<void> {
  return writeText(text);
}
