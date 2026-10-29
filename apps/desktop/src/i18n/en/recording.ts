import { numberFormat } from "../locale";
export const recording = {
  // The assistant call's microphone. Recording has its own scoped copy below.
  microphoneMute: {
    mute: "Mute",
    unmute: "Unmute",
    muted: "Microphone muted",
  },
  recordingMute: {
    help: "Leaves your voice out of Orbit’s recording. Your video call is unchanged.",
    mute: "Mute",
    unmute: "Unmute",
    muted: "Your voice is left out of Orbit’s recording. Your video call is unchanged.",
  },
  meeting: {
    // Opens «Nueva grabación»; the menu bar alone starts at once.
    recordNow: "Record meeting",
    defaultTitle: "Meeting",
    starting: "Preparing to record…",
    recording: "Recording",
    paused: "Paused",
    finishing: "Saving recording…",
    systemSilent: "No system audio",
    microphoneSilent: "Microphone silent",
    settingsHelp: "Open Settings help",
    openNote: "Open note",
    meet: "Record meeting",
    pause: "Pause",
    resume: "Resume",
    finish: "End",
    // «Nueva grabación».
    startDialog: "New recording",
    startSubtitle: "Orbit records the meeting and writes the note when it ends.",
    titleLabel: "Title",
    untitled: (time: string) => `Meeting ${time}`,
    suggestStarts: (title: string, minutes: number) =>
      `Use “${title}”, starting in ${numberFormat().format(minutes)} min`,
    suggestStarted: (title: string, time: string) => `Use “${title}”, started at ${time}`,
    titleHint: "It will appear under Meetings with this name.",
    sourceLabel: "Audio to record",
    sourceSystem: "System audio",
    sourceSystemHelp: "Sound from your Mac and microphone. For video calls.",
    sourceMicrophone: "My microphone only",
    sourceMicrophoneHelp: "For an in-person meeting.",
    sourceApplication: "A specific app",
    sourceApplicationHelp: "Only sound from the app you choose.",
    sourceSearching: "Finding apps playing audio…",
    sourceNoApplications: "No apps are playing audio. Start audio in the app and search again.",
    sourceUnknown: "Couldn’t determine which apps are playing audio.",
    searchAgain: "Search again",
    playingNow: "Playing now:",
    wholeApplication: "Records the whole app, not just one browser tab.",
    limit:
      "Recording continues when you close the window. After 2 min of silence, Orbit asks whether to end it.",
    cancel: "Cancel",
    record: "Start recording",
    startPending: "Starting…",
    missingMicrophone: "Microphone permission missing",
    missingMicrophoneHelp: "You can record once you allow it.",
    missingScreen: "Screen & System Audio permission missing",
    missingScreenHelp: "Without it, Orbit can only record your microphone.",
    openSystemSettings: "Open System Settings",
    privacyStopped: "Everything is stopped",
    privacyStoppedHelp: "Orbit won’t record until you rearm it.",
    rearm: "Rearm Orbit",
    // At the end of a recording; «Tu nota está lista» follows when it is.
    saved: "Meeting saved",
    savedDetail: "We’ll let you know when the note is ready.",
  },
  // «Terminar sin guardar», next to «Terminar» in the «Grabando» card and the
  // floating panel; the first click only asks, in place.
  discard: {
    action: "End without saving",
    question: "End without saving?",
    detail: "The recording is deleted and no note is written.",
    confirm: "Delete and end",
    keep: "Keep recording",
    pending: "Deleting…",
  },
  // The floating panel and «Al detectar una llamada».
  callDetection: {
    panelLabel: "Call notice",
    offer: (application: string) => `Call in ${application}`,
    record: "Record",
    notNow: "Not now",
    stop: "Stop",
    ending: (seconds: number) =>
      `Call ended · Recording stops in ${numberFormat().format(seconds)} s`,
    keep: "Keep recording",
    // The floating panel's own cards.
    recordingAt: (elapsed: string) => `Recording · ${elapsed}`,
    pausedAt: (elapsed: string) => `Paused · ${elapsed}`,
    ended: "Call ended",
    endingIn: (seconds: number) => `Recording stops in ${numberFormat().format(seconds)} s`,
    recordPending: "Starting…",
    stopPending: "Stopping…",
    keepPending: "Continuing…",
  },
} as const;
