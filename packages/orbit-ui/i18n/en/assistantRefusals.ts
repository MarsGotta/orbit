import { adapterRefusal } from "./adapterRefusals";
import { numberFormat } from "../locale";
import { count, text, type Numbers } from "./refusalNumbers";

/**
 * The words of the hub's refusals of an assistant's profile, of adding and
 * removing one and of signing it in and out (hub-v1 «Assistants from
 * Orbit»); `undefined` when a number they need is missing.
 */
export const assistantRefusals: Record<string, (n: Numbers) => string | undefined> = {
  agent_profile_invalid: (n) => {
    const field = text(n, "field");
    const rule = text(n, "rule");
    const what =
      field === "description" ? "The description" : field === "color" ? "The color" : "The name";
    if (rule === "empty")
      return field === "name" ? "Give it a name. That’s how you’ll call it." : undefined;
    if (rule === "too_long") {
      const characters = count(n, "characters");
      const max = count(n, "max_characters");
      if (characters === undefined || max === undefined) return undefined;
      return `${what} has ${numberFormat().format(characters)} characters; the maximum is ${numberFormat().format(max)}.`;
    }
    if (rule === "control_characters")
      return `${what} contains control characters. Write it on one line.`;
    if (rule === "unknown_color") return "That isn’t one of Orbit’s four colors.";
    return undefined;
  },
  agent_name_taken: () => "You already have an assistant with that name.",
  agent_primary_required: () =>
    "There must always be a primary assistant. Make another assistant primary first.",
  agent_add_unsupported: () =>
    "Add this type on your hub. Hermes is created from a Hermes profile.",
  agent_adapter_missing: adapterRefusal,
  too_many_agents: (n) => {
    const agents = count(n, "count");
    const max = count(n, "max");
    if (agents === undefined || max === undefined) return undefined;
    return `Your hub already has ${numberFormat().format(agents)} assistants; the maximum is ${numberFormat().format(max)}.`;
  },
  agent_not_removable: () =>
    "Your hub provides this assistant. Only whoever manages it can remove it.",
  agent_folder_not_removed: (n) => {
    const errno = count(n, "errno");
    if (errno === undefined) return undefined;
    return `The hub couldn’t delete the assistant’s folder (errno ${numberFormat().format(errno)}), so it’s still here. Tell whoever manages your hub.`;
  },
  sign_in_unsupported: () =>
    "This assistant doesn’t sign in through Orbit. It’s configured by whoever manages your hub.",
  sign_in_not_found: () =>
    "The hub no longer has this sign-in. It expired or finished. Start again.",
  sign_in_not_waiting_for_code: () => "Sign-in is no longer waiting for a code. Start again.",
  sign_in_code_invalid: (n) => {
    const characters = count(n, "characters");
    if (characters === undefined) return undefined;
    return `That isn’t the code from the page (${numberFormat().format(characters)} characters). Copy it in full, including its “#”.`;
  },
  sign_out_failed: (n) => {
    const status = count(n, "exit_status");
    if (status === undefined) return undefined;
    return `The provider tool didn’t sign out (exit code ${numberFormat().format(status)}). Try again.`;
  },
  sign_out_timeout: (n) => {
    const timeout = count(n, "timeout_ms");
    if (timeout === undefined) return undefined;
    return `The provider tool didn’t sign out within ${numberFormat().format(Math.round(timeout / 1000))} s. Try again.`;
  },
};
