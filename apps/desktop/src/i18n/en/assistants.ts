import { numberFormat } from "../locale";
// Several assistants in the app: «Tus asistentes», each one's conversation,
// the pencil, the gear, signing in, the plan's limit and «Añadir un asistente».
// Every text names the assistant by the name the person gave it; the service
// is named only where it is chosen or configured.
import { assistantsAccountCopy } from "./assistantsAccount";
import { hourOf, shortDay, usageWindow, vendorOf, whenBack } from "./assistantsFormat";
import { assistantsGearCopy } from "./assistantsGear";

export { hourOf, shortDay, vendorOf, whenBack };

/** «Hermes · API Gateway», «Codex · tu suscripción»: the service in one line. */
function serviceShort(service: string | undefined): string | undefined {
  switch (service) {
    case "hermes":
      return "Hermes · API Gateway";
    case "codex":
      return "Codex · your subscription";
    case "claude-code":
      return "Claude · your subscription";
    case "openai":
      return "Your hub’s model";
    default:
      return service;
  }
}

export const assistantsCopy = {
  // The column.
  title: "Assistant",
  listLabel: "Assistants and conversations",
  yours: "Your assistants",
  add: "Add assistant",
  conversations: "Conversations",
  /** The service in one line, under the name and in the header's pill. */
  serviceShort,
  /** «Funciona con …», where it is chosen or configured. */
  serviceFull: (service: string | undefined): string | undefined => {
    switch (service) {
      case "hermes":
        return "Hermes, through your hub’s API Gateway";
      case "codex":
        return "Codex, with your ChatGPT subscription";
      case "claude-code":
        return "Claude, with your Claude subscription";
      case "openai":
        return "a model configured by your hub";
      default:
        return service;
    }
  },
  worksWith: (line: string) => `Runs on ${line}`,
  /** On the cards of «¿Con quién quieres hablar?»: «Codex · tu suscripción de ChatGPT». */
  serviceCard: (service: string | undefined): string | undefined => {
    const vendor = vendorOf(service);
    const short = serviceShort(service);
    return vendor === undefined || short === undefined ? short : `${short} by ${vendor}`;
  },
  serviceName: (service: string | undefined): string | undefined =>
    ({ hermes: "Hermes", codex: "Codex", "claude-code": "Claude", openai: "OpenAI" })[
      service ?? ""
    ] ?? service,
  // What goes wrong, at the right of its row.
  rowSignIn: "Sign in",
  rowUntil: (atMs: number) => `Until ${hourOf(atMs)}`,
  rowLimit: "At its limit",
  // The conversation's row.
  rowLabel: (line: string, when: string, name: string | undefined) =>
    name === undefined ? `${line} · ${when}` : `${line} · ${when} · with ${name}`,

  // Its status, under its name.
  available: "Available",
  availableHub: "Available · through your hub",
  availableUsage: (percent: number, window: string | undefined) =>
    `Available · ${numberFormat().format(percent)}% of ${usageWindow(window)} usage`,
  signedOut: (vendor: string) => `Not signed in to ${vendor}`,
  expired: (vendor: string) => `${vendor} session expired`,
  limitUntil: (atMs: number, nowMs: number) => `At its limit until ${whenBack(atMs, nowMs)}`,
  limitUnknown: "Your plan’s limit has been reached",
  principal: "Main",

  // A new conversation.
  newConversation: "New conversation",
  newConversationHint: "choose who to talk to",
  chooseTitle: "Who would you like to talk to?",
  chooseLead:
    "Each assistant uses a different service. You can switch until you send the first message.",
  chooseWake: (primary: string | undefined, other: string | undefined) =>
    other === undefined
      ? primary === undefined
        ? "You can also call them by name during a meeting."
        : `You can also call it by name during a meeting: “Hey ${primary}, …”.`
      : primary === undefined
        ? `You can also call them by name during a meeting: “Hey ${other}, …”.`
        : `You can also call them by name during a meeting: “Hey ${other}, …”. Without a name, ${primary}, your primary assistant, answers.`,
  talkingTo: "Talk to",

  // Each one's conversation.
  starters: "Get started",
  /** Three ideas that fill the box, by what the assistant works with. */
  startersFor: (service: string | undefined): readonly string[] => {
    switch (service) {
      case "codex":
        return [
          "Review a script and suggest improvements",
          "Compare two tables and find what doesn’t match",
          "Explain the last Terminal error",
        ];
      case "claude-code":
        return [
          "Review this contract before I sign",
          "Draft a proposal from my notes",
          "Summarize this long document in one page",
        ];
      default:
        return [
          "What’s on my agenda today?",
          "Prepare me for my next meeting",
          "Summarize this week’s unfinished work",
        ];
    }
  },
  continueWhere: "Pick up where you left off",
  noEarlier: (name: string) => `You haven’t talked to ${name} yet.`,
  loadingEarlier: "Reading your conversations…",

  // The header.
  editTip: "Edit name and description",
  editLabel: (name: string) => `Edit name and description for ${name}`,
  settingsTip: (name: string) => `Settings for ${name}`,
  limitState: (atMs: number | undefined) =>
    atMs === undefined ? "limit reached" : `limit reached · until ${hourOf(atMs)}`,
  continuesFrom: (name: string) => `continue from ${name}`,

  // The pencil.
  editTitle: (name: string) => `Edit ${name}`,
  name: "Name",
  nameHint: (name: string) => `Call it with “Hey ${name}”.`,
  nameEmpty: "Give it a name. That’s how you’ll call it.",
  nameTaken: "You already have an assistant with that name.",
  nameLong: (max: number) => `Up to ${numberFormat().format(max)} characters.`,
  description: "Description",
  descriptionHint: "Shown when choosing who to talk to.",
  counter: (count: number, max: number) =>
    `${numberFormat().format(count)} / ${numberFormat().format(max)}`,
  color: "Color",
  colors: ["Purple", "Blue", "Green", "Amber"] as const,
  primary: "Primary assistant",
  primaryIs: "Answers to “Hey…” without a name and in the floating panel.",
  primaryKeep: "To change this, make another assistant primary.",
  primaryWill: (current: string) =>
    `Will answer to “Hey…” without a name and in the floating panel, replacing ${current}.`,
  primaryNow: (current: string) => `Currently ${current}.`,
  hubManaged:
    "This only changes how it appears in Orbit. Your hub manages its personality and capabilities.",
  moreSettings: "More settings",
  saved: "✓ Saved",
  ...assistantsAccountCopy,
  ...assistantsGearCopy,
} as const;
