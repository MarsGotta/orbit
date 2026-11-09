import { globalShortcuts } from "./shortcuts";

/** «Buscar y órdenes»: what the palette offers, and asking the agent what matches nothing. */
export function paletteCopy(assistantName: string) {
  return {
    palette: {
      dialog: "Paleta de órdenes",
      // Only while Orbit's window has the focus: not a global shortcut.
      shortcut: "⌘⇧␣",
      inputLabel: "Escribe una orden o una pregunta",
      placeholder: `Escribe una orden o pregunta a ${assistantName}…`,
      escape: "esc",
      close: "Cerrar la paleta",
      groups: { actions: "Acciones", goTo: "Ir a", privacy: "Privacidad" },
      record: { title: "Grabar una reunión", subtitle: "Elige qué audio y empieza" },
      call: { title: `Llamar a ${assistantName}`, subtitle: "Habla en voz alta" },
      voice: { title: "Mandar una nota de voz", subtitle: "Graba lo que digas" },
      prepare: { title: "Preparar la siguiente reunión" },
      stopAll: { title: "Parar todo", subtitle: "Micrófono, pantalla y subidas, al instante" },
      shortcuts: globalShortcuts,
      ask: `Preguntar a ${assistantName}:`,
      quoted: (text: string) => `«${text}»`,
      noMatch: `Ninguna orden se llama así. Pulsa ↵ y se lo preguntamos a ${assistantName}.`,
      footer: { move: "↑↓ moverte", open: "↵ abrir", close: "esc cerrar" },
    },
  } as const;
}
