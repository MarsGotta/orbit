import { hourOf, whenBack } from "./assistantsFormat";

// Signing an assistant in from Orbit and its plan's limit: what the person
// does in the browser is «iniciar sesión»; tokens and OAuth stay out.
export const assistantsAccountCopy = {
  // Signing in.
  signIn: (vendor: string) => `Sign in to ${vendor}`,
  signInAgainTitle: (vendor: string) => `Sign in to ${vendor} again`,
  signInFirstTitle: (vendor: string) => `Sign in to ${vendor}`,
  signInAgainText: (name: string) =>
    `Your conversations with ${name} are still here. Sign in so it can respond.`,
  signInFirstText: (name: string) =>
    `${name} will use your plan when you sign in. Your text stays here.`,
  waitingTitle: "Waiting for you to sign in through the browser…",
  waitingText: (vendor: string) =>
    `Your browser is open. Sign in with your ${vendor} account, then return here.`,
  deviceCode: (code: string) => `Enter this code on the page: ${code}`,
  pasteCode: "Code from the page",
  pasteCodeHelp: (vendor: string) => `When you finish, ${vendor} gives you a code. Paste it here.`,
  sendCode: "Send code",
  cancelSignIn: "Cancel",
  connected: "✓ Connected",
  connectedAs: (email: string | undefined, plan: string | undefined) =>
    email === undefined
      ? plan === undefined
        ? "Using your account"
        : `Using your account · ${plan}`
      : plan === undefined
        ? `As ${email}`
        : `As ${email} · ${plan}`,
  sameAccount: (other: string) =>
    `${other} already uses this account. You can continue; both assistants share the same plan allowance.`,
  browserNotOpened: "Couldn’t open the browser. Copy this address and open it yourself:",
  /** A sign-in that ended without one, by the hub's cause. */
  signInEnded: (cause: string | undefined, vendor: string, name: string): string => {
    switch (cause) {
      case "cancelled":
        return `You canceled sign-in. ${name} hasn’t changed.`;
      case "device_code_disabled":
        return `Your ${vendor} account doesn’t allow device code sign-in. Enable it in your account security settings and try again.`;
      case "plan_required":
        return vendor === "Claude"
          ? "This account doesn’t have the plan Claude Code needs. Sign in with Claude Pro, Max, Team or Enterprise."
          : "This account doesn’t have the plan Codex needs. Sign in with ChatGPT Plus, Pro, Business, Edu or Enterprise.";
      case "expired":
        return `The ${vendor} code expired before you finished. Try again.`;
      case "code_rejected":
        return `${vendor} rejected the code. Copy the entire code from the page and try again.`;
      case "login_disabled":
        return `Your hub’s ${vendor} configuration doesn’t allow this sign-in method. Ask whoever manages your hub.`;
      case "timed_out":
        return "Sign-in wasn’t completed within 15 minutes. Try again.";
      case "no_url":
        return `The ${vendor} tool didn’t provide a sign-in address within 30 seconds. Try again; if it happens again, tell whoever manages your hub.`;
      case "no_credentials":
        return `The ${vendor} tool finished without saving your session. Try again.`;
      case "not_started":
        return `The ${vendor} tool couldn’t start on your hub. Tell whoever manages your hub.`;
      case "exited":
        return `The ${vendor} tool exited with an error. Try again.`;
      default:
        return `Sign-in didn’t finish (${cause ?? "no cause given"}). Try again.`;
    }
  },
  tryAgain: "Try again",
  composerSignIn: (name: string, vendor: string) =>
    `${name} can’t respond until you sign in to ${vendor}. Your text stays here.`,
  composerLimit: (name: string, atMs: number | undefined) =>
    atMs === undefined
      ? `${name} can’t respond until your plan renews. Your text stays here.`
      : `${name} can’t respond until ${hourOf(atMs)}. Your text stays here.`,

  // The plan's limit.
  limitTitle: (name: string, vendor: string) => `${name} has reached your ${vendor} plan’s limit`,
  limitTitlePlan: (name: string) => `${name} has reached its plan’s limit`,
  limitText: (atMs: number | undefined, nowMs: number) =>
    `${atMs === undefined ? "Available again when your plan renews." : `Available again ${whenBack(atMs, nowMs)}.`} Your question is safe. You can continue with another assistant, which will read the conversation so far.`,
  continueWith: (name: string) => `Continue with ${name}`,
  signInFirst: (name: string) => `${name} · sign in first`,
  signInFirstTip: (vendor: string) => `Sign in to ${vendor} first`,
  waitUntil: (atMs: number) => `Wait until ${hourOf(atMs)}`,
  whyPrimary: (name: string) => `We suggest ${name} because it’s your primary assistant.`,
  handedOver: (name: string) => `Continuing with ${name} · the conversation has been shared`,
  waitingFor: (name: string, atMs: number) =>
    `We’ll let you know when ${name} is available again (${hourOf(atMs)}), then send your question.`,
  continueWithOther: "Continue with another",
} as const;
