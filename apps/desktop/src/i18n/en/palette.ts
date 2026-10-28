import { globalShortcuts } from "./shortcuts";

/** «Buscar y órdenes»: what the palette offers, and asking the agent what matches nothing. */
export function paletteCopy(assistantName: string) {
  return {
    palette: {
      dialog: "Command palette",
      // Only while Orbit's window has the focus: not a global shortcut.
      shortcut: "⌘⇧␣",
      inputLabel: "Type a command or question",
      placeholder: `Type a command or ask ${assistantName}…`,
      escape: "esc",
      close: "Close palette",
      groups: { actions: "Actions", goTo: "Go to", privacy: "Privacy" },
      record: { title: "Record a meeting", subtitle: "Choose audio and start" },
      call: { title: `Call ${assistantName}`, subtitle: "Speak aloud" },
      voice: { title: "Send a voice note", subtitle: "Record what you say" },
      prepare: { title: "Prepare for the next meeting" },
      stopAll: {
        title: "Stop everything",
        subtitle: "Microphone, screen and uploads, immediately",
      },
      shortcuts: globalShortcuts,
      ask: `Ask ${assistantName}:`,
      quoted: (text: string) => `“${text}”`,
      noMatch: `No command with that name. Press ↵ to ask ${assistantName}.`,
      footer: { move: "↑↓ move", open: "↵ open", close: "esc close" },
    },
  } as const;
}
