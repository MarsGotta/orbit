import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
import { getLocale } from "../locale";
/** What the chat says about copying, retrying, listening and editing an answer. */
const base = {
  chatActions: {
    copy: "Copiar",
    copied: "✓ Copiado",
    // An icon's tooltip once it copied: the ✓ is the icon.
    copiedTip: "Copiado",
    copyFailed: (cause: string) => `No se pudo copiar: ${cause}`,
    copyAnswer: "Copiar respuesta",
    copyMessage: "Copiar mensaje",
    copyCode: "Copiar código",
    copyCommand: "Copiar comando",
    command: "Comando",
    copyTable: "Copiar tabla",
    listen: "Escuchar",
    stopListening: "Parar",
    openEditor: "Editar y copiar",
    openingEditor: "Abriendo el editor…",
    replyToThis: "Responder a esto",
    // A command that deletes is approved one at a time (hub-v1 «Choices and questions»).
    deletes: (paths?: number) =>
      paths === undefined
        ? "Borra archivos. No se puede deshacer."
        : `Borra ${numberFormat().format(paths)} ${pluralRules().select(paths) === "one" ? "archivo" : "archivos"}. No se puede deshacer.`,
    deletesOneAtATime: "Un comando que borra solo se permite de uno en uno.",
    copyConversation: "Copiar conversación",
    conversationPartial: (copied: number, total: number) =>
      `Se copiaron ${numberFormat().format(copied)} de ${numberFormat().format(total)} mensajes: el hub no dio el resto.`,
    lastCodeCopied: "✓ Último bloque de código copiado",
    noCodeBlock: "No hay ningún bloque de código en la conversación.",
    you: "Tú",
    voiceNoteSuffix: "(nota de voz)",
    editorTitle: "Editar la respuesta",
    editorTextLabel: "Texto de la respuesta",
    charactersCopied: (count: number) =>
      `${count.toLocaleString(getLocale())} caracteres · se copia como Markdown`,
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
