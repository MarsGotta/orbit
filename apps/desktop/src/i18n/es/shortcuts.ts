/**
 * The global shortcuts the macOS helper registers, by the action it reports
 * (native/macos/Sources/OrbitNativeHost/GlobalHotkeys.swift). A text that names
 * a global shortcut takes it from here; this table and what the helper
 * registers are kept equal by hand. The command palette's shortcut is not one of them:
 * it only works while Orbit's window has the focus.
 */
export const globalShortcuts = {
  ptt: "⌃⌥Espacio",
} as const;
