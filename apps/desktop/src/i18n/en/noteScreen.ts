import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
// The note of a meeting. Each step says where it stands and, when it failed,
// what to do; the hub's codes and numbers stay in the log.
import { listOf } from "./notes";

const left = (value: number) =>
  value === 0
    ? "All done"
    : pluralRules().select(value) === "one"
      ? "1 task left"
      : `${numberFormat().format(value)} tasks left`;

const base = {
  noteScreen: {
    titleLabel: "Meeting title",
    titleNotOnHub: "You can rename it once the recording finishes uploading to the hub.",
    changesOnMac:
      "Changes saved on this Mac haven’t reached the hub yet. They’ll be sent automatically.",
    whileRecording: "The note is written when recording ends.",
    withPeople: (names: string[]) => listOf([...names, "you"]),
    minutes: (value: number) => `${numberFormat().format(value)} min`,
    hours: (hours: number, minutes: number) =>
      minutes === 0
        ? `${numberFormat().format(hours)} h`
        : `${numberFormat().format(hours)} h ${numberFormat().format(minutes)} min`,

    // How Orbit writes this note: proposals 8 and 9, nothing rewritten on a
    // mere change.
    // The harness reads the language as the second part (scripts/e2e-mac/checks.py).
    howLabel: (template: string, language: string | null, dictionaries: number) =>
      [
        `Template: ${template}`,
        ...(language === null ? [] : [language]),
        ...(dictionaries === 0
          ? []
          : [
              pluralRules().select(dictionaries) === "one"
                ? "1 dictionary"
                : `${numberFormat().format(dictionaries)} dictionaries`,
            ]),
      ].join(" · "),
    howTitle: "How Orbit writes this note",
    // «Borrar reunión», in the note's menu, confirmed in place.
    meetingMenu: "More",
    meetingMenuTitle: "This meeting",
    deleteMeeting: "Delete meeting",
    deleteQuestion: "Delete this meeting?",
    deleteDetail:
      "Audio, transcript and note will be deleted from this Mac and the hub. This can’t be undone.",
    deleteWhenClosed: "Will be deleted once it finishes closing",
    template: "Template",
    language: "Meeting language",
    dictionaries: "Word dictionaries",
    noDictionaries: "None: transcription won’t correct words",
    manageDictionaries: "Manage dictionaries in Settings",
    useForSeries: "Use the same settings for the whole series",
    rewriteWarning: (language: string | null) =>
      `Orbit will rewrite the note${language === null ? "" : ` and retranscribe the meeting in ${language}`}. If you edited the summary, your version will be kept.`,
    rewriteNote: "Rewrite note",
    rewriting: (template: string) =>
      `Rewriting the note with “${template}”. Your previous version will be kept.`,
    retranscribing: (done: number, total: number) =>
      `Transcribing again… ${numberFormat().format(done)} of ${numberFormat().format(total)}`,
    templatesUnread: "Templates haven’t been read from the hub yet.",
    templatesFailed: "Couldn’t read templates from the hub.",
    dictionariesUnread: "Dictionaries haven’t been read from the hub yet.",
    seriesRefused: (code: string) => seriesCodes[code],
    close: "Close",
    save: "Save",
    redoSummary: "Redo summary",
    redoSummaryTitle: "Redo the summary?",
    redoSummaryBody: (template: string) =>
      `Orbit will rewrite it using the ${template} template and current transcript. If you edited it, your version will be kept.`,

    // Writing the note.
    preparing: "Orbit is preparing your note",
    preparingBody:
      "Usually takes less than a minute. Keep doing what you need to; we’ll let you know when it’s ready.",
    progressLabel: "Progress",
    steps: {
      upload: {
        done: "Audio saved",
        active: "Saving audio…",
        waiting: "Save audio",
        failed: "Couldn’t save audio",
      },
      transcript: {
        done: "Transcript ready",
        active: "Transcribing…",
        waiting: "Transcribe",
        failed: "Couldn’t transcribe",
      },
      summary: {
        done: "Summary written",
        active: "Writing summary…",
        waiting: "Write summary",
        failed: "Couldn’t write summary",
      },
    },
    hubUnavailable: "No connection to the hub. Continues when it reconnects.",
    noteFailed: "Couldn’t write note",
    noteFailedBody: "The hub couldn’t write the note. Try again.",
    notConfigured:
      "The hub has no model for writing notes. Configure one on the hub and try again.",
    notRequested: "This meeting has no note yet. Choose “Redo summary” to write one.",
    transcriptFailed: "Couldn’t transcribe the whole meeting",
    transcriptFailedBody: "Some audio couldn’t be transcribed. Try again.",
    retry: "Try again",

    // The parts of the note.
    partsLabel: "Note sections",
    summaryTab: "Summary",
    actionsTab: "Actions",
    actionsTabCount: (value: number) => `Actions · ${numberFormat().format(value)}`,
    transcriptTab: "Transcript",
    privateTab: "Private notes",

    copySummary: "Copy summary",
    summaryLater: (template: string) =>
      `The summary will appear here once it’s written, with the sections from your “${template}” template.`,
    decisions: "Decisions",
    openTopics: "Open topics",
    whoSpoke: "Who spoke",
    share: (name: string, percent: number) => `${name} ${numberFormat().format(percent)}%`,
    supersededAvailable: "Your previous version was set aside when the note was rewritten.",
    restoreSuperseded: "Restore my version",
    edit: "Edit",
    done: "Done",
    editSection: (name: string) => `Edit “${name}”`,
    sectionLabel: (name: string) => `Text of “${name}”`,

    copyActions: "Copy all",
    actionsLeft: left,
    actionsLater: "Tasks will appear once the note is written, grouped by person with yours first.",
    noActions: "No tasks came up.",
    unassigned: "Unassigned",
    you: "You",
    actionDue: (due: string) => `due ${due}`,

    privateLabel: "Private notes",

    // The notice when a note is ready (proposal 7).
    ready: "Your note is ready",
    open: "Open",

    // The list's one line.
    listRecording: "Recording",
    listUploading: "Saving audio…",
    listTranscribing: "Transcribing…",
    listTranscribed: "Transcript ready",
    listTranscriptFailed: "Couldn’t transcribe",
    listWriting: "Writing note…",
    listFailed: "Couldn’t write note",
  },
} as const;

// «Usar lo mismo en toda la serie» refused by the hub, by its code; nothing
// was saved. Other codes fall back to the library's or the dictionary's words.
const seriesCodes: Record<string, string | undefined> = {
  meeting_series_missing: "This meeting isn’t part of a calendar series.",
  recording_not_found: "The hub no longer has this meeting.",
  hub_route_missing: "Your hub doesn’t support saving settings for a series yet. Update it.",
};

export function noteScreenCopy(assistantName: string) {
  return {
    ...base,
    noteScreen: { ...base.noteScreen, askDeimos: `Ask ${assistantName}` },
  } as const;
}
