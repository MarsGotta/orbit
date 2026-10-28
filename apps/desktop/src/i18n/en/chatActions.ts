import { numberFormat } from "../locale";
import { getLocale } from "../locale";
/** What the chat says about copying, retrying, listening and editing an answer. */
const base = {
  chatActions: {
    copy: "Copy",
    copied: "✓ Copied",
    // An icon's tooltip once it copied: the ✓ is the icon.
    copiedTip: "Copied",
    copyFailed: (cause: string) => `Couldn’t copy: ${cause}`,
    copyAnswer: "Copy answer",
    copyMessage: "Copy message",
    copyCode: "Copy code",
    copyCommand: "Copy command",
    command: "Command",
    copyTable: "Copy table",
    listen: "Listen",
    stopListening: "Stop",
    openEditor: "Edit and copy",
    openingEditor: "Opening editor…",
    replyToThis: "Reply to this",
    // A command that deletes is approved one at a time (hub-v1 «Choices and questions»).
    deletes: (paths?: number) =>
      paths === undefined
        ? "Deletes files. Can’t be undone."
        : `Deletes ${numberFormat().format(paths)} ${paths === 1 ? "file" : "files"}. Can’t be undone.`,
    deletesOneAtATime: "Only one delete command can be allowed at a time.",
    copyConversation: "Copy conversation",
    conversationPartial: (copied: number, total: number) =>
      `Copied ${numberFormat().format(copied)} of ${numberFormat().format(total)} messages. The hub didn’t provide the rest.`,
    lastCodeCopied: "✓ Last code block copied",
    noCodeBlock: "There are no code blocks in this conversation.",
    you: "You",
    voiceNoteSuffix: "(voice note)",
    editorTitle: "Edit answer",
    editorTextLabel: "Answer text",
    charactersCopied: (count: number) =>
      `${count.toLocaleString(getLocale())} characters · copied as Markdown`,
    copiedConversation: "✓ Copied",
    close: "Close",
    closeEditor: "Close editor",
  },
} as const;

export function chatActionsCopy(assistantName: string) {
  return {
    chatActions: { ...base.chatActions, sendBack: `Send to ${assistantName}` },
  } as const;
}
