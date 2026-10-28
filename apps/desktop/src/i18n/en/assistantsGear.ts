import { numberFormat } from "../locale";
import { hourOf, sameDay, usageWindow, weekday } from "./assistantsFormat";

// An assistant's settings, behind its gear, and «Añadir un asistente».
export const assistantsGearCopy = {
  // The gear.
  backTo: (name: string) => `Conversation with ${name}`,
  settingsCrumb: "Settings",
  settingsTitle: (name: string) => `Settings for ${name}`,
  settingsLead:
    "Use the pencil to change the name, description, color and primary assistant. Other settings are here.",
  connection: "Connection",
  service: "Service",
  where: "Where",
  whereHub: (hub: string) => `${hub} API Gateway`,
  account: "Account",
  state: "Status",
  stateConnected: "Connected",
  stateExpired: "Session expired",
  stateSignedOut: "Not signed in",
  stateUnknown: "Not checked yet",
  stateLimit: (atMs: number | undefined) =>
    atMs === undefined ? "At your plan’s limit" : `At your plan’s limit until ${hourOf(atMs)}`,
  hubConfigured:
    "Configured by whoever manages your hub. Personality and capabilities are managed in Hermes; this is how it’s currently set up.",
  usage: "Plan usage",
  usageLine: (percent: number, window: string | undefined) =>
    `${numberFormat().format(percent)}% of ${usageWindow(window, true)} usage`,
  renews: (atMs: number, nowMs: number) => {
    const at = new Date(atMs);
    if (sameDay(at, new Date(nowMs))) return `Renews today at ${hourOf(atMs)}`;
    return `Renews on ${weekday.format(at)}`;
  },
  usageHint: "If it reaches the limit, Orbit will offer another assistant to continue with.",
  seePlan: (vendor: string) => `View my ${vendor} plan`,
  signOut: "Sign out",
  signedOutDone: (name: string) =>
    `Signed out. ${name} and its conversations stay in “Your assistants”. Sign in again to write to it.`,
  expiredBanner: (vendor: string) => `${vendor} session expired`,
  expiredBannerText: (name: string) =>
    `Sign in again so ${name} can respond. Its conversations are still here.`,
  inOrbit: "In Orbit",
  inMeeting: "In a meeting",
  inMeetingText: (name: string) => `Say “Hey ${name}, …” and it answers in the floating panel.`,
  voice: "Voice",
  ownVoice: "Has its own voice for calls and reading responses aloud.",
  borrowedVoice: (primary: string) =>
    `Has no voice of its own. Orbit reads its responses using ${primary}’s voice.`,
  history: "Your assistants’ activity on this Mac",
  historyShared: "Activity from all your assistants, named when there’s more than one.",
  remove: (name: string) => `Remove ${name} from Orbit`,
  removeQuestion: (name: string) => `Remove ${name}?`,
  removeText:
    "Its conversations stay, but you won’t be able to write to it. Your subscription won’t change.",
  removeConfirm: "Remove",
  removing: "Removing…",
  notRemovable: (name: string) =>
    `Your hub provides ${name}. Only whoever manages it can remove this assistant.`,
  unknownAssistant: "That assistant is no longer on your hub.",
  backToConversation: "Back to conversation",

  // «Añadir un asistente».
  addStep: (step: number) => `Add an assistant · step ${numberFormat().format(step)} of 4`,
  kindTitle: "What will it run on?",
  kindLead:
    "Each assistant uses a service. If you already pay for one, you can use that subscription.",
  kindLabel: "Service",
  kinds: {
    "claude-code": {
      title: "Your Claude subscription",
      text: "Sign in with your Claude account (Pro or Max). Counts toward your plan’s usage.",
      tag: "Claude",
    },
    codex: {
      title: "Your ChatGPT subscription",
      text: "Sign in with your ChatGPT account (Plus or Pro) to use Codex.",
      tag: "Codex",
    },
    hermes: {
      title: "Hermes on your hub",
      text: "Created on your hub from a Hermes profile, then appears in Orbit.",
      tag: "Hermes",
    },
  } as const,
  notOffered: "This hub version doesn’t include the service. Redeploy the hub to install it.",
  connectTitle: (vendor: string) => `Connect your ${vendor} account`,
  connectLead:
    "Orbit will use your plan for this assistant. Usage counts toward your subscription.",
  connectHint:
    "Your browser will open. Orbit never sees your password; it only receives permission to use your plan.",
  hubKindTitle: "An assistant from your hub",
  hubKindLead: "This type isn’t created in Orbit.",
  askHub: "Add it on your hub",
  askHubText:
    "Create the assistant on your hub from a Hermes profile. Once added there, it will appear here.",
  preparing: "Preparing the assistant on your hub…",
  nameTitle: "Give it a name",
  nameLead: (name: string) => `How you’ll call it in Orbit and in meetings (“Hey ${name}, …”).`,
  accessTitle: "What it can use · you can change this in its settings",
  create: "Create assistant",
  creating: "Creating…",
  readyTitle: (name: string) => `${name} is ready`,
  readyText: (name: string) =>
    `Find it in “Your assistants”. Start a new conversation or say “Hey ${name}, …” in a meeting.`,
  talkTo: (name: string) => `Talk to ${name}`,
  back: "Back",
  next: "Continue",
  close: "Close",
} as const;
