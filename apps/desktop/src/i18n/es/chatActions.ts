/** What the chat says about copying, retrying, listening and editing an answer. */
const base = {
  chatActions: {
    copy: "Copiar",
    copied: "✓ Copiado",
    copyFailed: (cause: string) => `No se pudo copiar: ${cause}`,
    copyAnswer: "Copiar respuesta",
    copyMessage: "Copiar mensaje",
    copyCode: "Copiar código",
    copyTable: "Copiar tabla",
    listen: "Escuchar",
    stopListening: "Parar",
    openEditor: "Editar y copiar",
    openingEditor: "Abriendo el editor…",
    replyToThis: "Responder a esto",
    copyConversation: "Copiar conversación",
    conversationPartial: (copied: number, total: number) =>
      `Se copiaron ${copied} de ${total} mensajes: el hub no dio el resto.`,
    lastCodeCopied: "✓ Último bloque de código copiado",
    noCodeBlock: "No hay ningún bloque de código en la conversación.",
    you: "Tú",
    voiceNoteSuffix: "(nota de voz)",
    editorTitle: "Editar la respuesta",
    editorTextLabel: "Texto de la respuesta",
    charactersCopied: (count: number) =>
      `${count.toLocaleString("es-ES")} caracteres · se copia como Markdown`,
    copiedConversation: "✓ Copiada",
    close: "Cerrar",
    closeEditor: "Cerrar el editor",
  },
} as const;

export function chatActionsCopy(assistantName: string) {
  return {
    chatActions: { ...base.chatActions, sendBack: `Mandar a ${assistantName}` },
  } as const;
}
