import { numberFormat } from "../locale";
import { macPermissions } from "./permissions";
import { settingsLists } from "./settingsLists";
import { globalShortcuts } from "./shortcuts";

const base = {
  settings: {
    spanish: "Español",
    english: "English",
    language: "Language",
    systemLanguage: "Same as the system",
    title: "Settings",
    leads: {
      general: "Your account, this Mac and how Orbit looks.",
      privacidad: "What Orbit can see and hear on this Mac, and how to stop everything.",
      grabacion: "What Orbit does when you start a video call.",
      asistentes:
        "Edit each assistant from its conversation: the pencil changes its name, description and color; the gear opens its other settings.",
      reuniones:
        "How your meeting notes are written and which words transcription corrects. What reaches your vault afterward is in Settings › Notes.",
    },
    index: "Settings sections",
    missing: (count: number) =>
      count === 1 ? "1 missing" : `${numberFormat().format(count)} missing`,
    appVersion: (version: string) => `Orbit ${version}`,
    exit: "Quit Orbit",
    /** Closes Orbit in order and opens it again: the helper starts with it. */
    restart: "Restart Orbit",

    // General
    account: "Your account",
    yourName: "Your name",
    yourNameHelp: "How Orbit greets you each morning.",
    yourNamePlaceholder: "How you’d like Orbit to greet you",
    thisMac: "This Mac",
    pairedWith: (host: string) => `Paired with ${host}`,
    unpaired: "This Mac isn’t paired.",
    revoked: "The hub revoked this Mac. Pair it again.",
    pair: "Pair",
    pairAgain: "Pair again",
    pairAgainQuestion: "Pair this Mac again?",
    confirmPairAgain: "Yes, pair again",
    cancelPairAgain: "Cancel",
    reopenOnboarding: "Show welcome again",
    appearance: "Appearance",
    theme: "Theme",
    dark: "Dark",
    light: "Light",
    startup: "When your Mac starts",
    openAtLogin: "Open Orbit at login",
    // The launcher could not be read when the app started.
    loginUnknown: "macOS hasn’t reported whether Orbit opens at login. Choose here.",
    updates: "Updates",
    version: (version: string) => `Version ${version}`,
    // The app checks at start and every six hours
    // (`AUTOMATIC_CHECK_INTERVAL` in orbit-app-core's update.rs).
    updatesIdle: "Orbit checks for updates at startup and every 6 hours",
    updatesDisabled: "This build has no update source configured (ORBIT_UPDATE_BASE_URL).",
    searchUpdate: "Check for updates",
    checking: "Checking for updates…",
    available: (version: string | null) =>
      version === null ? "An update is available" : `Update available: ${version}`,
    upToDate: "Orbit is up to date",
    installing: "Installing…",
    updateFailed: "Couldn’t check for updates. Try again.",
    lastInstallFailed: "The last update didn’t finish. Try again.",
    installRestart: "Install and restart",
    updateNotice: (version: string) => `A new version of Orbit is available (${version})`,
    updateAction: "Refresh",

    // Privacidad y permisos
    stopTitle: "Stop everything",
    stoppedTitle: "Everything is stopped",
    stoppedText:
      "Orbit won’t record, listen or upload anything, even after you close it. Rearming won’t start recording by itself.",
    stopNow: "Stop everything now",
    rearm: "Rearm Orbit",
    rearming: "Rearming…",
    stopShortcut: `Also with ${globalShortcuts.stop} from any app`,
    systemPermissions: "macOS permissions",
    granted: "Allowed",
    notGranted: "Not allowed",
    checkingPermission: "Checking…",
    request: "Allow",
    requestPermission: "Request permission",
    openSystemSettings: "Open System Settings",
    helperMissing: "Orbit.app isn’t running. Open it to see permissions.",
    microphoneReady: "Microphone always ready",
    microphoneHint: `for voice notes with ${globalShortcuts.ptt}`,
    microphoneBlocked: "Allow access in System Settings first",
    improve: "Help improve Orbit",
    errorReports: "Send anonymous error reports",
    errorReportsHelp: "Error codes only. Never the content of your meetings or conversations.",

    // Grabación y llamadas
    whenCall: "When you start a video call",
    whenCallLabel: "When a call is detected",
    shortcuts: "Shortcuts from any app",

    // Asistentes: a summary; each one's own settings are behind its gear.
    yourAssistants: "Your assistants",
    theirSettings: "Assistant settings",
    theirSettingsLabel: (name: string) => `Settings for ${name}`,
    sessionExpired: "Session expired",
    usage: (percent: number, window: string) =>
      `${numberFormat().format(percent)}% of ${window} usage`,
    primaryHelp:
      "Answers in the floating panel, in notes when you choose “Ask”, and when you say “Hey…” without a name. Call the others by name.",
    primaryLabel: "Main",
    primarySaving: "Saving to the hub…",
    controlHistory: "Activity on this Mac",
    historyUnreadable: "Couldn’t read activity on this Mac.",
    today: "Today",
    yesterday: "Yesterday",

    // Reuniones
    templates: "Templates",
    dictionaries: "Dictionaries",
  },
  // A permission row: what Orbit uses it for, as the person reads it.
  permissionRows: {
    microphone: { title: macPermissions.microphone.label },
    screen: { title: "Screen & System Audio" },
    accessibility: { title: macPermissions.accessibility.label },
    calendars: {
      title: macPermissions.calendar.label,
      help: "To view and change your agenda in Orbit.",
    },
  },
  fullControl: {
    performed: "Done",
    cancelled: "Canceled",
    failed: "Failed",
    unknown: "Outcome uncertain",
  },
  ...settingsLists,
} as const;

export function settingsCopy(assistantName: string) {
  return {
    ...base,
    settings: {
      ...base.settings,
      deimos: assistantName,
      engine: {
        helperNotConnected: "Orbit.app isn’t running. Open it to continue.",
        missing: (names: string) =>
          `${assistantName} needs ${names} to use this Mac. Allow access in Privacy & Permissions.`,
        failed: `${assistantName} can’t use this Mac right now. Orbit will try again.`,
        screenRecording: macPermissions.screen_recording.name,
        and: " and ",
      },
      privacyArmed: `Immediately stops everything Orbit hears or sees: recordings, calls, voice notes and “Hey ${assistantName}”. Stays stopped even after you close Orbit.`,
      pairAgainWarning: `${assistantName} will see it as a new device, and you’ll need a new code.`,
      shortcutList: [
        [globalShortcuts.meeting, "Start or end a recording"],
        [globalShortcuts.ptt, `Hold to speak to ${assistantName}; during a meeting, ask about it`],
        [globalShortcuts.call, `Call ${assistantName} or hang up`],
        [globalShortcuts.stop, "Stop everything immediately"],
      ],
    },
    permissionRows: {
      ...base.permissionRows,
      microphone: {
        ...base.permissionRows.microphone,
        help: `To record meetings, send voice notes and talk to ${assistantName}.`,
      },
      screen: {
        ...base.permissionRows.screen,
        help: `To record video call audio and let ${assistantName} see your screen when asked. Without it, Orbit only records your microphone.`,
      },
      accessibility: {
        ...base.permissionRows.accessibility,
        help: `To let ${assistantName} use your Mac and read selected text.`,
      },
    },
    fullControl: {
      ...base.fullControl,
      noHistory: `${assistantName} hasn’t done anything on this Mac yet.`,
    },
  } as const;
}
