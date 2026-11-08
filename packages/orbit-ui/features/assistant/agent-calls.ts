import { getCopy } from "../../i18n";
import type { AgentCall } from "../../bridge/generated/AgentCall";
import type { AgentOutcome } from "../../bridge/generated/AgentOutcome";

import type { AgentPort } from "../settings/agents/use-agent-config";

/** What one assistant call came to: what the hub answered, or why there is no answer, in words. */
export type CallResult =
  | { ok: true; outcome: Exclude<AgentOutcome, { outcome: "refused" | "failed" }> }
  | { ok: false; text: string };

/**
 * One call of the assistants' own (profile, add, sign in…) to the hub. A
 * refusal carries the hub's cause and numbers; a call the Mac could not make
 * says why. Nothing is retried: the person sees the cause and decides.
 */
export async function callAgent(port: AgentPort | undefined, call: AgentCall): Promise<CallResult> {
  if (port === undefined)
    return { ok: false, text: getCopy().extras.agentFailureText({ code: "unavailable" }) };
  try {
    const { outcome } = await port(call);
    if (outcome.outcome === "refused")
      return { ok: false, text: getCopy().extras.agentRefusalText(outcome.refusal) };
    if (outcome.outcome === "failed")
      return { ok: false, text: getCopy().extras.agentFailureText(outcome.refusal) };
    return { ok: true, outcome };
  } catch {
    return { ok: false, text: getCopy().extras.agentFailureText({ code: "transport" }) };
  }
}
