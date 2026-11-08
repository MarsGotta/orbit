import { getLocale } from "../locale";
/** The Markdown editor every note, answer, section, personality and template opens in. */
export const editorCopy = {
  editor: {
    placeholder: "Write or press / to insert…",
    help: [
      ["/", "insert"],
      ["[[", "link a note"],
      ["@", "mention"],
      ["**", "bold"],
      ["- [ ]", "task"],
      ["⌘/", "view Markdown"],
    ] as const,
    helpLabel: "Editor shortcuts",
    markdown: "Markdown",
    backToText: "Back to text",
    showMarkdown: "View Markdown · ⌘/",
    showFormatted: "View formatted text · ⌘/",
    rawLabel: (label: string) => `${label} in Markdown`,

    // The «/» menu: each option and the Markdown it writes.
    insert: "Insert",
    heading: "Title",
    subheading: "Heading",
    bullet: "List",
    ordered: "Numbered list",
    task: "Task",
    quote: "Quote",
    code: "Code",
    rule: "Divider",
    linkNote: "Link a note",
    mention: "Mention someone",
    keepWriting: (name: string) => `Ask ${name} to keep writing`,
    nothingNamed: "No match. Keep typing to leave it as text.",
    menuKeys: ["↑↓ choose", "↵ insert", "esc close"] as const,

    // «[[» and «@».
    linkMenu: "Link a note",
    noNoteNamed: "No note with that name.",
    mentionMenu: "Mention",
    nobodyNamed: "No one with that name in your meetings.",
    invited: "Meeting guest",
    spoke: "Spoke in this meeting",
    known: "From your calendar",

    // The bar over a selection.
    formatBar: "Format selected text",
    bold: "Bold · ⌘B",
    italic: "Italic · ⌘I",
    inlineCode: "Code · ⌘E",
    linkTo: "Link a note · ⌘K",
    highlight: "Highlight · ⌘⇧H",
    ask: (name: string) => `Ask ${name}`,

    // A task's box.
    markTask: "Toggle task",

    // What an assistant wrote, until it is accepted.
    writtenBy: (name: string) => `Written by ${name}`,
    accept: "Accept",
    discard: "Discard",
    keepWritingAsk: "Continue writing this note from here:",

    // Saving.
    saving: "Saving…",
    savedHere: "Saved on this Mac · only you can see it",
    savedHub: "✓ Saved",
    savedMac: "Saved on this Mac · not on the hub yet",
    onlyHere: "Changes stay here until you copy or send",
    notSaved: (why: string) => `Not saved: ${why} Your text is still here.`,
    notSavedUnknown: "Not saved. Your text is still here.",
    retrySave: "Try again",
    nearLimit: (left: number, max: number) =>
      `${left.toLocaleString(getLocale())} of ${max.toLocaleString(getLocale())} characters left.`,
    atLimit: (max: number) =>
      `You’ve reached the ${max.toLocaleString(getLocale())} character limit. Further text won’t be entered.`,
  },
} as const;
