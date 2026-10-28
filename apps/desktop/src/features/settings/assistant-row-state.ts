import { getCopy } from "../../i18n";
import type { Agent } from "../../bridge/generated/Agent";
import { healthOf } from "../assistant/assistants";

/** What its row says at the right: what stops it, in amber, or its plan's usage. */
export function rowState(agent: Agent, nowMs: number): { text: string; warn: boolean } | null {
  const health = healthOf(agent, nowMs);
  if (health.kind === "sign_in")
    return {
      text:
        health.reason === "expired"
          ? getCopy().settings.sessionExpired
          : getCopy().extras.assistantsCopy.rowSignIn,
      warn: true,
    };
  if (health.kind === "limit")
    return {
      text:
        health.until === undefined
          ? getCopy().extras.assistantsCopy.rowLimit
          : getCopy().extras.assistantsCopy.rowUntil(health.until),
      warn: true,
    };
  if (agent.usage !== undefined)
    return {
      text: getCopy().settings.usage(
        Math.round(agent.usage.used_percent),
        getCopy().extras.usageWindow(agent.usage.window),
      ),
      warn: false,
    };
  return null;
}
