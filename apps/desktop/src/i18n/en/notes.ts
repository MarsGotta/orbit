import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
import { transcriptionRefusals } from "./transcription";
import { mmss } from "../../time-format";

// Meetings: the list of meetings and the transcript of one. Each meeting
// has a note; «Notas» names the vault that comes later (decided 2026-10-22).
const count = (value: number, one: string, many: string) =>
  `${numberFormat().format(value)} ${pluralRules().select(value) === "one" ? one : many}`;

/** «Bruno», «Bruno y Celia», «Bruno, Celia y Dora». */
export function listOf(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

export const notes = {
  notes: {
    section: "Meetings",
    recordedOn: (name: string) => `Recorded on ${name}`,
    pendingAudioOn: (name: string) =>
      `Pending audio must upload from ${name}. Keep Orbit open there until it finishes.`,
    listLabel: "Meeting list",
    refresh: "Refresh",
    loadMore: "Load more meetings",
    loadingMore: "Loading…",
    loadingIndex: "Reading your meetings from the hub…",
    loadingNote: "Reading the note from the hub…",
    empty: "No meetings yet",
    emptyBody: "Record your first meeting and Orbit will write the note for you.",
    record: "Record meeting",
    choose: "Choose a meeting",
    chooseBody: "Its note, actions and transcript open here.",
    // A read that failed says so and offers «Reintentar»; its cause
    // stays in the log.
    noteReadFailed: "Couldn’t open note",
    noteReadFailedBody: "Try again. Your meetings are still saved on the hub.",
    // A meeting deleted or thrown away, here or on another Mac: not a
    // failure, and nothing to retry.
    noteGone: "This meeting no longer exists",
    noteGoneBody: "It was deleted on this Mac or another. Choose another from the list.",
    indexFailed: "Couldn’t read the meeting list. Try again.",
    // The hub out of reach is not a failure; the connection says
    // itself at the top of the window.
    needsHub: "Your meetings are read from the hub. They’ll appear when Orbit connects.",
    retry: "Try again",
    today: "Today",
    yesterday: "Yesterday",
    thisWeek: "This week",
    last30Days: "Last 30 days",
    before: "Earlier",

    // The transcript.
    retranscribe: "Redo transcript",
    // Transcribing it again (annex, change 7): what it does first, then how
    // far it got, then what is left to do. The hub gives no estimate before
    // the pass starts, so the question does not promise how long it takes.
    retranscribeTitle: "Redo the transcript?",
    retranscribeBody: (length: string, language: string | null, dictionaries: string[]) =>
      `Orbit will listen to the recording again (${length})${language === null ? "" : ` in ${language}`}${dictionaries.length === 0 ? "" : ` with ${listOf(dictionaries)}`}. You can keep doing other things meanwhile.`,
    retranscribeKeeps:
      "Your speaker names will be kept. The summary won’t change until you redo it.",
    // A hub up to 0.10.1 writes the note again after the pass (proposal 35's fallback).
    retranscribeKeepsOlderHub: "Your speaker names will be kept. Orbit will then rewrite the note.",
    stillRecording: "Can’t redo the transcript: the recording hasn’t closed yet.",
    finalPassQueued: "Queued to redo the transcript.",
    finalPassRunning: "Redoing transcript…",
    finalPassAside: " · keep doing other things",
    remaining: (ms: number) => {
      const minutes = Math.ceil(ms / 60_000);
      return minutes <= 1 ? "less than 1 min" : `about ${numberFormat().format(minutes)} min`;
    },
    retranscribed: "Transcript redone.",
    retranscribedRedo: "Transcript redone. Redo the summary to use it in the note.",
    finalPassFailed: (code: string | null, timeoutMs?: number, providerStatus?: number) => {
      if (code && transcriptionRefusals[code])
        return `${transcriptionRefusals[code]}${providerStatus === undefined ? "" : ` (HTTP ${numberFormat().format(providerStatus)})`} The previous transcript is kept.`;
      if (code === "final_pass_incomplete")
        return "The new transcript lost parts of the previous one, so the previous transcript is kept.";
      if (code === "final_pass_timeout") {
        const budget =
          timeoutMs == null
            ? "the hub allows"
            : `${numberFormat({ maximumFractionDigits: 1 }).format(timeoutMs / 60000)} minutes allowed by the hub`;
        return `The new transcript took longer than ${budget}. The previous transcript is kept.`;
      }
      return `Couldn’t complete the new transcript. The previous transcript is kept.${code ? ` (${code})` : ""}`;
    },
    // A stretch the hub never got (a chunk it refused), said where it is.
    missingAudio: (track: string, atMs: number | null, durationMs: number | null) => {
      const source = track === "mic_local" ? "from the microphone" : "from the meeting";
      const from = atMs === null ? "" : ` from ${mmss(atMs)}`;
      const length =
        durationMs === null
          ? ""
          : ` for ${numberFormat().format(Math.max(1, Math.round(durationMs / 1_000)))} s`;
      return `Audio ${source}${from}${length} is missing: it didn’t reach the hub, so this part isn’t in the transcript.`;
    },
    // «Silenciar»: a stretch where the microphone was muted, from the meeting's start.
    mutedMicrophone: (startMs: number, endMs: number) =>
      `Microphone muted · ${mmss(startMs)} – ${mmss(endMs)}`,
    // A completed pass that kept spans of the live draft where the new text
    // lost them or looped: not a failure, the better of the two was used.
    keptDraft: (spans: number) =>
      `The live transcript was better in ${numberFormat().format(spans)} ${pluralRules().select(spans) === "one" ? "span" : "spans"}, so it was kept there.`,
    unmatchedSpeakerNames: (value: number) =>
      `${numberFormat().format(value)} ${pluralRules().select(value) === "one" ? "name belonged to a voice no longer present and wasn’t assigned" : "names belonged to voices no longer present and weren’t assigned"} to other voices.`,
    // The harness reads the transcript from this line on (scripts/e2e-mac/checks.py).
    transcriptHelp:
      "Click a name to change the speaker. Hover over a word and click to correct it. The correction is saved in this meeting’s dictionaries.",
    transcriptArriving: "The transcript is arriving. Click a name to change the speaker.",
    moreTranscript: "More transcript arriving",
    noTranscript: "The transcript isn’t ready yet",
    transcriptFailed: "Couldn’t load transcript.",
    searchLabel: "Search transcript",
    searchResults: (value: number) =>
      value === 0 ? "No results" : count(value, "result", "results"),
    copy: "Copy",
    copied: "✓ Copied",
    languages: {
      es: "Español",
      en: "English",
      ca: "Català",
      fr: "Français",
      pt: "Português",
      de: "Deutsch",
      it: "Italiano",
    } as const,
    // The same languages inside a sentence: «en español con Nubaria».
    languagesInSentence: {
      es: "Spanish",
      en: "English",
      ca: "Catalan",
      fr: "French",
      pt: "Portuguese",
      de: "German",
      it: "Italian",
    } as const,

    // Who speaks.
    nameIt: "· add a name",
    whoIs: (name: string) => `Who is ${name}?`,
    speakerSearch: "Search or add a name",
    guests: "Meeting guests",
    recordingSpeakers: "Already in this recording",
    addSomeone: "+ Add someone",
    addNamed: (name: string) => `+ Add “${name}”`,
    wherePut: (name: string) => `Where should “${name}” go?`,
    onlyThisTurn: (at: string) => `Only this turn (${at})`,
    allSpeakerTurns: (turns: number, name: string) =>
      pluralRules().select(turns) === "one"
        ? `All turns by ${name}`
        : `All ${numberFormat().format(turns)} turns by ${name}`,
    back: "Back",
    applySpeaker: "Apply",
    renamed: (turns: number) => `Changed in ${count(turns, "turn", "turns")}.`,
    usesOnRewrite: "The note will use this change when you redo the summary.",

    // Correcting a word (annex, change 5): the menu beside it, then the band.
    pressToCorrect: "Click to correct",
    correct: (word: string) => `Correct “${word}”`,
    timesHere: (times: number) =>
      pluralRules().select(times) === "one"
        ? "Appears once in this meeting."
        : `Appears ${numberFormat().format(times)} times in this meeting.`,
    shouldSay: "Should say",
    // A dictionary corrects every place it is said: the hub has no
    // correction for one place alone, so this cannot be unticked.
    correctEvery: (times: number) => `Correct all ${numberFormat().format(times)} occurrences`,
    addTo: "Add to · you can choose several",
    chooseDictionaries: "Choose one or more",
    thisMeeting: "this meeting",
    dictionaryDetail: (series: string[], words: number) =>
      `${series.length === 0 ? "No series" : listOf(series)} · ${count(words, "word", "words")}`,
    newDictionary: "+ Create a new dictionary",
    dictionaryName: "Dictionary name",
    appliesIn: (names: string[]) => `Orbit will correct it in meetings that use ${listOf(names)}.`,
    chooseOne: "Choose at least one dictionary to save the correction.",
    // A selection that is not whole words: a dictionary would never find it.
    notWhole: (text: string) =>
      `“${text}” doesn’t appear as a whole word or phrase in this meeting. Dictionaries correct whole words and phrases.`,
    correctAndAdd: "Correct and add",
    corrected: (wrong: string, right: string, places: number, names: string[]) =>
      `“${wrong}” is now “${right}” in ${count(places, "place", "places")} and saved in ${listOf(names)}.`,
    undo: "Undo",
    undone: (wrong: string, names: string[]) =>
      `Correction undone. “${wrong}” is no longer in ${listOf(names)}.`,
    // Not drawn: two dictionaries of the meeting hear the form otherwise.
    conflict: (other: string, wrong: string, otherCorrect: string, applied: string | null) =>
      `${other} corrects “${wrong}” to “${otherCorrect}”. When dictionaries disagree, the first wins${applied === null ? "" : `: ${applied} in this meeting`}.`,

    save: "Save",
    cancel: "Cancel",
  },
} as const;
