import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
import { listOf } from "./notes";
import { macPermissions } from "./permissions";
import { globalShortcuts } from "./shortcuts";

const base = {
  stepOf: (step: number, total: number, name: string) =>
    `Step ${numberFormat().format(step)} of ${numberFormat().format(total)} · ${name}`,
  stepsLabel: "Steps",
  stepLabel: (step: number, name: string) => `Step ${numberFormat().format(step)}: ${name}`,
  stepNames: ["Welcome", "Your hub", "Permissions", "Assistants", "Your AI", "Done"],
  welcome: "Welcome to Orbit",
  welcomeBody:
    "Ready in a couple of minutes: connect this Mac to your hub, choose what Orbit can access and which assistants you’ll talk to.",
  welcomePoints: [
    {
      title: "Record meetings and leave the notes to Orbit",
      body: "Orbit writes the summary, everyone’s tasks and the transcript.",
    },
    {
      title: "Your day at a glance",
      body: "Your usual calendar, with unfinished work from each meeting.",
    },
    {
      title: "Assistants that lend a hand",
      body: "On your Mac, assistants act with the macOS access you give Orbit. On the hub, they ask with a card before each command or edit. Stopping everything cuts them off at once.",
    },
  ],
  planLabel: "What’s next",
  welcomeControl: `You decide what Orbit sees and hears. Stop everything instantly with ${globalShortcuts.stop}.`,
  begin: "Get started",
  continue: "Continue",
  back: "Back",
  pair: "Pair",
  pairTitle: "Connect this Mac to your hub",
  pairing: "Pairing…",
  hubUrl: "Hub address",
  hubUrlPlaceholder: "https://hub.example",
  code: "Pairing code",
  codeHelp: "Single use. Expires in 5 minutes.",
  codeDigits: (count: number) =>
    `It has ${numberFormat().format(count)} ${pluralRules().select(count) === "one" ? "digit" : "digits"}; it needs 6.`,
  deviceName: "This Mac’s name",
  pairFailed: "Couldn’t pair",
  pairFailedHint:
    "The code has 6 digits and works once. Check the hub address too, or ask whoever manages it for a new code.",
  permissions: "Choose what Orbit can access",
  permissionsBody:
    "macOS will ask about each permission. You can change these in Settings at any time.",
  permissionsLater:
    "You can continue without some permissions. Orbit will tell you what’s missing when needed.",
  allow: "Request",
  allowed: "✓ Granted",
  notGranted: "Not granted",
  openSettings: "Open Settings",
  openSystemSettings: "Open System Settings",
  waitingForSettings: "Waiting for System Settings…",
  accessibility: macPermissions.accessibility.label,
  assistants: "Your assistants",
  assistantsBody: (hub: string) =>
    `Each uses a different service. ${hub} comes with your hub; sign in with your subscription for the others.`,
  readyWithHub: "✓ Ready with your hub",
  connected: "✓ Connected",
  needsHub: "Needs your hub",
  pairHub: "Pair hub",
  signInWith: (vendor: string) => `Sign in with ${vendor}`,
  waitingForBrowser: "Waiting for the browser…",
  account: (email: string | undefined, plan: string | undefined) =>
    [email, plan].filter((part) => part !== undefined && part !== "").join(" · "),
  primary: "Primary assistant",
  primaryHelp: (other: string | undefined) =>
    other === undefined
      ? "Answers to “Hey…” without a name and in the floating panel."
      : `Answers to “Hey…” without a name and in the floating panel. Call the others by name: “Hey ${other}, …”.`,
  noPrimary: "Connect at least one to choose the primary assistant.",
  assistantsLater:
    "You can continue without some assistants. Add more or change accounts at any time under Assistant.",
  ready: "All set",
  readyBody: "Orbit is ready. You can change anything in Settings.",
  summaryLabel: "What you’ve set up",
  summaryHub: "Your hub",
  summaryPermissions: "Permissions",
  summaryAssistants: "Assistants",
  hubPaired: (host: string, device: string | null) =>
    device === null || device === "" ? host : `${host} · this Mac is named “${device}”`,
  hubUnpaired: "Not paired. Pair at any time in Settings › General.",
  allGranted: "All four granted",
  someGranted: (granted: number, missing: string[]) =>
    `${numberFormat().format(granted)} of 4 granted · missing ${listOf(missing)}`,
  assistantsReady: (names: string[]) => listOf(names),
  asPrimary: (name: string) => `${name} (primary)`,
  noAssistants: "None connected yet",
  shortcutsTitle: "Three shortcuts that work from any app",
  // «Configurarlo más tarde» skips the whole first run; «Emparejar después», pairing.
  skipAll: "Set up later",
  skipStep: "Pair later",
  start: "Start using Orbit",
} as const;

/** The first run: five steps, each saying where it is and what each skip leaves. */
export function onboardingCopy(assistantName: string) {
  return {
    onboarding: {
      ...base,
      /** The steps to come, on the welcome; `others` are the assistants with a subscription. */
      plan: (hub: string, others: string[]) => [
        { title: "Your hub", body: "Connect this Mac to the hub where your meetings are saved." },
        { title: "Permissions", body: "Microphone, screen, accessibility and calendars." },
        {
          title: "Assistants",
          body:
            others.length === 0
              ? `${hub} comes with your hub; the others use your subscription.`
              : `${hub} comes with your hub; ${listOf(others)} use your subscription.`,
        },
        { title: "Your AI", body: "Start with your hub or add a provider." },
        { title: "Done", body: "Three shortcuts to get started." },
      ],
      pairBody: (hub: string) =>
        `The hub stores your meetings and notes and hosts ${hub}. Whoever manages it gives you the address and a code.`,
      paired: (host: string, hub: string) => `✓ Mac paired with ${host}. ${hub} is ready.`,
      // Why each permission, in the order of the rows.
      permissionWhy: {
        microphone: "To record meetings, send voice notes and talk to your assistants.",
        screen: "To record video call audio and let your assistants see your screen when asked.",
        accessibility: "To let your assistants use the Mac and read selected text.",
        calendars: "To view and change your agenda in Orbit.",
      },
      shortcuts: (primary: string = assistantName) => [
        {
          keys: globalShortcuts.meeting,
          title: "Record a meeting",
          body: "Press again to end.",
        },
        {
          keys: globalShortcuts.ptt,
          title: `Talk to ${primary}`,
          body: "Hold, speak and release.",
        },
        {
          keys: globalShortcuts.stop,
          title: "Stop everything instantly",
          body: "Microphone, screen and uploads stop until you rearm.",
        },
      ],
    },
  } as const;
}
