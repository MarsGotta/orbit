/**
 * Whether this webview is the floating panel's window. Tauri names it
 * `panel` (`src-tauri/tauri.conf.json`); in a browser, `?ventana=panel` shows
 * the same root for development.
 */
export function isPanelWindow(): boolean {
  const scope = window as unknown as {
    __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: unknown } } };
  };
  try {
    if (scope.__TAURI_INTERNALS__?.metadata?.currentWindow?.label === "panel") return true;
    return new URLSearchParams(window.location.search).get("ventana") === "panel";
  } catch {
    return false;
  }
}
