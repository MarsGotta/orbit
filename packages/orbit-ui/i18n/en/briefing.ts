import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
import { dateTime, listFormat } from "../locale";
import { duration } from "./duration";
import { globalShortcuts } from "./shortcuts";

function dayBounds(): string {
  const clock = dateTime({ hour: "numeric", minute: "2-digit", timeZone: "UTC" });
  return `${clock.format(8 * 3_600_000)} to ${clock.format(20 * 3_600_000)}`;
}

const pendingLabel = "unfinished work from last time";

const base = {
  briefing: {
    morning: "Good morning",
    afternoon: "Good afternoon",
    night: "Good evening",
    // Without the Mac's zone the hour of the day is not known.
    hello: "Hello",
    week: (week: number) => `week ${numberFormat().format(week)}`,
    daySentence: (meetings: number, focusMinutes: number) => {
      if (meetings === 0) return "You have no meetings today.";
      const count =
        pluralRules().select(meetings) === "one"
          ? "1 meeting"
          : `${numberFormat().format(meetings)} meetings`;
      return focusMinutes === 0
        ? `You have ${count} today and no free time to focus.`
        : `You have ${count} today and ${duration(focusMinutes)} free to focus.`;
    },
    nextStartsIn: (minutes: number) => `Up next · starts in ${duration(minutes)}`,
    nextNow: "Now · in progress",
    videoCall: "video call",
    withPeople: (names: string[]) => `With ${listFormat({ type: "conjunction" }).format(names)}`,
    previousTitle: (title: string) => `Previous note: ${title}`,
    joinRecord: "Join and record",
    // An event with no video call has nothing to join.
    onlyRecord: "Record only",
    previousNote: "Open previous note",
    pending: "Unfinished work from last time",
    pendingReading: `Reading ${pendingLabel}…`,
    pendingFailed: `Couldn’t read ${pendingLabel}.`,
    pendingFirst: "This is the first in the series. Nothing to follow up on yet.",
    pendingNone: "Nothing left to follow up on.",
    pendingNoNote: "No note was written last time, so unfinished work is unknown.",
    pendingWriting: "Last time’s note is still being written.",
    pendingNoteFailed: "Last time’s note couldn’t be written.",
    pendingTask: (owner: string | null, text: string) =>
      owner === null || owner === "" ? text : `${owner}: ${text}`,
    reminder: "Personal reminder",
    reminderPlaceholder: "Something you don’t want to forget…",
    reminderHelp: "Only you can see this. It won’t go in the note.",
    reminderNotKept: (cause: "quota" | "unavailable") =>
      cause === "quota"
        ? "Not saved: there’s no space left for Orbit’s local data on this Mac. Your text is still here."
        : "Not saved: this Mac won’t let Orbit save local data right now. Your text is still here.",
    recordingClock: (clock: string) => `Recording · ${clock}`,
    paused: "Paused",
    recordingHint:
      "Jot down anything you don’t want to forget while recording. Orbit will use it to write the note.",
    recordsUntil: (clock: string) => `Records until ${clock}`,
    privateNotes: "Write in my private notes",
    finish: "End",
    during: "During the meeting",
    askAloud: "Ask aloud",
    askSilently: "Ask without speaking",
    pttShortcut: globalShortcuts.ptt,
    finishAnywhere: "End from any app",
    // The helper's `meeting_toggle` (GlobalHotkeys.swift).
    meetingShortcut: "⌃⌥M",
    agenda: "Today’s agenda",
    seeWeek: "View week",
    dayLine: (now: string | null) =>
      now === null
        ? `Your day from ${dayBounds()}.`
        : `Your day from ${dayBounds()}. It’s now ${now}.`,
    noteReady: "Note ready",
    recording: "Recording",
    peopleCount: (count: number) =>
      count === 0
        ? "Just you"
        : pluralRules().select(count) === "one"
          ? "1 person"
          : `${numberFormat().format(count)} people`,
    readingAgenda: "Reading your agenda…",
    agendaFailed: "Couldn’t read your agenda.",
    noMeetingsTitle: "Nothing on your agenda today",
    noneLeftTitle: "No more meetings today",
    noMeetingsText: "If a meeting comes up, record it and Orbit will write the note for you.",
    record: "Record meeting",
    calendarTitle: "Connect your calendar",
    calendarText:
      "Orbit uses your Mac’s calendars (iCloud, Google, Exchange…) without keeping a copy.",
    calendarAllow: "Allow calendar access",
    calendarHint: "Previously denied access? Enable it in System Settings.",
    calendarDenied: "Orbit can’t access your calendars. Enable access in System Settings.",
    openSystemSettings: "Open System Settings",
    // What stops the agenda when the permission is there; the top bar's
    // notice carries the button that fixes it.
    blocked: {
      privacy_stopped: "Everything is stopped. Orbit won’t read your agenda until you rearm it.",
      helper_not_running:
        "Orbit can’t read your agenda right now: its calendar reader isn’t running.",
      not_paired: "Pair this Mac with your hub to see your agenda.",
      mac_disconnected: "This Mac is disconnected. Your agenda will appear when it reconnects.",
    },
    timezoneFailed: "Couldn’t read your Mac’s time zone, so Today can’t place your meetings.",
  },
} as const;

export function briefingCopy(assistantName: string) {
  return { briefing: { ...base.briefing, heyDeimos: `“Hey ${assistantName}…”` } } as const;
}
