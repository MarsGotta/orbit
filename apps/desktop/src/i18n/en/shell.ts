import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
// The window: the sidebar with its recording and call cards, the top bar
// with its notices, and what the window says before Orbit answers.
import { duration } from "./duration";

/** The already-localized time the recording runs until. */
function until(time: string): string {
  return `until ${time}`;
}

function sidebarCopy(assistantName: string) {
  return {
    shell: {
      /** The agent every text names by default: the open conversation's. */
      assistantName,
      viewsLabel: "Main",
      hideSidebar: "Hide Sidebar",
      showSidebar: "Show Sidebar",
      breadcrumbsLabel: "Where you are",
      record: "Record meeting",
      recordShortcut: "⌃⌥M",
      briefing: "Today",
      calendar: "Calendar",
      notes: "Meetings",
      vault: "Notes",
      assistant: "Assistant",
      settings: "Settings",
      inProgress: (count: number) => `${numberFormat().format(count)} in progress`,
      needsYou: "Needs you",
      localUser: "You",
      ask: `Ask ${assistantName}`,
      search: `Search or ask ${assistantName}`,
      searchShortcut: "⌘⇧␣",
      changeTheme: "Change theme",
      toLight: "Switch to light theme",
      toDark: "Switch to dark theme",
      loadingView: "Loading…",
      notice: "Orbit notice",
      closeNotice: "Dismiss notice",
      settingsSections: {
        general: "General",
        privacy: "Privacy & Permissions",
        recording: "Recording & Calls",
        assistants: "Assistants",
        meetings: "Meetings",
        notes: "Notes",
        ai: "AI",
      },
    },
  } as const;
}

function recordingCopy(assistantName: string) {
  return {
    recordingCard: {
      label: "Recording in progress",
      starting: "Preparing to record…",
      recording: "Recording",
      paused: "Paused",
      finishing: "Saving recording…",
      elapsed: "Recorded time",
      audio: {
        system: "System audio",
        microphone: "Microphone only",
        application: "App audio",
      },
      audioOf: (application: string) => `${application} audio`,
      until,
      listening: `“Hey ${assistantName}” is listening`,
      notInPause: `“Hey ${assistantName}” doesn’t listen while paused`,
      ask: `Ask ${assistantName}`,
      pause: "Pause",
      resume: "Resume",
      finish: "End",
      check: "Review",
    },
    askMenu: {
      title: "Ask about this meeting",
      fieldLabel: "Your question",
      placeholder: "What was decided about the budget?",
      windows: "Listen to the last",
      window: (minutes: number) => `${numberFormat().format(minutes)} min`,
      voiceHint: `Or say “Hey ${assistantName}” aloud`,
      send: "Ask",
      close: "Close",
      alsoInConversation: `Also saved in your conversation with ${assistantName}`,
      another: "Another question",
    },
  } as const;
}

const callCopy = {
  callCard: {
    label: "Call in progress",
    inCall: "On a call",
    elapsed: "Call duration",
    seesScreen: "Can see your screen",
    noScreen: "Can’t see your screen",
    left: (minutes: number) => (minutes <= 1 ? "1 min left" : `${duration(minutes)} left`),
    hangUp: "Hang up",
  },
} as const;

const noticesCopy = {
  // The top bar says nothing when all is well; otherwise the most
  // important thing, one at a time, with the button that fixes it.
  // Nothing here names a server, a device or a count.
  problem: {
    orbitLost: "Orbit isn’t responding",
    privacyStopped: "Everything is stopped",
    unpaired: "This Mac isn’t paired",
    revoked: "The hub revoked this Mac",
    offline: "No connection to the hub",
    missingPermission: (name: string) => `${name} permission is missing`,
    missingPermissions: "Permissions missing",
    retrying: "Retrying…",
    connectingHub: "Connecting to your hub…",
    preparing: "Orbit is getting ready…",
    pair: "Pair",
    fix: "Fix",
    rearm: "Rearm",
    rearming: "Rearming…",
    more: (count: number) => `+${numberFormat().format(count)}`,
    moreLabel: (count: number) =>
      pluralRules().select(count) === "one"
        ? "Show 1 more notice"
        : `Show ${numberFormat().format(count)} more notices`,
    all: "Notices",
  },
  appState: {
    starting: "Connecting to Orbit…",
    startingHint: (seconds: number) =>
      `If it takes more than ${numberFormat().format(seconds)} seconds, we’ll tell you what’s happening.`,
    slow: "Orbit is taking longer than usual",
    slowText: "Orbit may be stuck. Close and reopen it. Your saved recordings are safe.",
    lost: "Orbit isn’t responding",
    lostText: "Close and reopen it. Your saved recordings are safe.",
    close: "Close Orbit",
  },
} as const;

export function shellCopy(assistantName: string) {
  return {
    ...sidebarCopy(assistantName),
    ...recordingCopy(assistantName),
    ...callCopy,
    ...noticesCopy,
  } as const;
}
