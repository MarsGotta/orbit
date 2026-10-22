const base = {
  appName: "Orbit",
  loading: "Conectando con Orbit…",
  connectFailed: "No se pudo conectar con Orbit. Ciérralo y vuelve a abrirlo.",
  retry: "Reintentar",
  design: {
    askOrbit: "Paleta de órdenes",
    commandInputLabel: "Comando",
    commandPlaceholder: "Pregunta, dicta o llama…",
    escape: "esc",
    undo: "Deshacer",
    expandCall: "Abrir",
    collapseCall: "Minimizar",
    hangUp: "Colgar",
    endCallFromOrb: "Colgar desde el orbe",
  },
  context: {
    now: "ahora",
  },
} as const;

/** The window's frame, with the conversation's agent where it is asked or called. */
export function statusCopy(assistantName: string) {
  return {
    ...base,
    design: {
      ...base.design,
      inCall: `En llamada con ${assistantName}`,
      callAssistant: `Llamar a ${assistantName}`,
    },
  } as const;
}
